use axiom::{
    engine,
    storage::{self, ledger, retention, Class, Ledger, Owner},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn owner() -> Owner {
    Owner {
        project: storage::SYSTEM_PROJECT.into(),
        wish: None,
    }
}

fn copy(home: &Path, version: &str, used: i64) -> PathBuf {
    let path = engine::install(home, version, &[("rules.md", "old")]).unwrap();
    let ledger = Ledger::open(home).unwrap();
    ledger
        .register(&path, Class::EngineCopy, &owner(), json!({}))
        .unwrap();
    ledger.touch(&path, used).unwrap();
    path
}

#[test]
fn engine_retention_keeps_current_previous_and_live_copies() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let current = engine::plugin_dir(home).unwrap();
    let old = copy(home, "0.0.1", 10);
    let live = copy(home, "0.0.2", 20);
    let previous = copy(home, "0.0.3", 30);
    let older = copy(home, "0.0.4", 5);
    let ledger = Ledger::open(home).unwrap();
    let _lease = ledger.lease(&live).unwrap();
    let records = engine::prune_engine_copies(home, 100).unwrap();
    for path in [&current, &previous, &live] {
        assert!(path.is_dir());
    }
    for path in [&old, &older] {
        assert!(!path.exists());
    }
    assert_eq!(fs::read_dir(home.join("engine")).unwrap().count(), 3);
    assert_eq!(records.len(), 2);
    assert!(records
        .iter()
        .all(|r| r.reason == "engine_retention" && r.outcome == "done"));
    let audit = fs::read_to_string(retention::audit_log(home)).unwrap();
    let audited: Vec<retention::AuditRecord> = audit
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for path in [&old, &older] {
        assert!(audited
            .iter()
            .any(|r| r.path == ledger::normalize(path).to_string_lossy()
                && r.reason == "engine_retention"
                && r.outcome == "done"));
    }
}

#[test]
fn plugin_dir_registers_engine_copy() {
    let temp = tempfile::tempdir().unwrap();
    let dir = engine::plugin_dir(temp.path()).unwrap();
    let ledger = Ledger::open(temp.path()).unwrap();
    let entry = ledger.get(&dir).unwrap().unwrap();
    assert_eq!(entry.class, Class::EngineCopy);
    assert_eq!(entry.owner, owner());
    assert!(entry.last_used_ms > 0);
    assert!(ledger.in_use(&dir).unwrap());
}

#[test]
fn prune_adopts_unregistered_copies() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let current = engine::plugin_dir(home).unwrap();
    let old = copy(home, "0.0.1", 1);
    let adopted = engine::install(home, "0.0.2", &[("rules.md", "unregistered")]).unwrap();
    let ledger = Ledger::open(home).unwrap();
    assert!(ledger.get(&adopted).unwrap().is_none());
    let records = engine::prune_engine_copies(home, 100).unwrap();
    let entry = ledger.get(&adopted).unwrap().unwrap();
    assert_eq!(entry.class, Class::EngineCopy);
    assert_eq!(entry.owner, owner());
    assert_eq!(entry.last_used_ms, 0);
    assert!(current.is_dir() && adopted.is_dir());
    assert!(!old.exists());
    assert_eq!(records.len(), 1);
}

#[test]
fn prune_never_touches_outside_engine() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    engine::plugin_dir(home).unwrap();
    let previous = copy(home, "0.0.1", 50);
    let old = copy(home, "0.0.2", 20);
    let outside = home.join("outside");
    fs::create_dir(&outside).unwrap();
    let ledger = Ledger::open(home).unwrap();
    ledger
        .register(&outside, Class::EngineCopy, &owner(), json!({}))
        .unwrap();
    ledger.touch(&outside, 1000).unwrap();
    let nested = previous.join("nested");
    fs::create_dir(&nested).unwrap();
    ledger
        .register(&nested, Class::EngineCopy, &owner(), json!({}))
        .unwrap();
    engine::prune_engine_copies(home, 100).unwrap();
    assert!(outside.is_dir() && nested.is_dir() && previous.is_dir());
    assert!(!old.exists());
}

#[test]
fn current_and_one_other_are_not_removed() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let current = engine::plugin_dir(home).unwrap();
    let previous = copy(home, "0.0.1", 1);
    assert!(engine::prune_engine_copies(home, 100).unwrap().is_empty());
    assert!(current.is_dir() && previous.is_dir());
}

#[test]
fn plugin_dir_prunes_after_installing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let old = copy(home, "0.0.1", 1);
    let previous = copy(home, "0.0.2", 2);
    let current = engine::plugin_dir(home).unwrap();
    assert!(current.is_dir() && previous.is_dir());
    assert!(!old.exists());
    let audit = fs::read_to_string(retention::audit_log(home)).unwrap();
    let record: retention::AuditRecord =
        serde_json::from_str(audit.lines().next().unwrap()).unwrap();
    assert_eq!(record.reason, "engine_retention");
    assert_eq!(record.outcome, "done");
}

#[test]
fn plugin_dir_succeeds_when_storage_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("storage"), "not a directory").unwrap();
    let current = engine::plugin_dir(temp.path()).unwrap();
    assert!(current.is_dir());
}
