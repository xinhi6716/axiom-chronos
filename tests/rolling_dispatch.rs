mod common;

use axiom::session::background::{BackgroundTasks, TaskChange};
use serde_json::{json, Value};
use std::{
    fs,
    io::Cursor,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn background_task_events_are_idempotent() {
    let mut tasks = BackgroundTasks::default();
    let start =
        json!({"type":"system","subtype":"task_started","task_id":"a","is_backgrounded":true});
    let foreground =
        json!({"type":"system","subtype":"task_started","task_id":"fg","is_backgrounded":false});
    assert_eq!(tasks.observe(&foreground), None);
    assert_eq!(tasks.observe(&start), Some(TaskChange::Started("a".into())));
    assert_eq!(tasks.observe(&start), None);
    assert_eq!(tasks.open(), 1);
    let failed =
        json!({"type":"system","subtype":"task_notification","task_id":"a","status":"failed"});
    assert_eq!(
        tasks.observe(&failed),
        Some(TaskChange::Finished {
            id: "a".into(),
            status: "failed".into()
        })
    );
    assert_eq!(tasks.observe(&failed), None);
    assert_eq!(tasks.observe(&start), None);
    assert_eq!(tasks.open(), 0);
    let b = json!({"type":"system","subtype":"task_started","task_id":"b","is_backgrounded":true});
    let completed =
        json!({"type":"system","subtype":"task_notification","task_id":"b","status":"completed"});
    assert_eq!(tasks.observe(&b), Some(TaskChange::Started("b".into())));
    assert_eq!(
        tasks.observe(&completed),
        Some(TaskChange::Finished {
            id: "b".into(),
            status: "completed".into()
        })
    );
    assert_eq!(tasks.observe(&completed), None);
    assert_eq!(tasks.open(), 0);
}

#[test]
fn rolling_worker() {
    let Ok(home) = std::env::var("ROLLING_WORKER_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    let mut claude = common::fake("fake_claude");
    if std::env::var("FAKE_ROLLING_CONTROL").as_deref() == Ok("foreground") {
        let script = claude.pop().unwrap();
        claude.extend(["-c".into(), "import os,runpy,sys; os.environ['CLAUDE_CODE_DISABLE_BACKGROUND_TASKS']='1'; sys.argv=sys.argv[1:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(), script]);
    }
    if std::env::var("FAKE_ROLLING_CONTROL").as_deref() == Ok("closed-stdin") {
        let fake = claude;
        claude = vec![common::python(), "-I".into(), "-c".into(), r#"
import json, subprocess, sys
child = subprocess.Popen(sys.argv[1:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding='utf-8')
child.stdin.write(sys.stdin.readline())
child.stdin.flush()
for line in child.stdout:
    sys.stdout.write(line)
    sys.stdout.flush()
    if json.loads(line).get('type') == 'result':
        child.stdin.close()
        break
for line in child.stdout:
    sys.stdout.write(line)
    sys.stdout.flush()
sys.exit(child.wait())
"#.into()];
        claude.extend(fake);
    }
    let config = axiom::session::Config {
        claude: Ok(claude),
        home: home.clone(),
        bin: env!("CARGO_BIN_EXE_axiom").into(),
        clock: axiom::session::Clock::system(),
    };
    let mut output = Vec::new();
    let result = axiom::session::run_with(
        &config,
        "wish",
        "rolling fixture",
        &home,
        None,
        Box::new(Cursor::new(Vec::<u8>::new())),
        &mut output,
        &Default::default(),
    );
    assert_eq!(result.unwrap(), 0, "{}", String::from_utf8_lossy(&output));
}

fn run_case(control: &str) -> bool {
    let temp = common::temp();
    let home = temp.path();
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## later\nPURPOSE x\nPATHS later/\nDEPENDS fast\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    fs::write(
        store.join("fast.brief"),
        "GOAL x\nSCOPE fast/\nVERIFY echo ok\n",
    )
    .unwrap();
    let disk = home.join("disk.json");
    fs::write(&disk, r#"{"free_bytes":1099511627776}"#).unwrap();
    let gate = home.join("gate");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "rolling_worker", "--nocapture"])
        .env("ROLLING_WORKER_HOME", home)
        .env("AXIOM_FAKE_DISK", &disk)
        .env("FAKE_CLAUDE_RECORD", home.join("record.json"))
        .env("FAKE_CLAUDE_SCENARIO", "rolling")
        .env("FAKE_ROLLING_STORE", &store)
        .env("FAKE_ROLLING_GATE", &gate)
        .env("FAKE_ROLLING_CONTROL", control)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut gated = false;
    let mut status = None;
    while Instant::now() < deadline {
        let events = fs::read_to_string(store.join("events.jsonl")).unwrap_or_default();
        if !gated
            && events
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .any(|e| e["kind"] == "module_claimed" && e["module"] == "later")
        {
            fs::write(&gate, "release").unwrap();
            gated = true;
        }
        if let Some(s) = child.try_wait().unwrap() {
            status = Some(s);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    if status.is_none() {
        fs::write(&gate, "timeout").unwrap();
        child.kill().ok();
        child.wait().ok();
    }
    let events = fs::read_to_string(store.join("events.jsonl")).unwrap_or_default();
    let parsed: Vec<Value> = events
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let later = parsed
        .iter()
        .position(|e| e["kind"] == "module_claimed" && e["module"] == "later");
    let slow = parsed
        .iter()
        .position(|e| e["kind"] == "activity_finish" && e["module"] == "slow");
    if !control.is_empty() {
        assert!(
            parsed
                .iter()
                .any(|e| e["kind"] == "module_claimed" && e["module"] == "fast"),
            "events: {events}"
        );
        assert!(
            parsed
                .iter()
                .any(|e| e["kind"] == "module_claimed" && e["module"] == "slow"),
            "events: {events}"
        );
        if control == "foreground" {
            let record: Value =
                serde_json::from_slice(&fs::read(home.join("record.json")).unwrap()).unwrap();
            assert_eq!(record["env"]["CLAUDE_CODE_DISABLE_BACKGROUND_TASKS"], "1");
            assert!(
                parsed
                    .iter()
                    .any(|e| e["kind"] == "activity_finish" && e["module"] == "fast"),
                "events: {events}"
            );
        } else {
            assert!(
                status.is_some(),
                "stdin-closure proxy did not finish; events: {events}"
            );
        }
    }
    if control.is_empty() {
        assert!(
            status.is_some_and(|s| s.success()),
            "worker failed; events: {events}; units: {}; evidence: {}; log: {}",
            fs::read_to_string(store.join("units.tsv")).unwrap_or_default(),
            fs::read_to_string(store.join("reports/fast-unit.evidence.tsv")).unwrap_or_default(),
            fs::read_dir(home.join("logs"))
                .ok()
                .and_then(|mut d| d.next())
                .and_then(|entry| entry.ok())
                .and_then(|e| fs::read_to_string(e.path()).ok())
                .unwrap_or_default()
        );
        let fast_claim = parsed
            .iter()
            .position(|e| e["kind"] == "module_claimed" && e["module"] == "fast")
            .unwrap();
        let slow_claim = parsed
            .iter()
            .position(|e| e["kind"] == "module_claimed" && e["module"] == "slow")
            .unwrap();
        let fast_finish = parsed
            .iter()
            .position(|e| e["kind"] == "activity_finish" && e["module"] == "fast")
            .unwrap();
        assert!(
            fast_claim < fast_finish && slow_claim < fast_finish,
            "events: {events}"
        );
        assert!(
            gated && later.zip(slow).is_some_and(|(a, b)| a < b),
            "events: {events}"
        );
    }
    gated && later.zip(slow).is_some_and(|(a, b)| a < b)
}

#[test]
fn later_is_claimed_before_slow_finishes() {
    assert!(run_case(""));
}

#[test]
fn foreground_negative_control_cannot_reach_gate() {
    assert!(!run_case("foreground"));
}

#[test]
fn closed_stdin_negative_control_cannot_reach_gate() {
    assert!(!run_case("closed-stdin"));
}
