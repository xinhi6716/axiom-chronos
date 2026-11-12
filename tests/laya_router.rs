use axiom::{
    goal,
    laya::{
        infer::{Abstain, FakeClassifier, Laya, LoadSpec, Prediction},
        ort,
        router::*,
    },
    orch::{Finding, Needs, Severity, Verdict},
    usage,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

static GLOBAL: Mutex<()> = Mutex::new(());
fn prediction(kind: DecisionKind) -> Prediction {
    Prediction {
        labels: kind.labels().iter().map(|x| x.to_string()).collect(),
        probs: vec![0.1, 0.2, 0.7],
        top: 2,
        confidence: 0.7,
        tokens: 23,
        abstain: Abstain::Answer,
    }
}
fn fake_classifier() -> Arc<FakeClassifier> {
    Arc::new(FakeClassifier {
        version: "fake".into(),
        table: DecisionKind::ALL
            .into_iter()
            .map(|k| (k.as_str().into(), prediction(k)))
            .collect::<BTreeMap<_, _>>(),
    })
}
fn decision(kind: DecisionKind) -> Decision {
    Decision {
        kind,
        wish_id: Some("w-123".into()),
        subject: "subject".into(),
        context: "context".into(),
        claude: kind.labels()[0].into(),
    }
}
fn config(model: &Path, runtime: &Path) -> RouterConfig {
    RouterConfig {
        mode: Mode::Shadow,
        model_dir: Some(model.into()),
        calibration: None,
        runtime_lib: Some(runtime.into()),
    }
}
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn paired_decisions_truncation_usage() {
    let _guard = GLOBAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let observer = ShadowObserver::new(home.path().into(), Some(fake_classifier()), None);
    for kind in DecisionKind::ALL {
        assert_eq!(DecisionKind::parse(kind.as_str()), Some(kind));
        observer.observe(&decision(kind));
    }
    let mut d = decision(DecisionKind::Routing);
    d.context = "界".repeat(2000);
    observer.observe(&d);
    let (records, skipped) = read_records(home.path()).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 5);
    for r in &records {
        assert_eq!(r.schema, 1);
        assert_eq!(r.executed, "claude");
        assert_eq!(r.mode, "shadow");
        assert_eq!(r.claude, r.kind.labels()[0]);
        assert_ne!(r.claude, r.laya.as_ref().unwrap().prediction.labels[2]);
        assert!(r.laya_error.is_none());
        assert_eq!(r.policy_version, axiom::engine::build_id());
    }
    assert!(records[4].context.len() <= 4096);
    assert_eq!(records[4].context, "界".repeat(1365));
    let usage = usage::laya::summarize(home.path()).unwrap();
    assert_eq!(usage.models[0].events, 5);
    assert_eq!(usage.models[0].tokens, 115);
}
#[test]
fn real_fixture_routing() {
    let _guard = GLOBAL.lock().unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let model = tempfile::tempdir().unwrap();
    copy_dir(&repo.join("tests/fixtures/laya/model"), model.path());
    let runtime = ort::find_runtime(None, Some(repo)).unwrap_or_else(|e| panic!("ONNX Runtime is required; set AXIOM_LAYA_ORT_LIB or install it with tools/laya/fetch.py: {e}"));
    let classifier = Laya::load(&LoadSpec {
        model_dir: model.path().into(),
        calibration: Some(repo.join("tests/fixtures/laya_calibration/valid.json")),
        runtime: runtime.clone(),
        intra_threads: 1,
    })
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    ShadowObserver::new(home.path().into(), Some(Arc::new(classifier)), None)
        .observe(&decision(DecisionKind::Routing));
    let (records, _) = read_records(home.path()).unwrap();
    let p = &records[0].laya.as_ref().unwrap().prediction;
    assert_eq!(p.probs.len(), 3);
    assert!(p.probs.iter().all(|p| p.is_finite()));
    assert!((p.probs.iter().sum::<f64>() - 1.0).abs() < 1e-6);
    let mut c = config(model.path(), &runtime);
    c.calibration = Some(repo.join("tests/fixtures/laya_calibration/valid.json"));
    save_config(home.path(), &c).unwrap();
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    let records = read_records(home.path()).unwrap().0;
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|r| r.laya.is_some()));
}
#[test]
fn config_kill_off_and_load_failure() {
    let _guard = GLOBAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    assert_eq!(load_config(home.path()).unwrap(), RouterConfig::default());
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    assert!(!home.path().join("laya").exists());
    let c = config(
        &home.path().join("bad-model"),
        &home.path().join("bad-runtime"),
    );
    save_config(home.path(), &c).unwrap();
    assert_eq!(load_config(home.path()).unwrap(), c);
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    let (r, _) = read_records(home.path()).unwrap();
    assert_eq!(r.len(), 1);
    assert!(r[0].laya.is_none());
    assert!(r[0].laya_error.is_some());
    kill(home.path()).unwrap();
    assert!(killed(home.path()));
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    assert_eq!(read_records(home.path()).unwrap().0.len(), 1);
    unkill(home.path()).unwrap();
    assert!(!killed(home.path()));
    let old = std::env::var_os("AXIOM_LAYA_KILL");
    unsafe { std::env::set_var("AXIOM_LAYA_KILL", "1") };
    assert!(killed(home.path()));
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    assert_eq!(read_records(home.path()).unwrap().0.len(), 1);
    unsafe {
        if let Some(v) = old {
            std::env::set_var("AXIOM_LAYA_KILL", v);
        } else {
            std::env::remove_var("AXIOM_LAYA_KILL");
        }
    }
    assert!(!killed(home.path()));
    observer(home.path()).observe(&decision(DecisionKind::Routing));
    assert_eq!(read_records(home.path()).unwrap().0.len(), 2);
}
#[test]
fn concurrent_appends_and_bad_line() {
    let _guard = GLOBAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let seed = ShadowObserver::new(home.path().into(), Some(fake_classifier()), None);
    seed.observe(&decision(DecisionKind::Needs));
    let r = read_records(home.path()).unwrap().0.remove(0);
    fs::remove_file(home.path().join("laya/decisions.jsonl")).unwrap();
    std::thread::scope(|scope| {
        for thread in 0..8 {
            let r = &r;
            let home = home.path();
            scope.spawn(move || {
                for i in 0..50 {
                    let mut r = r.clone();
                    r.id = format!("{thread}-{i}");
                    append_record(home, &r).unwrap();
                }
            });
        }
    });
    let (records, skipped) = read_records(home.path()).unwrap();
    assert_eq!(records.len(), 400);
    assert_eq!(skipped, 0);
    assert_eq!(
        records.iter().map(|r| &r.id).collect::<HashSet<_>>().len(),
        400
    );
    writeln!(
        OpenOptions::new()
            .append(true)
            .open(home.path().join("laya/decisions.jsonl"))
            .unwrap(),
        "not-json"
    )
    .unwrap();
    assert_eq!(read_records(home.path()).unwrap().1, 1);
}
struct PanicObserver;
impl DecisionObserver for PanicObserver {
    fn observe(&self, _: &Decision) {
        panic!("observer panic");
    }
}
#[test]
fn helpers_are_best_effort_and_do_not_change_decisions() {
    let _guard = GLOBAL.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let old = std::env::var_os("AXIOM_DATA_HOME");
    unsafe { std::env::set_var("AXIOM_DATA_HOME", home.path()) };
    let fake = Arc::new(FakeObserver::default());
    install_override(Some(fake.clone()));
    let goal = goal::open(home.path(), "session", "wish", None).unwrap();
    goal::interject(home.path(), "session", "new prompt").unwrap();
    observe_interjection(home.path(), "session", "answer");
    let result = goal::classify(home.path(), "session", "answer").unwrap();
    assert!(result.interjection.is_none());
    let verdict = Verdict {
        unit: "u".into(),
        tree: "tree".into(),
        verdict: "fail".into(),
        findings: vec![Finding {
            number: 1,
            check: "check".into(),
            severity: Severity::Major,
            location: "loc".into(),
            evidence: "evidence".into(),
            fix: "fix".into(),
            needs: Some(Needs::Smoke),
        }],
    };
    observe_verdict(&home.path().join("w-xxx"), &verdict);
    observe_routing(
        Some(home.path()),
        Some(&home.path().join("w-xxx")),
        Some("u"),
        "astra",
        "brief",
    );
    observe_routing(Some(home.path()), None, None, "sol", "brief");
    observe_routing(None, None, None, "sol", "ignored");
    let seen = fake.seen.lock().unwrap();
    assert_eq!(seen.len(), 5);
    assert_eq!(seen[0].kind, DecisionKind::Interjection);
    assert_eq!(seen[0].context, "new prompt");
    assert_eq!(seen[0].wish_id, goal.wish_id);
    assert_eq!(seen[1].kind, DecisionKind::Triage);
    assert_eq!(seen[1].claude, "major");
    assert_eq!(seen[2].kind, DecisionKind::Needs);
    assert_eq!(seen[2].claude, "smoke");
    assert_eq!(seen[1].wish_id.as_deref(), Some("w-xxx"));
    assert_eq!(seen[3].claude, "astra");
    assert_eq!(seen[4].claude, "sol");
    drop(seen);
    install_override(Some(Arc::new(ShadowObserver::new(
        home.path().into(),
        Some(fake_classifier()),
        None,
    ))));
    observe_routing(Some(home.path()), None, None, "astra", "brief");
    observe_routing(Some(home.path()), None, None, "sol", "brief");
    let records = read_records(home.path()).unwrap().0;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].claude, "astra");
    assert_eq!(records[1].claude, "sol");
    assert!(records
        .iter()
        .all(|r| r.executed == "claude" && r.laya.is_some()));
    install_override(Some(Arc::new(PanicObserver)));
    observe_routing(Some(home.path()), None, None, "sol", "brief");
    observe_verdict(&home.path().join("w-xxx"), &verdict);
    goal::interject(home.path(), "session", "again").unwrap();
    observe_interjection(home.path(), "session", "answer");
    assert!(goal::classify(home.path(), "session", "answer").is_ok());
    install_override(None);
    unsafe {
        if let Some(v) = old {
            std::env::set_var("AXIOM_DATA_HOME", v);
        } else {
            std::env::remove_var("AXIOM_DATA_HOME");
        }
    }
}
