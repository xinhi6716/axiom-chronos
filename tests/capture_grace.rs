mod common;

use axiom::base::proc::{self, Captured, ShellRun, HOLDERS_ENDED_NOTE};
use std::{
    fs,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

struct Fixture {
    dir: tempfile::TempDir,
    script: PathBuf,
    pid: PathBuf,
}

fn python() -> String {
    let default = common::python();
    if !cfg!(windows) {
        return default;
    }
    // The Windows Store launcher may create a process outside the shell's job.
    // Prefer an ordinary installation when one is available on PATH.
    for candidate in [default.as_str(), "python3.14.exe", "python3.13.exe", "python3.12.exe", "python3.11.exe", "python3.10.exe", "python3.9.exe", "python3.8.exe"] {
        if let Ok(output) = proc::cmd(candidate)
            .args(["-I", "-c", "import sys; print(sys.executable)"])
            .output()
        {
            let executable = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if output.status.success() && !executable.to_ascii_lowercase().contains("windowsapps") {
                return executable;
            }
        }
    }
    default
}

impl Fixture {
    fn new() -> Self {
        let dir = common::temp();
        let script = dir.path().join("main.py");
        let pid = dir.path().join("holder.pid");
        fs::write(&script, r#"import os, subprocess, sys, time
holder = 'import os,sys,time; open(sys.argv[1], "w").write(str(os.getpid())); time.sleep(float(sys.argv[2]))'
subprocess.Popen([sys.executable, '-I', '-c', holder, sys.argv[1], sys.argv[3]], close_fds=False)
while not os.path.exists(sys.argv[1]) or os.path.getsize(sys.argv[1]) == 0:
    time.sleep(0.01)
if sys.argv[4] == 'output':
    sys.stdout.buffer.write(b'out\x00')
    sys.stderr.buffer.write(b'err')
sys.exit(int(sys.argv[2]))
"#).unwrap();
        Self { dir, script, pid }
    }

    fn args(&self, code: i32, sleep: u64, output: bool) -> Vec<String> {
        vec![
            python(),
            "-I".into(),
            self.script.to_string_lossy().into_owned(),
            self.pid.to_string_lossy().into_owned(),
            code.to_string(),
            sleep.to_string(),
            if output { "output" } else { "empty" }.into(),
        ]
    }

    fn capture(&self, code: i32, sleep: u64, timeout: u64) -> Captured {
        let child = proc::command(&self.args(code, sleep, true))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        proc::capture(child, Duration::from_secs(timeout)).unwrap()
    }

    fn shell(&self, code: i32, sleep: u64, timeout: u64, output: bool) -> ShellRun {
        let args = self.args(code, sleep, output);
        let mut command = args
            .iter()
            .map(|s| format!("\"{}\"", s.replace('\\', "/")))
            .collect::<Vec<_>>()
            .join(" ");
        // Let capture assign the shell to its job before the shell creates descendants.
        if cfg!(windows) {
            command = format!("ping -n 2 127.0.0.1 >nul & {command}");
        }
        proc::run_shell(&command, self.dir.path(), Duration::from_secs(timeout)).unwrap()
    }

    fn assert_holder_dead(&self) {
        let pid: u32 = fs::read_to_string(&self.pid).unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while proc::pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !proc::pid_alive(pid),
            "holder {pid} still alive: {}",
            self.pid.display()
        );
    }
}

fn assert_grace_time(start: Instant) {
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_secs(9) && elapsed < Duration::from_secs(30),
        "elapsed: {elapsed:?}"
    );
}

#[test]
fn capture_ends_holders_and_keeps_main_exit_code() {
    for code in [0, 3] {
        let f = Fixture::new();
        let start = Instant::now();
        let run = f.capture(code, 120, 60);
        assert_grace_time(start);
        assert_eq!(run.exit_code, code);
        assert!(!run.timed_out);
        assert!(run.holders_ended);
        assert_eq!(run.stdout, b"out\0");
        assert_eq!(run.stderr, b"err");
        f.assert_holder_dead();
    }
}

#[test]
fn shell_ends_holders_and_notes_it() {
    for code in [0, 3] {
        let f = Fixture::new();
        let start = Instant::now();
        let run = f.shell(code, 120, 60, true);
        assert_grace_time(start);
        assert_eq!(run.exit_code, code);
        assert!(!run.timed_out);
        assert_eq!(
            run.output.replace("\r\n", "\n"),
            format!("outerr\n{HOLDERS_ENDED_NOTE}\n")
        );
        f.assert_holder_dead();
    }
}

#[test]
fn holder_exits_before_grace() {
    let f = Fixture::new();
    let run = f.capture(3, 2, 60);
    assert_eq!(run.exit_code, 3);
    assert!(!run.timed_out);
    assert!(!run.holders_ended);
    let f = Fixture::new();
    let run = f.shell(3, 2, 60, true);
    assert_eq!(run.exit_code, 3);
    assert!(!run.timed_out);
    assert_eq!(run.output.replace("\r\n", "\n"), "outerr");
    assert!(!run.output.contains(HOLDERS_ENDED_NOTE));
}

#[test]
fn deadline_before_grace_is_timeout() {
    let f = Fixture::new();
    let run = f.capture(3, 120, 2);
    assert_eq!(run.exit_code, -1);
    assert!(run.timed_out);
    assert!(!run.holders_ended);
    f.assert_holder_dead();
    let f = Fixture::new();
    let run = f.shell(3, 120, 2, true);
    assert_eq!(run.exit_code, -1);
    assert!(run.timed_out);
    assert!(!run.output.contains(HOLDERS_ENDED_NOTE));
    f.assert_holder_dead();
}

#[test]
fn shell_note_follows_even_empty_output_on_its_own_line() {
    let f = Fixture::new();
    let run = f.shell(0, 120, 60, false);
    assert_eq!(run.exit_code, 0);
    assert!(!run.timed_out);
    assert_eq!(run.output, format!("\n{HOLDERS_ENDED_NOTE}\n"));
    f.assert_holder_dead();
}


