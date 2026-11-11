#[path = "common/race.rs"]
mod race;

use axiom::{
    base,
    orch::{self, Begin, Evidence},
};
use race::{child, children, LiveProcess};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn row(kind: &str, command: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: 0,
        commit: String::new(),
        ts: base::now_iso(),
    }
}
fn empty_store(root: &Path) -> std::path::PathBuf {
    let store = root.join("store");
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    store
}

#[test]
fn a_live_holder_is_waited_for() {
    let t = tempfile::tempdir().unwrap();
    let holder = LiveProcess::start();
    let lock = t.path().join(".orch.lock");
    fs::write(&lock, format!("{}\n", holder.pid())).unwrap();
    let release = std::thread::spawn({
        let lock = lock.clone();
        move || {
            std::thread::sleep(Duration::from_millis(300));
            fs::remove_file(lock).unwrap();
        }
    });
    let started = Instant::now();
    let entered = orch::with_lock(t.path(), false, || Ok(started.elapsed()));
    release.join().unwrap();
    let waited = entered.expect("the store lock must be waited for, not refused");
    assert!(waited >= Duration::from_millis(250), "{waited:?}");
    assert!(!lock.exists());
}

#[test]
fn evidence_appended_by_many_processes_is_all_kept() {
    const WRITERS: usize = 8;
    const ROWS: usize = 20;
    if let Some((dir, i)) = child() {
        for n in 0..ROWS {
            orch::append_evidence(
                &dir.join("store"),
                "u",
                &row("run", &format!("writer-{i}-{n}")),
            )
            .unwrap();
        }
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    children(
        "evidence_appended_by_many_processes_is_all_kept",
        t.path(),
        WRITERS,
    );
    let rows = orch::evidence(&store, "u").unwrap();
    assert_eq!(rows.len(), WRITERS * ROWS);
    let distinct: HashSet<_> = rows.iter().map(|r| r.command.as_str()).collect();
    assert_eq!(distinct.len(), WRITERS * ROWS, "a row was written twice");
    assert!(!store.join(".orch.lock").exists());
}

#[test]
fn units_added_through_the_cli_by_many_processes_all_land() {
    const WRITERS: usize = 16;
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .env_remove("ORCH_STORE")
            .arg("orch")
            .arg("--store")
            .arg(&store)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    assert!(cli(&["init"]).wait_with_output().unwrap().status.success());
    let running: Vec<_> = (0..WRITERS)
        .map(|i| cli(&["unit", "add", &format!("u{i}"), "--track", "t"]))
        .collect();
    let refused: Vec<_> = running
        .into_iter()
        .map(|c| c.wait_with_output().unwrap())
        .filter(|o| !o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .collect();
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(orch::read_units(&store).unwrap().len(), WRITERS);
}

#[test]
fn distinct_units_claimed_by_many_processes_all_hold() {
    const WORKERS: usize = 6;
    if let Some((dir, i)) = child() {
        let begun = orch::begin_unit(
            &dir.join("store"),
            &format!("u{i}"),
            "worker",
            &dir.join("repo"),
            false,
        );
        assert!(matches!(begun, Ok(Begin::Run { .. })), "{:?}", begun.err());
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    let repo = t.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let mut units = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
    for i in 0..WORKERS {
        let brief = store.join(format!("u{i}.brief"));
        fs::write(&brief, format!("GOAL g\nSCOPE f{i}.txt\nVERIFY echo ok\n")).unwrap();
        units.push_str(&format!("u{i}\tm\tpending\t\t\t\t{}\n", brief.display()));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    children(
        "distinct_units_claimed_by_many_processes_all_hold",
        t.path(),
        WORKERS,
    );
    let claims = fs::read_to_string(store.join("claims.tsv")).unwrap();
    let held: HashSet<_> = claims
        .lines()
        .skip(1)
        .map(|l| l.split('\t').next().unwrap().to_owned())
        .collect();
    assert_eq!(held.len(), WORKERS, "{claims}");
}

#[test]
fn a_dead_holders_lock_is_replaced_by_one_process_at_a_time() {
    const PROCESSES: usize = 8;
    const ROUNDS: usize = 8;
    if let Some((dir, _)) = child() {
        orch::with_lock(&dir, false, || {
            let count = dir.join("count");
            let seen: usize = fs::read_to_string(&count).unwrap().trim().parse().unwrap();
            std::thread::sleep(Duration::from_millis(3));
            fs::write(&count, (seen + 1).to_string()).unwrap();
            Ok(())
        })
        .unwrap();
        return;
    }
    for _ in 0..ROUNDS {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("count"), "0").unwrap();
        fs::write(t.path().join(".orch.lock"), "4294967295\n").unwrap();
        children(
            "a_dead_holders_lock_is_replaced_by_one_process_at_a_time",
            t.path(),
            PROCESSES,
        );
        assert_eq!(
            fs::read_to_string(t.path().join("count")).unwrap(),
            PROCESSES.to_string(),
            "two processes were inside the lock at once"
        );
        let left: Vec<_> = fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n.to_string_lossy().contains(".orch.lock"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }
}

#[cfg(windows)]
#[test]
fn atomic_write_outlasts_a_reader_that_blocks_replacement() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("state.json");
    fs::write(&target, "old").unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&target)
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
    });
    let result = base::atomic_write(&target, b"new");
    release.join().unwrap();
    result.expect("a reader closing the file soon must not fail the write");
    assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1, "temp file left");
}

const HEADER: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
const EVIDENCE: &str = "kind\tcommand\texit_code\tcommit\tts\n";

/// A store holding unit `u` over `f.txt` in a git repository, padded so reading units takes a moment.
fn unit_in_repo(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let store = empty_store(root);
    let repo = root.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("f.txt"), "one").unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL g\nSCOPE f.txt\nVERIFY echo ok\n").unwrap();
    let mut units = format!("{HEADER}u\tm\tpending\t\t\t\t{}\n", brief.display());
    for i in 0..20_000 {
        units.push_str(&format!("padding{i}\tother\tpending\t\t\t\t\n"));
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    (store, repo)
}
fn cached_state(store: &Path) -> String {
    orch::read_units(store)
        .unwrap()
        .into_iter()
        .find(|u| u.id == "u")
        .unwrap()
        .state
}
fn evidence_state(store: &Path) -> String {
    orch::derive_state(&orch::evidence(store, "u").unwrap(), false)
}
fn save_state_by_hand(store: &Path, state: &str) {
    let text = fs::read_to_string(store.join("units.tsv")).unwrap();
    let mut lines = text.lines();
    let mut out = format!("{}\n", lines.next().unwrap());
    for line in lines {
        let mut cells: Vec<_> = line.split('\t').collect();
        if cells[0] == "u" {
            cells[2] = state;
        }
        out.push_str(&cells.join("\t"));
        out.push('\n');
    }
    fs::write(store.join("units.tsv"), out).unwrap();
}

/// Another process appends a failing review and saves the state it derives, inside the
/// gap after `racer` wrote the row named by `marker` and before `racer` saves its state.
fn failing_review_lands_during(store: &Path, marker: &str, racer: impl FnOnce() + Send) {
    let file = store.join("reports/u.evidence.tsv");
    std::thread::scope(|scope| {
        let racer = scope.spawn(racer);
        while !racer.is_finished()
            && !fs::read_to_string(&file).is_ok_and(|t| t.contains(marker))
        {
            std::hint::spin_loop();
        }
        orch::with_lock(store, false, || {
            std::thread::sleep(Duration::from_millis(150));
            let mut text = fs::read_to_string(&file).unwrap();
            text.push_str("review\tclaude-review\t1\tnewer\t9\n");
            base::atomic_write(&file, text.as_bytes()).unwrap();
            save_state_by_hand(store, &evidence_state(store));
            Ok(())
        })
        .unwrap();
        racer.join().unwrap();
    });
}

#[test]
fn a_stale_state_cannot_hide_a_newer_failing_review() {
    for _ in 0..3 {
        let t = tempfile::tempdir().unwrap();
        let (store, repo) = unit_in_repo(t.path());
        let tree = orch::snapshot(&repo, &["f.txt".into()]);
        fs::write(
            store.join("reports/u.evidence.tsv"),
            format!("{EVIDENCE}run\tgpt-worker\t0\t\t1\nverify\ttest\t0\t{tree}\t2\n"),
        )
        .unwrap();
        failing_review_lands_during(&store, "\nreview\t", || {
            orch::record_review(&store, &["u".into()], "claude-review", true, &repo).unwrap();
        });
        assert_eq!(cached_state(&store), "review-failed");
        assert_eq!(cached_state(&store), evidence_state(&store));
    }
}

#[test]
fn a_stale_state_from_a_finished_unit_cannot_hide_a_newer_failing_review() {
    for _ in 0..3 {
        let t = tempfile::tempdir().unwrap();
        let (store, repo) = unit_in_repo(t.path());
        let attempt = match orch::begin_unit(&store, "u", "worker", &repo, false).unwrap() {
            Begin::Run { attempt_id, .. } => attempt_id,
            Begin::Reused => panic!("nothing to reuse"),
        };
        failing_review_lands_during(&store, "\nverify\t", || {
            orch::finish_unit(
                &store,
                "u",
                &attempt,
                &repo,
                "worker",
                "gpt-worker",
                &serde_json::json!({"status":"completed"}),
            )
            .unwrap();
        });
        assert_eq!(cached_state(&store), "review-failed");
        assert_eq!(cached_state(&store), evidence_state(&store));
    }
}

#[test]
fn the_goal_cannot_close_while_a_unit_attempt_is_running() {
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    fs::write(
        store.join("units.tsv"),
        format!("{HEADER}u\tm\tverified\t\t\t\t\n"),
    )
    .unwrap();
    fs::write(
        store.join("reports/_full.evidence.tsv"),
        format!("{EVIDENCE}full\ttest\t0\t\t9999\n"),
    )
    .unwrap();
    assert!(orch::done_gate(&store, true).is_ok());
    fs::write(
        store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\nu\t2-running\tf.txt\t2026-01-01T00:00:00Z\n",
    )
    .unwrap();
    let refused = orch::done_gate(&store, true).unwrap_err();
    assert_eq!(refused.kind, "not_done");
    assert!(refused.message.contains('u'), "{}", refused.message);
    fs::write(store.join("claims.tsv"), "unit\tattempt\tscope\tstarted\n").unwrap();
    assert!(orch::done_gate(&store, true).is_ok());
}

#[test]
fn a_verified_unit_cannot_be_set_back_by_hand() {
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    fs::write(
        store.join("units.tsv"),
        format!("{HEADER}done\tm\tverified\t\t\t\t\nopen\tm\tpending\t\t\t\t\n"),
    )
    .unwrap();
    let set = |unit: &str, state: &str| {
        orch::run(
            &store,
            &["unit", "set", unit, "--state", state].map(String::from),
            false,
            false,
        )
    };
    let refused = set("done", "pending");
    assert_eq!(refused.exit_code, 1);
    assert!(refused.stderr.contains("verified"), "{}", refused.stderr);
    let states: Vec<_> = orch::read_units(&store)
        .unwrap()
        .into_iter()
        .map(|u| u.state)
        .collect();
    assert_eq!(states, ["verified", "pending"]);
    assert_eq!(set("open", "review-failed").exit_code, 0);
}

fn inbox(store: &Path, args: &[&str]) -> orch::Output {
    let mut command = vec!["inbox".to_string()];
    command.extend(args.iter().map(|a| a.to_string()));
    orch::run(store, &command, true, false)
}

#[test]
fn inbox_readers_survive_a_concurrent_drain() {
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    fs::write(store.join("units.tsv"), HEADER).unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let failures = std::thread::scope(|s| {
        let mut jobs = vec![];
        for _ in 0..2 {
            jobs.push(s.spawn(|| {
                let mut failures = vec![];
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    for args in [&["drain", "--peek"][..], &["count"]] {
                        let out = inbox(&store, args);
                        if out.exit_code != 0 {
                            failures.push(out.stderr);
                        }
                    }
                }
                failures
            }));
        }
        let mut failures = vec![];
        for n in 0..150 {
            let unit = format!("u{n}");
            for args in [&["push", "worker", &unit, "done"][..], &["drain"]] {
                let out = inbox(&store, args);
                if out.exit_code != 0 {
                    failures.push(out.stderr);
                }
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        failures.extend(jobs.into_iter().flat_map(|j| j.join().unwrap()));
        failures
    });
    assert!(failures.is_empty(), "{} failed: {:?}", failures.len(), failures[0]);
}

#[test]
fn no_inbox_pointer_is_lost_between_processes_and_a_drain() {
    const PUSHERS: usize = 6;
    const EACH: usize = 25;
    if let Some((dir, i)) = child() {
        for n in 0..EACH {
            let pushed = inbox(
                &dir.join("store"),
                &["push", "worker", &format!("u{i}-{n}"), "done"],
            );
            assert_eq!(pushed.exit_code, 0, "{}", pushed.stderr);
        }
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    let seen = std::sync::Mutex::new(Vec::new());
    let finished = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut last_pass = false;
            loop {
                let drained = inbox(&store, &["drain"]);
                assert_eq!(drained.exit_code, 0, "{}", drained.stderr);
                let rows: serde_json::Value = serde_json::from_str(&drained.stdout).unwrap();
                seen.lock().unwrap().extend(
                    rows.as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r["unit"].as_str().unwrap().to_owned()),
                );
                if last_pass {
                    break;
                }
                last_pass = finished.load(std::sync::atomic::Ordering::SeqCst);
            }
        });
        children(
            "no_inbox_pointer_is_lost_between_processes_and_a_drain",
            t.path(),
            PUSHERS,
        );
        finished.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let seen = seen.into_inner().unwrap();
    let distinct: HashSet<_> = seen.iter().collect();
    assert_eq!(seen.len(), PUSHERS * EACH, "pointers lost or duplicated");
    assert_eq!(distinct.len(), PUSHERS * EACH);
}

#[cfg(windows)]
#[test]
fn a_pointer_held_open_does_not_fail_a_drain() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let t = tempfile::tempdir().unwrap();
    let store = empty_store(t.path());
    assert_eq!(inbox(&store, &["push", "worker", "u", "done"]).exit_code, 0);
    let pointer = fs::read_dir(store.join("inbox")).unwrap().next().unwrap().unwrap().path();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&pointer)
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(held);
    });
    let drained = inbox(&store, &["drain"]);
    release.join().unwrap();
    assert_eq!(drained.exit_code, 0, "{}", drained.stderr);
    let left: serde_json::Value =
        serde_json::from_str(&inbox(&store, &["count"]).stdout).unwrap();
    assert_eq!(left["count"], 0);
}
