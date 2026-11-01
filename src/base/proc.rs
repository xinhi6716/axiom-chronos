use super::{Failure, Res};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
pub fn command(argv: &[String]) -> Command {
    let mut c = Command::new(argv.first().map(String::as_str).unwrap_or(""));
    c.args(argv.iter().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    c
}
pub fn kill_tree(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        kill_windows_descendants(child.id());
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(windows)]
fn kill_windows_descendants(root: u32) {
    use std::ffi::c_void;
    #[repr(C)]
    struct Entry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        exe: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut Entry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut Entry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(2, 0);
        if snapshot as isize == -1 {
            return;
        }
        let mut entry: Entry = std::mem::zeroed();
        entry.size = std::mem::size_of::<Entry>() as u32;
        let mut edges = Vec::new();
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            edges.push((entry.parent, entry.pid));
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut descendants = vec![root];
        let mut i = 0;
        while i < descendants.len() {
            let parent = descendants[i];
            for &(_, pid) in edges.iter().filter(|(p, _)| *p == parent) {
                if !descendants.contains(&pid) {
                    descendants.push(pid);
                }
            }
            i += 1;
        }
        for pid in descendants.into_iter().skip(1).rev() {
            let process = OpenProcess(1, 0, pid);
            if !process.is_null() {
                TerminateProcess(process, 1);
                CloseHandle(process);
            }
        }
    }
}
pub fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let r = unsafe { libc::kill(pid as i32, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}
pub struct ShellRun {
    pub exit_code: i32,
    pub output: String,
    pub timed_out: bool,
}
pub fn run_shell(cmd: &str, cwd: &Path, timeout: Duration) -> Res<ShellRun> {
    #[cfg(unix)]
    let args = vec!["sh".into(), "-c".into(), cmd.into()];
    #[cfg(windows)]
    let args = vec!["cmd".into(), "/D".into(), "/S".into(), "/C".into()];
    let mut shell = command(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        shell.raw_arg(format!("\"{cmd}\""));
    }
    let mut child = shell
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let reader = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.read_to_end(&mut b);
            b
        })
    };
    let a = reader(Box::new(out));
    let b = reader(Box::new(err));
    let start = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s.code().unwrap_or(-1), false),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                kill_tree(&mut child);
                break (-1, true);
            }
            Err(e) => {
                kill_tree(&mut child);
                return Err(Failure::new("process", e.to_string()));
            }
        }
    };
    let mut bytes = a.join().unwrap_or_default();
    bytes.extend(b.join().unwrap_or_default());
    let output = String::from_utf8_lossy(&bytes).replace('\0', "");
    Ok(ShellRun {
        exit_code: code,
        output,
        timed_out,
    })
}
