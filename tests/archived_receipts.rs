use axiom::{
    app,
    evolve::{collect::collect, Inputs},
    orch::{
        receipts::{read_runs, record_run, RunRecord},
        report,
    },
};
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use std::{fs, io::Write, path::Path};

fn put(path: &Path, data: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}
fn archive(path: &Path) {
    let data = fs::read(path).unwrap();
    let modified = fs::metadata(path).unwrap().modified().unwrap();
    let gz = axiom::storage::archive::archived_path(path);
    let mut encoder = GzEncoder::new(fs::File::create(&gz).unwrap(), Compression::default());
    encoder.write_all(&data).unwrap();
    let file = encoder.finish().unwrap();
    file.set_modified(modified).unwrap();
    drop(file);
    fs::remove_file(path).unwrap();
}
fn goal(home: &Path) {
    put(
        &home.join("goals/s.json"),
        &json!({"wish":"x", "status":"active", "project":"p", "cwd":"/fixture"}).to_string(),
    );
}

#[test]
fn archived_receipts_remain_readable_and_resumable() {
    let tmp = tempfile::tempdir().unwrap();
    let row = RunRecord {
        unit: "u".into(),
        attempt: "a".into(),
        started_ms: 1,
        ended_ms: 2,
        status: "done".into(),
    };
    record_run(tmp.path(), &row).unwrap();
    let file = tmp.path().join("receipts/runs.jsonl");
    archive(&file);
    assert_eq!(read_runs(tmp.path()).unwrap(), vec![row.clone()]);
    let next = RunRecord {
        attempt: "b".into(),
        ..row.clone()
    };
    record_run(tmp.path(), &next).unwrap();
    assert_eq!(read_runs(tmp.path()).unwrap(), vec![row, next]);
    assert!(file.is_file());
    assert!(!axiom::storage::archive::archived_path(&file).exists());
}

#[test]
fn archived_logs_list_like_plain() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    goal(home);
    let log = home.join("logs/20260101-000000-wish.jsonl");
    put(
        &log,
        &format!(
            "{}\n{}\n",
            json!({"type":"system","subtype":"init","session_id":"s","cwd":"/fixture"}),
            json!({"type":"result","result":"done"})
        ),
    );
    put(&home.join("logs/x.jsonl.gz.bak"), "not a log");
    let mut poller = app::Poller::new(home);
    let before = poller.poll();
    assert_eq!(before.wishes[0].runs.len(), 1);
    archive(&log);
    let after = poller.poll();
    assert_eq!(
        serde_json::to_value(&before.wishes[0].runs).unwrap(),
        serde_json::to_value(&after.wishes[0].runs).unwrap()
    );
    assert_eq!(after.wishes[0].runs[0].log, log);
    assert!(after.warnings.is_empty(), "{:?}", after.warnings);
}

#[test]
fn archived_evidence_read_by_store_and_report() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    goal(home);
    let store = home.join("orchestrate/p");
    put(
        &store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tverified\t\t\t\t\n",
    );
    let ev =
        "kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6.1-sol\t0\tcommit\t2026-01-01T00:00:00Z\n";
    for name in [
        "u.evidence.tsv",
        "_full.evidence.tsv",
        "_stages.evidence.tsv",
    ] {
        put(&store.join("reports").join(name), ev);
    }
    put(
        &store.join("reports/u.json"),
        &json!({"file_changes":[{"path":"hello.rs"}]}).to_string(),
    );
    let before = app::snapshot(home);
    assert_eq!(before.wishes[0].units[0].evidence.len(), 1);
    let digest = report::digest(&store, home, None).unwrap();
    assert!(digest.contains("hello.rs"));
    assert!(digest.contains("sol: ran"));
    for name in [
        "u.evidence.tsv",
        "_full.evidence.tsv",
        "_stages.evidence.tsv",
        "u.json",
    ] {
        archive(&store.join("reports").join(name));
    }
    let after = app::snapshot(home);
    assert_eq!(
        serde_json::to_value(&before.wishes[0].units).unwrap(),
        serde_json::to_value(&after.wishes[0].units).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&before.wishes[0].stages).unwrap(),
        serde_json::to_value(&after.wishes[0].stages).unwrap()
    );
    assert_eq!(digest, report::digest(&store, home, None).unwrap());
}

#[test]
fn archived_evolve_evidence_and_hook_match_plain() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    goal(&home);
    let inputs = Inputs {
        home: home.clone(),
        projects: tmp.path().join("projects"),
        now_ms: 1_700_000_600_000,
        since_days: 7,
    };
    let evidence = home.join("orchestrate/p/reports/u.evidence.tsv");
    put(
        &evidence,
        "kind\tcommand\texit_code\tcommit\tts\nverify\ttest\t1\ta\t2023-11-14T22:13:20Z\n",
    );
    let hook = home.join("hook/refusals.jsonl");
    put(
        &hook,
        &format!(
            "{}\n",
            json!({"session":"s", "ts":"2023-11-14T22:13:20Z", "reason":"denied"})
        ),
    );
    let expected = collect(&inputs).unwrap();
    assert!(expected
        .events
        .iter()
        .any(|e| e.problem == "orch.verify-failed"));
    assert!(expected.events.iter().any(|e| e.problem == "hook.refusal"));
    archive(&evidence);
    archive(&hook);
    assert_eq!(expected, collect(&inputs).unwrap());
}
