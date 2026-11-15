use super::{extreme::*, storage::*};
use crate::desktop::{
    i18n::Lang,
    ux::seams::{
        CleanupPlan, CleanupProgress, DiskSpace, FakeStorage, StorageItem, StorageReport,
        StorageService, TurnPolicy,
    },
};
use std::{
    prelude::v1::test,
    sync::Mutex,
    time::{Duration, Instant},
};
fn item(
    id: &str,
    category: &str,
    project: Option<&str>,
    wish: Option<&str>,
    bytes: u64,
    reclaimable: bool,
) -> StorageItem {
    StorageItem {
        id: id.into(),
        category: category.into(),
        project: project.map(str::to_owned),
        wish: wish.map(str::to_owned),
        path: format!("/{id}"),
        bytes,
        measured: true,
        reclaimable,
    }
}
#[test]
fn low_disk_thresholds() {
    assert!(is_low(&DiskSpace {
        free_bytes: LOW_FREE_BYTES - 1,
        total_bytes: LOW_FREE_BYTES
    }));
    assert!(!is_low(&DiskSpace {
        free_bytes: LOW_FREE_BYTES,
        total_bytes: LOW_FREE_BYTES * 20
    }));
    assert!(is_low(&DiskSpace {
        free_bytes: LOW_FREE_BYTES,
        total_bytes: LOW_FREE_BYTES * 20 + 1
    }));
    assert!(!is_low(&DiskSpace {
        free_bytes: LOW_FREE_BYTES,
        total_bytes: 0
    }));
    let free = u64::MAX / 20;
    assert!(is_low(&DiskSpace {
        free_bytes: free,
        total_bytes: free * 20 + 1
    }));
    assert!(!is_low(&DiskSpace {
        free_bytes: free,
        total_bytes: free * 20
    }));
}
#[test]
fn groups_are_ordered_and_sum_reclaimable() {
    let items = vec![
        item("a", "z", Some("p"), Some("w"), 10, true),
        item("b", "a", Some("p"), Some("x"), 20, false),
        item("c", "z", None, Some("w"), 30, true),
    ];
    assert_eq!(
        group(&items, GroupBy::Category),
        vec![("a".into(), 20, 0), ("z".into(), 40, 40)]
    );
    assert_eq!(
        group(&items, GroupBy::Project),
        vec![("".into(), 30, 30), ("p".into(), 30, 10)]
    );
    assert_eq!(
        group(&items, GroupBy::Wish),
        vec![("w".into(), 40, 40), ("x".into(), 20, 0)]
    );
}
struct RecordingStorage {
    inner: FakeStorage,
    submissions: Mutex<Vec<(String, u64, bool)>>,
}
impl StorageService for RecordingStorage {
    fn report(&self) -> crate::base::Res<StorageReport> {
        self.inner.report()
    }
    fn preview(&self, ids: &[String]) -> crate::base::Res<CleanupPlan> {
        self.inner.preview(ids)
    }
    fn clean(&self, id: &str, version: u64, confirmed: bool) -> crate::base::Res<String> {
        self.submissions
            .lock()
            .unwrap()
            .push((id.into(), version, confirmed));
        self.inner.clean(id, version, confirmed)
    }
    fn progress(&self, job: &str) -> crate::base::Res<CleanupProgress> {
        self.inner.progress(job)
    }
    fn space(&self) -> Option<DiskSpace> {
        self.inner.space()
    }
}
#[test]
fn cleanup_requires_confirmation_and_preserves_plan() {
    let service = RecordingStorage {
        inner: FakeStorage::new(vec![item("a", "c", None, None, 5, true)], None),
        submissions: Mutex::new(vec![]),
    };
    assert_eq!(Flow::Idle.confirm(&service), Flow::Idle);
    let selecting = Flow::Selecting(vec!["a".into()]);
    assert_eq!(selecting.clone().confirm(&service), selecting);
    let preview = Flow::Selecting(vec!["a".into()]).preview(&service);
    let Flow::Previewed(plan) = &preview else {
        panic!()
    };
    assert!(service.submissions.lock().unwrap().is_empty());
    let expected = plan.clone();
    let Flow::Running { plan, .. } = preview.confirm(&service) else {
        panic!()
    };
    assert_eq!(
        (plan.id, plan.version),
        (expected.id.clone(), expected.version)
    );
    assert_eq!(
        *service.submissions.lock().unwrap(),
        vec![(expected.id, expected.version, true)]
    );
    assert!(service.inner.deleted().is_empty());
    let stale = Flow::Selecting(vec!["a".into()]).preview(&service);
    service.inner.touch();
    assert!(matches!(stale.confirm(&service), Flow::Failed(_)));
}
#[test]
fn flash_and_policy_snapshots() {
    let now = Instant::now();
    assert_eq!(flash_level(Some(now), now, true), 0.);
    assert_eq!(flash_level(Some(now), now, false), 1.);
    assert_eq!(flash_level(Some(now), now + FLASH, false), 0.);
    assert!(
        flash_level(Some(now), now + Duration::from_millis(100), false)
            > flash_level(Some(now), now + Duration::from_millis(300), false)
    );
    let p = PrefsTurnPolicy::default();
    let first = p.begin_turn();
    let next = p.set_extreme(true);
    assert!(!first.extreme);
    assert!(next.extreme);
    assert_eq!(next.revision, 1);
    assert_eq!(p.begin_turn(), next);
    assert_eq!(p.set_extreme(true).revision, 1);
    assert_eq!(p.fast_cost(), None);
}
#[test]
fn sizes_are_localized() {
    assert_eq!(bytes_text(12, Lang::En), "12 B");
    assert_eq!(bytes_text(12, Lang::ZhTw), "12 位元組");
    assert_eq!(bytes_text(1024, Lang::En), "1.0 KiB");
    assert_eq!(bytes_text(1024, Lang::ZhTw), "1.0 KiB");
}
