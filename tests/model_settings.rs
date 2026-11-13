use axiom::engine::policy::*;
#[test]
fn defaults_when_nothing_set() {
    let d = tempfile::tempdir().unwrap();
    let e = effective_models(None, d.path()).unwrap();
    assert_eq!(e.version, "default:0");
    assert_eq!(e.models.self_check.tier, "sol");
    assert_eq!(e.models.self_check.effort, "xhigh");
    assert_eq!(e.models.reviewer.tier, "astra");
    assert_eq!(e.models.reviewer.effort, "max");
    assert_eq!(
        (
            &*e.models.core.planning,
            &*e.models.core.build,
            &*e.models.core.escalation,
            &*e.models.core.close
        ),
        ("opus", "sonnet", "opus", "opus")
    );
    assert!(e.models.worker.is_empty());
}
#[test]
fn global_used_without_policy_models() {
    let h = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let mut m = Models::default();
    assert_eq!(save_global_models(h.path(), &m).unwrap(), 1);
    m.core.build = "haiku".into();
    assert_eq!(save_global_models(h.path(), &m).unwrap(), 2);
    let e = effective_models(Some(h.path()), d.path()).unwrap();
    assert_eq!(e.version, "global:2");
    assert_eq!(e.models, m);
}
#[test]
fn policy_beats_global() {
    let h = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    save_global_models(h.path(), &Models::default()).unwrap();
    set(d.path(), "models.core.build", "haiku").unwrap();
    let e = effective_models(Some(h.path()), d.path()).unwrap();
    assert_eq!(e.version, "policy:1");
    assert_eq!(e.models.core.build, "haiku");
}
#[test]
fn set_accepts_valid_keys() {
    let d = tempfile::tempdir().unwrap();
    for (i, (k, v)) in [
        ("models.self_check", "sol/max"),
        ("models.reviewer", "astra/high"),
        ("models.core.build", "haiku"),
        ("models.worker.sol", "some-id"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(set(d.path(), k, v).unwrap().version, i as u64 + 1);
    }
    let m = load(d.path()).unwrap().models.unwrap();
    assert_eq!(m.self_check.effort, "max");
    assert_eq!(m.reviewer.effort, "high");
    assert_eq!(m.core.build, "haiku");
    assert_eq!(m.worker["sol"], "some-id");
}
#[test]
fn invalid_values_change_nothing() {
    let d = tempfile::tempdir().unwrap();
    set(d.path(), "pm_qa", "on").unwrap();
    let p = d.path().join("policy.json");
    let before = std::fs::read(&p).unwrap();
    for (k, v) in [
        ("models.self_check", "ultra"),
        ("models.self_check", "sol"),
        ("models.self_check", "foo/high"),
        ("models.self_check", "luna/high"),
        ("models.core.verify", "x"),
        ("models.core.build", ""),
        ("models.worker.luna", "x"),
        ("models.worker.sol", "a b"),
    ] {
        assert_eq!(set(d.path(), k, v).unwrap_err().kind, "invalid_argument");
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }
}
#[test]
fn unknown_models_field_rejected() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("policy.json"), r#"{"models":{"bogus":1}}"#).unwrap();
    assert_eq!(load(d.path()).unwrap_err().kind, "invalid_argument");
}
#[test]
fn old_policy_unchanged() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("policy.json"), "{}").unwrap();
    assert_eq!(load(d.path()).unwrap(), Policy::default());
    assert_eq!(
        effective_models(None, d.path()).unwrap().version,
        "default:0"
    );
}
mod common;
fn server(d: &std::path::Path) -> axiom::codex::rpc::AppServer {
    let w = d.join("wrapper.py");
    std::fs::write(&w,"import os,runpy,sys\nos.environ['FAKE_CODEX_MODELS']='gpt-6.1-sol'\nsys.argv=sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    axiom::codex::rpc::AppServer::new(vec![
        common::python(),
        w.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ])
}
fn attempt(
    s: &axiom::codex::rpc::AppServer,
    d: &std::path::Path,
    id: &str,
) -> axiom::codex::routing::RoutingSnapshot {
    if !d.join("units.tsv").exists() {
        std::fs::write(
            d.join("units.tsv"),
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t\n",
        )
        .unwrap();
    }
    let b =
        axiom::orch::parse::parse_brief("GOAL test\nSCOPE src/a\nVERIFY test\nTIER sol\n").unwrap();
    axiom::codex::routing::for_attempt(s, None, d, "u", id, "terra", &b, None).unwrap()
}
#[test]
fn worker_override_applied() {
    let d = tempfile::tempdir().unwrap();
    set(d.path(), "models.worker.sol", "custom-id").unwrap();
    let s = axiom::codex::rpc::AppServer::new(vec![]);
    let r = attempt(&s, d.path(), "a");
    assert_eq!(r.model, "custom-id");
    assert_eq!(r.settings_version.as_deref(), Some("policy:1"));
}
#[test]
fn started_attempt_keeps_snapshot() {
    let d = tempfile::tempdir().unwrap();
    set(d.path(), "models.worker.sol", "first").unwrap();
    let s = axiom::codex::rpc::AppServer::new(vec![]);
    let a = attempt(&s, d.path(), "a");
    set(d.path(), "models.worker.sol", "second").unwrap();
    assert_eq!(attempt(&s, d.path(), "a"), a);
    let b = attempt(&s, d.path(), "b");
    assert_eq!(b.model, "second");
    assert_eq!(b.settings_version.as_deref(), Some("policy:2"));
}
#[test]
fn worker_none_removes() {
    let d = tempfile::tempdir().unwrap();
    set(d.path(), "models.worker.sol", "custom").unwrap();
    set(d.path(), "models.worker.sol", "none").unwrap();
    assert!(load(d.path()).unwrap().models.unwrap().worker.is_empty());
    assert_eq!(
        attempt(&server(d.path()), d.path(), "b").model,
        "gpt-6.1-sol"
    );
}
#[test]
fn global_write_atomic() {
    let h = tempfile::tempdir().unwrap();
    let mut m = Models::default();
    save_global_models(h.path(), &m).unwrap();
    let p = h.path().join("settings/models.json");
    let before = std::fs::read(&p).unwrap();
    m.self_check.tier = "luna".into();
    assert_eq!(
        save_global_models(h.path(), &m).unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::read_dir(p.parent().unwrap()).unwrap().count(), 1);
}
