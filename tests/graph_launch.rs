use axiom::{
    goal::Goal,
    graph::{
        launch::{build, inject_once, LaunchPacket, LAUNCH_BUDGET},
        GraphStore,
    },
    orch,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, time::Duration};

fn goal(project_dir: &std::path::Path) -> Goal {
    serde_json::from_value(json!({
        "wish": "unique wish never copied into the packet",
        "wish_id": "w-1234567890123-abcdef",
        "cwd": project_dir,
        "project": "demo",
        "status": "active"
    }))
    .unwrap()
}

#[test]
fn empty_graph_constraints_hash_and_small_budget() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = project.path().join("CONSTRAINTS.md");
    fs::write(&path, "- Keep the interface stable.\n").unwrap();
    let wish = "unique wish never copied into the packet";
    let packet = build(
        home.path(),
        "demo",
        project.path(),
        wish,
        LAUNCH_BUDGET,
        Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(packet.graph_state, "empty");
    assert!(packet.reason.as_ref().is_some_and(|s| !s.is_empty()));
    assert!(packet.text.starts_with(&format!(
        "LAUNCH CONTEXT (graph revision {}, empty)\n",
        packet.revision
    )));
    assert!(packet.text.contains("graph empty: "));
    assert!(packet
        .text
        .contains("CONSTRAINTS\n- Keep the interface stable."));
    assert!(packet
        .sources
        .contains(&path.to_string_lossy().into_owned()));
    assert!(!packet.text.contains(wish));
    assert_eq!(
        packet.wish_sha256,
        format!("{:x}", Sha256::digest(wish.as_bytes()))
    );
    assert_eq!(
        packet.revision,
        GraphStore::open(home.path()).unwrap().revision().unwrap()
    );
    assert_eq!(packet.bytes, packet.text.len());
    assert!(packet.bytes <= packet.budget);
    let tiny = build(
        home.path(),
        "demo",
        project.path(),
        wish,
        9,
        Duration::from_secs(3),
    )
    .unwrap();
    assert!(tiny.text.len() <= 9);
    assert_eq!(tiny.bytes, tiny.text.len());
}

#[test]
fn unavailable_preserves_project_constraints() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("file-home");
    fs::write(&home, "not a directory").unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("CONSTRAINTS.md"), "- Project rule.\n").unwrap();
    let packet = build(
        &home,
        "demo",
        project.path(),
        "unrelated unique wish",
        2400,
        Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(packet.graph_state, "unavailable");
    assert!(packet.reason.as_ref().is_some_and(|r| !r.is_empty()));
    assert!(packet.text.contains("graph unavailable: "));
    assert!(packet.text.contains("- Project rule."));
}

#[test]
fn inject_is_once_and_does_not_change_goal() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let g = goal(project.path());
    let goal_dir = home.path().join("goals");
    fs::create_dir_all(&goal_dir).unwrap();
    let goal_file = goal_dir.join("test.json");
    let original = serde_json::to_vec(&g).unwrap();
    fs::write(&goal_file, &original).unwrap();
    let store = orch::store_for_goal(home.path(), &g).unwrap();
    let first = inject_once(home.path(), &g).unwrap().unwrap();
    let saved: LaunchPacket =
        serde_json::from_slice(&fs::read(store.join("launch.json")).unwrap()).unwrap();
    assert_eq!(saved, first);
    assert!(inject_once(home.path(), &g).unwrap().is_none());
    assert_eq!(fs::read(goal_file).unwrap(), original);
    assert_eq!(serde_json::to_vec(&g).unwrap(), original);
    assert_eq!(g.wish, "unique wish never copied into the packet");
    let mut missing = g.clone();
    missing.cwd = None;
    assert!(inject_once(home.path(), &missing).unwrap().is_none());
    missing.cwd = g.cwd.clone();
    missing.project = None;
    assert!(inject_once(home.path(), &missing).unwrap().is_none());
}

#[test]
fn bundles_are_included_or_omitted_whole() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("example.rs"),
        "pub fn needle() {}\npub fn needle_extra() {}\n",
    )
    .unwrap();
    let wish = "needle x unrelated special phrase";
    let full = build(
        home.path(),
        "demo",
        project.path(),
        wish,
        6000,
        Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(full.graph_state, "fresh");
    assert!(!full.sources.is_empty());
    assert!(full.text.contains("needle"));
    assert!(!full.text.contains(wish));
    for source in &full.sources {
        assert!(full.text.contains(source));
    }
    let small = build(
        home.path(),
        "demo",
        project.path(),
        wish,
        120,
        Duration::from_secs(3),
    )
    .unwrap();
    assert!(small.text.len() <= 120);
    assert!(small.omitted > full.omitted);
    assert!(small.text.contains(&format!("OMITTED {}", small.omitted)));
    assert!(small.sources.len() < full.sources.len());
}

#[test]
fn deadline_produces_partial_state() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::write(project.path().join("example.rs"), "pub fn needle() {}\n").unwrap();
    let packet = build(
        home.path(),
        "demo",
        project.path(),
        "unrelated words",
        2400,
        Duration::ZERO,
    )
    .unwrap();
    assert_eq!(packet.graph_state, "partial");
    assert!(packet
        .reason
        .as_deref()
        .is_some_and(|reason| !reason.is_empty()));
    assert!(packet.text.contains("graph partial: "));
}
