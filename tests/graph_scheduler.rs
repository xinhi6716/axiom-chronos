mod common;
use axiom::{
    goal,
    graph::{code::index, conflicts, GraphStore},
    orch::{self, lifecycle::scheduler, Module},
};
use std::{fs, path::Path, time::Duration};

fn module(id: &str, path: &str, owner: Option<&str>) -> Module {
    Module {
        id: id.into(),
        purpose: "x".into(),
        paths: vec![path.into()],
        depends: vec![],
        contract: "x".into(),
        accept_file: "test".into(),
        accept_command: "echo ok".into(),
        serves: vec![],
        routing: vec![],
        new_paths: vec![],
        shared_owner: owner.map(str::to_owned),
    }
}
fn setup(home: &Path, linked: bool) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = home.join("repo");
    fs::create_dir_all(repo.join("a")).unwrap();
    fs::create_dir_all(repo.join("b")).unwrap();
    fs::write(repo.join("a/a.rs"), "pub fn alpha() {}\n").unwrap();
    fs::write(
        repo.join("b/b.rs"),
        if linked {
            "use crate::a::alpha;\nfn beta() { alpha(); }\n"
        } else {
            "fn beta() {}\n"
        },
    )
    .unwrap();
    let g = goal::open(home, "graph-test", "test", Some(repo.to_str().unwrap())).unwrap();
    let store =
        orch::wish_store(home, g.project.as_deref().unwrap(), g.wish_id.as_deref()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"),"## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## b\nPURPOSE x\nPATHS b/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    (repo, store)
}
#[test]
fn disjoint_run_together() {
    let t = common::temp();
    let (_, store) = setup(t.path(), false);
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["a", "b"]);
}
#[test]
fn references_block_and_removal_unblocks() {
    let t = common::temp();
    let (repo, store) = setup(t.path(), true);
    let a = module("a", "a/", None);
    let b = module("b", "b/", None);
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["a"]);
    let next = scheduler::next(&store, None, Duration::ZERO).unwrap();
    let wait = next.waiting.iter().find(|w| w.module == "b").unwrap();
    assert_eq!(wait.on, vec!["a"]);
    assert!(wait.conflict.as_deref().unwrap().starts_with("symbol "));
    let e = scheduler::claim(&store, "b").unwrap_err();
    assert_eq!(e.kind, "conflict");
    assert!(e.message.contains("a"));
    let before = conflicts::conflict(t.path(), &repo, &a, &b).unwrap();
    let graph = GraphStore::open(t.path()).unwrap();
    let rev = graph.revision().unwrap();
    let computations = conflicts::surface_computations();
    assert_eq!(
        before,
        conflicts::conflict(t.path(), &repo, &a, &b).unwrap()
    );
    assert_eq!(rev, graph.revision().unwrap());
    assert_eq!(computations, conflicts::surface_computations());
    fs::write(repo.join("b/b.rs"), "fn beta() {}\n").unwrap();
    index::sync(&graph, &repo, None, &Default::default()).unwrap();
    assert_eq!(conflicts::conflict(t.path(), &repo, &a, &b).unwrap(), None);
    assert!(conflicts::surface_computations() > computations);
}

#[test]
fn earlier_claim_blocks_until_released() {
    let t = common::temp();
    let (_, store) = setup(t.path(), true);
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["a"]);
    assert!(scheduler::claim_ready(&store).unwrap().is_empty());
    assert_eq!(scheduler::claimed_modules(&store).unwrap(), vec!["a"]);
    scheduler::release(&store, "a", "finished").unwrap();
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["b"]);
    assert_eq!(scheduler::claimed_modules(&store).unwrap(), vec!["b"]);
}

#[test]
fn partial_sync_is_conservative() {
    let t = common::temp();
    let (repo, _) = setup(t.path(), false);
    let graph = GraphStore::open(t.path()).unwrap();
    let options = index::SyncOptions {
        limits: index::Limits {
            files: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let report = index::sync(&graph, &repo, None, &options).unwrap();
    assert!(report.partial);
    let conflict = conflicts::conflict_with_sync_options(
        t.path(),
        &repo,
        &module("a", "a/", None),
        &module("b", "b/", None),
        &options,
    )
    .unwrap()
    .unwrap();
    assert!(conflict.reason == "graph stale" || conflict.reason == "graph missing");
}
#[test]
fn hotspots_and_owner_exemption() {
    let t = common::temp();
    let (repo, _) = setup(t.path(), false);
    fs::write(repo.join("a/lib.rs"), "pub fn root() {}\n").unwrap();
    let a = module("a", "a/lib.rs", None);
    let b = module("b", "a/a.rs", None);
    assert!(conflicts::conflict(t.path(), &repo, &a, &b)
        .unwrap()
        .unwrap()
        .reason
        .starts_with("hotspot "));
    let b = module("b", "a/a.rs", Some("a"));
    assert_eq!(conflicts::conflict(t.path(), &repo, &a, &b).unwrap(), None);
}
#[test]
fn unknown_repo_is_path_only() {
    let t = common::temp();
    let (_, known) = setup(t.path(), true);
    let store = t.path().join("orchestrate/legacy");
    fs::create_dir_all(&store).unwrap();
    fs::copy(known.join("units.tsv"), store.join("units.tsv")).unwrap();
    fs::copy(known.join("modules.md"), store.join("modules.md")).unwrap();
    assert!(conflicts::repo_for_store(&store).is_none());
    assert_eq!(scheduler::claim_ready(&store).unwrap(), vec!["a", "b"]);
}
#[test]
fn empty_index_is_conservative() {
    let t = common::temp();
    let repo = t.path().join("empty");
    fs::create_dir(&repo).unwrap();
    let c = conflicts::conflict(
        t.path(),
        &repo,
        &module("a", "a/", None),
        &module("b", "b/", None),
    )
    .unwrap()
    .unwrap();
    assert!(c.reason == "graph missing" || c.reason == "graph stale");
}

#[test]
fn stale_changes_and_unindexed_hotspots_invalidate_cache() {
    let t = common::temp();
    let (repo, _) = setup(t.path(), false);
    let a = module("a", "a/Cargo.toml", None);
    let b = module("b", "a/a.rs", None);
    assert_eq!(conflicts::conflict(t.path(), &repo, &a, &b).unwrap(), None);
    fs::write(repo.join("a/Cargo.toml"), "[package]\nname = 'x'\n").unwrap();
    assert!(conflicts::conflict(t.path(), &repo, &a, &b)
        .unwrap()
        .unwrap()
        .reason
        .starts_with("hotspot "));
    let a = module("a", "a/", None);
    let b = module("b", "b/", None);
    assert_eq!(conflicts::conflict(t.path(), &repo, &a, &b).unwrap(), None);
    fs::write(
        repo.join("b/b.rs"),
        "use crate::a::alpha;\nfn beta() { alpha(); }\n",
    )
    .unwrap();
    assert!(conflicts::conflict(t.path(), &repo, &a, &b)
        .unwrap()
        .unwrap()
        .reason
        .starts_with("symbol "));
}

#[test]
fn parse_failure_is_conservative() {
    let t = common::temp();
    let (repo, _) = setup(t.path(), false);
    fs::write(repo.join("b/b.rs"), "fn broken( {\n").unwrap();
    let c = conflicts::conflict(
        t.path(),
        &repo,
        &module("a", "a/", None),
        &module("b", "b/", None),
    )
    .unwrap()
    .unwrap();
    assert_eq!(c.reason, "graph stale");
}

#[test]
fn ambiguous_references_block() {
    let t = common::temp();
    let (repo, _) = setup(t.path(), false);
    fs::write(repo.join("b/b.rs"), "fn beta() { alpha(); }\n").unwrap();
    let graph = GraphStore::open(t.path()).unwrap();
    let report = index::sync(&graph, &repo, None, &Default::default()).unwrap();
    assert!(report.ambiguous > 0);
    let c = conflicts::conflict(
        t.path(),
        &repo,
        &module("a", "a/", None),
        &module("b", "b/", None),
    )
    .unwrap()
    .unwrap();
    assert!(c.reason.starts_with("symbol "));
}
