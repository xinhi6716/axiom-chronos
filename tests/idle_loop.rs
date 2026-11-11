#[path = "common/mod.rs"]
mod common;

use axiom::{goal, hook, orch, session};
use serde_json::{json, Value};
use std::{fs, io::Cursor, path::Path, sync::Mutex};

const SESSION: &str = "12345678-1234-1234-1234-123456789abc";
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn stop(home: &Path, session: &str) -> Option<Value> {
    hook::handle(
        &json!({"hook_event_name":"Stop", "session_id":session}),
        Some(home),
        true,
    )
}

#[test]
fn stop_allows_an_unchanged_goal_and_resets_on_each_progress_source() {
    let dir = common::temp();
    let home = dir.path();
    goal::open(home, SESSION, "wish", Some("demo")).unwrap();
    let store = orch::store_for(home, "demo").unwrap();
    fs::create_dir_all(store.join("handoffs/module")).unwrap();
    assert_eq!(stop(home, SESSION).unwrap()["decision"], "block");
    assert!(
        stop(home, SESSION).is_none(),
        "the same Stop must not block twice"
    );
    for file in [
        "units.tsv",
        "ledger.tsv",
        "claims.tsv",
        "handoffs/wish.json",
        "handoffs/module/m1.json",
    ] {
        fs::write(store.join(file), "changed").unwrap();
        assert_eq!(stop(home, SESSION).unwrap()["decision"], "block", "{file}");
        fs::write(store.join(file), "changed").unwrap();
        assert!(
            stop(home, SESSION).is_none(),
            "rewriting {file} is not progress"
        );
    }
    goal::mark_dispatched(home, SESSION).unwrap();
    assert!(
        stop(home, SESSION).is_some(),
        "goal changes are progress too"
    );
    assert!(stop(home, SESSION).is_none());
    goal::open(home, "other", "wish", Some("demo")).unwrap();
    assert!(stop(home, "other").is_some(), "state is per session");
    goal::reopen(home, SESSION).unwrap();
    assert!(
        stop(home, SESSION).is_some(),
        "explicit resume gets a fresh guard"
    );
}

struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Drop for Restore {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn drive_idle(limit: Option<&str>, progress: bool, refusal: bool) -> (String, Value, goal::Goal) {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let keys = [
        "AXIOM_IDLE_CONTINUES",
        "FAKE_CLAUDE_RECORD",
        "FAKE_CLAUDE_SCENARIO",
        "FAKE_CLAUDE_SESSION",
    ];
    let _restore = Restore(
        keys.into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
    );
    let dir = common::temp();
    let home = dir.path().join("home");
    let record = dir.path().join("record.json");
    match limit {
        Some(limit) => std::env::set_var("AXIOM_IDLE_CONTINUES", limit),
        None => std::env::remove_var("AXIOM_IDLE_CONTINUES"),
    }
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    std::env::set_var(
        "FAKE_CLAUDE_SCENARIO",
        if progress {
            "idle-progress"
        } else if refusal {
            "idle-refusal"
        } else {
            "idle"
        },
    );
    goal::open(&home, SESSION, "wish", Some("demo")).unwrap();
    let store = orch::store_for(&home, "demo").unwrap();
    assert_eq!(
        orch::run(&store, &["init".into()], false, false).exit_code,
        0
    );
    orch::record_handoff(
        &store,
        &["handoff".into(), "--next".into(), "check acceptance".into()],
    )
    .unwrap();
    let config = session::Config {
        home: home.clone(),
        bin: dir.path().join("axiom"),
        claude: Ok(common::fake("fake_claude")),
        clock: session::Clock::system(),
    };
    let mut out = Vec::new();
    let code = session::run(
        &config,
        "wish",
        "wish",
        dir.path(),
        None,
        Box::new(Cursor::new(Vec::<u8>::new())),
        &mut out,
    )
    .unwrap();
    assert_eq!(code, 0, "same exit as an ordinary blocked result");
    let record = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    (
        String::from_utf8(out).unwrap(),
        record,
        goal::load(&home, SESSION).unwrap().unwrap(),
    )
}

fn check_idle(limit: Option<&str>, progress: bool, expected: usize) {
    let (out, record, goal) = drive_idle(limit, progress, false);
    assert_eq!(
        record["messages"].as_array().unwrap().len() - 1,
        expected,
        "{out}"
    );
    assert_eq!(goal.status, "blocked");
    let note = goal.note.unwrap();
    let n = limit
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(3);
    assert!(
        note.contains(&format!("no progress for {n} continues")),
        "{note}"
    );
    assert!(
        note.contains("handoffs") && note.contains("wish.json"),
        "{note}"
    );
    assert!(out.contains(&format!("goal blocked: {note}")), "{out}");
    for message in record["messages"].as_array().unwrap().iter().skip(1) {
        let text = message["message"]["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with(session::CONTINUE_TAG));
        assert!(
            text.contains("wish.json") && text.contains("chronos:dispatcher"),
            "{text}"
        );
    }
}

#[test]
fn driver_stops_after_three_idle_continues_by_default() {
    check_idle(None, false, 3);
}

#[test]
fn driver_honors_idle_override() {
    check_idle(Some("2"), false, 2);
}

#[test]
fn driver_progress_resets_the_idle_count() {
    check_idle(None, true, 5);
}

#[test]
fn driver_invalid_idle_override_uses_default() {
    for value in ["bogus", "0"] {
        check_idle(Some(value), false, 3);
    }
}

#[test]
fn driver_quotes_the_last_refusal_with_a_unicode_safe_limit() {
    let (out, record, goal) = drive_idle(Some("1"), false, true);
    assert_eq!(record["messages"].as_array().unwrap().len(), 2, "{out}");
    let note = goal.note.unwrap();
    assert!(
        note.contains(&format!(
            "last refusal or stop reason: \"拒絕{}",
            "願".repeat(298)
        )),
        "{note}"
    );
    assert!(
        !note.contains(&"願".repeat(299)),
        "the quote is cut to 300 characters"
    );
}
