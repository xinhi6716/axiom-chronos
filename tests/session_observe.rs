mod common;

use axiom::{
    goal,
    session::{run::Launch, wait, Clock, Config},
};
use std::{
    cell::Cell,
    io::{BufRead, Cursor, Write},
    rc::Rc,
    sync::Mutex,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SESSION: &str = "abcdef12-1234-1234-1234-123456789abc";

struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvRestore {
    fn new() -> Self {
        Self(
            [
                "FAKE_CLAUDE_RECORD",
                "FAKE_CLAUDE_SCENARIO",
                "FAKE_CLAUDE_SESSION",
                "FAKE_CLAUDE_BATCHES",
            ]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
        )
    }
}
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}

fn config(temp: &std::path::Path) -> Config {
    Config {
        claude: Ok(common::fake("fake_claude")),
        home: temp.join("home"),
        bin: temp.join("axiom"),
        clock: Clock::system(),
    }
}

#[test]
fn reports_the_init_session_once_and_matches_run_with() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let config = config(temp.path());
    std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "resume");
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();

    let mut sessions = Vec::new();
    let mut observed_out = Vec::new();
    let observed = wait::run_observing(
        &config,
        "wish",
        "wish",
        temp.path(),
        Some(9),
        empty(),
        &mut observed_out,
        &Launch::default(),
        &mut |id| sessions.push(id.to_owned()),
    )
    .unwrap();
    goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();
    let mut normal_out = Vec::new();
    let normal = wait::run_with(
        &config,
        "wish",
        "wish",
        temp.path(),
        Some(9),
        empty(),
        &mut normal_out,
        &Launch::default(),
    )
    .unwrap();
    assert_eq!(sessions, [SESSION]);
    assert_eq!(observed, normal);
    // Each run creates a distinct log file, so its path is expected to differ.
    let comparable = |output: Vec<u8>| {
        String::from_utf8(output)
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with("log: "))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(comparable(observed_out), comparable(normal_out));
}

#[test]
fn no_init_does_not_report_a_session() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let mut config = config(temp.path());
    let script = temp.path().join("no_init.py");
    std::fs::write(&script, "import sys\nsys.stdin.readline()\nsys.exit(3)\n").unwrap();
    config.claude = Ok(vec![
        common::python(),
        "-I".into(),
        script.to_string_lossy().into_owned(),
    ]);
    let mut calls = 0;
    let code = wait::run_observing(
        &config,
        "wish",
        "wish",
        temp.path(),
        None,
        empty(),
        &mut Vec::new(),
        &Launch::default(),
        &mut |_| calls += 1,
    )
    .unwrap();
    assert_eq!(calls, 0);
    assert_eq!(code, 3);
}

#[test]
fn reports_before_follow_and_only_once_across_resume() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let config = config(temp.path());
    let marker = temp.path().join("done");
    std::fs::write(&marker, "").unwrap();
    let batch = goal::Batch {
        progress_file: temp.path().join("progress").to_string_lossy().into_owned(),
        marker_file: marker.to_string_lossy().into_owned(),
        pid: std::process::id(),
    };
    std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "batch");
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    std::env::set_var(
        "FAKE_CLAUDE_BATCHES",
        serde_json::to_string(&[batch]).unwrap(),
    );
    goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();

    struct CheckFollow {
        reported: Rc<Cell<bool>>,
        output: Vec<u8>,
    }
    impl Write for CheckFollow {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if String::from_utf8_lossy(bytes).contains("waiting on batch") {
                assert!(self.reported.get(), "follow started before callback");
            }
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let reported = Rc::new(Cell::new(false));
    let mut out = CheckFollow {
        reported: Rc::clone(&reported),
        output: Vec::new(),
    };
    let mut sessions = Vec::new();
    let code = wait::run_observing(
        &config,
        "wish",
        "wish",
        temp.path(),
        None,
        empty(),
        &mut out,
        &Launch::default(),
        &mut |id| {
            reported.set(true);
            sessions.push(id.to_owned());
        },
    )
    .unwrap();
    assert_eq!(code, 0);
    assert_eq!(sessions, [SESSION]);
    assert!(String::from_utf8(out.output)
        .unwrap()
        .contains("batch finished"));
    assert!(temp.path().join("record.1").exists());
}
