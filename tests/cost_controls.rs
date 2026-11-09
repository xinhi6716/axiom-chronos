use axiom::{goal, hook, orch};
use serde_json::{json, Value};
use std::{fs, path::Path};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";

fn tool(name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":name,"tool_input":input})
}
fn agent(mut e: Value, id: &str) -> Value {
    e["agent_id"] = json!(id);
    e
}
fn change_goal(home: &Path) {
    goal::open(home, "s", "wish", None).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
}
fn denied(home: &Path, e: &Value) -> bool {
    hook::handle(e, Some(home), false).is_some_and(|v| {
        v["hookSpecificOutput"]["permissionDecision"] == "deny"
    })
}
fn unit(id: &str, role: &str) -> Value {
    tool(CODEX, json!({"project":"demo","unit":id,"role":role}))
}
fn store_with_units(home: &Path, map: bool) {
    let store = orch::store_for(home, "demo").unwrap();
    fs::create_dir_all(&store).unwrap();
    let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for (id, track) in [("a1", "m1"), ("a2", "m1"), ("b1", "m2"), ("s1", "build")] {
        rows.push_str(&format!("{id}\t{track}\tpending\t\t\t\t\n"));
    }
    fs::write(store.join("units.tsv"), rows).unwrap();
    if map {
        fs::write(store.join("modules.md"), "map").unwrap();
    }
}

#[test]
fn dispatcher_must_run_in_the_foreground() {
    let d = tempfile::tempdir().unwrap();
    change_goal(d.path());
    let background = json!({"subagent_type":"chronos:dispatcher","run_in_background":true});
    let foreground = json!({"subagent_type":"chronos:dispatcher"});
    assert!(denied(d.path(), &tool("Agent", background)));
    assert!(!denied(d.path(), &tool("Agent", foreground)));
}

fn stop_with(tasks: Value) -> Value {
    json!({"hook_event_name":"Stop","session_id":"s","background_tasks":tasks})
}
fn blocks(home: &Path, event: &Value) -> bool {
    hook::handle(event, Some(home), false).is_some_and(|v| v["decision"] == "block")
}
fn agent_task(agent_type: &str, status: &str) -> Value {
    json!({"id":"t1","type":"subagent","status":status,"description":"d","agent_type":agent_type})
}

#[test]
fn turn_may_end_while_a_chronos_agent_runs_in_the_background() {
    let d = tempfile::tempdir().unwrap();
    goal::open(d.path(), "s", "wish", None).unwrap();
    for name in ["chronos:dispatcher", "chronos:astra", "chronos:critic"] {
        for status in ["running", "pending"] {
            assert!(!blocks(d.path(), &stop_with(json!([agent_task(name, status)]))));
        }
    }
    for tasks in [
        json!([]),
        json!([agent_task("chronos:dispatcher", "completed")]),
        json!([agent_task("general-purpose", "running")]),
        json!([{"id":"b","type":"shell","status":"running","description":"d","command":"sleep 9"}]),
    ] {
        assert!(blocks(d.path(), &stop_with(tasks)));
    }
    assert!(blocks(d.path(), &json!({"hook_event_name":"Stop","session_id":"s"})));
}

#[test]
fn a_pending_interjection_is_classified_before_the_turn_ends() {
    let d = tempfile::tempdir().unwrap();
    goal::open(d.path(), "s", "wish", None).unwrap();
    goal::interject(d.path(), "s", "change the plan").unwrap();
    let running = stop_with(json!([agent_task("chronos:dispatcher", "running")]));
    assert!(blocks(d.path(), &running));
    goal::classify(d.path(), "s", "amend").unwrap();
    assert!(!blocks(d.path(), &running));
}

#[test]
fn the_core_cannot_poll_with_powershell() {
    let d = tempfile::tempdir().unwrap();
    change_goal(d.path());
    let ps = |command: &str| tool("PowerShell", json!({"command":command}));
    for command in [
        "while(-not (Test-Path x)){ Start-Sleep 30 }; Get-Content x",
        "$t=0; while($t -lt 570){ sleep 10; $t+=10 }",
        "Wait-Process -Id 12",
        "START-SLEEP -s 5",
    ] {
        assert!(denied(d.path(), &ps(command)), "{command}");
        assert!(!denied(d.path(), &agent(ps(command), "x")), "{command}");
    }
    for command in ["Get-Content units.tsv", "Start-Process python batch.py"] {
        assert!(!denied(d.path(), &ps(command)), "{command}");
    }
    goal::set_status(d.path(), "s", "blocked", Some("note")).unwrap();
    assert!(!denied(d.path(), &ps("Start-Sleep 5")));
}

#[test]
fn large_files_cannot_be_read_whole() {
    let d = tempfile::tempdir().unwrap();
    change_goal(d.path());
    let big = d.path().join("data.csv");
    fs::write(&big, vec![b'x'; 300 * 1024]).unwrap();
    let small = d.path().join("small.csv");
    fs::write(&small, "x").unwrap();
    let image = d.path().join("page.PNG");
    fs::write(&image, vec![b'x'; 300 * 1024]).unwrap();
    let read = |path: &Path, extra: Value| {
        let mut input = json!({"file_path": path});
        if let Value::Object(more) = extra {
            input.as_object_mut().unwrap().extend(more);
        }
        tool("Read", input)
    };
    assert!(denied(d.path(), &read(&big, json!({}))));
    assert!(denied(d.path(), &agent(read(&big, json!({"limit":501})), "x")));
    assert!(!denied(d.path(), &read(&big, json!({"limit":500}))));
    assert!(!denied(d.path(), &read(&small, json!({}))));
    assert!(!denied(d.path(), &read(&image, json!({}))));
    goal::set_status(d.path(), "s", "blocked", Some("note")).unwrap();
    assert!(!denied(d.path(), &read(&big, json!({}))));
}

#[test]
fn a_dispatcher_stays_on_one_module() {
    let d = tempfile::tempdir().unwrap();
    change_goal(d.path());
    store_with_units(d.path(), true);
    assert!(!denied(d.path(), &agent(unit("a1", "terra"), "first")));
    assert!(!denied(d.path(), &agent(unit("a2", "terra"), "first")));
    assert!(denied(d.path(), &agent(unit("b1", "terra"), "first")));
    assert!(!denied(d.path(), &agent(unit("b1", "sol"), "first")));
    assert!(!denied(d.path(), &agent(unit("b1", "terra"), "second")));
    assert!(denied(d.path(), &agent(unit("a1", "terra"), "second")));
}

#[test]
fn without_a_map_one_dispatcher_may_run_every_unit() {
    let d = tempfile::tempdir().unwrap();
    change_goal(d.path());
    store_with_units(d.path(), false);
    assert!(!denied(d.path(), &agent(unit("a1", "terra"), "first")));
    assert!(!denied(d.path(), &agent(unit("b1", "terra"), "first")));
}

#[test]
fn waiting_goal_lets_the_session_end_and_can_resume() {
    let d = tempfile::tempdir().unwrap();
    goal::open(d.path(), "s", "wish", None).unwrap();
    let stop = json!({"hook_event_name":"Stop","session_id":"s"});
    assert!(hook::handle(&stop, Some(d.path()), false).is_some());
    assert_eq!(
        goal::set_status(d.path(), "s", "waiting", Some("  "))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    goal::set_status(d.path(), "s", "waiting", Some("batch; progress.json; done.flag")).unwrap();
    assert_eq!(goal::load(d.path(), "s").unwrap().unwrap().status, "waiting");
    assert!(hook::handle(&stop, Some(d.path()), false).is_none());
    let reopened = goal::reopen(d.path(), "s").unwrap();
    assert_eq!(reopened.status, "active");
    assert!(hook::handle(&stop, Some(d.path()), false).is_some());
}
