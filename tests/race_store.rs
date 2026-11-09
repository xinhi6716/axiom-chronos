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
