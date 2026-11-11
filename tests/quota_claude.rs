use axiom::{
    app::{Poller, Scan, Snapshot},
    usage::quota::{claude_log_time, ProviderQuota},
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const FIRST: &str = include_str!("fixtures/quota/claude-rate-limit-1.jsonl");
const SECOND: &str = include_str!("fixtures/quota/claude-rate-limit-2.jsonl");
const THIRD: &str = include_str!("fixtures/quota/claude-rate-limit-3.jsonl");

fn setup(home: &Path, name: &str, body: &str) -> PathBuf {
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::create_dir_all(home.join("logs")).unwrap();
    fs::write(
        home.join("goals/s1.json"),
        json!({"wish":"x", "status":"done", "amendments":[]}).to_string(),
    )
    .unwrap();
    let log = home.join("logs").join(name);
    fs::write(
        &log,
        format!(
            "{}\n{body}",
            json!({"type":"system", "subtype":"init", "session_id":"s1"})
        ),
    )
    .unwrap();
    log
}

fn scan(snapshot: &Snapshot) -> &Scan {
    snapshot
        .wishes
        .iter()
        .find(|w| w.session == "s1")
        .unwrap()
        .runs[0]
        .scan
        .as_ref()
        .unwrap()
}

fn append(log: &Path, text: &str) {
    fs::OpenOptions::new()
        .append(true)
        .open(log)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

fn quota(snapshot: &Snapshot) -> &ProviderQuota {
    scan(snapshot).rate_limit.as_ref().unwrap()
}

#[test]
fn incremental_scan_uses_preceding_timestamp_and_merges_per_window() {
    let dir = tempfile::tempdir().unwrap();
    let timestamp = "2026-11-10T13:17:08.566Z";
    let log = setup(
        dir.path(),
        "20261110-120000-wish-1.jsonl",
        &format!("{}\n{FIRST}", json!({"type":"user", "timestamp":timestamp})),
    );
    let mut poller = Poller::with_budget(dir.path(), 80);
    poller.follow("s1");
    let first = loop {
        let snapshot = poller.poll();
        if scan(&snapshot).complete {
            break snapshot;
        }
    };
    let observed = 1_794_316_628;
    assert_eq!(claude_log_time(Some(timestamp), None), Some(observed));
    let first_quota = quota(&first);
    assert_eq!(
        (
            first_quota.five_hour.as_ref().unwrap().used,
            first_quota.weekly.as_ref().unwrap().used
        ),
        (0.17, 0.11)
    );
    assert_eq!(
        first_quota.five_hour.as_ref().unwrap().observed_at,
        observed
    );
    assert_eq!(first_quota.weekly.as_ref().unwrap().observed_at, observed);

    // Preserve the timestamp across polls, even when the quota line arrives later.
    let newer = "2026-11-10T14:17:08.566Z";
    append(
        &log,
        &format!("{}\n", json!({"type":"chronos", "timestamp":newer})),
    );
    assert!(scan(&poller.poll()).complete);
    append(&log, SECOND);
    let second = loop {
        let snapshot = poller.poll();
        if scan(&snapshot).complete {
            break snapshot;
        }
    };
    assert_eq!(quota(&second).five_hour.as_ref().unwrap().used, 0.39);
    assert_eq!(quota(&second).weekly.as_ref().unwrap().used, 0.42);
    assert_eq!(
        quota(&second).five_hour.as_ref().unwrap().observed_at,
        observed + 3600
    );

    // A partial older observation must not replace either newer window.
    let partial = json!({"type":"rate_limit_event", "rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.9,"resetsAt":1,"status":"allowed"}}}});
    append(
        &log,
        &format!(
            "{}\n{partial}\n",
            json!({"type":"other", "timestamp":"2026-11-10T12:17:08Z"})
        ),
    );
    let older = loop {
        let snapshot = poller.poll();
        if scan(&snapshot).complete {
            break snapshot;
        }
    };
    assert_eq!(quota(&older), quota(&second));

    // A newer one-window update keeps the other window and its observation time.
    append(
        &log,
        &format!(
            "{}\n{partial}\n",
            json!({"type":"system", "timestamp":"2026-11-10T15:17:08Z"})
        ),
    );
    let partial_newer = loop {
        let snapshot = poller.poll();
        if scan(&snapshot).complete {
            break snapshot;
        }
    };
    assert_eq!(quota(&partial_newer).five_hour.as_ref().unwrap().used, 0.9);
    assert_eq!(
        quota(&partial_newer)
            .five_hour
            .as_ref()
            .unwrap()
            .observed_at,
        observed + 7200
    );
    assert_eq!(quota(&partial_newer).weekly, quota(&second).weekly);

    // With unchanged length and mtime, even different on-disk bytes are not rescanned.
    let modified = fs::metadata(&log).unwrap().modified().unwrap();
    let original = fs::read(&log).unwrap();
    let replacement = vec![b' '; original.len()];
    fs::write(&log, replacement).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&log)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    let unchanged = poller.poll();
    assert_eq!(scan(&unchanged).scanned, original.len() as u64);
    assert_eq!(scan(&unchanged).events, scan(&partial_newer).events);
    assert_eq!(quota(&unchanged), quota(&partial_newer));
    assert!(serde_json::to_value(scan(&unchanged))
        .unwrap()
        .get("rate_limit")
        .is_none());
}

#[test]
fn fallback_start_and_unknown_time() {
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path(), "20261110-120000-wish-1.jsonl", THIRD);
    let mut poller = Poller::new(dir.path());
    poller.follow("s1");
    let snap = poller.poll();
    let run = &snap.wishes[0].runs[0];
    let started = run.started_unix.unwrap();
    assert_eq!(
        quota(&snap).five_hour.as_ref().unwrap().observed_at,
        started
    );
    assert_eq!(quota(&snap).weekly.as_ref().unwrap().observed_at, started);
    assert_eq!(quota(&snap).five_hour.as_ref().unwrap().used, 0.03);

    let unknown = tempfile::tempdir().unwrap();
    setup(unknown.path(), "wish.jsonl", FIRST);
    let mut poller = Poller::new(unknown.path());
    poller.follow("s1");
    assert!(scan(&poller.poll()).rate_limit.is_none());
}

#[test]
fn cost_result_and_wait_are_not_quota() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{}\n{}\n{}\n",
        json!({"type":"assistant", "timestamp":"2026-11-10T13:17:08Z", "cost_usd":1.0}),
        json!({"type":"result", "total_cost_usd":3.0}),
        json!({"type":"chronos", "subtype":"limit_wait", "until":1794319000})
    );
    setup(dir.path(), "20261110-120000-wish-1.jsonl", &body);
    let mut poller = Poller::new(dir.path());
    poller.follow("s1");
    assert!(scan(&poller.poll()).rate_limit.is_none());
}
