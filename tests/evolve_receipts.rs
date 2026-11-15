use axiom::{
    evolve::{
        self,
        history::RunRecord,
        receipts::{self, Cost, Receipt, TestResult},
        Inputs, SourceStatus, StartOutcome,
    },
    goal, orch,
    session::{Clock, Config},
};
use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
mod common;

fn run_record() -> RunRecord {
    RunRecord {
        id: "run-1".into(),
        started_ms: 1,
        repo: "repo".into(),
        base_commit: "base123".into(),
        branch: "branch".into(),
        since_days: 7,
        start_ms: 0,
        end_ms: 1,
        problems: vec![],
        metrics: vec![],
        state: "ended".into(),
        session: None,
        goal_status: None,
        exit_code: Some(0),
        error: None,
    }
}

#[test]
fn persistence_and_invalid_ids() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    assert_eq!(receipts::read(home, "missing").unwrap(), None);
    assert!(receipts::list(home).unwrap().is_empty());
    let mut receipt = receipts::from_run(home, &run_record(), &[]);
    let expected = home.join("evolve/receipts/run-1.json");
    assert_eq!(receipts::path(home, "run-1"), expected);
    assert_eq!(receipts::write(home, &receipt).unwrap(), expected);
    assert_eq!(
        receipts::read(home, "run-1").unwrap(),
        Some(receipt.clone())
    );
    let json: serde_json::Value = serde_json::from_slice(&fs::read(expected).unwrap()).unwrap();
    assert!(json["cost"]["claude_usd"].is_null());
    assert!(json["cost"]["codex_usd"].is_null());
    assert_eq!(json["schema"], 1);
    receipt.run_id = "a".into();
    receipts::write(home, &receipt).unwrap();
    assert_eq!(
        receipts::list(home)
            .unwrap()
            .iter()
            .map(|r| r.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "run-1"]
    );
    receipt.run_id = "../x".into();
    assert_eq!(
        receipts::write(home, &receipt).unwrap_err().kind,
        "invalid_argument"
    );
    assert!(!home.join("evolve/x.json").exists());
}

#[test]
fn derives_from_goal_and_orch_records() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let mut run = run_record();
    let sources = vec![SourceStatus {
        family: "goals".into(),
        state: "ok".into(),
        samples: 1,
        note: String::new(),
    }];
    let empty = receipts::from_run(home, &run, &sources);
    assert_eq!(empty.rollback_commit, Some("base123".into()));
    assert_eq!(empty.engine_build, axiom::engine::build_id());
    assert_eq!(empty.context_sources, vec!["goals"]);
    assert!(empty.tests.is_empty());
    assert!(empty.human_corrections.is_empty());
    assert_eq!(
        empty.cost,
        Cost {
            claude_usd: None,
            codex_usd: None
        }
    );
    assert_eq!(empty.role_routing, BTreeMap::new());
    assert!(empty.tools.is_empty() && empty.adopted_artifacts.is_empty());
    let goal = goal::open(home, "session-1", "wish", Some(home.to_str().unwrap())).unwrap();
    goal::interject(home, "session-1", "correction").unwrap();
    run.session = Some("session-1".into());
    let store = orch::store_for_goal(home, &goal).unwrap();
    for (unit, status) in [
        ("z", "completed"),
        ("a", "failed"),
        ("a", "completed"),
        ("z", "failed"),
        ("a", "failed"),
        ("b", "failed"),
        ("b", "completed"),
    ] {
        orch::receipts::record_run(
            &store,
            &orch::receipts::RunRecord {
                unit: unit.into(),
                attempt: "attempt".into(),
                started_ms: 1,
                ended_ms: 2,
                status: status.into(),
            },
        )
        .unwrap();
    }
    let receipt = receipts::from_run(home, &run, &sources);
    assert_eq!(receipt.human_corrections, vec!["correction"]);
    assert_eq!(
        receipt.tests,
        vec![
            TestResult {
                name: "a".into(),
                passed: false,
                retries: 2
            },
            TestResult {
                name: "b".into(),
                passed: true,
                retries: 1
            },
            TestResult {
                name: "z".into(),
                passed: false,
                retries: 1
            }
        ]
    );
    assert_eq!(receipt.retries, 4);
    fs::remove_file(store.join("receipts/runs.jsonl")).unwrap();
    assert!(receipts::from_run(home, &run, &sources).tests.is_empty());
}

fn git(dir: &Path, args: &[&str]) {
    let result = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.test"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn start_writes_receipts_and_preserves_outcomes_on_write_error() {
    let temp = tempfile::tempdir().unwrap();
    let old_record = std::env::var_os("FAKE_CLAUDE_RECORD");
    let old_scenario = std::env::var_os("FAKE_CLAUDE_SCENARIO");
    std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("fake.json"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "ok");
    for (i, fail_launch, fail_write) in [
        (0, false, false),
        (1, true, false),
        (2, false, true),
        (3, true, true),
    ] {
        let root = temp.path().join(format!("case-{i}"));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init"]);
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"axiom-chronos\"\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "fixture"]);
        let home = root.join("home");
        fs::create_dir_all(home.join("goals")).unwrap();
        fs::write(
            home.join("goals/blocked.json"),
            r#"{"wish":"test","status":"blocked","note":"need input"}"#,
        )
        .unwrap();
        if fail_write {
            fs::create_dir_all(home.join("evolve")).unwrap();
            fs::write(home.join("evolve/receipts"), "block directory").unwrap();
        }
        let inputs = Inputs {
            home: home.clone(),
            projects: root.join("projects"),
            now_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + 1000,
            since_days: 7,
        };
        let sources = evolve::preview(&inputs).unwrap().digest.sources;
        let config = Config {
            claude: Ok(if fail_launch {
                vec![root.join("missing").to_string_lossy().into_owned()]
            } else {
                common::fake("fake_claude")
            }),
            home: home.clone(),
            bin: home.join("axiom"),
            clock: Clock::system(),
        };
        let outcome = evolve::start(
            &inputs,
            Some(&repo),
            &config,
            Box::new(Cursor::new(Vec::<u8>::new())),
            &mut Vec::new(),
        );
        if fail_launch {
            assert_eq!(outcome.unwrap_err().kind, "launch_failed");
        } else {
            assert!(matches!(
                outcome.unwrap(),
                StartOutcome::Ran { exit_code: 0, .. }
            ));
        }
        let records = evolve::list_runs(&home).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(
            record.state,
            if fail_launch {
                "launch-failed"
            } else {
                "ended"
            }
        );
        if fail_write {
            assert!(record
                .error
                .as_ref()
                .unwrap()
                .contains("receipt write failed"));
        } else {
            let receipt: Receipt = receipts::read(&home, &record.id).unwrap().unwrap();
            assert_eq!(receipt, receipts::from_run(&home, record, &sources));
            assert_eq!(receipts::list(&home).unwrap().len(), 1);
        }
    }
    for (key, value) in [
        ("FAKE_CLAUDE_RECORD", old_record),
        ("FAKE_CLAUDE_SCENARIO", old_scenario),
    ] {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }
}
