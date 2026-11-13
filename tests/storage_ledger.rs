use axiom::storage::{usage, Class, EntryState, Ledger, Owner, Policy};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
    time::Duration,
};
fn owner(project: &str, wish: Option<&str>) -> Owner {
    Owner {
        project: project.into(),
        wish: wish.map(str::to_owned),
    }
}
fn write(p: &Path, data: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}
#[test]
fn ledger_records_all_storage_classes() {
    let t = tempfile::tempdir().unwrap();
    let l = Ledger::open(t.path()).unwrap();
    let o = owner("project", Some("wish"));
    for c in Class::ALL {
        let p = t.path().join(c.as_str());
        fs::create_dir_all(&p).unwrap();
        let e = l.register(&p, c, &o, json!({"a": 1})).unwrap();
        assert_eq!(e.class, c);
        assert_eq!(Class::parse(c.as_str()), Some(c));
        let e2 = l.register(&p, c, &o, json!({"b": 2})).unwrap();
        assert_eq!(e.id, e2.id);
        assert_eq!(e2.meta, json!({"a":1,"b":2}));
    }
    assert_eq!(l.entries().unwrap().len(), 9);
    let p = t.path().join("worktree");
    assert_eq!(
        l.register(&p, Class::Worktree, &owner("other", None), json!({}))
            .unwrap_err()
            .kind,
        "ownership_conflict"
    );
    assert!(l
        .register(
            &t.path().join("invalid"),
            Class::Logs,
            &owner("", None),
            json!({})
        )
        .is_err());
    l.touch(&p, 42).unwrap();
    assert_eq!(l.get(&p).unwrap().unwrap().last_used_ms, 42);
    l.set_meta(&p, json!({"c":3})).unwrap();
    assert_eq!(l.get(&p).unwrap().unwrap().meta["c"], 3);
    let lease = l.lease(&p.join("child")).unwrap();
    assert!(l.in_use(&p).unwrap());
    drop(lease);
    assert!(!l.in_use(&p).unwrap());
    l.mark_deleted(&p, 50).unwrap();
    assert_eq!(l.get(&p).unwrap().unwrap().state, EntryState::Deleted);
    assert_eq!(l.entries().unwrap().len(), 8);
}
#[test]
fn storage_usage_groups_without_double_counting() {
    let t = tempfile::tempdir().unwrap();
    let l = Ledger::open(t.path()).unwrap();
    let root = t.path().join("store");
    let nested = root.join("scratch");
    write(&root.join("a"), b"12345");
    write(&nested.join("b"), b"1234567");
    let a = owner("alpha", Some("w"));
    l.register(&root, Class::UnitStore, &a, json!({})).unwrap();
    l.register(&nested, Class::Scratch, &a, json!({})).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&nested, root.join("link")).unwrap();
    }
    assert_eq!(
        l.owner_of(&nested.join("b")).unwrap().unwrap().class,
        Class::Scratch
    );
    assert_eq!(l.measure(&root, 100).unwrap(), Some(5));
    assert_eq!(l.measure(&nested, 100).unwrap(), Some(7));
    let third = root.join("another");
    fs::create_dir_all(&third).unwrap();
    l.register(&third, Class::Evidence, &a, json!({})).unwrap();
    assert_eq!(l.get(&root).unwrap().unwrap().size_bytes, None);
    l.mark_deleted(&third, 101).unwrap();
    assert_eq!(l.measure(&root, 102).unwrap(), Some(5));
    let u = usage::summarize(&l, t.path(), &|e| e.class == Class::Scratch).unwrap();
    assert_eq!(u.total.bytes, 12);
    assert_eq!(u.total.entries, 2);
    assert_eq!(u.by_project["alpha"].bytes, 12);
    assert_eq!(u.by_wish["alpha/w"].bytes, 12);
    assert_eq!(u.by_class[&Class::UnitStore].bytes, 5);
    assert_eq!(u.reclaimable.bytes, 7);
    let rendered = usage::render_text(&u);
    for header in [
        "Class:\n",
        "Project:\n",
        "Wish:\n",
        "Total:",
        "Reclaimable:\n",
        "Untracked:\n",
    ] {
        assert!(rendered.contains(header), "missing {header}");
    }
}
#[test]
fn ledger_survives_concurrent_writers() {
    let t = tempfile::tempdir().unwrap();
    Ledger::open(t.path()).unwrap();
    let barrier = Arc::new(Barrier::new(5));
    let mut threads = Vec::new();
    for i in 0..4 {
        let home = t.path().to_owned();
        let b = barrier.clone();
        threads.push(std::thread::spawn(move || {
            let l = Ledger::open(&home).unwrap();
            b.wait();
            for j in 0..20 {
                let p = home.join(format!("entry-{i}-{j}"));
                l.register(&p, Class::Scratch, &owner("p", None), json!({}))
                    .unwrap();
                l.register(
                    &home.join("shared"),
                    Class::Logs,
                    &owner("p", None),
                    json!({format!("t{i}"):true}),
                )
                .unwrap();
            }
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }
    let other = Ledger::open(t.path()).unwrap();
    assert_eq!(other.entries().unwrap().len(), 81);
    let meta = other.get(&t.path().join("shared")).unwrap().unwrap().meta;
    for i in 0..4 {
        assert_eq!(meta[format!("t{i}")], true);
    }
}
#[test]
fn reconcile_adopts_layout() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    for p in [
        "engine/v1",
        "logs",
        "build-cache/key",
        "goals",
        "memory",
        "codex-slots",
        "council-running",
        "council-switch",
        "evolve",
        "hook",
        "service",
        "orchestrate/alpha/w1/scratch",
        "orchestrate/alpha/w1/reports",
        "orchestrate/alpha/w1/receipts",
        "unknown",
    ] {
        fs::create_dir_all(h.join(p)).unwrap();
    }
    let l = Ledger::open(h).unwrap();
    assert!(l.reconcile(h).unwrap() > 0);
    assert_eq!(l.reconcile(h).unwrap(), 0);
    for (p, c, project, wish) in [
        ("engine/v1", Class::EngineCopy, "_chronos", None),
        ("logs", Class::Logs, "_chronos", None),
        ("build-cache/key", Class::BuildCache, "_chronos", None),
        ("storage", Class::UnitStore, "_chronos", None),
        ("goals", Class::UnitStore, "_chronos", None),
        ("orchestrate/alpha", Class::UnitStore, "alpha", None),
        (
            "orchestrate/alpha/w1",
            Class::UnitStore,
            "alpha",
            Some("w1"),
        ),
        (
            "orchestrate/alpha/w1/scratch",
            Class::Scratch,
            "alpha",
            Some("w1"),
        ),
        (
            "orchestrate/alpha/w1/reports",
            Class::Evidence,
            "alpha",
            Some("w1"),
        ),
        (
            "orchestrate/alpha/w1/receipts",
            Class::Evidence,
            "alpha",
            Some("w1"),
        ),
    ] {
        let e = l.get(&h.join(p)).unwrap().unwrap();
        assert_eq!((e.class, e.owner), (c, owner(project, wish)));
    }
    assert_eq!(l.unregistered(h).unwrap(), vec![h.join("unknown")]);
    assert!(l
        .entries()
        .unwrap()
        .iter()
        .all(|e| !e.owner.project.is_empty()));
    let policy = Policy::load(h).unwrap();
    assert_eq!(policy.build_cache_days, 7);
    assert_eq!(policy.estimates.setup_step, 512 * 1024 * 1024);
}
#[test]
fn unmeasured_is_not_zero() {
    let t = tempfile::tempdir().unwrap();
    let l = Ledger::open(t.path()).unwrap();
    let p = t.path().join("missing");
    l.register(&p, Class::Download, &owner("p", None), json!({}))
        .unwrap();
    let u = usage::summarize(&l, t.path(), &|_| false).unwrap();
    assert_eq!(u.total.unmeasured, 1);
    assert_eq!(u.total.entries, 1);
    assert_eq!(l.measure(&p, 50).unwrap(), Some(0));
    assert_eq!(l.get(&p).unwrap().unwrap().measured_ms, None);
    fs::create_dir_all(&p).unwrap();
    assert_eq!(l.measure_stale(1, Duration::from_secs(5), 100).unwrap(), 1);
    let u = usage::summarize(&l, t.path(), &|_| false).unwrap();
    assert_eq!(u.total.unmeasured, 0);
    assert_eq!(u.total.bytes, 0);
}

#[test]
fn directory_helpers_register_and_log_conflicts() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let p = home.join("created/nested");
    let a = owner("alpha", None);
    assert_eq!(
        axiom::storage::ensure_dir(home, &p, Class::Scratch, &a).unwrap(),
        p
    );
    assert!(p.is_dir());
    assert_eq!(
        Ledger::open(home).unwrap().get(&p).unwrap().unwrap().owner,
        a
    );
    assert!(axiom::storage::ensure_dir(home, &p, Class::Scratch, &owner("other", None)).is_ok());
    assert!(fs::read_to_string(home.join("storage/errors.log"))
        .unwrap()
        .contains("ownership conflict"));
    let file = home.join("file");
    fs::write(&file, b"x").unwrap();
    assert!(axiom::storage::ensure_dir(home, &file.join("child"), Class::Scratch, &a).is_err());
    let missing = home.join("quiet");
    axiom::storage::register_quiet(home, &missing, Class::Scratch, &a, json!({}));
    assert!(!missing.exists());
    assert!(Ledger::open(home).unwrap().get(&missing).unwrap().is_some());
}

#[test]
fn policy_overrides_ignore_unknown_keys() {
    let t = tempfile::tempdir().unwrap();
    write(
        &t.path().join("storage/policy.json"),
        br#"{"build_cache_days":3,"unknown":42,"estimates":{"wish":123,"unknown":7}}"#,
    );
    let p = Policy::load(t.path()).unwrap();
    assert_eq!(p.build_cache_days, 3);
    assert_eq!(p.estimates.wish, 123);
    assert_eq!(p.compress_after_days, Policy::default().compress_after_days);
    assert_eq!(p.estimates.batch, Policy::default().estimates.batch);
}

#[test]
fn dead_leases_are_pruned_and_zero_budget_does_nothing() {
    let t = tempfile::tempdir().unwrap();
    let l = Ledger::open(t.path()).unwrap();
    let p = t.path().join("item");
    fs::create_dir_all(&p).unwrap();
    l.register(&p, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    let db = rusqlite::Connection::open(t.path().join("storage/ledger.sqlite3")).unwrap();
    db.execute(
        "INSERT INTO ledger_leases(path,pid) VALUES(?1,?2)",
        rusqlite::params![p.to_string_lossy(), u32::MAX],
    )
    .unwrap();
    assert!(!l.in_use(&p).unwrap());
    assert_eq!(
        db.query_row("SELECT count(*) FROM ledger_leases", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(l.measure_stale(0, Duration::ZERO, 100).unwrap(), 0);
    assert_eq!(l.get(&p).unwrap().unwrap().measured_ms, None);
}

#[test]
fn links_are_not_followed_when_measuring() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("root");
    let outside = t.path().join("outside");
    write(&root.join("file"), b"abc");
    write(&outside.join("file"), b"123456789");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("link"))
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success());
    let l = Ledger::open(t.path()).unwrap();
    l.register(&root, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    assert_eq!(l.measure(&root, 1).unwrap(), Some(3));
}

#[cfg(windows)]
#[test]
fn windows_keys_ignore_case_but_preserve_path_case() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("MiXeD-Not-Created");
    let alternate = t.path().join("mixed-not-created");
    let l = Ledger::open(t.path()).unwrap();
    let first = l
        .register(&p, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    assert_eq!(first.path.file_name().unwrap(), "MiXeD-Not-Created");
    let second = l
        .register(&alternate, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(l.get(&alternate).unwrap().unwrap().path, first.path);
    assert_eq!(l.entries().unwrap().len(), 1);
    fs::create_dir_all(&p).unwrap();
    assert_eq!(
        l.owner_of(&alternate.join("child")).unwrap().unwrap().id,
        first.id
    );
    let lease = l.lease(&alternate.join("child")).unwrap();
    assert!(l.in_use(&p).unwrap());
    drop(lease);
}

#[test]
fn unregistered_reports_uncovered_children_of_partially_registered_dirs() {
    let t = tempfile::tempdir().unwrap();
    let parent = t.path().join("partial");
    let covered = parent.join("covered");
    let uncovered = parent.join("uncovered");
    fs::create_dir_all(&covered).unwrap();
    fs::create_dir_all(&uncovered).unwrap();
    let l = Ledger::open(t.path()).unwrap();
    l.register(&covered, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    assert!(l.unregistered(t.path()).unwrap().contains(&uncovered));
    assert!(uncovered.exists());
    assert!(!l.unregistered(t.path()).unwrap().contains(&covered));
}

#[test]
fn stale_measurement_errors_are_logged_and_do_not_abort() {
    let t = tempfile::tempdir().unwrap();
    let l = Ledger::open(t.path()).unwrap();
    let bad = t.path().join("a-bad");
    let good = t.path().join("z-good");
    fs::create_dir_all(&bad).unwrap();
    write(&good.join("file"), b"abc");
    let bad_entry = l
        .register(&bad, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    l.register(&good, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    let db = rusqlite::Connection::open(t.path().join("storage/ledger.sqlite3")).unwrap();
    db.execute_batch(&format!("CREATE TRIGGER fail_measure BEFORE UPDATE OF size_bytes ON ledger_entries WHEN OLD.id={} BEGIN SELECT RAISE(FAIL, 'measurement test failure'); END;", bad_entry.id)).unwrap();
    assert_eq!(l.measure_stale(0, Duration::from_secs(5), 100).unwrap(), 2);
    assert_eq!(l.get(&bad).unwrap().unwrap().measured_ms, None);
    assert_eq!(l.get(&good).unwrap().unwrap().size_bytes, Some(3));
    assert!(fs::read_to_string(t.path().join("storage/errors.log"))
        .unwrap()
        .contains("measurement test failure"));
}

#[test]
fn measure_walk_does_not_hold_write_lock() {
    assert!(
        register_finishes_during_walk(false),
        "register did not finish while the directory walk was held open"
    );
}

#[test]
fn measure_walk_lock_check_detects_held_write_lock() {
    assert!(
        !register_finishes_during_walk(true),
        "lock check failed to detect a write lock held during the walk"
    );
}

fn register_finishes_during_walk(hold_write_lock: bool) -> bool {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("root");
    fs::create_dir_all(&root).unwrap();
    for i in 0..50 {
        fs::write(root.join(format!("file-{i}")), b"x").unwrap();
    }
    let l = Ledger::open(t.path()).unwrap();
    l.register(&root, Class::Scratch, &owner("p", None), json!({}))
        .unwrap();
    let received_registered = Arc::new(AtomicBool::new(false));
    let (walk_started_tx, walk_started_rx) = mpsc::channel();
    let (registered_tx, registered_rx) = mpsc::channel();
    let home = t.path().to_owned();
    let received = received_registered.clone();
    let thread = std::thread::spawn(move || {
        let l = Ledger::open(&home).unwrap();
        l.measure_with_walk_hook(&root, 1, &|| {
            let db = if hold_write_lock {
                let db = rusqlite::Connection::open(home.join("storage/ledger.sqlite3")).unwrap();
                db.execute_batch("BEGIN IMMEDIATE").unwrap();
                Some(db)
            } else {
                None
            };
            walk_started_tx.send(()).unwrap();
            let timeout = if hold_write_lock {
                Duration::from_millis(300)
            } else {
                Duration::from_secs(20)
            };
            received.store(
                registered_rx.recv_timeout(timeout).is_ok(),
                Ordering::SeqCst,
            );
            if let Some(db) = db {
                db.execute_batch("ROLLBACK").unwrap();
            }
        })
    });
    walk_started_rx
        .recv_timeout(Duration::from_secs(20))
        .expect("directory walk did not start");
    let registered = l.register(
        &t.path().join("other"),
        Class::Logs,
        &owner("p", None),
        json!({}),
    );
    // The negative control times out and may drop its receiver before register returns.
    let _ = registered_tx.send(());
    let measured = thread.join().unwrap();
    assert!(registered.is_ok(), "register failed: {registered:?}");
    assert_eq!(measured.unwrap(), Some(50));
    received_registered.load(Ordering::SeqCst)
}
