mod common;

use axiom::codex::{AppServer, RunRequest};
use std::sync::atomic::AtomicBool;

fn request(cwd: &std::path::Path, brief: &str) -> RunRequest {
    RunRequest {
        brief: brief.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        model: "gpt-6.1-terra".into(),
        sandbox: "workspace-write".into(),
        effort: None,
        output_schema: None,
        developer_instructions: None,
        timeout_s: 10,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    }
}

#[test]
fn reports_record_times_only_after_slot_acquisition() {
    let home = common::temp();
    let server = AppServer::new(common::fake_codex(&[])).with_slots_home(home.path().join("slots"));
    for (brief, status) in [
        ("normal", "completed"),
        ("AUTH", "failed"),
        ("CRASH", "bridge_error"),
    ] {
        let server =
            AppServer::new(common::fake_codex(&[])).with_slots_home(home.path().join("slots"));
        let r = server.run(&request(home.path(), brief)).unwrap();
        assert_eq!(r["status"], status, "{r}");
        let start = r["run_started_ms"].as_i64().unwrap();
        let end = r["run_ended_ms"].as_i64().unwrap();
        assert!(start > 0 && start <= end, "{r}");
    }
    let mut timeout = request(home.path(), "WAIT_FOR_INTERRUPT");
    timeout.timeout_s = 1;
    let timed = server.run(&timeout).unwrap();
    assert_eq!(timed["status"], "timeout", "{timed}");
    assert!(timed["run_started_ms"].as_i64().unwrap() <= timed["run_ended_ms"].as_i64().unwrap());
    let capped = server
        .run_capped(&request(home.path(), "normal"), Some(1))
        .unwrap();
    assert_eq!(capped["status"], "token_cap", "{capped}");
    assert!(capped["run_started_ms"].as_i64().unwrap() <= capped["run_ended_ms"].as_i64().unwrap());
    let cancelled = AtomicBool::new(true);
    let r = server
        .run_cancellable(&request(home.path(), "normal"), None, Some(&cancelled))
        .unwrap();
    assert_eq!(r["status"], "interrupted");
    assert!(r.get("run_started_ms").is_none());
    assert!(r.get("run_ended_ms").is_none());
}
