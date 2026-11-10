use super::{Failure, Res};
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
/// The one place a child process is created; applies CREATE_NO_WINDOW when this process has no console.
pub fn cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    if no_window(has_console()) {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
fn has_console() -> bool {
    !unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null()
}
#[cfg_attr(not(windows), allow(dead_code))]
fn no_window(has_console: bool) -> bool {
    !has_console
}
pub fn command(argv: &[String]) -> Command {
    let mut c = cmd(argv.first().map(String::as_str).unwrap_or(""));
    c.args(argv.iter().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    c
}
pub fn terminate_tree(child: &mut Child) -> Result<(), Failure> {
    let pid = child.id();
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(e) => return Err(Failure::new("kill_failed", format!("pid {pid}: {e}"))),
    }
    #[cfg(unix)]
    {
        if unsafe { libc::kill(-(pid as i32), libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if matches!(child.try_wait(), Ok(Some(_))) || !pid_alive(pid) {
                return Ok(());
            }
            return Err(Failure::new(
                "kill_failed",
                format!("pid {pid}: {error} (errno {:?})", error.raw_os_error()),
            ));
        }
    }
    #[cfg(windows)]
    {
        let result = cmd("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .output();
        match result {
            Ok(output) if !output.status.success() && pid_alive(pid) => {
                return Err(taskkill_failure(
                    pid,
                    output.status.code(),
                    &output.stdout,
                    &output.stderr,
                ));
            }
            Err(error) if pid_alive(pid) => {
                return Err(Failure::new(
                    "kill_failed",
                    format!("pid {pid}: taskkill failed to start: {error}"),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
#[cfg(windows)]
fn taskkill_failure(pid: u32, code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Failure {
    let mut output = stdout.to_vec();
    output.extend_from_slice(stderr);
    Failure::new(
        "kill_failed",
        format!(
            "pid {pid}: taskkill exit code {code:?}: {}",
            String::from_utf8_lossy(&output).trim()
        ),
    )
}
pub fn kill_tree(child: &mut Child) {
    if let Err(error) = terminate_tree(child) {
        eprintln!("axiom: {}", error.message);
    }
    let _ = child.kill();
    reap(child);
}
fn reap(child: &mut Child) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
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
            return std::io::Error::last_os_error().raw_os_error() == Some(5);
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
    let child = shell
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    let run = capture(child, timeout)?;
    let mut bytes = run.stdout;
    bytes.extend(run.stderr);
    let output = String::from_utf8_lossy(&bytes).replace('\0', "");
    Ok(ShellRun {
        exit_code: run.exit_code,
        output,
        timed_out: run.timed_out,
    })
}
/// The most of each stream `capture` keeps: its tail, where results and failures are printed.
pub const OUTPUT_CAP: usize = 8 << 20;
/// How long the streams may take to close once everything holding them has been killed.
const DRAIN_GRACE: Duration = Duration::from_secs(5);
pub struct Captured {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

/// Reads one stream to its end on its own thread, keeping at most the last `OUTPUT_CAP` bytes.
struct Drain {
    kept: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}
impl Drain {
    fn start(mut stream: impl Read + Send + 'static) -> Self {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let shared = Arc::clone(&kept);
        std::thread::spawn(move || {
            let mut chunk = vec![0; 64 << 10];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut kept = shared.lock().unwrap_or_else(|e| e.into_inner());
                        kept.extend_from_slice(&chunk[..n]);
                        if kept.len() > 2 * OUTPUT_CAP {
                            let excess = kept.len() - OUTPUT_CAP;
                            kept.drain(..excess);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = tx.send(());
        });
        Self { kept, done }
    }
    /// True once the stream has ended.
    fn ended_by(&self, deadline: Instant) -> bool {
        let wait = deadline.saturating_duration_since(Instant::now());
        !matches!(self.done.recv_timeout(wait), Err(mpsc::RecvTimeoutError::Timeout))
    }
    fn take(self) -> Vec<u8> {
        let mut kept = std::mem::take(&mut *self.kept.lock().unwrap_or_else(|e| e.into_inner()));
        if kept.len() > OUTPUT_CAP {
            kept.drain(..kept.len() - OUTPUT_CAP);
        }
        kept
    }
}

/// Everything a child started, including what outlives it: its process group on Unix (`command`
/// makes one), a job object on Windows. A process started in the moment before the child joins
/// its job, or one that leaves its group or job, is out of reach.
struct Group {
    #[cfg(windows)]
    job: Option<Job>,
}
impl Group {
    fn of(_child: &Child) -> Self {
        Self {
            #[cfg(windows)]
            job: Job::holding(_child),
        }
    }
    fn kill(&self, child: &mut Child) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        if self.job.as_ref().is_some_and(Job::terminate) {
            reap(child);
            return;
        }
        kill_tree(child);
    }
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Job {
    fn holding(child: &Child) -> Option<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return None;
        }
        let job = Self(handle);
        (unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } != 0).then_some(job)
    }
    fn terminate(&self) -> bool {
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) != 0 }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Runs `child`, which `command` built with piped stdout and stderr, to its end or `timeout`.
/// The deadline covers the streams too: a process the child leaves behind can hold them open, and
/// at the deadline everything the child started is killed. A run that has not closed its
/// streams by then is `timed_out`, with exit code -1 and whatever output arrived.
pub fn capture(mut child: Child, timeout: Duration) -> Res<Captured> {
    let group = Group::of(&child);
    let out = Drain::start(child.stdout.take().unwrap());
    let err = Drain::start(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let exited = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s.code().unwrap_or(-1)),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => break None,
            Err(e) => {
                group.kill(&mut child);
                return Err(Failure::new("process", e.to_string()));
            }
        }
    };
    let finished = exited.filter(|_| out.ended_by(deadline) && err.ended_by(deadline));
    if finished.is_none() {
        group.kill(&mut child);
        let grace = Instant::now() + DRAIN_GRACE;
        // Only a holder out of the group's reach can keep a drain thread past this.
        let _ = out.ended_by(grace) && err.ended_by(grace);
    }
    Ok(Captured {
        exit_code: finished.unwrap_or(-1),
        stdout: out.take(),
        stderr: err.take(),
        timed_out: finished.is_none(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_is_hidden_only_without_a_console() {
        assert!(super::no_window(false));
        assert!(!super::no_window(true));
    }
    #[test]
    fn no_bare_command_new_outside_the_helper() {
        fn walk(dir: &std::path::Path, bad: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, bad);
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("base/proc.rs") {
                    if std::fs::read_to_string(&p).unwrap().contains("Command::new(") {
                        bad.push(p.display().to_string());
                    }
                }
            }
        }
        let mut bad = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut bad);
        assert!(bad.is_empty(), "use base::proc::cmd instead of Command::new in: {bad:?}");
    }
    #[cfg(windows)]
    #[test]
    fn taskkill_failure_includes_details() {
        let error = super::taskkill_failure(123, Some(7), b"stdout", b"stderr");
        assert_eq!(error.kind, "kill_failed");
        for detail in ["123", "7", "stdout", "stderr"] {
            assert!(error.message.contains(detail), "{}", error.message);
        }
    }
}
