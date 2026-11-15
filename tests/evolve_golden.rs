mod common;
use axiom::base::{Failure, Res};
use axiom::evolve::golden::*;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

struct Fake {
    flipped: AtomicBool,
    missing: bool,
}
impl Replayer for Fake {
    fn replay(
        &self,
        repo: &Path,
        overrides: &BTreeMap<String, String>,
        f: &Fixture,
        _: u64,
    ) -> Res<BTreeMap<String, bool>> {
        if self.missing {
            return Err(Failure::new("test", "missing"));
        }
        let content = overrides
            .get("prompt.txt")
            .cloned()
            .unwrap_or_else(|| fs::read_to_string(repo.join("prompt.txt")).unwrap_or_default());
        Ok(f.gates
            .iter()
            .map(|g| {
                (
                    g.clone(),
                    if g == "target" {
                        content.contains("improve") && !self.flipped.load(Ordering::SeqCst)
                    } else {
                        !content.contains("break")
                    },
                )
            })
            .collect())
    }
}
fn fx() -> FixtureSet {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/evolve_golden");
    FixtureSet {
        dev: root.join("dev"),
        heldout: root.join("heldout"),
        seeds: vec![1, 2],
    }
}
fn candidate(content: &str) -> Candidate {
    Candidate {
        id: "one".into(),
        kind: CandidateKind::HarnessEdit,
        target: "prompt.txt".into(),
        content: content.into(),
        rationale: "test".into(),
    }
}
fn fake() -> Fake {
    Fake {
        flipped: AtomicBool::new(false),
        missing: false,
    }
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = common::temp();
    let home = common::temp();
    fs::write(repo.path().join("prompt.txt"), "original").unwrap();
    (repo, home)
}
#[test]
fn adoption_and_regression_revert() {
    let (repo, home) = setup();
    let r = fake();
    let mut c = candidate("improve");
    c.kind = CandidateKind::Prompt;
    let cmp = evaluate(repo.path(), &c, &fx(), "target", &r).unwrap();
    assert_eq!(decide(&cmp), Verdict::Adopt);
    let v = adopt(home.path(), repo.path(), &c, &cmp).unwrap();
    assert_eq!(v.kind, CandidateKind::Prompt);
    assert_eq!(v.n, 1);
    assert_eq!(
        fs::read_to_string(repo.path().join("prompt.txt")).unwrap(),
        "improve"
    );
    let dir = home.path().join("evolve/versions/prompt_txt");
    assert_eq!(
        fs::read_to_string(dir.join("v1.before")).unwrap(),
        "original"
    );
    assert_eq!(fs::read_to_string(dir.join("v1.after")).unwrap(), "improve");
    r.flipped.store(true, Ordering::SeqCst);
    let revert = check_regression(home.path(), repo.path(), "prompt.txt", &fx(), "target", &r)
        .unwrap()
        .unwrap();
    assert_eq!(revert.candidate, "revert:one");
    assert_eq!(revert.n, 2);
    assert_eq!(
        fs::read_to_string(repo.path().join("prompt.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        versions(home.path(), "prompt.txt").unwrap(),
        vec![v, revert]
    );
    assert!(
        check_regression(home.path(), repo.path(), "prompt.txt", &fx(), "target", &r)
            .unwrap()
            .is_none()
    );
}
#[test]
fn reject_regression_tie_and_missing() {
    let (repo, home) = setup();
    let r = fake();
    for content in ["break", "original", "improve break"] {
        let c = candidate(content);
        let cmp = evaluate(repo.path(), &c, &fx(), "target", &r).unwrap();
        assert!(matches!(decide(&cmp), Verdict::Reject(_)));
        assert!(adopt(home.path(), repo.path(), &c, &cmp).is_err());
    }
    let missing = Fake {
        flipped: AtomicBool::new(false),
        missing: true,
    };
    let cmp = evaluate(
        repo.path(),
        &candidate("improve"),
        &fx(),
        "target",
        &missing,
    )
    .unwrap();
    assert!(matches!(decide(&cmp), Verdict::Reject(_)));
    assert_eq!(cmp.baseline_dev.missing, 2);
}
#[test]
fn negative_control_gate_drop_hidden_by_total_gain() {
    fn score(target: f64, gates: &[(&str, (u64, u64))]) -> Score {
        Score {
            per_gate: gates.iter().map(|(g, c)| (g.to_string(), *c)).collect(),
            missing: 0,
            target,
        }
    }
    let baseline = score(0.0, &[("target", (0, 4)), ("guard", (4, 4))]);
    let improved = score(1.0, &[("target", (4, 4)), ("guard", (3, 4))]);
    assert!(
        improved.per_gate.values().map(|x| x.0).sum::<u64>()
            > baseline.per_gate.values().map(|x| x.0).sum::<u64>()
    );
    let cmp = Comparison {
        baseline_dev: baseline.clone(),
        candidate_dev: improved.clone(),
        baseline_heldout: baseline,
        candidate_heldout: improved,
    };
    assert!(matches!(decide(&cmp), Verdict::Reject(_)));
}
#[test]
fn paths_and_fixture_isolation() {
    let (repo, home) = setup();
    let r = fake();
    let mut sets = fx();
    sets.heldout = sets.dev.clone();
    assert!(evaluate(repo.path(), &candidate("improve"), &sets, "target", &r).is_err());
    for target in ["../x", "/tmp/x", "C:\\x", "a\\..\\x", "\\root"] {
        let mut c = candidate("improve");
        c.target = target.into();
        assert!(
            evaluate(repo.path(), &c, &fx(), "target", &r).is_err(),
            "{target}"
        );
    }
    let plugin = axiom::engine::plugin_dir(home.path()).unwrap();
    let marker = plugin.join("marker.txt");
    fs::write(&marker, "unchanged").unwrap();
    let cmp = evaluate(repo.path(), &candidate("improve"), &fx(), "target", &r).unwrap();
    assert!(adopt(home.path(), &plugin, &candidate("improve"), &cmp).is_err());
    assert_eq!(fs::read_to_string(marker).unwrap(), "unchanged");
    assert!(!plugin.join("prompt.txt").exists());
}
#[test]
fn command_replayer_checks_whole_lines_and_override() {
    let repo = common::temp();
    let r = CommandReplayer;
    let fixture = Fixture {
        id: "command".into(),
        gates: vec!["gate".into()],
        data: serde_json::json!({"command": [common::python(), "-c", "import os; print(open('input.txt').read()); print(os.environ['AXIOM_GOLDEN_SEED'])"], "expect": {"gate": ["ok", "42"]}}),
    };
    let pass = r
        .replay(
            repo.path(),
            &BTreeMap::from([("input.txt".into(), "ok".into())]),
            &fixture,
            42,
        )
        .unwrap();
    assert_eq!(pass["gate"], true);
    let fail = r
        .replay(
            repo.path(),
            &BTreeMap::from([("input.txt".into(), "not ok".into())]),
            &fixture,
            42,
        )
        .unwrap();
    assert_eq!(fail["gate"], false);
}

#[test]
fn regression_check_preserves_passing_adoption() {
    let (repo, home) = setup();
    let r = fake();
    let c = candidate("improve");
    let cmp = evaluate(repo.path(), &c, &fx(), "target", &r).unwrap();
    let v = adopt(home.path(), repo.path(), &c, &cmp).unwrap();
    assert!(
        check_regression(home.path(), repo.path(), &c.target, &fx(), "target", &r)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(repo.path().join(&c.target)).unwrap(), b"improve");
    assert_eq!(versions(home.path(), &c.target).unwrap(), vec![v]);
}

#[test]
fn regression_missing_reverts_adoption() {
    let (repo, home) = setup();
    let c = candidate("improve");
    let cmp = evaluate(repo.path(), &c, &fx(), "target", &fake()).unwrap();
    let v = adopt(home.path(), repo.path(), &c, &cmp).unwrap();
    let missing = Fake {
        flipped: AtomicBool::new(false),
        missing: true,
    };
    let revert = check_regression(
        home.path(),
        repo.path(),
        &c.target,
        &fx(),
        "target",
        &missing,
    )
    .unwrap()
    .unwrap();
    assert_eq!(revert.candidate, "revert:one");
    assert_eq!(revert.n, 2);
    assert!(revert.score.missing > 0);
    assert_eq!(fs::read(repo.path().join(&c.target)).unwrap(), b"original");
    assert_eq!(versions(home.path(), &c.target).unwrap(), vec![v, revert]);
}

#[test]
fn regression_removes_newly_adopted_target() {
    let (repo, home) = setup();
    let path = repo.path().join("prompt.txt");
    fs::remove_file(&path).unwrap();
    let r = fake();
    let c = candidate("improve");
    let cmp = evaluate(repo.path(), &c, &fx(), "target", &r).unwrap();
    let v = adopt(home.path(), repo.path(), &c, &cmp).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"improve");
    r.flipped.store(true, Ordering::SeqCst);
    let revert = check_regression(home.path(), repo.path(), &c.target, &fx(), "target", &r)
        .unwrap()
        .unwrap();
    assert_eq!(revert.n, 2);
    assert_eq!(revert.candidate, "revert:one");
    assert_eq!(revert.sha256, v.previous_sha256);
    assert!(!path.exists());
    assert_eq!(versions(home.path(), &c.target).unwrap(), vec![v, revert]);
}

#[test]
fn sequential_adoptions_increment_versions() {
    struct Incremental;
    impl Replayer for Incremental {
        fn replay(
            &self,
            repo: &Path,
            overrides: &BTreeMap<String, String>,
            f: &Fixture,
            seed: u64,
        ) -> Res<BTreeMap<String, bool>> {
            let content = overrides
                .get("prompt.txt")
                .cloned()
                .unwrap_or_else(|| fs::read_to_string(repo.join("prompt.txt")).unwrap());
            Ok(f.gates
                .iter()
                .map(|g| {
                    (
                        g.clone(),
                        g != "target" || content == "second" || (content == "first" && seed == 1),
                    )
                })
                .collect())
        }
    }
    let (repo, home) = setup();
    let first = candidate("first");
    let cmp1 = evaluate(repo.path(), &first, &fx(), "target", &Incremental).unwrap();
    let v1 = adopt(home.path(), repo.path(), &first, &cmp1).unwrap();
    let mut second = candidate("second");
    second.id = "two".into();
    let cmp2 = evaluate(repo.path(), &second, &fx(), "target", &Incremental).unwrap();
    let v2 = adopt(home.path(), repo.path(), &second, &cmp2).unwrap();
    assert_eq!((v1.n, v2.n), (1, 2));
    assert_eq!(fs::read(repo.path().join("prompt.txt")).unwrap(), b"second");
    assert_eq!(versions(home.path(), "prompt.txt").unwrap(), vec![v1, v2]);
}

#[test]
fn mutating_apis_reject_bad_targets_and_plugin_repo() {
    let (repo, home) = setup();
    let r = fake();
    let cmp = evaluate(repo.path(), &candidate("improve"), &fx(), "target", &r).unwrap();
    for target in ["../x", "C:\\x", "/x", "a\\..\\b"] {
        let mut c = candidate("improve");
        c.target = target.into();
        assert!(
            adopt(home.path(), repo.path(), &c, &cmp).is_err(),
            "adopt: {target}"
        );
        assert!(
            check_regression(home.path(), repo.path(), target, &fx(), "target", &r).is_err(),
            "check: {target}"
        );
        assert!(versions(home.path(), target).is_err(), "versions: {target}");
    }
    let plugin = axiom::engine::plugin_dir(home.path()).unwrap();
    let marker = plugin.join("prompt.txt");
    fs::write(&marker, "plugin content").unwrap();
    assert!(check_regression(home.path(), &plugin, "prompt.txt", &fx(), "target", &r).is_err());
    assert_eq!(fs::read(marker).unwrap(), b"plugin content");
    assert!(!home.path().join("evolve/versions").exists());
}

#[test]
fn command_replayer_exit_and_invalid_commands() {
    let repo = common::temp();
    let r = CommandReplayer;
    let overrides = BTreeMap::new();
    let mut f = Fixture {
        id: "exit".into(),
        gates: vec!["expected".into(), "unlisted".into()],
        data: serde_json::json!({"command": [common::python(), "-c", "print('ok'); raise SystemExit(1)"], "expect": {"expected": ["ok"]}}),
    };
    let results = r.replay(repo.path(), &overrides, &f, 1).unwrap();
    assert_eq!(
        results,
        BTreeMap::from([("expected".into(), false), ("unlisted".into(), false)])
    );
    f.data = serde_json::json!({"command": [common::python(), "-c", "print('ok')"], "expect": {"expected": ["ok"]}});
    let results = r.replay(repo.path(), &overrides, &f, 1).unwrap();
    assert_eq!(
        results,
        BTreeMap::from([("expected".into(), true), ("unlisted".into(), true)])
    );
    f.data = serde_json::json!({"command": "not an array"});
    assert!(r.replay(repo.path(), &overrides, &f, 1).is_err());
    f.data = serde_json::json!({"command": [repo.path().join("nonexistent-executable").to_string_lossy().to_string()]});
    assert!(r.replay(repo.path(), &overrides, &f, 1).is_err());
}

#[test]
fn harness_edit_serde_round_trip() {
    assert_eq!(
        serde_json::to_string(&CandidateKind::HarnessEdit).unwrap(),
        "\"harness-edit\""
    );
    assert_eq!(
        serde_json::from_str::<CandidateKind>("\"harness-edit\"").unwrap(),
        CandidateKind::HarnessEdit
    );
}

fn fixture_file(dir: &Path, name: &str, id: &str, gates: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(name),
        serde_json::to_vec(&Fixture {
            id: id.into(),
            gates: gates.iter().map(|g| (*g).into()).collect(),
            data: serde_json::json!({}),
        })
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn evaluate_loads_fixtures_in_filename_order() {
    use std::sync::Mutex;
    struct Recording(Mutex<Vec<String>>);
    impl Replayer for Recording {
        fn replay(
            &self,
            _: &Path,
            _: &BTreeMap<String, String>,
            f: &Fixture,
            _: u64,
        ) -> Res<BTreeMap<String, bool>> {
            self.0.lock().unwrap().push(f.id.clone());
            Ok(f.gates.iter().map(|g| (g.clone(), true)).collect())
        }
    }
    let (repo, _) = setup();
    let root = common::temp();
    let sets = FixtureSet {
        dev: root.path().join("dev"),
        heldout: root.path().join("heldout"),
        seeds: vec![1],
    };
    for (dir, prefix) in [(&sets.dev, "dev"), (&sets.heldout, "heldout")] {
        fixture_file(dir, "z.json", &format!("{prefix}-z"), &["target"]);
        fixture_file(dir, "a.json", &format!("{prefix}-a"), &["target"]);
    }
    let r = Recording(Mutex::new(vec![]));
    evaluate(repo.path(), &candidate("improve"), &sets, "target", &r).unwrap();
    assert_eq!(
        *r.0.lock().unwrap(),
        [
            "dev-a",
            "dev-z",
            "dev-a",
            "dev-z",
            "heldout-a",
            "heldout-z",
            "heldout-a",
            "heldout-z"
        ]
    );
}

#[test]
fn evaluate_rejects_gate_regression_despite_higher_total() {
    let (repo, _) = setup();
    let root = common::temp();
    let sets = FixtureSet {
        dev: root.path().join("dev"),
        heldout: root.path().join("heldout"),
        seeds: vec![1, 2],
    };
    for dir in [&sets.dev, &sets.heldout] {
        fixture_file(dir, "a.json", "target-a", &["target"]);
        fixture_file(dir, "b.json", "target-b", &["target"]);
        fixture_file(dir, "c.json", "guard", &["guard"]);
    }
    let cmp = evaluate(
        repo.path(),
        &candidate("improve break"),
        &sets,
        "target",
        &fake(),
    )
    .unwrap();
    for (baseline, improved) in [
        (&cmp.baseline_dev, &cmp.candidate_dev),
        (&cmp.baseline_heldout, &cmp.candidate_heldout),
    ] {
        assert_eq!(baseline.per_gate["target"], (0, 4));
        assert_eq!(improved.per_gate["target"], (4, 4));
        assert_eq!(baseline.per_gate["guard"], (2, 2));
        assert_eq!(improved.per_gate["guard"], (0, 2));
        assert!(
            improved.per_gate.values().map(|v| v.0).sum::<u64>()
                > baseline.per_gate.values().map(|v| v.0).sum::<u64>()
        );
        assert_eq!((baseline.missing, improved.missing), (0, 0));
    }
    assert!(matches!(decide(&cmp), Verdict::Reject(reason) if reason == "gate regression"));
}
