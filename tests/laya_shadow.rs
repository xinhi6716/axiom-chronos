mod common;
use axiom::laya::{
    curate::Outcome,
    infer::{Abstain, Prediction},
    router::{DecisionKind as K, DecisionRecord, LayaSide},
    shadow::*,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, process::Command};

fn record(id: &str, confidence: f64, top: usize, abstain: Abstain, claude: &str) -> DecisionRecord {
    DecisionRecord {
        schema: 1,
        id: id.into(),
        ts_ms: 0,
        kind: K::Needs,
        wish_id: None,
        subject: String::new(),
        context: String::new(),
        claude: claude.into(),
        laya: Some(LayaSide {
            model_version: "m".into(),
            prediction: Prediction {
                labels: vec!["smoke".into(), "none".into()],
                probs: vec![confidence, 1.0 - confidence],
                top,
                confidence,
                tokens: 1,
                abstain,
            },
        }),
        laya_error: None,
        policy_version: String::new(),
        mode: "shadow".into(),
        executed: claude.into(),
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-12, "{a} != {b}");
}
fn good() -> TypeMetrics {
    TypeMetrics {
        kind: K::Needs,
        samples: 200,
        agreement: Some(0.9),
        verified_samples: 200,
        verified_accuracy: Some(0.9),
        ece: Some(0.05),
        abstain_rate: Some(0.3),
    }
}
fn report(t: &Thresholds, hash: &str) -> EvalReport {
    EvalReport {
        model_hash: hash.into(),
        split: "acceptance".into(),
        thresholds_sha256: thresholds_sha256(t),
        metrics: BTreeMap::from([(K::Needs, good())]),
        ts_ms: 1,
    }
}
fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn init(repo: &Path) {
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"axiom-chronos\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    git(repo, &["add", "Cargo.toml"]);
    git(repo, &["commit", "-m", "init"]);
}

#[test]
fn metric_arithmetic_and_empty() {
    let records = vec![
        record("a", 0.8, 0, Abstain::Answer, "smoke"),
        record("b", 0.6, 1, Abstain::Answer, "smoke"),
        record("c", 0.9, 0, Abstain::Abstain, "smoke"),
        record(
            "d",
            0.6,
            1,
            Abstain::Uncalibrated {
                reason: "missing".into(),
            },
            "none",
        ),
        {
            let mut r = record("e", 0.7, 0, Abstain::Answer, "smoke");
            r.laya = None;
            r
        },
    ];
    let outcomes = vec![
        Outcome {
            record_id: "a".into(),
            label: "none".into(),
            verified_by: "adjudicated".into(),
            ts_ms: 0,
        },
        Outcome {
            record_id: "b".into(),
            label: "none".into(),
            verified_by: "adjudicated".into(),
            ts_ms: 0,
        },
    ];
    let all = metrics(&records, &outcomes, 10);
    let m = &all[&K::Needs];
    assert_eq!((m.samples, m.verified_samples), (4, 2));
    near(m.agreement.unwrap(), 1.0 / 4.0);
    near(m.verified_accuracy.unwrap(), 1.0 / 2.0);
    near(m.abstain_rate.unwrap(), 2.0 / 4.0);
    // 0.8 wrong, two 0.6 correct, 0.9 correct; unverified c/d use Claude.
    near(m.ece.unwrap(), (0.8 + (2.0 - 1.2) + 0.1) / 4.0);
    let e = &all[&K::Routing];
    assert_eq!(e.samples, 0);
    assert_eq!(
        (e.agreement, e.verified_accuracy, e.ece, e.abstain_rate),
        (None, None, None, None)
    );
    assert_eq!(
        passes(e, &load_thresholds(None).unwrap().0)
            .unwrap_err()
            .len(),
        5
    );
}
#[test]
fn boundaries_cutoff_and_threshold_hash() {
    let (t, h) = load_thresholds(None).unwrap();
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/laya/shadow/thresholds.json"),
    )
    .unwrap();
    assert_eq!(h, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(h, thresholds_sha256(&t));
    let temp = common::temp();
    let p = temp.path().join("thresholds.json");
    std::fs::write(&p, &bytes).unwrap();
    assert_eq!(load_thresholds(Some(&p)).unwrap(), (t.clone(), h));
    assert!(passes(&good(), &t).is_ok());
    let mut m = good();
    m.samples -= 1;
    m.agreement = Some(t.min_agreement - 1e-9);
    m.ece = Some(t.max_ece + 1e-9);
    m.abstain_rate = Some(t.max_abstain_rate + 1e-9);
    assert_eq!(passes(&m, &t).unwrap_err().len(), 4);
    assert_eq!(choose_cutoff(&[], 0.5), None);
    let dev = [(0.2, false), (0.5, true), (0.8, true)];
    assert_eq!(choose_cutoff(&dev, 1.0 / 3.0), Some(0.5));
    assert_eq!(choose_cutoff(&dev, 0.0), Some(0.2));
    assert_eq!(choose_cutoff(&[(0.4, true), (0.6, true)], 1.0), Some(0.4));
    assert_eq!(
        choose_cutoff(&[(0.5, false), (0.5, true), (0.9, true)], 0.5),
        Some(0.5)
    );
}
#[test]
fn reports_and_promotion_error_order() {
    let home = common::temp();
    let (t, _) = load_thresholds(None).unwrap();
    let r = report(&t, "hash");
    let path = save_report(home.path(), &r).unwrap();
    assert_eq!(
        serde_json::from_slice::<EvalReport>(&std::fs::read(path).unwrap()).unwrap(),
        r
    );
    assert_eq!(
        save_report(home.path(), &r).unwrap_err().kind,
        "acceptance_already_scored"
    );
    let invalid_repo = home.path().join("missing");
    let call = |k: &[K], r: &EvalReport, c: Option<&Confirmation>| {
        promote(home.path(), &invalid_repo, k, r, &t, c, "2026-01-01")
            .unwrap_err()
            .kind
    };
    assert_eq!(
        call(&[K::Needs, K::Routing], &r, None),
        "one_type_at_a_time"
    );
    let mut bad = r.clone();
    bad.split = "dev".into();
    assert_eq!(call(&[K::Needs], &bad, None), "acceptance_required");
    bad = r.clone();
    bad.thresholds_sha256 = "old".into();
    assert_eq!(call(&[K::Needs], &bad, None), "stale_thresholds");
    bad = r.clone();
    bad.metrics.get_mut(&K::Needs).unwrap().samples = 0;
    assert_eq!(call(&[K::Needs], &bad, None), "below_threshold");
    assert_eq!(call(&[K::Needs], &r, None), "confirmation_required");
    assert_eq!(
        call(
            &[K::Needs],
            &r,
            Some(&Confirmation {
                kind: K::Routing,
                model_hash: "hash".into()
            })
        ),
        "confirmation_required"
    );
    assert_eq!(
        call(
            &[K::Needs],
            &r,
            Some(&Confirmation {
                kind: K::Needs,
                model_hash: "other".into()
            })
        ),
        "confirmation_required"
    );
    assert_eq!(
        call(
            &[K::Needs],
            &r,
            Some(&Confirmation {
                kind: K::Needs,
                model_hash: "hash".into()
            })
        ),
        "not_git_repo"
    );
}
#[test]
fn promotion_and_drift_survive_reload() {
    let home = common::temp();
    let repo = common::temp();
    init(repo.path());
    let (t, _) = load_thresholds(None).unwrap();
    let first = report(&t, "first");
    let c = Confirmation {
        kind: K::Needs,
        model_hash: "first".into(),
    };
    let state = promote(
        home.path(),
        repo.path(),
        &[K::Needs],
        &first,
        &t,
        Some(&c),
        "2026-01-01",
    )
    .unwrap();
    assert_eq!(state, load_state(home.path()).unwrap());
    let branch = Command::new("git")
        .current_dir(repo.path())
        .args(["branch", "--show-current"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&branch.stdout).trim(),
        "laya/needs-2026-01-01"
    );
    let second = report(&t, "second");
    let c2 = Confirmation {
        kind: K::Needs,
        model_hash: "second".into(),
    };
    let state = promote(
        home.path(),
        repo.path(),
        &[K::Needs],
        &second,
        &t,
        Some(&c2),
        "2026-01-02",
    )
    .unwrap();
    assert_eq!(state.types[&K::Needs].history, vec!["first"]);
    let mut recent = good();
    recent.agreement = Some(0.86);
    assert_eq!(
        check_drift(home.path(), K::Needs, &recent, &t).unwrap(),
        None
    );
    assert_eq!(load_state(home.path()).unwrap(), state);
    recent.agreement = Some(0.8);
    let rolled = check_drift(home.path(), K::Needs, &recent, &t)
        .unwrap()
        .unwrap();
    assert_eq!(rolled.decider, "laya");
    assert_eq!(rolled.model_hash.as_deref(), Some("first"));
    assert_eq!(load_state(home.path()).unwrap().types[&K::Needs], rolled);
    recent.agreement = Some(0.9);
    recent.ece = Some(0.051);
    let rolled = check_drift(home.path(), K::Needs, &recent, &t)
        .unwrap()
        .unwrap();
    assert_eq!(rolled.decider, "claude");
    assert_eq!(rolled.model_hash, None);
    assert_eq!(load_state(home.path()).unwrap().types[&K::Needs], rolled);
    assert_eq!(
        check_drift(home.path(), K::Needs, &recent, &t).unwrap(),
        None
    );
}
