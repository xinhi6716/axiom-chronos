use super::{Failure, Res};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(windows)]
#[path = "job.rs"]
mod job;

struct Process {
    child: Child,
    #[cfg(windows)]
    job: job::Job,
}
impl Process {
    fn kill(&mut self) {
        #[cfg(windows)]
        self.job.terminate();
        kill_tree(&mut self.child);
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<Mutex<Processes>>);
#[derive(Default)]
struct Processes {
    cancelled: bool,
    children: HashMap<u32, Arc<Mutex<Process>>>,
}
thread_local! {
    static CANCELLATION: RefCell<Option<Cancellation>> = const { RefCell::new(None) };
}
impl Cancellation {
    /// Bind synchronous orch calls to their owning Bridge, including post-model VERIFY.
    /// The binding is thread-local, never a process-wide cancellation flag.
    pub fn scope<T>(&self, body: impl FnOnce() -> T) -> T {
        struct Restore(Option<Cancellation>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CANCELLATION.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CANCELLATION.with(|slot| slot.replace(Some(self.clone()))));
        body()
    }
    pub fn cancel(&self) {
        let children = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.cancelled = true;
            state.children.values().cloned().collect::<Vec<_>>()
        };
        for child in children {
            child.lock().unwrap_or_else(|e| e.into_inner()).kill();
        }
    }
    fn cancelled(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).cancelled
    }
}
struct Registration(Option<Cancellation>, u32);
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .children
                .remove(&self.1);
        }
    }
}
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
    // The shell may already have exited while a descendant still owns its pipes.
    // Its process group / descendants must still be terminated in that case.
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let killed = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if !killed.is_ok_and(|status| status.success()) {
            // taskkill's process discovery can be denied in a restricted Windows
            // token even when we can terminate our own children. Use ToolHelp to
            // discover only this process's descendants, then kill leaves first.
            kill_descendants(child.id());
        }
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
fn kill_descendants(root: u32) {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut processes = Vec::new();
        let mut more = Process32FirstW(snapshot, &mut entry);
        while more != 0 {
            processes.push((entry.th32ProcessID, entry.th32ParentProcessID));
            more = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut descendants = vec![root];
        let mut index = 0;
        while index < descendants.len() {
            let parent = descendants[index];
            for &(pid, ppid) in &processes {
                if ppid == parent && !descendants.contains(&pid) {
                    descendants.push(pid);
                }
            }
            index += 1;
        }
        for &pid in descendants[1..].iter().rev() {
            let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
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
        shell.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
    }
    let cancellation = CANCELLATION.with(|slot| slot.borrow().clone());
    // Serialize spawn/registration against cancellation: a cancelled session cannot
    // launch a verifier after its model turn has completed or shutdown has returned.
    let mut state = cancellation
        .as_ref()
        .map(|c| c.0.lock().unwrap_or_else(|e| e.into_inner()));
    if state.as_ref().is_some_and(|s| s.cancelled) {
        return Ok(ShellRun {
            exit_code: -1,
            output: "verification cancelled".into(),
            timed_out: false,
        });
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
    let pid = child.id();
    #[cfg(windows)]
    let job = match job::Job::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            kill_tree(&mut child);
            return Err(e);
        }
    };
    let process = Process {
        child,
        #[cfg(windows)]
        job,
    };
    #[cfg(windows)]
    let process = {
        let mut process = process;
        if let Err(e) = job::resume(&process.child) {
            process.kill();
            return Err(e);
        }
        process
    };
    let child = Arc::new(Mutex::new(process));
    if let Some(state) = &mut state {
        state.children.insert(pid, child.clone());
    }
    drop(state);
    let _registration = Registration(cancellation.clone(), pid);
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
        let mut child = child.lock().unwrap_or_else(|e| e.into_inner());
        if cancellation.as_ref().is_some_and(Cancellation::cancelled) {
            child.kill();
            break (-1, false);
        }
        match child.child.try_wait() {
            Ok(Some(s)) => break (s.code().unwrap_or(-1), false),
            Ok(None) if start.elapsed() < timeout => {
                drop(child);
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                child.kill();
                break (-1, true);
            }
            Err(e) => {
                child.kill();
                return Err(Failure::new("process", e.to_string()));
            }
        }
    };
    let mut bytes = a.join().unwrap_or_default();
    bytes.extend(b.join().unwrap_or_default());
    let mut output = String::from_utf8_lossy(&bytes).replace('\0', "");
    let cancelled = cancellation.as_ref().is_some_and(Cancellation::cancelled);
    if cancelled {
        output.push_str("\nverification cancelled");
    }
    Ok(ShellRun {
        exit_code: if cancelled { -1 } else { code },
        output,
        timed_out,
    })
}
