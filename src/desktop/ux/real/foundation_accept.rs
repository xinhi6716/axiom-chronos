use crate::desktop::{
    doctor::storage::{group, item_size_text, GroupBy},
    i18n::Lang,
    ux::seams::{
        BackgroundService, FakeBackground, FakeStorage, FakeTurnPolicy, FakeUnitActions,
        StorageItem, StorageService, TurnPolicy, UnitActionKind, UnitActions, UnitEvidence,
    },
};

#[test]
fn waive_accepts_a_finding() {
    let actions = FakeUnitActions::new();
    actions.set_evidence("session", "unit", UnitEvidence::default());
    let kind = UnitActionKind::Waive {
        reason: "reviewed".into(),
        finding: Some(3),
    };
    assert!(matches!(
        &kind,
        UnitActionKind::Waive {
            finding: Some(3),
            ..
        }
    ));
    assert!(
        actions
            .act("session", "unit", &kind, "key")
            .unwrap()
            .accepted
    );
    assert_eq!(actions.count("unit", "waive"), 1);
}

fn item(id: &str, bytes: u64, measured: bool) -> StorageItem {
    StorageItem {
        id: id.into(),
        category: "cache".into(),
        project: Some("project".into()),
        wish: Some("wish".into()),
        path: id.into(),
        bytes,
        measured,
        reclaimable: true,
    }
}

#[test]
fn unmeasured_storage_is_excluded_and_displayed_as_unknown() {
    let measured = item("measured", 12, true);
    let unknown = item("unknown", 0, false);
    // Even a malformed unmeasured item with nonzero bytes must not affect totals.
    let negative_control = item("negative-control", 999, false);
    let items = vec![measured, unknown.clone(), negative_control];
    let storage = FakeStorage::new(items.clone(), None);
    let ids = items.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(storage.preview(&ids).unwrap().bytes, 12);
    for by in [GroupBy::Category, GroupBy::Project, GroupBy::Wish] {
        let groups = group(&items, by);
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].1, groups[0].2), (12, 12));
    }
    assert_eq!(item_size_text(&unknown, Lang::En), "unknown");
    assert_eq!(item_size_text(&unknown, Lang::ZhTw), "未知");
}

#[test]
fn provided_methods_work_for_fakes() {
    let policy = FakeTurnPolicy::new(None);
    let settings = policy.try_set_extreme(true).unwrap();
    assert!(settings.extreme);
    assert_eq!(settings, policy.current());
    let background = FakeBackground::new(true);
    for stop in [false, true] {
        assert_eq!(background.quit(stop).unwrap_err().kind, "unsupported");
    }
}

#[test]
fn session_store_resolves_goal_and_rejects_unknown_session() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let goal = crate::goal::open(
        home.path(),
        "session",
        "wish",
        Some(project.path().to_str().unwrap()),
    )
    .unwrap();
    assert_eq!(
        super::session_store(home.path(), "session").unwrap(),
        crate::orch::store_for_goal(home.path(), &goal).unwrap()
    );
    assert_eq!(
        super::session_store(home.path(), "missing")
            .unwrap_err()
            .kind,
        "not_found"
    );
}
