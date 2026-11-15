mod common;

use axiom::{goal, hook, orch};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(home: &Path, project: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(args)
        .current_dir(project)
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env_remove("ORCH_STORE")
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake_codex(&[&common::tested_codex_flag()])).unwrap(),
        )
        .output()
        .unwrap()
}

fn output(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn doctor_is_read_only_and_reports_refreshed_index() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.rs"), "pub fn launch() {}\n").unwrap();
    let out = cli(home.path(), project.path(), &["doctor"]);
    let text = output(&out);
    let row = text
        .lines()
        .find(|line| line.contains("graph index"))
        .unwrap();
    assert!(
        row.starts_with("warn") && row.contains("axiom graph refresh"),
        "{row}"
    );
    assert!(!home.path().join("memory/memory.db").exists());
    let refreshed = cli(home.path(), project.path(), &["graph", "refresh", "--json"]);
    assert_eq!(
        refreshed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    let report: Value = serde_json::from_slice(&refreshed.stdout).unwrap();
    assert_eq!(report["code"]["files_seen"], 1);
    let text = output(&cli(home.path(), project.path(), &["doctor"]));
    let row = text
        .lines()
        .find(|line| line.contains("graph index"))
        .unwrap();
    assert!(
        row.starts_with("ok") && row.contains("files 1") && row.contains("freshness ready"),
        "{row}"
    );
    for part in [
        "parser",
        "schema",
        "FTS",
        "integrity",
        "index version",
        "last sync",
        "coverage",
        "parse failures",
        "unresolved",
    ] {
        assert!(row.contains(part), "{row}");
    }
}

#[test]
fn graph_and_dream_commands_validate_and_run() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.rs"), "pub fn needle() {}\n").unwrap();
    for args in [
        &["dream", "--bad"][..],
        &["graph"][..],
        &["graph", "refresh", "--dir", "relative"][..],
        &["graph", "explore"][..],
        &["graph", "explore", "needle", "--view", "bad"][..],
        &["graph", "explore", "needle", "--budget", "bad"][..],
    ] {
        let out = cli(home.path(), project.path(), args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let invalid_home = home.path().join("not-a-directory");
    fs::write(&invalid_home, "x").unwrap();
    let failure = cli(&invalid_home, project.path(), &["graph", "refresh"]);
    assert_eq!(failure.status.code(), Some(1));
    let dir = project.path().to_str().unwrap();
    for args in [
        &["graph", "refresh", "--dir", dir][..],
        &[
            "graph", "explore", "needle", "--dir", dir, "--view", "code", "--budget", "500",
        ][..],
        &[
            "dream",
            "--project",
            "demo",
            "--dir",
            dir,
            "--dry-run",
            "--json",
        ][..],
    ] {
        let out = cli(home.path(), project.path(), args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if args[1] == "explore" {
            let payload: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(payload["status"], "completed");
            assert_eq!(payload["query"]["text"], "needle");
            assert_eq!(payload["freshness"]["state"], "fresh");
        }
    }
}

#[test]
fn wish_hook_injects_saved_packet_once_and_preserves_wish() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.rs"), "pub fn needle() {}\n").unwrap();
    fs::write(
        project.path().join("CONSTRAINTS.md"),
        "- Keep the interface stable.\n",
    )
    .unwrap();
    let event = json!({"hook_event_name":"UserPromptSubmit", "session_id":"session-1", "prompt":"/chronos:wish implement needle", "cwd":project.path()});
    let first = hook::handle(&event, Some(home.path()), false).unwrap();
    let text = first["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let saved_goal = goal::load(home.path(), "session-1").unwrap().unwrap();
    assert_eq!(saved_goal.wish, "implement needle");
    let store = orch::store_for_goal(home.path(), &saved_goal).unwrap();
    let packet: Value =
        serde_json::from_slice(&fs::read(store.join("launch.json")).unwrap()).unwrap();
    assert!(text.contains(packet["text"].as_str().unwrap()));
    assert!(text.contains("Keep the interface stable."));
    let mut resume = event.clone();
    resume["prompt"] = json!("/chronos:wish continue this session");
    let second = hook::handle(&resume, Some(home.path()), false).unwrap();
    assert!(!second["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("LAUNCH CONTEXT"));
    let resumed = goal::load(home.path(), "session-1").unwrap().unwrap();
    assert_eq!(resumed.wish_id, saved_goal.wish_id);
    assert_eq!(resumed.wish, saved_goal.wish);
}

#[test]
fn wish_hook_works_without_existing_graph() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let event = json!({"hook_event_name":"UserPromptSubmit", "session_id":"session-2", "prompt":"/chronos:wish hello", "cwd":project.path()});
    let result = hook::handle(&event, Some(home.path()), false).unwrap();
    let text = result["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("Chronos goal session-2") && text.contains("LAUNCH CONTEXT"),
        "{text}"
    );
    assert_eq!(
        goal::load(home.path(), "session-2").unwrap().unwrap().wish,
        "hello"
    );
}

#[test]
fn refresh_ingests_the_current_wish_store() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("main.rs"), "pub fn needle() {}\n").unwrap();
    let goal = goal::open(home.path(), "current", "test", project.path().to_str()).unwrap();
    let wish_store = orch::store_for_goal(home.path(), &goal).unwrap();
    fs::create_dir_all(&wish_store).unwrap();
    fs::write(
        wish_store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tapp\tverified\t\t\t\t\n",
    )
    .unwrap();
    let out = cli(home.path(), project.path(), &["graph", "refresh", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        report["orch"]["nodes_written"].as_u64().unwrap() > 0,
        "{report}"
    );
}

#[test]
fn wish_hook_reports_launch_failure_without_losing_context() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let saved = goal::open(home.path(), "fail", "hello", project.path().to_str()).unwrap();
    let blocker = orch::store_for_goal(home.path(), &saved)
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    fs::create_dir_all(blocker.parent().unwrap()).unwrap();
    fs::write(blocker, "not a directory").unwrap();
    let event = json!({"hook_event_name":"UserPromptSubmit", "session_id":"fail", "prompt":"/chronos:wish hello", "cwd":project.path()});
    let result = hook::handle(&event, Some(home.path()), false).unwrap();
    let text = result["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        text.contains("Chronos goal fail") && text.contains("\nlaunch context unavailable: "),
        "{text}"
    );
    assert_eq!(
        goal::load(home.path(), "fail").unwrap().unwrap().wish,
        "hello"
    );
}
