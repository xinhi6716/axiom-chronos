use super::storage::LedgerStorage;
use crate::{
    desktop::{
        doctor::storage::{group, item_size_text, GroupBy},
        i18n::Lang,
        ux::seams::{CleanupProgress, StorageService},
    },
    storage::{Class, Ledger, Owner},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn owner() -> Owner {
    Owner {
        project: "project".into(),
        wish: Some("wish".into()),
    }
}

/// A registered scratch directory that is old enough to be reclaimable.
fn scratch(home: &Path, name: &str) -> PathBuf {
    let path = home.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("data.bin"), vec![7u8; 100]).unwrap();
    let ledger = Ledger::open(home).unwrap();
    ledger
        .register(&path, Class::Scratch, &owner(), json!({}))
        .unwrap();
    ledger.touch(&path, 1).unwrap();
    path
}

fn id_of(storage: &LedgerStorage, name: &str) -> String {
    storage
        .report()
        .unwrap()
        .items
        .into_iter()
        .find(|i| i.path.ends_with(name))
        .unwrap()
        .id
}

fn wait(storage: &LedgerStorage, job: &str) -> CleanupProgress {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let progress = storage.progress(job).unwrap();
        if progress.finished {
            return progress;
        }
        assert!(Instant::now() < deadline, "cleanup did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn unknown_size_is_unknown_and_excluded_from_totals() {
    let home = tempfile::tempdir().unwrap();
    let path = scratch(home.path(), "unmeasured");
    let storage = LedgerStorage::new(home.path());
    let item = storage.report().unwrap().items.remove(0);
    assert!(!item.measured);
    assert_eq!(item.bytes, 0);
    assert!(item.reclaimable);
    assert_eq!(item_size_text(&item, Lang::En), "unknown");
    assert_eq!(item_size_text(&item, Lang::ZhTw), "未知");
    assert_eq!(group(&[item], GroupBy::Category)[0].1, 0);

    // Negative control: once measured, the size is real and counted.
    Ledger::open(home.path())
        .unwrap()
        .measure(&path, 5)
        .unwrap();
    let item = storage.report().unwrap().items.remove(0);
    assert!(item.measured);
    assert_eq!(item.bytes, 100);
    assert_eq!(group(&[item], GroupBy::Category)[0].1, 100);
}

#[test]
fn clean_requires_confirmation_and_a_fresh_plan() {
    let home = tempfile::tempdir().unwrap();
    let first = scratch(home.path(), "first");
    let storage = LedgerStorage::new(home.path());
    let id = id_of(&storage, "first");

    let plan = storage.preview(std::slice::from_ref(&id)).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(
        storage
            .clean(&plan.id, plan.version, false)
            .unwrap_err()
            .kind,
        "unconfirmed"
    );
    assert_eq!(
        storage
            .clean("missing", plan.version, true)
            .unwrap_err()
            .kind,
        "stale"
    );
    assert_eq!(
        storage
            .clean(&plan.id, plan.version.wrapping_add(1), true)
            .unwrap_err()
            .kind,
        "stale"
    );
    // The ledger changes after the preview, so the plan is stale and nothing is deleted.
    scratch(home.path(), "second");
    assert_eq!(
        storage
            .clean(&plan.id, plan.version, true)
            .unwrap_err()
            .kind,
        "stale"
    );
    assert!(first.exists());

    // Negative control: a fresh plan over the current ledger runs and deletes.
    let plan = storage.preview(&[id.clone()]).unwrap();
    let job = storage.clean(&plan.id, plan.version, true).unwrap();
    let progress = wait(&storage, &job);
    assert_eq!(progress.done, vec![id]);
    assert!(progress.failed.is_empty());
    assert_eq!(progress.remaining, 0);
    assert!(!first.exists());
}

#[test]
fn in_use_data_is_never_deleted() {
    let home = tempfile::tempdir().unwrap();
    let busy = scratch(home.path(), "busy");
    let free = scratch(home.path(), "free");
    let storage = LedgerStorage::new(home.path());
    let ids = vec![id_of(&storage, "busy"), id_of(&storage, "free")];
    let plan = storage.preview(&ids).unwrap();
    assert_eq!(plan.items.len(), 2);

    let ledger = Ledger::open(home.path()).unwrap();
    let lease = ledger.lease(&busy).unwrap();
    let job = storage.clean(&plan.id, plan.version, true).unwrap();
    let progress = wait(&storage, &job);
    assert!(busy.join("data.bin").exists());
    assert!(!free.exists());
    assert_eq!(progress.done.len(), 1);
    assert!(progress.done[0].ends_with("free"));
    assert_eq!(progress.failed.len(), 1);
    assert!(progress.failed[0].0.ends_with("busy"));
    assert_eq!(progress.failed[0].1, "protected");
    let records: Vec<crate::storage::retention::AuditRecord> =
        fs::read_to_string(crate::storage::retention::audit_log(home.path()))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    assert_eq!(records.len(), 2);
    assert!(records
        .iter()
        .any(|r| r.path.ends_with("busy") && r.outcome == "skipped: in use"));
    assert!(records
        .iter()
        .any(|r| r.path.ends_with("free") && r.outcome == "done"));
    drop(lease);

    // A leased entry is also left out of a new preview.
    let lease = ledger.lease(&busy).unwrap();
    assert!(storage
        .preview(&[id_of(&storage, "busy")])
        .unwrap()
        .items
        .is_empty());
    drop(lease);
}

#[test]
fn changing_an_existing_ledger_entry_makes_the_plan_stale() {
    let home = tempfile::tempdir().unwrap();
    let path = scratch(home.path(), "first");
    let storage = LedgerStorage::new(home.path());
    let id = id_of(&storage, "first");
    let plan = storage.preview(&[id]).unwrap();
    Ledger::open(home.path()).unwrap().touch(&path, 2).unwrap();
    assert_eq!(
        storage
            .clean(&plan.id, plan.version, true)
            .unwrap_err()
            .kind,
        "stale"
    );
    assert!(path.exists());
}

#[test]
fn finished_progress_ignores_later_audit_records_for_the_same_path() {
    let home = tempfile::tempdir().unwrap();
    let path = scratch(home.path(), "first");
    let storage = LedgerStorage::new(home.path());
    let id = id_of(&storage, "first");
    let plan = storage.preview(&[id.clone()]).unwrap();
    let job = storage.clean(&plan.id, plan.version, true).unwrap();
    let completed = wait(&storage, &job);
    assert_eq!(completed.done, vec![id]);
    crate::storage::retention::remove_entry(home.path(), &path, "later operation", 3);
    assert_eq!(storage.progress(&job).unwrap(), completed);
}

#[test]
fn space_comes_from_the_disk_probe() {
    let home = tempfile::tempdir().unwrap();
    let storage = LedgerStorage::new(home.path());
    let space = storage.space().unwrap();
    assert!(space.free_bytes > 0);
    assert!(storage.report().unwrap().space.is_some());
    assert!(storage.progress("none").is_err());
}
