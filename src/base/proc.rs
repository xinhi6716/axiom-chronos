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
        let result = Command::new("taskkill")
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
pub struct Captured {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}
pub fn capture(mut child: Child, timeout: Duration) -> Res<Captured> {
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
    let (exit_code, timed_out) = loop {
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
    Ok(Captured {
        exit_code,
        stdout: a.join().unwrap_or_default(),
        stderr: b.join().unwrap_or_default(),
        timed_out,
    })
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn taskkill_failure_includes_details() {
        let error = super::taskkill_failure(123, Some(7), b"stdout", b"stderr");
        assert_eq!(error.kind, "kill_failed");
        for detail in ["123", "7", "stdout", "stderr"] {
            assert!(error.message.contains(detail), "{}", error.message);
        }
    }
}
