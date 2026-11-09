#[path = "common/race.rs"]
mod race;

use axiom::{base::Res, goal, hook};
use race::{child, children};
use serde_json::json;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Barrier},
};

/// Runs every operation at the same instant on its own thread and returns their results.
fn at_once<T: Send + 'static>(
    home: &Path,
    ops: Vec<Box<dyn FnOnce(&Path) -> T + Send>>,
) -> Vec<T> {
    let barrier = Arc::new(Barrier::new(ops.len()));
    let threads: Vec<_> = ops
        .into_iter()
        .map(|op| {
            let barrier = barrier.clone();
            let home = home.to_path_buf();
            std::thread::spawn(move || {
                barrier.wait();
                op(&home)
            })
        })
        .collect();
    threads.into_iter().map(|t| t.join().unwrap()).collect()
}
fn loaded(home: &Path) -> goal::Goal {
    goal::load(home, "s").unwrap().unwrap()
}

#[test]
fn interjections_from_many_processes_are_all_recorded() {
    const WRITERS: usize = 8;
    const EACH: usize = 20;
    if let Some((dir, i)) = child() {
        for n in 0..EACH {
            goal::interject(&dir, "s", &format!("message-{i}-{n}")).unwrap();
        }
        return;
    }
    let t = tempfile::tempdir().unwrap();
    goal::open(t.path(), "s", "wish", None).unwrap();
    children(
        "interjections_from_many_processes_are_all_recorded",
        t.path(),
        WRITERS,
    );
    let g = loaded(t.path());
    assert_eq!(g.amendments.len(), WRITERS * EACH);
    assert!(g.interjection.is_some());
    assert!(!t.path().join("goals/.orch.lock").exists());
}

#[test]
fn hook_processes_record_every_prompt() {
    const HOOKS: usize = 16;
    let t = tempfile::tempdir().unwrap();
    goal::open(t.path(), "s", "wish", None).unwrap();
    let running: Vec<_> = (0..HOOKS)
        .map(|i| {
            let mut child = Command::new(env!("CARGO_BIN_EXE_axiom"))
                .arg("hook")
                .env("AXIOM_DATA_HOME", t.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let event = json!({"hook_event_name":"UserPromptSubmit","session_id":"s","prompt":format!("message {i}")});
            child
                .stdin
                .take()
                .unwrap()
                .write_all(event.to_string().as_bytes())
                .unwrap();
            child
        })
        .collect();
    for child in running {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    assert_eq!(loaded(t.path()).amendments.len(), HOOKS);
}

#[test]
fn a_closing_goal_is_not_overwritten_by_racing_updates() {
    for _ in 0..150 {
        let t = tempfile::tempdir().unwrap();
        goal::open(t.path(), "s", "wish", None).unwrap();
        goal::interject(t.path(), "s", "first").unwrap();
        let results = at_once(
            t.path(),
            vec![
                Box::new(|h: &Path| goal::set_status(h, "s", "done", None).map(|_| ())),
                Box::new(|h: &Path| goal::mark_dispatched(h, "s")),
                Box::new(|h: &Path| goal::interrupt(h, "s", "crash").map(|_| ())),
                Box::new(|h: &Path| match goal::classify(h, "s", "amend") {
                    Err(e) if e.kind == "invalid_argument" => Ok(()),
                    other => other.map(|_| ()),
                }),
            ],
        );
        for result in results {
            result.unwrap();
        }
        let g = loaded(t.path());
        assert_eq!(g.status, "done", "the closing update was overwritten");
        assert!(g.dispatched, "the racing flag was lost");
    }
}

#[test]
fn the_mode_is_declared_only_once_under_contention() {
    for _ in 0..80 {
        let t = tempfile::tempdir().unwrap();
        goal::open(t.path(), "s", "wish", None).unwrap();
        let results = at_once(
            t.path(),
            ["change", "review", "answer", "change"]
                .into_iter()
                .map(|mode| {
                    Box::new(move |h: &Path| goal::set_mode(h, "s", mode).map(|g| g.mode))
                        as Box<dyn FnOnce(&Path) -> Res<Option<String>> + Send>
                })
                .collect(),
        );
        let winners: Vec<_> = results.iter().flatten().collect();
        assert_eq!(winners.len(), 1, "{results:?}");
        assert_eq!(&&loaded(t.path()).mode, winners.first().unwrap());
        assert!(results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|e| e.kind == "mode_locked"));
    }
}

#[test]
fn one_round_of_questions_survives_parallel_tool_calls() {
    let ask = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"AskUserQuestion","tool_input":{"questions":[{}]}});
    for _ in 0..100 {
        let t = tempfile::tempdir().unwrap();
        goal::open(t.path(), "s", "wish", None).unwrap();
        let results = at_once(
            t.path(),
            (0..4)
                .map(|_| {
                    let ask = ask.clone();
                    Box::new(move |h: &Path| hook::handle(&ask, Some(h), false).is_none())
                        as Box<dyn FnOnce(&Path) -> bool + Send>
                })
                .collect(),
        );
        assert_eq!(results.iter().filter(|allowed| **allowed).count(), 1);
    }
}

#[test]
fn a_done_goal_cannot_be_moved_to_blocked_or_waiting() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    goal::open(h, "s", "wish", None).unwrap();
    goal::set_status(h, "s", "done", Some("finished")).unwrap();
    let before = std::fs::read(h.join("goals/s.json")).unwrap();
    for status in ["blocked", "waiting"] {
        assert_eq!(
            goal::set_status(h, "s", status, Some("note"))
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
    }
    assert_eq!(std::fs::read(h.join("goals/s.json")).unwrap(), before);
    goal::interject(h, "s", "one more thing").unwrap();
    goal::set_status(h, "s", "blocked", Some("needs a key")).unwrap();
    assert_eq!(loaded(h).status, "blocked");
}

#[test]
fn ordinary_sessions_get_no_lock_or_goal_directory() {
    let t = tempfile::tempdir().unwrap();
    assert!(goal::interject(t.path(), "s", "hello").unwrap().is_none());
    assert!(!t.path().join("goals").exists());
}
