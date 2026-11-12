mod common;
use axiom::{
    base::{Failure, Res},
    laya::{
        curate::{self, CurateSpec, Outcome, Split, VerifiedOutcomeSource},
        infer::{Abstain, Prediction},
        ort,
        pipeline::{self, PipelineSpec},
        router::{self, DecisionKind as K, DecisionRecord, LayaSide, Mode, RouterConfig},
        shadow::{self, Confirmation, EvalReport, TypeMetrics},
        train::{Backend, Caps, TrainOutput, TrainSpec, Trainer},
    },
    usage::laya::UsageEvent,
};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn init(repo: &Path) {
    fs::create_dir_all(repo).unwrap();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"axiom-chronos\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    git(repo, &["add", "Cargo.toml"]);
    git(repo, &["commit", "-m", "init"]);
}
fn make_spec(root: &Path) -> PipelineSpec {
    let repo = root.join("repo");
    init(&repo);
    PipelineSpec {
        home: root.join("home"),
        repo,
        work_dir: root.join("work"),
        curate: CurateSpec::new(0),
        date: "2026-11-12".into(),
        dry_run: false,
        train: TrainSpec {
            dataset_dir: root.join("dataset"),
            out_dir: root.join("output"),
            upstream: root.into(),
            base: "base".into(),
            revision: "main".into(),
            epochs: 3,
            seed: 0,
            backend: Backend::Local,
            caps: Caps {
                wall_clock_s: 30,
                compute_units: 10.0,
            },
        },
    }
}
struct Fake {
    calls: AtomicUsize,
    fail: bool,
}
impl Fake {
    fn new(fail: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail,
        }
    }
}
impl Trainer for Fake {
    fn train(&self, spec: &TrainSpec, _: &AtomicBool) -> Res<TrainOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(Failure::new("fake", "training failed"));
        }
        let dir = spec.out_dir.join("onnx");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.onnx"), b"model").unwrap();
        ort::write_manifest(&dir, "v1", "fused", "fake", "main")?;
        Ok(TrainOutput {
            checkpoint: spec.out_dir.join("checkpoint"),
            onnx_dir: dir,
            tokenizer: spec.out_dir.join("tokenizer"),
            calibration: spec.out_dir.join("calibration"),
            usage: UsageEvent {
                id: "train".into(),
                ts_ms: 1,
                model_version: "v1".into(),
                kind: "train".into(),
                backend: "local".into(),
                tokens: None,
                gpu_hours: Some(1.0),
                compute_units: None,
            },
        })
    }
}
fn good(kind: K) -> TypeMetrics {
    TypeMetrics {
        kind,
        samples: 200,
        agreement: Some(0.9),
        verified_samples: 200,
        verified_accuracy: Some(0.9),
        ece: Some(0.05),
        abstain_rate: Some(0.3),
    }
}
fn report(hash: &str, metrics: Vec<TypeMetrics>) -> EvalReport {
    EvalReport {
        model_hash: hash.into(),
        split: "acceptance".into(),
        thresholds_sha256: shadow::load_thresholds(None).unwrap().1,
        metrics: metrics.into_iter().map(|m| (m.kind, m)).collect(),
        ts_ms: 1,
    }
}
fn statuses(report: &pipeline::PipelineReport) -> Vec<&str> {
    report.steps.iter().map(|s| s.status.as_str()).collect()
}
fn confirmation(kind: K, hash: &str) -> Confirmation {
    Confirmation {
        kind,
        model_hash: hash.into(),
    }
}

#[test]
fn full_chain_and_wrong_confirmation() {
    let tmp = common::temp();
    let spec = make_spec(tmp.path());
    let fake = Fake::new(false);
    let result = pipeline::run(
        &spec,
        &fake,
        &|_| Ok(report("hash", vec![good(K::Needs)])),
        Some(&confirmation(K::Needs, "hash")),
    )
    .unwrap();
    assert_eq!(
        statuses(&result),
        ["ok", "ok", "ok", "ok", "ok", "ok", "skipped"]
    );
    assert_eq!(result.steps[6].detail, "no drift");
    assert_eq!(result.promoted, Some(K::Needs));
    assert!(spec.home.join("laya/promotion.json").is_file());
    git(
        &spec.repo,
        &["show-ref", "--verify", "refs/heads/laya/needs-2026-11-12"],
    );
    let tmp = common::temp();
    let spec = make_spec(tmp.path());
    let result = pipeline::run(
        &spec,
        &Fake::new(false),
        &|_| Ok(report("hash", vec![good(K::Needs)])),
        Some(&confirmation(K::Needs, "wrong")),
    )
    .unwrap();
    assert_eq!(
        statuses(&result),
        ["ok", "ok", "ok", "ok", "ok", "failed", "skipped"]
    );
    assert_eq!(result.promoted, None);
    assert!(!spec.home.join("laya/promotion.json").exists());
}
#[test]
fn waiting_failure_and_dry_run() {
    let tmp = common::temp();
    let mut spec = make_spec(tmp.path());
    let fake = Fake::new(false);
    let result = pipeline::run(
        &spec,
        &fake,
        &|_| Ok(report("hash", vec![good(K::Needs)])),
        None,
    )
    .unwrap();
    assert_eq!(
        statuses(&result),
        ["ok", "ok", "ok", "ok", "waiting", "skipped", "skipped"]
    );
    assert_eq!(result.promoted, None);
    assert!(!spec.home.join("laya/promotion.json").exists());
    let result = pipeline::run(&spec, &Fake::new(true), &|_| panic!("eval called"), None).unwrap();
    assert_eq!(
        statuses(&result),
        ["ok", "failed", "skipped", "skipped", "skipped", "skipped", "skipped"]
    );
    spec.dry_run = true;
    spec.train.dataset_dir = tmp.path().join("dry-dataset");
    let fake = Fake::new(false);
    let result = pipeline::run(&spec, &fake, &|_| panic!("eval called"), None).unwrap();
    assert_eq!(statuses(&result), ["preview"; 7]);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert!(!spec.train.dataset_dir.exists());
    assert_eq!(result.promoted, None);
}
#[test]
fn existing_type_drift_is_rolled_back() {
    let tmp = common::temp();
    let spec = make_spec(tmp.path());
    let (t, _) = shadow::load_thresholds(None).unwrap();
    shadow::promote(
        &spec.home,
        &spec.repo,
        &[K::Triage],
        &report("old", vec![good(K::Triage)]),
        &t,
        Some(&confirmation(K::Triage, "old")),
        "2026-11-11",
    )
    .unwrap();
    let mut bad = good(K::Triage);
    bad.agreement = Some(0.1);
    let result = pipeline::run(
        &spec,
        &Fake::new(false),
        &|_| Ok(report("new", vec![good(K::Needs), bad.clone()])),
        Some(&confirmation(K::Needs, "new")),
    )
    .unwrap();
    assert_eq!(
        statuses(&result),
        ["ok", "ok", "ok", "ok", "ok", "ok", "ok"]
    );
    assert!(result.steps[6].detail.contains("triage"));
    let state = shadow::load_state(&spec.home).unwrap();
    assert_eq!(state.types[&K::Triage].decider, "claude");
    assert_eq!(state.types[&K::Needs].decider, "laya");
}

#[test]
fn rollback_skips_missing_metrics_with_and_without_drift() {
    for has_drift in [false, true] {
        let tmp = common::temp();
        let spec = make_spec(tmp.path());
        let (thresholds, _) = shadow::load_thresholds(None).unwrap();
        for kind in [K::Triage, K::Routing] {
            shadow::promote(
                &spec.home,
                &spec.repo,
                &[kind],
                &report("old", vec![good(kind)]),
                &thresholds,
                Some(&confirmation(kind, "old")),
                "2026-11-11",
            )
            .unwrap();
        }
        let mut metrics = vec![good(K::Needs)];
        if has_drift {
            let mut bad = good(K::Triage);
            bad.agreement = Some(0.1);
            metrics.push(bad);
        }
        let result = pipeline::run(
            &spec,
            &Fake::new(false),
            &|_| Ok(report("new", metrics.clone())),
            Some(&confirmation(K::Needs, "new")),
        )
        .unwrap();
        let rollback = &result.steps[6];
        assert_eq!(rollback.status, if has_drift { "ok" } else { "skipped" });
        assert!(rollback.detail.contains("missing metrics: "));
        assert!(rollback.detail.contains("routing"));
        assert!(rollback.detail.contains("triage"));
        let state = shadow::load_state(&spec.home).unwrap();
        assert_eq!(state.types[&K::Routing].decider, "laya");
        assert_eq!(
            state.types[&K::Triage].decider,
            if has_drift { "claude" } else { "laya" }
        );
    }
}

#[test]
fn status_text_reports_state_runtime_and_usage() {
    let tmp = common::temp();
    let spec = make_spec(tmp.path());
    let fallback = ort::find_runtime(Some(&spec.home), None).map_or_else(
        |_| "not found".to_string(),
        |p| format!("found {}", p.display()),
    );
    let kill = if router::killed(&spec.home) {
        "yes"
    } else {
        "no"
    };
    assert_eq!(
        pipeline::status_text(&spec.home).unwrap(),
        format!("mode: off\nkill: {kill}\nruntime: {fallback}\npromotion: none\n")
    );

    let runtime = tmp.path().join("runtime.lib");
    fs::write(&runtime, b"runtime").unwrap();
    router::save_config(
        &spec.home,
        &RouterConfig {
            mode: Mode::Shadow,
            runtime_lib: Some(runtime.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    router::kill(&spec.home).unwrap();
    let (thresholds, _) = shadow::load_thresholds(None).unwrap();
    for kind in [K::Needs, K::Triage] {
        shadow::promote(
            &spec.home,
            &spec.repo,
            &[kind],
            &report("hash", vec![good(kind)]),
            &thresholds,
            Some(&confirmation(kind, "hash")),
            "2026-11-12",
        )
        .unwrap();
    }
    axiom::usage::laya::record(
        &spec.home,
        &UsageEvent {
            id: "status".into(),
            ts_ms: 1,
            model_version: "v1".into(),
            kind: "train".into(),
            backend: "local".into(),
            tokens: Some(12),
            gpu_hours: Some(1.0),
            compute_units: None,
        },
    )
    .unwrap();
    assert_eq!(pipeline::status_text(&spec.home).unwrap(), format!(
        "mode: shadow\nkill: yes\nruntime: found {}\npromotion needs: laya\npromotion triage: laya\nLaya:\n  v1: tokens 12, GPU hours 1, compute units unknown\n",
        runtime.display()
    ));
}

static ENV_LOCK: Mutex<()> = Mutex::new(());
struct Restore(Option<std::ffi::OsString>);
impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(v) = &self.0 {
            std::env::set_var("AXIOM_DATA_HOME", v);
        } else {
            std::env::remove_var("AXIOM_DATA_HOME");
        }
    }
}
fn call(args: &[&str]) -> i32 {
    pipeline::cli(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}
#[test]
fn cli_operations() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = common::temp();
    let _restore = Restore(std::env::var_os("AXIOM_DATA_HOME"));
    std::env::set_var("AXIOM_DATA_HOME", tmp.path());
    assert_eq!(call(&["status"]), 0);
    assert_eq!(
        call(&[
            "shadow",
            "on",
            "--model",
            "D",
            "--calibration",
            "F",
            "--runtime",
            "R"
        ]),
        0
    );
    let cfg: RouterConfig =
        serde_json::from_slice(&fs::read(tmp.path().join("laya/router.json")).unwrap()).unwrap();
    assert_eq!(cfg.mode, Mode::Shadow);
    assert_eq!(cfg.model_dir.as_deref(), Some(Path::new("D")));
    assert_eq!(cfg.calibration.as_deref(), Some(Path::new("F")));
    assert_eq!(cfg.runtime_lib.as_deref(), Some(Path::new("R")));
    assert_eq!(call(&["shadow", "off"]), 0);
    assert_eq!(router::load_config(tmp.path()).unwrap().mode, Mode::Off);
    assert_eq!(call(&["kill"]), 0);
    assert!(router::killed(tmp.path()));
    assert_eq!(call(&["unkill"]), 0);
    assert!(!router::killed(tmp.path()));
    for args in [
        &["unknown"][..],
        &["eval", "--split", "bogus"],
        &["promote", "needs", "--repo", "."],
        &["promote", "unknown", "--repo", ".", "--confirm", "unknown"],
        &["promote", "needs", "--repo", ".", "--confirm", "triage"],
        &["run"],
    ] {
        assert_eq!(call(args), 2);
    }
    assert_eq!(call(&["run", "--dry-run"]), 0);
    let json: serde_json::Value =
        serde_json::from_str(&pipeline::dry_run_json(tmp.path()).unwrap()).unwrap();
    let steps = json["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 7);
    assert!(steps.iter().all(|step| step["status"] == "preview"));
    assert!(json["promoted"].is_null());
    let dataset = tmp.path().join("dataset");
    let bundle = tmp.path().join("bundle");
    assert_eq!(call(&["curate", "--out", dataset.to_str().unwrap()]), 0);
    assert!(dataset.join("datasheet.json").is_file());
    assert_eq!(
        call(&[
            "bundle",
            "--dataset",
            dataset.to_str().unwrap(),
            "--out",
            bundle.to_str().unwrap()
        ]),
        0
    );
    assert!(bundle.join("manifest.json").is_file());
    std::env::remove_var("AXIOM_DATA_HOME");
    assert_eq!(call(&["status"]), 2);
}

#[test]
fn cli_eval_uses_curated_split_and_promote_uses_current_model() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = common::temp();
    let _restore = Restore(std::env::var_os("AXIOM_DATA_HOME"));
    std::env::set_var("AXIOM_DATA_HOME", tmp.path());
    let model = tmp.path().join("model");
    fs::create_dir_all(&model).unwrap();
    fs::write(model.join("model.onnx"), "model").unwrap();
    ort::write_manifest(&model, "v1", "fused", "fake", "main").unwrap();
    router::save_config(
        tmp.path(),
        &RouterConfig {
            model_dir: Some(model.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    for i in 0..40 {
        let id = format!("record-{i}");
        let record = DecisionRecord {
            schema: 1,
            id: id.clone(),
            ts_ms: i,
            kind: K::Needs,
            wish_id: Some(format!("wish-{i}")),
            subject: String::new(),
            context: format!("context-{i}"),
            claude: "smoke".into(),
            laya: Some(LayaSide {
                model_version: "v1".into(),
                prediction: Prediction {
                    labels: vec!["smoke".into(), "none".into()],
                    probs: vec![0.9, 0.1],
                    top: 0,
                    confidence: 0.9,
                    tokens: 1,
                    abstain: Abstain::Answer,
                },
            }),
            laya_error: None,
            policy_version: String::new(),
            mode: "shadow".into(),
            executed: "smoke".into(),
        };
        router::append_record(tmp.path(), &record).unwrap();
        curate::record_outcome(
            tmp.path(),
            &Outcome {
                record_id: id,
                label: "smoke".into(),
                verified_by: "adjudicated".into(),
                ts_ms: i,
            },
        )
        .unwrap();
    }
    let (records, _) = router::read_records(tmp.path()).unwrap();
    let source = VerifiedOutcomeSource {
        home: tmp.path().into(),
    };
    let data = curate::curate(&records, &[&source], &CurateSpec::new(0)).unwrap();
    let expected = data.splits[&Split::Dev].len();
    assert!(expected > 0 && expected < records.len());
    assert_eq!(call(&["eval", "--split", "dev"]), 0);
    let hash = ort::sha256_file(&model.join("axiom-laya.json")).unwrap();
    let eval_path = tmp
        .path()
        .join("laya/eval")
        .join(format!("{hash}-dev.json"));
    let evaluated: EvalReport = serde_json::from_slice(&fs::read(eval_path).unwrap()).unwrap();
    assert_eq!(evaluated.metrics[&K::Needs].samples, expected as u64);
    assert_eq!(evaluated.split, "dev");
    let expected_acceptance = data.splits[&Split::Acceptance].len();
    assert!(expected_acceptance > 0 && expected_acceptance < records.len());
    assert_eq!(call(&["eval", "--split", "acceptance"]), 0);
    let acceptance_path = tmp
        .path()
        .join("laya/eval")
        .join(format!("{hash}-acceptance.json"));
    let acceptance: EvalReport =
        serde_json::from_slice(&fs::read(acceptance_path).unwrap()).unwrap();
    assert_eq!(acceptance.split, "acceptance");
    assert_eq!(
        acceptance.metrics[&K::Needs].samples,
        expected_acceptance as u64
    );
    ort::write_manifest(&model, "v2", "fused", "fake", "main").unwrap();
    let current_hash = ort::sha256_file(&model.join("axiom-laya.json")).unwrap();
    assert_ne!(current_hash, hash);
    shadow::save_report(tmp.path(), &report(&current_hash, vec![good(K::Needs)])).unwrap();
    let repo = tmp.path().join("repo");
    init(&repo);
    assert_eq!(
        call(&[
            "promote",
            "needs",
            "--repo",
            repo.to_str().unwrap(),
            "--confirm",
            "needs",
            "--date",
            "2026-11-12"
        ]),
        0
    );
    assert_eq!(
        shadow::load_state(tmp.path()).unwrap().types[&K::Needs]
            .model_hash
            .as_deref(),
        Some(current_hash.as_str())
    );
    git(
        &repo,
        &["show-ref", "--verify", "refs/heads/laya/needs-2026-11-12"],
    );
}
