use axiom::storage::{
    archive, ledger,
    retention::{self, Action, Mode},
    Class, Ledger, Owner, Policy,
};
use serde_json::json;
use std::{
    fs,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
fn owner() -> Owner {
    Owner {
        project: "project".into(),
        wish: Some("wish".into()),
    }
}
fn register(home: &Path, name: &str, class: Class, meta: serde_json::Value) -> std::path::PathBuf {
    let p = home.join(name);
    fs::create_dir_all(&p).unwrap();
    let l = Ledger::open(home).unwrap();
    l.register(&p, class, &owner(), meta).unwrap();
    p
}
fn old(l: &Ledger, p: &Path, t: i64) {
    l.touch(p, t - 1_000_000_000).unwrap();
}
fn records(home: &Path) -> Vec<retention::AuditRecord> {
    fs::read_to_string(retention::audit_log(home))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

#[test]
fn pressure_cleanup_only_removes_safe_classes() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let l = Ledger::open(h).unwrap();
    let scratch = register(h, "scratch", Class::Scratch, json!({}));
    old(&l, &scratch, n);
    let cache = register(
        h,
        "cache",
        Class::BuildCache,
        json!({"worktree_state":"merged","state_since_ms":n}),
    );
    let shared = register(
        h,
        "shared",
        Class::BuildCache,
        json!({"worktree_state":"gone","state_since_ms":0,"shared":true,"users":["worktree"]}),
    );
    let active = register(
        h,
        "active",
        Class::BuildCache,
        json!({"worktree_state":"gone","state_since_ms":0}),
    );
    let _lease = l.lease(&active).unwrap();
    let current = register(
        h,
        &format!(
            "engine/{}-{}",
            env!("CARGO_PKG_VERSION"),
            axiom::engine::build_id()
        ),
        Class::EngineCopy,
        json!({}),
    );
    let previous = register(h, "engine/previous", Class::EngineCopy, json!({}));
    l.touch(&previous, n - 10).unwrap();
    let obsolete = register(h, "engine/obsolete", Class::EngineCopy, json!({}));
    l.touch(&obsolete, n - 100).unwrap();
    let used = register(h, "engine/used", Class::EngineCopy, json!({}));
    l.touch(&used, n - 200).unwrap();
    let _engine_lease = l.lease(&used).unwrap();
    let routine = retention::plan(h, &Policy::default(), Mode::Routine, n).unwrap();
    assert!(!routine
        .items
        .iter()
        .any(|i| i.path == ledger::normalize(&cache)));
    let out = retention::pressure_cleanup(h, &Policy::default(), n).unwrap();
    for p in [&scratch, &cache, &obsolete] {
        assert!(!p.exists(), "{}", p.display());
    }
    for p in [&shared, &active, &current, &previous, &used] {
        assert!(p.exists(), "{}", p.display());
    }
    assert!(out
        .records
        .iter()
        .all(|r| r.action == Action::Delete && retention::SAFE_CLASSES.contains(&r.class)));
}

#[test]
fn cleanup_rejects_link_escape_and_changed_ownership() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let l = Ledger::open(h).unwrap();
    let changed_class = register(h, "changed_class", Class::Scratch, json!({}));
    let changed_owner = register(h, "changed_owner", Class::Scratch, json!({}));
    old(&l, &changed_class, n);
    old(&l, &changed_owner, n);
    let planned = retention::plan(h, &Policy::default(), Mode::Pressure, n).unwrap();
    // Registration refuses class and ownership changes; simulate concurrent updates directly.
    let db = rusqlite::Connection::open(h.join("storage/ledger.sqlite3")).unwrap();
    for (path, sql) in [
        (
            &changed_class,
            "UPDATE ledger_entries SET class='download' WHERE path=?1",
        ),
        (
            &changed_owner,
            "UPDATE ledger_entries SET project='other' WHERE path=?1",
        ),
    ] {
        assert_eq!(
            db.execute(sql, [ledger::normalize(path).to_string_lossy().as_ref()])
                .unwrap(),
            1
        );
        let item = planned
            .items
            .iter()
            .find(|i| i.path == ledger::normalize(path))
            .unwrap();
        assert_eq!(
            l.get(path).unwrap().unwrap().last_used_ms,
            item.last_used_ms
        );
    }
    let out = retention::execute(h, &planned, n, &AtomicBool::new(false)).unwrap();
    for path in [&changed_class, &changed_owner] {
        let record = out
            .records
            .iter()
            .find(|r| r.path == ledger::normalize(path).to_string_lossy())
            .unwrap();
        assert!(record.outcome.starts_with("skipped:"));
        assert!(record.outcome.contains("class or owner changed"));
        assert!(path.exists());
    }
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("keep"), b"safe").unwrap();
        let link = register(h, "link", Class::Scratch, json!({}));
        old(&l, &link, n);
        let plan = retention::plan(h, &Policy::default(), Mode::Pressure, n).unwrap();
        fs::remove_dir(&link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "junction creation failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let out = retention::execute(h, &plan, n, &AtomicBool::new(false)).unwrap();
        assert!(out
            .records
            .iter()
            .find(|r| r.path == ledger::normalize(h).join("link").to_string_lossy())
            .unwrap()
            .outcome
            .starts_with("skipped:"));
        assert!(outside.path().join("keep").exists());
    }
}

#[test]
fn routine_retains_recent_scratch_and_shared_cache_users() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let policy = Policy::default();
    let since = n - (policy.build_cache_days as i64 + 1) * 86_400_000;
    let cache = register(
        h,
        "merged",
        Class::BuildCache,
        json!({"worktree_state":"merged", "state_since_ms":since}),
    );
    let unused = register(
        h,
        "unused",
        Class::BuildCache,
        json!({"worktree_state":"merged", "state_since_ms":since, "shared":true, "users":[]}),
    );
    let busy = register(
        h,
        "busy",
        Class::BuildCache,
        json!({"worktree_state":"merged", "state_since_ms":since, "shared":true, "users":["worktree"]}),
    );
    let recent = register(h, "recent", Class::Scratch, json!({}));
    let l = Ledger::open(h).unwrap();
    l.touch(&recent, n - 599_999).unwrap();

    for (path, expected) in [
        (&cache, true),
        (&unused, true),
        (&busy, false),
        (&recent, false),
    ] {
        assert_eq!(
            retention::is_reclaimable(&l.get(path).unwrap().unwrap(), &policy, n),
            expected
        );
    }
    let plan = retention::plan(h, &policy, Mode::Routine, n).unwrap();
    for path in [&cache, &unused] {
        let item = plan
            .items
            .iter()
            .find(|i| i.path == ledger::normalize(path))
            .unwrap();
        assert_eq!(item.action, Action::Delete);
        assert_eq!(item.reason, "merged_worktree_cache");
    }
    for path in [&busy, &recent] {
        assert!(!plan.items.iter().any(|i| i.path == ledger::normalize(path)));
    }
    let out = retention::execute(h, &plan, n, &AtomicBool::new(false)).unwrap();
    for path in [&cache, &unused] {
        let record = out
            .records
            .iter()
            .find(|r| r.path == ledger::normalize(path).to_string_lossy())
            .unwrap();
        assert_eq!(record.outcome, "done");
        assert_eq!(record.reason, "merged_worktree_cache");
        assert!(!path.exists());
        assert!(!retention::is_reclaimable(
            &l.get(path).unwrap().unwrap(),
            &policy,
            n
        ));
    }
    assert!(busy.exists());
    assert!(recent.exists());
}

#[test]
fn compression_skips_logs_leased_after_planning() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let logs = register(h, "logs", Class::Logs, json!({}));
    let file = logs.join("old.log");
    fs::write(&file, b"keep").unwrap();
    fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(31 * 86400))
        .unwrap();
    let plan = retention::plan(h, &Policy::default(), Mode::Routine, n).unwrap();
    assert!(plan
        .items
        .iter()
        .any(|i| i.path == ledger::normalize(&file) && i.action == Action::Compress));
    let l = Ledger::open(h).unwrap();
    let _lease = l.lease(&logs).unwrap();
    let out = retention::execute(h, &plan, n, &AtomicBool::new(false)).unwrap();
    let record = out
        .records
        .iter()
        .find(|r| r.path == ledger::normalize(&file).to_string_lossy())
        .unwrap();
    assert_eq!(record.outcome, "skipped: in use");
    assert_eq!(fs::read(&file).unwrap(), b"keep");
    assert!(!archive::archived_path(&file).exists());
}

#[test]
fn receipts_compress_after_30_days_without_loss() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let logs = register(h, "logs", Class::Logs, json!({}));
    let ev = register(h, "evidence", Class::Evidence, json!({}));
    for p in [
        logs.join("old.log"),
        ev.join("receipt.json"),
        logs.join("events.jsonl"),
        ev.join("item.lock"),
    ] {
        fs::write(&p, b"recoverable content").unwrap();
        fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(31 * 86400))
            .unwrap();
    }
    let plan = retention::plan(h, &Policy::default(), Mode::Routine, n).unwrap();
    retention::execute(h, &plan, n, &AtomicBool::new(false)).unwrap();
    for p in [logs.join("old.log"), ev.join("receipt.json")] {
        assert!(!p.exists());
        assert_eq!(archive::read(&p).unwrap(), b"recoverable content");
    }
    assert!(logs.join("events.jsonl").exists());
    assert!(ev.join("item.lock").exists());
}

#[test]
fn retention_spares_project_data() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let mut paths = Vec::new();
    for (i, c) in [
        Class::UnitStore,
        Class::Logs,
        Class::Evidence,
        Class::ResearchCache,
        Class::Download,
    ]
    .into_iter()
    .enumerate()
    {
        paths.push(register(h, &format!("protected{i}"), c, json!({})));
    }
    let unregistered = h.join("project");
    fs::create_dir(&unregistered).unwrap();
    paths.push(unregistered);
    let l = Ledger::open(h).unwrap();
    for p in &paths {
        old(&l, p, n);
    }
    retention::pressure_cleanup(h, &Policy::default(), n).unwrap();
    assert!(paths.iter().all(|p| p.exists()));
    // A safe-class parent must not erase a separately registered protected descendant.
    let parent = register(h, "parent", Class::Scratch, json!({}));
    let child = register(h, "parent/data", Class::UnitStore, json!({}));
    old(&l, &parent, n);
    retention::pressure_cleanup(h, &Policy::default(), n).unwrap();
    assert!(child.exists());
}

#[test]
fn scratch_outside_home_requires_scratch_ancestor_and_class() {
    // The inherited system temp directory may itself be inside a scratch directory.
    let t = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let h = t.path().join("home");
    let l = Ledger::open(&h).unwrap();
    for (name, class, expected) in [
        ("store/scratch/u/a", Class::Scratch, "done"),
        (
            "elsewhere/u",
            Class::Scratch,
            "skipped: outside permitted roots",
        ),
        (
            "store/scratch/u/worktree",
            Class::Worktree,
            "skipped: class or owner changed",
        ),
        (
            "store/scratch/u/logs",
            Class::Logs,
            "skipped: class or owner changed",
        ),
    ] {
        let path = t.path().join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("data"), b"keep unless scratch").unwrap();
        l.register(&path, class, &owner(), json!({})).unwrap();
        let record = retention::remove_scratch(&h, &path, &owner(), "unit_ended", false);
        assert_eq!(record.outcome, expected, "{name}");
        assert_eq!(path.exists(), expected != "done", "{name}");
        if matches!(class, Class::Worktree | Class::Logs) {
            assert_eq!(
                retention::remove_entry(&h, &path, "unit_ended", now()).outcome,
                "skipped: class is protected"
            );
            assert_eq!(fs::read(path.join("data")).unwrap(), b"keep unless scratch");
        }
    }
}

#[test]
fn every_deletion_is_audited() {
    let t = tempfile::tempdir().unwrap();
    let h = t.path();
    let n = now();
    let l = Ledger::open(h).unwrap();
    let a = register(h, "a", Class::Scratch, json!({}));
    let b = register(h, "b", Class::Scratch, json!({}));
    old(&l, &a, n);
    old(&l, &b, n);
    let p = retention::plan(h, &Policy::default(), Mode::Pressure, n).unwrap();
    l.touch(&b, n).unwrap();
    let out = retention::execute(h, &p, n, &AtomicBool::new(false)).unwrap();
    assert_eq!(out.records.len(), 2);
    assert_eq!(records(h).len(), 2);
    assert!(out
        .records
        .iter()
        .any(|r| r.outcome.starts_with("skipped:")));
    let c = register(h, "c", Class::Scratch, json!({}));
    assert_eq!(
        retention::remove_scratch(h, &c, &owner(), "unit_ended", false).outcome,
        "done"
    );
    let d = register(h, "d", Class::Scratch, json!({}));
    assert_eq!(
        retention::remove_scratch(h, &d, &owner(), "unit_ended", true).outcome,
        "cancelled"
    );
    let missing = register(h, "missing", Class::Scratch, json!({}));
    fs::remove_dir(&missing).unwrap();
    assert!(
        retention::remove_scratch(h, &missing, &owner(), "unit_ended", false)
            .outcome
            .starts_with("failed:")
    );
    let protected = register(h, "download", Class::Download, json!({}));
    assert!(
        retention::remove_scratch(h, &protected, &owner(), "unit_ended", false)
            .outcome
            .starts_with("skipped:")
    );
    assert!(protected.exists());
    let wrong = register(h, "wrong", Class::Scratch, json!({}));
    let other = Owner {
        project: "other".into(),
        wish: None,
    };
    assert!(
        retention::remove_scratch(h, &wrong, &other, "unit_ended", false)
            .outcome
            .starts_with("skipped:")
    );
    assert!(wrong.exists());
    assert_eq!(records(h).len(), 7);
}
