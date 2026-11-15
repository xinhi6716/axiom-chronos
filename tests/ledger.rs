use axiom::orch::{self, events::*, receipts::*};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn append(store: &Path) {
    append_event(store, UNIT_STATE, Some("m"), Some("u"), json!({})).unwrap();
}

#[test]
fn event_process_writer() {
    if let Some(path) = std::env::var_os("AXIOM_LEDGER_CHILD_STORE") {
        for _ in 0..25 {
            append(Path::new(&path));
        }
    }
}

#[test]
fn receipt_process_writer() {
    if let Some(path) = std::env::var_os("AXIOM_LEDGER_RECEIPT_STORE") {
        for i in 0..25 {
            record_run(Path::new(&path), &run("child", i, i + 1, "ok")).unwrap();
        }
    }
}

#[test]
fn receipt_concurrent_writers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().to_path_buf();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "receipt_process_writer"])
        .env("AXIOM_LEDGER_RECEIPT_STORE", &path)
        .spawn()
        .unwrap();
    let threads: Vec<_> = (0..8)
        .map(|n| {
            let path = path.clone();
            std::thread::spawn(move || {
                for i in 0..25 {
                    record_run(&path, &run(&format!("u{n}"), i, i + 1, "failed")).unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert!(child.wait().unwrap().success());
    let rows = read_runs(&path).unwrap();
    assert_eq!(rows.len(), 225);
    for n in 0..8 {
        assert_eq!(
            rows.iter().filter(|r| r.unit == format!("u{n}")).count(),
            25
        );
    }
    assert_eq!(rows.iter().filter(|r| r.unit == "child").count(), 25);
}

#[test]
fn sequences_cursor_torn_and_wait() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().to_path_buf();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "event_process_writer"])
        .env("AXIOM_LEDGER_CHILD_STORE", &path)
        .spawn()
        .unwrap();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    append(&p);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert!(child.wait().unwrap().success());
    let rows = read_events(&path, 0).unwrap();
    assert_eq!(rows.len(), 225);
    assert_eq!(
        rows.iter().map(|e| e.seq).collect::<Vec<_>>(),
        (1..=225).collect::<Vec<_>>()
    );
    assert_eq!(
        read_events(&path, 223)
            .unwrap()
            .iter()
            .map(|e| e.seq)
            .collect::<Vec<_>>(),
        vec![224, 225]
    );
    assert_eq!(last_seq(&path).unwrap(), 225);
    let mut file = OpenOptions::new()
        .append(true)
        .open(path.join("events.jsonl"))
        .unwrap();
    write!(file, "{{\"seq\":226").unwrap();
    drop(file);
    assert_eq!(last_seq(&path).unwrap(), 225);
    append(&path);
    assert_eq!(last_seq(&path).unwrap(), 226);
    assert_eq!(read_events(&path, 225).unwrap().len(), 1);
    let p = path.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        append(&p);
    });
    let before = Instant::now();
    assert_eq!(
        wait_events(&path, 226, Duration::from_secs(3)).unwrap()[0].seq,
        227
    );
    assert!(before.elapsed() < Duration::from_secs(2));
    thread.join().unwrap();
    let before = Instant::now();
    assert!(wait_events(&path, 227, Duration::from_millis(100))
        .unwrap()
        .is_empty());
    assert!(before.elapsed() >= Duration::from_millis(90));
}

#[test]
fn activities_and_heartbeat() {
    let temp = tempfile::tempdir().unwrap();
    let p = temp.path();
    let beat = heartbeat(p).unwrap();
    assert_eq!(beat.data["latest_activity_at"], Value::Null);
    assert_eq!(latest_activity_at(p).unwrap(), None);
    for (actor, phase) in [("other", "brief"), ("core", "other")] {
        assert_eq!(
            activity_start(p, "m", actor, phase).unwrap_err().kind,
            "invalid_argument"
        );
    }
    let a = activity_start(p, "m", "core", "brief").unwrap();
    let b = activity_start(p, "m", "core", "brief").unwrap();
    let c = activity_start(p, "n", "dispatcher", "review").unwrap();
    let aid = a.data["activity"].as_str().unwrap();
    assert_eq!(aid, format!("a{}", a.seq));
    let closed = activity_finish(p, "m", "core", "brief", Some(aid), "finished")
        .unwrap()
        .unwrap();
    assert_eq!(closed.data["activity"], aid);
    let seq = last_seq(p).unwrap();
    assert!(
        activity_finish(p, "m", "core", "brief", Some(aid), "finished")
            .unwrap()
            .is_none()
    );
    assert_eq!(last_seq(p).unwrap(), seq);
    assert_eq!(
        close_open_activities(p, Some("m"), "cancelled").unwrap(),
        vec![b.data["activity"].as_str().unwrap().to_string()]
    );
    assert_eq!(open_activities(p).unwrap().len(), 1);
    assert_eq!(
        close_open_activities(p, None, "exit").unwrap(),
        vec![c.data["activity"].as_str().unwrap().to_string()]
    );
    assert!(open_activities(p).unwrap().is_empty());
    let d = activity_start(p, "m", "core", "decide").unwrap();
    let e = activity_start(p, "m", "core", "decide").unwrap();
    assert_eq!(
        activity_finish(p, "m", "core", "decide", None, "failed")
            .unwrap()
            .unwrap()
            .data["activity"],
        e.data["activity"]
    );
    assert_eq!(
        open_activities(p).unwrap()[0].id,
        d.data["activity"].as_str().unwrap()
    );
    let latest = latest_activity_at(p).unwrap();
    let beat = heartbeat(p).unwrap();
    assert_eq!(beat.data["latest_activity_at"], json!(latest));
    assert!(beat.data["emitted_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(latest_activity_at(p).unwrap(), latest);
}

fn run(unit: &str, start: i64, end: i64, status: &str) -> RunRecord {
    RunRecord {
        unit: unit.into(),
        attempt: format!("a{start}"),
        started_ms: start,
        ended_ms: end,
        status: status.into(),
    }
}
#[test]
fn concurrency_timeline() {
    let runs = vec![
        run("a", -5, 5, "failed"),
        run("b", 3, 8, "cancelled"),
        run("c", 8, 10, "ok"),
        run("d", 15, 25, "interrupted"),
    ];
    let c = concurrency(&runs, (0, 20), 4);
    assert_eq!(
        c,
        Concurrency {
            mean: 0.85,
            peak: 2,
            cap: 4,
            window_ms: 20
        }
    );
    assert_eq!(
        concurrency(&[run("a", 0, 5, "ok"), run("b", 5, 10, "ok")], (0, 10), 1).peak,
        1
    );
    assert_eq!(concurrency(&[], (0, 10), 9).mean, 0.0);
    assert_eq!(
        concurrency(&runs, (5, 5), 9),
        Concurrency {
            mean: 0.0,
            peak: 0,
            cap: 9,
            window_ms: 0
        }
    );
    assert_eq!(concurrency(&runs, (10, 5), 9).peak, 0);
}
fn setup(p: &Path) {
    fs::write(p.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tsuperseded:u2\t\t\t\t\nu2\tm\tsuperseded:u3\t\t\t\t\nu3\tm\tverified\t\t\t\t\nu4\tm\tpending\t\t\t\t\n").unwrap();
    fs::create_dir_all(p.join("reports")).unwrap();
}
fn verify(p: &Path, id: &str) {
    fs::write(p.join("reports").join(format!("{id}.evidence.tsv")), "kind\tcommand\texit_code\tcommit\tts\nrun\tcodex-worker\t0\ttree:x\t1970-01-01T00:00:01.000Z\nverify\ttest\t0\ttree:y\t1970-01-01T00:00:09.000Z\nreview\tchronos:reviewer\t0\ttree:y\t1970-01-01T00:00:10.000Z\n").unwrap();
}
#[test]
fn receipts_compute_write_and_digest() {
    let temp = tempfile::tempdir().unwrap();
    let p = temp.path();
    setup(p);
    let empty = compute(p, 7).unwrap();
    assert_eq!(
        (empty.verified_units, empty.total_ms, empty.per_unit_ms),
        (0, 0, None)
    );
    verify(p, "u1");
    verify(p, "u3");
    let no_runs = compute(p, 7).unwrap();
    assert_eq!(
        (
            no_runs.verified_units,
            no_runs.total_ms,
            no_runs.per_unit_ms
        ),
        (1, 0, Some(0.0))
    );
    assert_eq!(
        (
            no_runs.window_start_ms,
            no_runs.window_end_ms,
            no_runs.concurrency.peak
        ),
        (0, 0, 0)
    );
    for r in [
        run("u1", 1000, 2000, "failed"),
        run("u2", 2000, 3000, "cancelled"),
        run("u3", 4000, 6000, "ok"),
        run("u3", 7000, 8000, "ok"),
    ] {
        record_run(p, &r).unwrap();
    }
    let receipts = compute(p, 7).unwrap();
    assert_eq!(
        (
            receipts.window_start_ms,
            receipts.window_end_ms,
            receipts.total_ms,
            receipts.verified_units,
            receipts.per_unit_ms
        ),
        (1000, 10000, 9000, 1, Some(9000.0))
    );
    assert_eq!(
        (
            receipts.units[0].unit.as_str(),
            receipts.units[0].first_ms,
            receipts.units[0].verified_ms,
            receipts.units[0].wall_ms
        ),
        ("u3", 1000, 10000, 9000)
    );
    assert_eq!(receipts.runs, 4);
    let path = write_receipts(p, 7).unwrap();
    assert_eq!(path, p.join("receipts.json"));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()["verified_units"],
        1
    );
    for module in [None, Some("m")] {
        let digest = orch::report::digest(p, p, module).unwrap();
        assert!(
            digest.contains("receipts: T=9.0s N=1 T/N=9.0s mean="),
            "{digest}"
        );
        assert!(digest.contains("receipts file: "));
        assert_eq!(
            digest.lines().filter(|s| s.starts_with("receipts")).count(),
            2
        );
        fs::remove_file(p.join("receipts.json")).unwrap();
        fs::create_dir(p.join("receipts.json")).unwrap();
        let without_receipts = orch::report::digest(p, p, module).unwrap();
        let stripped = digest
            .lines()
            .filter(|s| !s.starts_with("receipts"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(stripped, without_receipts.trim_end());
        fs::remove_dir(p.join("receipts.json")).unwrap();
    }
    let mut file = OpenOptions::new()
        .append(true)
        .open(p.join("receipts/runs.jsonl"))
        .unwrap();
    write!(file, "{{bad").unwrap();
    drop(file);
    assert_eq!(read_runs(p).unwrap().len(), 4);
    record_run(p, &run("u4", 9, 10, "ok")).unwrap();
    assert_eq!(read_runs(p).unwrap().len(), 5);
    let data = fs::read(p.join("receipts/runs.jsonl")).unwrap();
    fs::write(p.join("receipts/runs.jsonl"), &data[..data.len() - 1]).unwrap();
    assert_eq!(read_runs(p).unwrap().len(), 5);
    record_run(p, &run("u4", 11, 12, "ok")).unwrap();
    assert_eq!(read_runs(p).unwrap().len(), 6);
}
