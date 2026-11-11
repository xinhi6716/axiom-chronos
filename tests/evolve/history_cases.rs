use axiom::evolve::history::{self, MetricValue, RunRecord};
use std::fs;

fn record(id: &str, started_ms: i64) -> RunRecord {
    RunRecord {
        id: id.into(),
        started_ms,
        repo: "/fixture".into(),
        base_commit: "abc".into(),
        branch: "evolve/2026-11-10".into(),
        since_days: 7,
        start_ms: 1,
        end_ms: 2,
        problems: vec!["p1".into()],
        metrics: vec![MetricValue {
            problem: "p1".into(),
            value: 0.5,
            samples: 3,
        }],
        state: "started".into(),
        session: None,
        goal_status: None,
        exit_code: None,
        error: None,
    }
}

#[test]
fn loads_record_in_existing_json_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("evolve/runs");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("old.json"),
        r#"{
        "id": "old", "started_ms": 100, "repo": "/fixture", "base_commit": "abc",
        "branch": "evolve/2026-11-10", "since_days": 7, "start_ms": 1, "end_ms": 2,
        "problems": ["p1"], "metrics": [{"problem": "p1", "value": 0.5, "samples": 3}],
        "state": "started", "session": null, "goal_status": null, "exit_code": null, "error": null
    }"#,
    )
    .unwrap();
    assert_eq!(history::list(tmp.path()).unwrap(), vec![record("old", 100)]);
}

#[test]
fn persists_orders_and_overwrites_records() {
    let tmp = tempfile::tempdir().unwrap();
    let a = record("a", 100);
    let mut b = record("b", 200);
    history::save(tmp.path(), &a).unwrap();
    history::save(tmp.path(), &b).unwrap();
    assert_eq!(
        history::list(tmp.path()).unwrap(),
        vec![b.clone(), a.clone()]
    );
    b.state = "ended".into();
    b.exit_code = Some(0);
    history::save(tmp.path(), &b).unwrap();
    let c = record("c", 200);
    history::save(tmp.path(), &c).unwrap();
    assert_eq!(history::list(tmp.path()).unwrap(), vec![c, b, a]);
}

#[test]
fn missing_directory_is_read_only_and_corrupt_records_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("evolve/runs");
    assert!(history::list(tmp.path()).unwrap().is_empty());
    assert!(!dir.exists());
    let good = record("good", 1);
    history::save(tmp.path(), &good).unwrap();
    fs::write(dir.join("bad.json"), "{broken").unwrap();
    fs::create_dir(dir.join("unreadable.json")).unwrap();
    assert_eq!(history::list(tmp.path()).unwrap(), vec![good]);
}

#[test]
fn ids_use_utc_timestamp_and_unique_suffix() {
    let a = history::new_id(1_699_999_999_999);
    let b = history::new_id(1_699_999_999_999);
    assert_ne!(a, b);
    assert!(regex::Regex::new(r"^20231114-221319-[0-9a-f]{32}$")
        .unwrap()
        .is_match(&a));
    assert!(history::new_id(-1).starts_with("19691231-235959-"));
    assert!(history::new_id(1_700_000_000_000) > a);
}
