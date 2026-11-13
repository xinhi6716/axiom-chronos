mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    codex::routing::{self, Effort, Risk, RoutingSnapshot, Rule, Tier},
    orch,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    log: PathBuf,
    bridge: Bridge,
}
impl Fixture {
    fn new(fields: &str) -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let brief = store.join("u.brief");
        fs::write(
            &brief,
            format!("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n{fields}\n"),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let log = temp.path().join("codex.log");
        let wrapper = temp.path().join("wrapper.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]);
        config.wsl = None;
        config.on_windows = false;
        Self {
            _temp: temp,
            repo,
            store,
            log,
            bridge: Bridge::new(config),
        }
    }
    fn run(&self, extra: Value) -> Value {
        let mut args = json!({"project":"demo","unit":"u","role":"sol","cwd":self.repo});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        call(&self.bridge, "codex_run", &args)["structuredContent"].clone()
    }
    fn requests(&self, method: &str) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|v| v["method"] == method)
            .map(|v| v["params"].clone())
            .collect()
    }
}

#[test]
fn sol_dispatch_before_and_after_passing_full_test() {
    for full in [false, true] {
        let f = Fixture::new("RISK trivial");
        if full {
            assert_eq!(
                orch::record_full(&f.store, &f.repo, Some("echo ok"))
                    .unwrap()
                    .exit_code,
                0
            );
        }
        let r = f.run(json!({}));
        assert_eq!(r["state"], "built", "{r}");
        assert_eq!(f.requests("thread/start").len(), 1);
        assert_eq!(f.requests("thread/start")[0]["model"], "gpt-6.10-terra");
    }
}

#[test]
fn luna_entries_are_refused_before_thread_start() {
    for (fields, model) in [
        ("TIER luna\nEFFORT low", None),
        ("EFFORT high", Some("gpt-6-LuNa")),
    ] {
        let f = Fixture::new(fields);
        let r = f.run(model.map_or(json!({}), |m| json!({"model":m})));
        assert_eq!(r["error"]["kind"], "model_floor", "{r}");
        assert!(f.requests("thread/start").is_empty());
    }
    let f = Fixture::new("RISK trivial");
    let r = call(
        &f.bridge,
        "codex_run",
        &json!({"brief":"x","cwd":f.repo,"sandbox":"read-only","model":"gpt-6-luna"}),
    )["structuredContent"]
        .clone();
    assert_eq!(r["error"]["kind"], "model_floor", "{r}");
    assert!(f.requests("thread/start").is_empty());
}

#[test]
fn luna_text_does_not_hide_other_brief_errors() {
    for brief in [
        "GOAL build\nTIER luna\nVERIFY echo ok\n",
        "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nCONTEXT\nTIER luna\n",
    ] {
        let f = Fixture::new("RISK trivial");
        fs::write(f.store.join("u.brief"), brief).unwrap();
        let r = f.run(json!({}));
        assert_eq!(r["error"]["kind"], "invalid_argument", "{r}");
        assert!(f.requests("thread/start").is_empty());
    }
}

#[test]
fn registering_luna_brief_is_refused_before_thread_start() {
    let f = Fixture::new("RISK trivial");
    let brief = f.store.join("new.brief");
    fs::write(
        &brief,
        "GOAL build\nTIER luna\nSCOPE src/mod.py\nVERIFY echo ok\n",
    )
    .unwrap();
    let r = f.run(json!({"unit":"new","track":"m","brief_file":brief}));
    assert_eq!(r["error"]["kind"], "model_floor", "{r}");
    assert!(f.requests("thread/start").is_empty());
    assert!(!orch::read_units(&f.store)
        .unwrap()
        .iter()
        .any(|u| u.id == "new"));
}

#[test]
fn rubric_and_receipt_follow_snapshot_not_role() {
    for (risk, tier, effort, model) in [
        ("trivial", Tier::Terra, "medium", "gpt-6.10-terra"),
        ("parser", Tier::Sol, "xhigh", "gpt-6-sol"),
    ] {
        let f = Fixture::new(&format!("RISK {risk}"));
        let r = f.run(json!({}));
        assert_eq!(r["state"], "built", "{r}");
        let snapshot = routing::read_snapshot(&f.store, "u").unwrap().unwrap();
        assert_eq!(snapshot.tier, Some(tier));
        assert_eq!(snapshot.rule, Rule::RiskRubric);
        let rows = orch::receipts::read_routing(&f.store).unwrap();
        assert_eq!(rows.len(), 1);
        let receipt = &rows[0];
        assert_eq!(receipt.model_assigned, snapshot.model);
        assert_eq!(receipt.model_assigned, model);
        assert_eq!(
            receipt.model_assigned,
            f.requests("thread/start")[0]["model"]
        );
        assert_eq!(receipt.effort.as_deref(), Some(effort));
        assert_eq!(
            receipt.effort.as_deref(),
            f.requests("turn/start")[0]["effort"].as_str()
        );
        assert_eq!(
            receipt.tier,
            serde_json::to_value(snapshot.tier)
                .unwrap()
                .as_str()
                .map(str::to_owned)
        );
        assert_eq!(receipt.risk.as_deref(), Some(risk));
        assert_eq!(receipt.rule, "risk_rubric");
    }
}

#[test]
fn missing_and_partial_routing_refused_and_claim_released() {
    for (fields, kind) in [
        ("", "routing_missing"),
        ("TIER sol", "routing_incomplete"),
        ("EFFORT high", "routing_incomplete"),
    ] {
        let f = Fixture::new(fields);
        for _ in 0..2 {
            let r = f.run(json!({}));
            assert_eq!(r["error"]["kind"], kind, "{r}");
            assert!(f.requests("thread/start").is_empty());
        }
    }
}

#[test]
fn resumed_luna_snapshot_is_refused() {
    let f = Fixture::new("RISK trivial");
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&f.store, "u", "sol", &f.repo, false).unwrap()
    else {
        panic!()
    };
    let checkpoint = fs::read(f.store.join("checkpoints/u.json")).unwrap();
    orch::release_claim(&f.store, "u", &attempt_id).unwrap();
    fs::write(f.store.join("checkpoints/u.json"), checkpoint).unwrap();
    fs::write(
        f.store.join("claims.tsv"),
        format!("unit\tattempt\tscope\tstarted\nu\t{attempt_id}\tsrc/mod.py\t\n"),
    )
    .unwrap();
    routing::write_snapshot(
        &f.store,
        &RoutingSnapshot {
            unit: "u".into(),
            attempt_id,
            role: "sol".into(),
            tier: Some(Tier::Sol),
            effort: Some(Effort::Xhigh),
            risk: Some(Risk::Parser),
            model: "gpt-6-luna".into(),
            rule: Rule::RiskRubric,
            reason: String::new(),
            settings_version: None,
        },
    )
    .unwrap();
    let r = f.run(json!({}));
    assert_eq!(r["error"]["kind"], "model_floor", "{r}");
    assert!(f.requests("thread/start").is_empty());
}

#[test]
fn resumed_snapshot_effort_does_not_fall_back_to_brief() {
    let f = Fixture::new("TIER sol\nEFFORT high");
    let orch::Begin::Run { attempt_id, .. } =
        orch::begin_unit(&f.store, "u", "sol", &f.repo, false).unwrap()
    else {
        panic!()
    };
    let checkpoint = fs::read(f.store.join("checkpoints/u.json")).unwrap();
    orch::release_claim(&f.store, "u", &attempt_id).unwrap();
    fs::write(f.store.join("checkpoints/u.json"), checkpoint).unwrap();
    fs::write(
        f.store.join("claims.tsv"),
        format!("unit\tattempt\tscope\tstarted\nu\t{attempt_id}\tsrc/mod.py\t\n"),
    )
    .unwrap();
    routing::write_snapshot(
        &f.store,
        &RoutingSnapshot {
            unit: "u".into(),
            attempt_id,
            role: "sol".into(),
            tier: Some(Tier::Sol),
            effort: None,
            risk: None,
            model: "gpt-6-sol".into(),
            rule: Rule::Brief,
            reason: String::new(),
            settings_version: None,
        },
    )
    .unwrap();
    let r = f.run(json!({"model":"gpt-6-terra"}));
    assert_eq!(r["state"], "built", "{r}");
    assert_eq!(f.requests("thread/start")[0]["model"], "gpt-6-sol");
    assert!(f.requests("turn/start")[0].get("effort").is_none());
    let receipt = orch::receipts::read_routing(&f.store).unwrap().remove(0);
    assert_eq!(receipt.effort, None);
}

#[test]
fn legacy_luna_worker_setting_cannot_override_routing() {
    use axiom::engine::policy::{self, Models, Policy};
    let f = Fixture::new("RISK standard");
    assert_eq!(
        policy::set(&f.store, "models.worker.sol", "gpt-6-luna")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert!(f.requests("thread/start").is_empty());
    let mut models = Models::default();
    models.worker.insert("sol".into(), "gpt-6-luna".into());
    let mut p = Policy::default();
    p.models = Some(models);
    policy::save(&f.store, &p).unwrap();
    let r = f.run(json!({}));
    assert_eq!(r["state"], "built", "{r}");
    assert_eq!(f.requests("thread/start")[0]["model"], "gpt-6-sol");
}

#[test]
fn fix_escalates_actual_parent_snapshot_or_refuses_missing_decision() {
    for decision in [false, true] {
        let f = Fixture::new("RISK parser");
        if decision {
            let r = f.run(json!({}));
            assert_eq!(r["state"], "built", "{r}");
            let s = routing::read_snapshot(&f.store, "u").unwrap().unwrap();
            assert_eq!((s.tier, s.effort), (Some(Tier::Sol), Some(Effort::Xhigh)));
        }
        let brief = f.store.join("fix.brief");
        fs::write(&brief, "GOAL fix\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        let args = [
            "unit",
            "add",
            "fix",
            "--track",
            "m",
            "--brief",
            brief.to_str().unwrap(),
            "--fixes",
            "u",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
        let result = orch::run(&f.store, &args, false, false);
        if decision {
            assert_eq!(result.exit_code, 0, "{}", result.stderr);
            let fix = routing::read_fix(&f.store, "fix").unwrap().unwrap();
            assert_eq!(
                (fix.parent_tier, fix.parent_effort, fix.tier, fix.effort),
                (
                    Some(Tier::Sol),
                    Some(Effort::Xhigh),
                    Tier::Astra,
                    Effort::Xhigh
                )
            );
            let r = call(
                &f.bridge,
                "codex_run",
                &json!({"project":"demo","unit":"fix","role":"sol","cwd":f.repo}),
            )["structuredContent"]
                .clone();
            assert_eq!(r["state"], "built", "{r}");
            assert_eq!(
                f.requests("thread/start").last().unwrap()["model"],
                "gpt-6-astra"
            );
            assert_eq!(f.requests("turn/start").last().unwrap()["effort"], "xhigh");
        } else {
            assert_ne!(result.exit_code, 0);
            assert!(
                result.stderr.contains("routing_missing"),
                "{}",
                result.stderr
            );
            assert!(routing::read_fix(&f.store, "fix").unwrap().is_none());
        }
    }
}
