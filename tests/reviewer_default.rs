use axiom::engine::policy::{self, ReviewerMode};
use std::{fs, path::Path};

fn routed_store() -> tempfile::TempDir {
    let store = tempfile::tempdir().unwrap();
    fs::write(store.path().join("modules.md"), "## m\nPURPOSE test\nPATHS m\nDEPENDS none\nCONTRACT test\nACCEPT file `test`\nROUTING\n  default sol/xhigh standard reason\n").unwrap();
    assert!(axiom::codex::routing::routed_store(store.path()).unwrap());
    store
}

fn write_policy(store: &Path, json: &str) {
    fs::write(store.join("policy.json"), json).unwrap();
}

#[test]
fn routed_no_policy_is_astra_max() {
    let store = routed_store();
    assert_eq!(
        policy::load(store.path()).unwrap().reviewer,
        ReviewerMode::AstraMax
    );
}

#[test]
fn routed_policy_without_reviewer_is_astra_max() {
    let store = routed_store();
    write_policy(store.path(), r#"{"pm_qa":true}"#);
    let policy = policy::load(store.path()).unwrap();
    assert!(policy.pm_qa);
    assert_eq!(policy.reviewer, ReviewerMode::AstraMax);
}

#[test]
fn unrouted_defaults_claude() {
    let store = tempfile::tempdir().unwrap();
    assert_eq!(
        policy::load(store.path()).unwrap().reviewer,
        ReviewerMode::Claude
    );
    write_policy(store.path(), r#"{"pm_qa":true}"#);
    assert_eq!(
        policy::load(store.path()).unwrap().reviewer,
        ReviewerMode::Claude
    );
    fs::create_dir(store.path().join("modules.md")).unwrap();
    assert!(axiom::codex::routing::routed_store(store.path()).is_err());
    assert_eq!(
        policy::load(store.path()).unwrap().reviewer,
        ReviewerMode::Claude
    );
}

#[test]
fn explicit_value_wins() {
    let store = routed_store();
    for (value, expected) in [
        ("claude", ReviewerMode::Claude),
        ("astra-max", ReviewerMode::AstraMax),
        ("both", ReviewerMode::Both),
    ] {
        write_policy(store.path(), &format!(r#"{{"reviewer":"{value}"}}"#));
        assert_eq!(policy::load(store.path()).unwrap().reviewer, expected);
    }
}

#[test]
fn set_then_load_roundtrip() {
    let store = routed_store();
    for (value, expected) in [
        ("claude", ReviewerMode::Claude),
        ("astra-max", ReviewerMode::AstraMax),
        ("both", ReviewerMode::Both),
    ] {
        policy::set(store.path(), "reviewer", value).unwrap();
        assert_eq!(policy::load(store.path()).unwrap().reviewer, expected);
    }
}
