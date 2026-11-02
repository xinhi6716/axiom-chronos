mod common;
use axiom::base::proc;
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

fn tree(orphan: bool) -> (tempfile::TempDir, std::process::Child, Vec<u32>) {
    let dir = common::temp();
    let file = dir.path().join("pids");
    let script = "import os,subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); f=sys.argv[1]; t=f+'.tmp'; open(t,'w').write(f'{os.getpid()}\\n{p.pid}\\nready\\n'); os.replace(t,f); time.sleep(0 if sys.argv[2]=='orphan' else 60)";
    let args = vec![
        common::python(),
        "-I".into(),
        "-c".into(),
        script.into(),
        file.to_string_lossy().into_owned(),
        if orphan { "orphan" } else { "live" }.into(),
    ];
    let child = proc::command(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ids = loop {
        if let Ok(s) = std::fs::read_to_string(&file) {
            if s.lines().last() == Some("ready") {
                break s.lines().take(2).map(|s| s.parse().unwrap()).collect();
            }
        }
        assert!(Instant::now() < deadline, "tree did not signal readiness");
        std::thread::sleep(Duration::from_millis(20));
    };
    (dir, child, ids)
}

fn cleanup(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
        };
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !handle.is_null() {
            TerminateProcess(handle, 1);
            CloseHandle(handle);
        }
    }
}

#[test]
fn terminate_live_tree() {
    let (_dir, mut child, ids) = tree(false);
    struct Cleanup<'a>(&'a [u32]);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            for &pid in self.0 {
                if proc::pid_alive(pid) {
                    cleanup(pid);
                }
            }
        }
    }
    let _cleanup = Cleanup(&ids);
    let result = proc::terminate_tree(&mut child);
    let deadline = Instant::now() + Duration::from_secs(5);
    let (exited, states) = loop {
        let exited = child.try_wait().unwrap().is_some();
        let states: Vec<_> = ids.iter().map(|&pid| (pid, proc::pid_alive(pid))).collect();
        if (exited && states.iter().all(|&(_, alive)| !alive)) || Instant::now() >= deadline {
            break (exited, states);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        result.is_ok() && exited && states.iter().all(|&(_, alive)| !alive),
        "termination result: {result:?}; root exited: {exited}; process states (pid, pid_alive): {states:?}"
    );
}

#[test]
fn terminate_exited_root() {
    let (_dir, mut child, ids) = tree(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let exited = child.try_wait().unwrap().is_some();
    let start = Instant::now();
    let result = proc::terminate_tree(&mut child);
    let elapsed = start.elapsed();
    let survivors: Vec<_> = ids[1..]
        .iter()
        .copied()
        .filter(|&pid| proc::pid_alive(pid))
        .collect();
    if !exited {
        cleanup(ids[0]);
    }
    for &pid in &survivors {
        cleanup(pid);
    }
    assert!(exited, "root did not exit; surviving pids: {survivors:?}");
    assert!(
        result.is_ok(),
        "{:?}; surviving pids: {survivors:?}",
        result.err()
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "termination hung; surviving pids: {survivors:?}"
    );
}

#[cfg(unix)]
#[test]
fn termination_failure_is_reported() {
    let mut child = std::process::Command::new(common::python())
        .args(["-c", "import time; time.sleep(60)"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let result = proc::terminate_tree(&mut child);
    let _ = child.kill();
    let _ = child.wait();
    let error = result.unwrap_err();
    assert_eq!(error.kind, "kill_failed");
    assert!(error.message.contains(&pid.to_string()));
    assert!(error.message.contains("errno"));
}
