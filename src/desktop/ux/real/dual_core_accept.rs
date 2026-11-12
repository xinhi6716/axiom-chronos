//! Tests for dual_core; owned by module engine-views-seam.
use super::dual_core::EventDualCore;
use crate::{
    desktop::ux::{celestial::DualCore, seams::dual_core::DualCoreSource},
    engine::{kinds, policy},
    orch::events,
};
use serde_json::json;
use std::path::PathBuf;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let goal = crate::goal::open(
        home.path(),
        "s1",
        "wish",
        Some(project.path().to_str().unwrap()),
    )
    .unwrap();
    let store = crate::orch::store_for_goal(home.path(), &goal).unwrap();
    std::fs::create_dir_all(&store).unwrap();
    (home, project, store)
}
fn enable(store: &PathBuf) {
    policy::set(store, "dual_core", "on").unwrap();
}
fn event(store: &PathBuf, data: serde_json::Value) {
    events::append_event(store, kinds::DUAL_CORE, None, None, data).unwrap();
}
fn state(home: &tempfile::TempDir) -> DualCore {
    EventDualCore::new(home.path().to_path_buf()).state("s1")
}
fn topic(t: &str) -> gpui_kit::SharedString {
    t.to_owned().into()
}

#[test]
fn off_without_policy_or_event_and_for_unknown_session() {
    let (home, _p, store) = setup();
    assert_eq!(state(&home), DualCore::Off);
    // An event with the policy off is still Off.
    event(&store, json!({"id":"a","kind":"goal_close","step":"co_core","agree":true}));
    assert_eq!(state(&home), DualCore::Off);
    enable(&store);
    assert_ne!(state(&home), DualCore::Off);
    assert_eq!(
        EventDualCore::new(home.path().to_path_buf()).state("missing"),
        DualCore::Off
    );
}

#[test]
fn agreed_and_deciding_projections() {
    let (home, _p, store) = setup();
    enable(&store);
    event(&store, json!({"id":"a","kind":"module_map","step":"co_core","agree":false}));
    assert_eq!(state(&home), DualCore::Deciding { topic: topic("module_map") });
    event(&store, json!({"id":"b","kind":"plan_acceptance","step":"co_core","agree":true}));
    assert_eq!(state(&home), DualCore::Agreed { topic: topic("plan_acceptance") });
}

#[test]
fn arbitrated_and_split_projections_replay() {
    let (home, _p, store) = setup();
    enable(&store);
    event(&store, json!({"id":"a","kind":"fix_rounds","step":"co_core","agree":false}));
    event(&store, json!({"id":"a","kind":"fix_rounds","step":"arbiter","choice":"core","agree":true}));
    let expected = DualCore::Adjudicated {
        topic: topic("fix_rounds"),
        by: "arbiter".into(),
    };
    assert_eq!(state(&home), expected);
    // A new source over the same files (a restart) gives the same answer.
    assert_eq!(state(&home), expected);
    event(&store, json!({"id":"c","kind":"goal_close","step":"co_core","agree":false}));
    event(&store, json!({"id":"c","kind":"goal_close","step":"arbiter","choice":"co_core","agree":false}));
    assert_eq!(
        state(&home),
        DualCore::Adjudicated { topic: topic("goal_close"), by: "arbiter".into() }
    );
    events::append_event(&store, kinds::PERSON_REQUEST, None, None, json!({"id":"c","kind":"goal_close","reason":"r"})).unwrap();
    assert_eq!(state(&home), DualCore::Split { topic: topic("goal_close") });
    // Negative control: a person request for another id does not make a split.
    event(&store, json!({"id":"d","kind":"council_ruling","step":"arbiter","choice":"co_core","agree":false}));
    assert_eq!(
        state(&home),
        DualCore::Adjudicated { topic: topic("council_ruling"), by: "arbiter".into() }
    );
}
