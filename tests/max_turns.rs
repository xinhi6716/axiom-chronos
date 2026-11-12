mod common;

use axiom::{
    goal,
    session::{self, Clock, Config},
};
use serde_json::Value;
use std::{
    path::Path,
    sync::{atomic::AtomicBool, Mutex},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvRestore {
    fn new() -> Self {
        Self(
            [
                "FAKE_CLAUDE_RECORD",
                "FAKE_CLAUDE_SCENARIO",
                "FAKE_CLAUDE_SESSION",
                "FAKE_CLAUDE_TURNS",
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
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn execute(temp: &Path, scenario: &str, max_turns: Option<u32>) -> (i32, String, Vec<Value>) {
    let home = temp.join("home");
    let record = temp.join("record.json");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", scenario);
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    std::env::set_var("FAKE_CLAUDE_TURNS", "9");
    goal::open(&home, SESSION, "test goal", None).unwrap();
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.join("axiom"),
        clock: Clock::system(),
    };
    let spawned = session::spawn(&config, "wish", "test", temp, max_turns, None).unwrap();
    let mut output = Vec::new();
    let mut events = Vec::new();
    let code = session::drive(
        spawned,
        &config,
        &mut output,
        &AtomicBool::new(false),
        &mut |event| events.push(event.clone()),
    )
    .unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    assert!(!record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|arg| arg == "--resume"));
    assert_eq!(
        std::fs::read_dir(home.join("logs")).unwrap().count(),
        1,
        "fake Claude must run exactly once"
    );
    (code, String::from_utf8(output).unwrap(), events)
}

fn assert_cut_off(home: &Path, turns: u32) {
    let goal = goal::load(home, SESSION).unwrap().unwrap();
    assert_eq!(goal.status, "blocked");
    assert_eq!(goal.reason.as_deref(), Some("max_turns"));
    assert_eq!(goal.turn_limit, Some(turns));
}

#[test]
fn max_turns_emits_resumable_stop() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    for (max_turns, expected) in [(None, 9), (Some(4), 4)] {
        let temp = common::temp();
        let (code, output, events) = execute(temp.path(), "max-turns", max_turns);
        assert_eq!(code, session::MAX_TURNS_EXIT);
        assert_eq!(code, 3);
        let cutoffs: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "chronos" && event["subtype"] == "max_turns")
            .collect();
        assert_eq!(cutoffs.len(), 1);
        assert_eq!(cutoffs[0]["session"], SESSION);
        assert_eq!(cutoffs[0]["turn_limit"], expected);
        assert_cut_off(&temp.path().join("home"), expected);
        let line =
            format!("wish cut off at {expected} turns; resume: axiom wish --resume {SESSION}");
        assert_eq!(
            output.lines().filter(|actual| *actual == line).count(),
            1,
            "{output}"
        );
        for absent in [
            "claude reported an error",
            "usage limit",
            "goal blocked:",
            "\nresume:",
        ] {
            assert!(!output.contains(absent), "{output}");
        }
    }
    let temp = common::temp();
    let (code, _, events) = execute(temp.path(), "error", None);
    assert_ne!(code, session::MAX_TURNS_EXIT);
    assert!(!events
        .iter()
        .any(|event| event["type"] == "chronos" && event["subtype"] == "max_turns"));
}

#[test]
fn max_turns_goal_can_be_reopened() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let (code, _, _) = execute(temp.path(), "max-turns", None);
    assert_eq!(code, session::MAX_TURNS_EXIT);
    let home = temp.path().join("home");
    let reopened = goal::reopen(&home, SESSION).unwrap();
    assert_eq!(reopened.status, "active");
    assert_eq!(reopened.turn_limit, None);
    assert_eq!(reopened.reason, None);
    assert!(goal::cut_off(&home, SESSION, 4).unwrap().is_some());
    assert_cut_off(&home, 4);
}
