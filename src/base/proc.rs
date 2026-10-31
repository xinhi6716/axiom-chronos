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
    fn group_empty(&self) -> bool {
        #[cfg(windows)]
        { self.job.empty() }
        #[cfg(unix)]
        {
            // ESRCH is the only positive observation. EPERM and zombies remain
            // unconfirmed; processes which leave the group are outside its scope.
            let result = unsafe { libc::kill(-(self.child.id() as i32), 0) };
            result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        }
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
    static DEADLINE: RefCell<Option<u64>> = const { RefCell::new(None) };
}
/// Absolute operation budget, including setup and every synchronous Rust phase.
/// Persist the timestamp, not a duration: restarting cannot renew this budget.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_millis() as u64
}
pub fn remaining(timeout: Duration) -> Duration {
    DEADLINE.with(|d| d.borrow().map_or(timeout, |end|
        timeout.min(Duration::from_millis(end.saturating_sub(unix_ms())))))
}
pub fn deadline_scope<T>(end: u64, body: impl FnOnce() -> T) -> T {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) { DEADLINE.with(|d| *d.borrow_mut() = self.0); }
    }
    let end = DEADLINE.with(|d| d.borrow().map_or(end, |old| old.min(end)));
    let _restore = Restore(DEADLINE.with(|d| d.replace(Some(end))));
    body()
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
    pub fn cancelled(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).cancelled
    }
}
pub fn current_cancellation() -> Option<Cancellation> {
    CANCELLATION.with(|slot| slot.borrow().clone())
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
    pub evidence: TerminalEvidence,
}
/// Observations, independent of the command's success, timeout or cancellation.
/// Group emptiness covers the assigned Windows job or Unix process group, not
/// descendants that deliberately escape it. False means unknown, not alive.
#[derive(Debug, Clone, Default)]
pub struct TerminalEvidence {
    pub launched: bool,
    pub root_reaped: bool,
    pub root_exit_code: Option<i32>,
    pub stdout_eof: bool,
    pub stderr_eof: bool,
    pub group_empty: bool,
    pub cancelled: bool,
}
pub fn run_shell(cmd: &str, cwd: &Path, timeout: Duration) -> Res<ShellRun> {
    run_shell_with_env(cmd,cwd,timeout,&[])
}
pub fn run_shell_with_env(cmd: &str, cwd: &Path, timeout: Duration, env: &[(&str,&std::ffi::OsStr)]) -> Res<ShellRun> {
    #[cfg(unix)]
    let args = vec!["sh".into(), "-c".into(), cmd.into()];
    #[cfg(windows)]
    let args = vec!["cmd".into(), "/D".into(), "/S".into(), "/C".into()];
    let mut shell = command(&args);
    shell.envs(env.iter().copied());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        shell.raw_arg(format!("\"{cmd}\""));
    }
    run_command(shell, cwd, timeout)
}
/// Bounded, cancellation-aware runner shared by verification and Host tools.
pub fn run_command(mut shell: Command, cwd: &Path, timeout: Duration) -> Res<ShellRun> {
    let observed_start = Instant::now();
    let timeout = remaining(timeout);
    if timeout.is_zero() {
        return Err(Failure::new("timeout", "operation deadline exhausted before command launch"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        shell.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
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
            evidence: TerminalEvidence { cancelled: true, ..Default::default() },
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
    let tail = Arc::new(Mutex::new(Vec::new()));
    let reader = |mut r: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        let tail = tail.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                let n = match r.read(&mut buf) {
                    Ok(0) => { let _ = tx.send(true); return; }
                    Ok(n) => n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => { let _ = tx.send(false); return; }
                };
                let mut tail = tail.lock().unwrap_or_else(|e| e.into_inner());
                tail.extend_from_slice(&buf[..n]);
                let excess = tail.len().saturating_sub(16384);
                tail.drain(..excess);
            }
        });
        rx
    };
    let a = reader(Box::new(out));
    let b = reader(Box::new(err));
    let start = observed_start;
    let mut code = -1;
    let mut observed_code = None;
    let timed_out;
    let mut exited = false;
    let mut out_done = false;
    let mut err_done = false;
    let mut out_eof = false;
    let mut err_eof = false;
    loop {
        if !out_done {
            if let Ok(eof) = a.try_recv() {
                out_done = true;
                out_eof = eof;
            }
        }
        if !err_done {
            if let Ok(eof) = b.try_recv() {
                err_done = true;
                err_eof = eof;
            }
        }
        if !exited {
            let mut process = child.lock().unwrap_or_else(|e| e.into_inner());
            match process.child.try_wait() {
                Ok(Some(s)) => {
                    code = s.code().unwrap_or(-1);
                    observed_code = s.code();
                    exited = true;
                }
                Ok(None) => {}
                Err(e) => {
                    process.kill();
                    return Err(Failure::new("process", e.to_string()));
                }
            }
        }
        if exited && out_done && err_done {
            // A successful exit first observed after the budget is terminal
            // evidence, not success of the expired command.
            timed_out = start.elapsed() >= timeout;
            break;
        }
        if cancellation.as_ref().is_some_and(Cancellation::cancelled) || start.elapsed() >= timeout
        {
            timed_out = !cancellation.as_ref().is_some_and(Cancellation::cancelled);
            child.lock().unwrap_or_else(|e| e.into_inner()).kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Cleanup may have reaped the root. Check again instead of inferring from
    // kill's return; bounded observation can still leave the group unconfirmed.
    let cleanup_observation = Instant::now();
    let mut group_empty;
    loop {
        {
            let mut process = child.lock().unwrap_or_else(|e| e.into_inner());
            if !exited {
                if let Ok(Some(status)) = process.child.try_wait() {
                    exited = true;
                    observed_code = status.code();
                }
            }
            group_empty = process.group_empty();
        }
        if !out_done { if let Ok(eof) = a.try_recv() { out_done = true; out_eof = eof; } }
        if !err_done { if let Ok(eof) = b.try_recv() { err_done = true; err_eof = eof; } }
        if (exited && group_empty && out_done && err_done)
            || cleanup_observation.elapsed() >= Duration::from_millis(500) { break; }
        std::thread::sleep(Duration::from_millis(10));
    }
    let bytes = tail.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut output = String::from_utf8_lossy(&bytes).replace('\0', "");
    let cancelled = cancellation.as_ref().is_some_and(Cancellation::cancelled);
    if cancelled {
        output.push_str("\nverification cancelled");
    }
    Ok(ShellRun {
        exit_code: if cancelled || timed_out { -1 } else { code },
        output,
        timed_out,
        evidence: TerminalEvidence {
            launched: true, root_reaped: exited, root_exit_code: observed_code,
            stdout_eof: out_eof, stderr_eof: err_eof, group_empty, cancelled,
        },
    })
}
