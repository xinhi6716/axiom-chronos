#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

const DIR: &str = "AXIOM_RACE_DIR";
const INDEX: &str = "AXIOM_RACE_INDEX";

/// In a process started by `children`: its directory and index, once every sibling is up.
pub fn child() -> Option<(PathBuf, usize)> {
    let dir = PathBuf::from(std::env::var_os(DIR)?);
    let index = std::env::var(INDEX).unwrap().parse().unwrap();
    while !dir.join("go").exists() {
        std::thread::yield_now();
    }
    Some((dir, index))
}

/// Runs `test` of this test binary in `n` processes at once; each must exit cleanly.
pub fn children(test: &str, dir: &Path, n: usize) {
    let running: Vec<Child> = (0..n)
        .map(|i| {
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture"])
                .env(DIR, dir)
                .env(INDEX, i.to_string())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    fs::write(dir.join("go"), "").unwrap();
    let failures: Vec<_> = running
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .filter(|o| !o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {n} processes failed; first:\n{}",
        failures.len(),
        failures[0]
    );
    fs::remove_file(dir.join("go")).unwrap();
}

/// A live process whose pid can stand for a lock holder; it exits when this drops.
pub struct LiveProcess(Child);
impl LiveProcess {
    pub fn start() -> Self {
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "more"]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", "cat"]);
            c
        };
        Self(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    pub fn pid(&self) -> u32 {
        self.0.id()
    }
}
impl Drop for LiveProcess {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}
