use axiom::orch::checkpoint::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[test]
fn preplan_research_claim_is_durable_but_engineer_is_forbidden() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    fs::create_dir(&store).unwrap();
    let intent = reference(&store.join("intent"), b"intent");
    let c = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: tmp.path().canonicalize().unwrap(),
                identity_digest: "repo".into(),
            },
            intent: intent.clone(),
        },
    )
    .unwrap();
    let mut spec = StepSpec {
        run_id: "r".into(),
        step_id: "research".into(),
        purpose: Purpose::Research,
        brief_ref: intent,
        scope: vec![],
        dependency_ids: vec![],
        check_ids: vec![],
    };
    let claim = claim_step(&store, c.revision, &spec, "input").unwrap();
    assert_eq!(claim.disposition, ClaimDisposition::New);
    assert_eq!(
        load_run(&store, "r").unwrap().attempts[&claim.attempt.attempt_id].state,
        AttemptState::Prepared
    );
    spec.step_id = "engineering".into();
    spec.purpose = Purpose::Engineer;
    spec.scope = vec!["src".into()];
    assert!(claim_step(&store, claim.revision, &spec, "input")
        .unwrap_err()
        .message
        .contains("approved research"));
    assert_eq!(load_run(&store, "r").unwrap().attempts.len(), 1);
}

#[test]
fn frozen_checks_bind_real_plan_source_and_command() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let store = tmp.path().join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("dirty"), "before").unwrap();
    let intent = reference(&store.join("intent"), b"intent");
    let research = reference(&store.join("research"), b"research");
    let plan = reference(&store.join("plan"), b"plan");
    let mut rev = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: repo.canonicalize().unwrap(),
                identity_digest: "repo".into(),
            },
            intent,
        },
    )
    .unwrap()
    .revision;
    rev = record_receipt(&store, "r", rev, "research", &research).unwrap();
    rev = record_core_decision(
        &store,
        "r",
        rev,
        &CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "yes".into(),
        },
    )
    .unwrap();
    assert!(axiom::orch::astra_gate(&store).is_ok());
    let mut check = FrozenCheck {
        check_id: "test".into(),
        program: "test".into(),
        args: vec!["original".into()],
        env: Default::default(),
        timeout_s: 10,
        snapshot: Snapshot {
            source_digest: String::new(),
            plan_digest: String::new(),
            checks_digest: String::new(),
        },
        spec_digest: String::new(),
    };
    check.spec_digest = check_spec_digest(&check).unwrap();
    check.snapshot = frozen_snapshot(&repo, &plan, &[check.clone()]).unwrap();
    let mut tampered = check.clone();
    tampered.args = vec!["changed".into()];
    assert!(install_plan(&store, "r", rev, &plan, &[], &[tampered]).is_err());
    fs::write(repo.join("dirty"), "after").unwrap();
    assert!(install_plan(&store, "r", rev, &plan, &[], &[check.clone()]).is_err());
    fs::write(repo.join("dirty"), "before").unwrap();
    assert!(install_plan(&store, "r", rev, &plan, &[], &[check]).is_ok());
}

fn reference(path: &Path, bytes: &[u8]) -> ArtifactRef {
    fs::write(path, bytes).unwrap();
    ArtifactRef {
        path: path.to_path_buf(),
        digest: format!("{:x}", Sha256::digest(bytes)),
    }
}
#[test]
fn recovery_keeps_unknown_reservation_and_replays_one_result() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    fs::create_dir(&store).unwrap();
    let repo = tmp.path().canonicalize().unwrap();
    let intent = reference(&tmp.path().join("intent"), b"intent");
    let research = reference(&tmp.path().join("research"), b"research");
    let plan = reference(&tmp.path().join("plan"), b"plan");
    let brief = reference(&tmp.path().join("brief"), b"brief");
    let mut rev = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: repo,
                identity_digest: "repo".into(),
            },
            intent,
        },
    )
    .unwrap()
    .revision;
    rev = record_receipt(&store, "r", rev, "research", &research).unwrap();
    rev = record_core_decision(
        &store,
        "r",
        rev,
        &CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "approved".into(),
        },
    )
    .unwrap();
    let spec = StepSpec {
        run_id: "r".into(),
        step_id: "a".into(),
        purpose: Purpose::Engineer,
        brief_ref: brief,
        scope: vec!["src".into()],
        dependency_ids: vec![],
        check_ids: vec![],
    };
    rev = install_plan(&store, "r", rev, &plan, &[spec.clone()], &[]).unwrap();
    let claim = claim_step(&store, rev, &spec, "input").unwrap();
    rev = claim.revision;
    assert!(matches!(
        reconcile_run(&store, "r", rev, &[]).unwrap().actions[0],
        RecoveryAction::DispatchPrepared { .. }
    ));
    rev = load_run(&store, "r").unwrap().revision;
    rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "e1".into(),
            attempt_id: claim.attempt.attempt_id.clone(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: 1,
                thread_id: "t".into(),
            },
        },
    )
    .unwrap();
    let duplicate = AttemptEvent {
        event_id: "e1".into(),
        attempt_id: claim.attempt.attempt_id.clone(),
        sequence: 1,
        kind: AttemptEventKind::Dispatching {
            generation: 1,
            thread_id: "t".into(),
        },
    };
    rev = record_attempt_event(&store, "r", rev, &duplicate).unwrap();
    let mut conflict = duplicate.clone();
    conflict.kind = AttemptEventKind::Dispatching {
        generation: 2,
        thread_id: "t".into(),
    };
    assert!(record_attempt_event(&store, "r", rev, &conflict).is_err());
    assert!(record_attempt_event(
        &store,
        "r",
        rev - 1,
        &AttemptEvent {
            event_id: "old".into(),
            attempt_id: claim.attempt.attempt_id.clone(),
            sequence: 2,
            kind: AttemptEventKind::Running {
                turn_id: "x".into()
            }
        }
    )
    .is_err());
    let reconcile = reconcile_run(&store, "r", rev, &[]).unwrap();
    rev = reconcile.revision;
    assert!(matches!(
        reconcile.actions[0],
        RecoveryAction::ReadHistory { .. }
    ));
    assert_eq!(load_run(&store, "r").unwrap().reservations.len(), 1);
    assert!(claim_step(&store, rev, &spec, "different").is_err());
    // Simulate a process loss after immutable raw report and outcome reached disk.
    let reports = store.join("runs/r/reports");
    fs::create_dir_all(&reports).unwrap();
    fs::write(
        reports.join(format!("{}.json", claim.attempt.attempt_id)),
        br#"{"ok":true}"#,
    )
    .unwrap();
    fs::write(
        reports.join(format!("{}.outcome.json", claim.attempt.attempt_id)),
        br#""completed""#,
    )
    .unwrap();
    let projected = store.join("runs/r/evidence");
    fs::create_dir_all(&projected).unwrap();
    fs::write(
        projected.join(format!("attempt-{}.json", claim.attempt.attempt_id)),
        br#"{"ok":true}"#,
    )
    .unwrap();
    rev = reconcile_run(&store, "r", rev, &[]).unwrap().revision;
    let result = commit_result(
        &store,
        "r",
        rev,
        &claim.attempt.attempt_id,
        AttemptOutcome::Completed,
        &json!({"ok":true}),
    )
    .unwrap();
    let repeat = commit_result(
        &store,
        "r",
        result.revision,
        &claim.attempt.attempt_id,
        AttemptOutcome::Completed,
        &json!({"ok":true}),
    )
    .unwrap();
    assert_eq!(repeat.value, result.value);
    assert!(commit_result(
        &store,
        "r",
        repeat.revision,
        &claim.attempt.attempt_id,
        AttemptOutcome::Completed,
        &json!({"ok":false})
    )
    .is_err());
    let c = load_run(&store, "r").unwrap();
    assert_eq!(c.pending_evidence.len(), 1);
    assert!(c.reservations.is_empty());
}

#[test]
fn verifier_review_requires_independent_thread_and_frozen_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let store = tmp.path().join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("file"), "source").unwrap();
    let intent = reference(&store.join("intent"), b"intent");
    let research = reference(&store.join("research"), b"research");
    let plan = reference(&store.join("plan"), b"plan");
    let brief = reference(&store.join("brief"), b"brief");
    let stdout = reference(&store.join("out"), b"ok");
    let stderr = reference(&store.join("err"), b"");
    let report = reference(&store.join("review"), b"review");
    let mut rev = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: repo.canonicalize().unwrap(),
                identity_digest: "repo".into(),
            },
            intent,
        },
    )
    .unwrap()
    .revision;
    rev = record_receipt(&store, "r", rev, "research", &research).unwrap();
    rev = record_core_decision(
        &store,
        "r",
        rev,
        &CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "approved".into(),
        },
    )
    .unwrap();
    let mut check = FrozenCheck {
        check_id: "check".into(),
        program: "verify".into(),
        args: vec![],
        env: Default::default(),
        timeout_s: 10,
        snapshot: Snapshot {
            source_digest: String::new(),
            plan_digest: String::new(),
            checks_digest: String::new(),
        },
        spec_digest: String::new(),
    };
    check.spec_digest = check_spec_digest(&check).unwrap();
    check.snapshot = frozen_snapshot(&repo, &plan, &[check.clone()]).unwrap();
    let step = |id: &str, purpose, scope: &str| StepSpec {
        run_id: "r".into(),
        step_id: id.into(),
        purpose,
        brief_ref: brief.clone(),
        scope: vec![scope.into()],
        dependency_ids: vec![],
        check_ids: vec![],
    };
    let engineer = step("engineer", Purpose::Engineer, "file");
    let verifier = step("verifier", Purpose::Verifier, "other");
    rev = install_plan(
        &store,
        "r",
        rev,
        &plan,
        &[engineer.clone(), verifier.clone()],
        &[check.clone()],
    )
    .unwrap();
    let e = claim_step(&store, rev, &engineer, "e").unwrap();
    rev = e.revision;
    rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "dispatch-e".into(),
            attempt_id: e.attempt.attempt_id.clone(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: 1,
                thread_id: "worker-thread".into(),
            },
        },
    )
    .unwrap();
    let v = claim_step(&store, rev, &verifier, "v").unwrap();
    rev = v.revision;
    rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "dispatch-v".into(),
            attempt_id: v.attempt.attempt_id.clone(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: 1,
                thread_id: "review-thread".into(),
            },
        },
    )
    .unwrap();
    let identity = ExecutorIdentity {
        attempt_id: v.attempt.attempt_id.clone(),
        thread_id: "review-thread".into(),
        model: "same-provider".into(),
        purpose: Purpose::Verifier,
        policy: ExecutionPolicy::ScratchCheck,
    };
    let mut result = CheckResult {
        check_id: "check".into(),
        snapshot: check.snapshot.clone(),
        spec_digest: check.spec_digest.clone(),
        exit_code: Some(0),
        policy_applied: false,
        stdout,
        stderr,
        executor: identity.clone(),
    };
    assert!(record_check(&store, "r", rev, &result).is_err());
    result.policy_applied = true;
    rev = record_check(&store, "r", rev, &result).unwrap();
    let mut reviewer = identity;
    reviewer.policy = ExecutionPolicy::ProductReadOnly;
    let mut review = IndependentReview {
        review_id: "review".into(),
        snapshot: check.snapshot,
        executor: reviewer,
        check_ids: vec!["check".into()],
        pass: true,
        report,
    };
    review.executor.attempt_id = e.attempt.attempt_id;
    review.executor.thread_id = "worker-thread".into();
    assert!(record_independent_review(&store, "r", rev, &review).is_err());
    review.executor.attempt_id = v.attempt.attempt_id;
    review.executor.thread_id = "review-thread".into();
    let good = review.snapshot.clone();
    review.snapshot.source_digest = "wrong".into();
    assert!(record_independent_review(&store, "r", rev, &review).is_err());
    review.snapshot = good;
    assert!(record_independent_review(&store, "r", rev, &review).is_ok());
}

#[test]
fn engineering_requires_typed_baseline_observations_and_the_gate() {
    // Baseline receipts alone are observations; only the typed gate receipt
    // authorizes writers, so an unprepared or infrastructure-failed baseline
    // cannot open engineering.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let store = tmp.path().join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let intent = reference(&store.join("intent"), b"intent");
    let research = reference(&store.join("research"), b"research");
    let plan = reference(&store.join("plan"), b"plan");
    let brief = reference(&store.join("brief"), b"brief");
    let mut rev = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: repo.canonicalize().unwrap(),
                identity_digest: "repo".into(),
            },
            intent,
        },
    )
    .unwrap()
    .revision;
    rev = record_receipt(&store, "r", rev, "research", &research).unwrap();
    rev = record_core_decision(
        &store,
        "r",
        rev,
        &CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "yes".into(),
        },
    )
    .unwrap();
    rev = advance_phase(&store, "r", rev, Phase::IntentReview).unwrap();
    rev = advance_phase(&store, "r", rev, Phase::Planning).unwrap();
    let mut check = FrozenCheck {
        check_id: "test".into(),
        program: "test".into(),
        args: vec![],
        env: Default::default(),
        timeout_s: 10,
        snapshot: Snapshot {
            source_digest: String::new(),
            plan_digest: String::new(),
            checks_digest: String::new(),
        },
        spec_digest: String::new(),
    };
    check.spec_digest = check_spec_digest(&check).unwrap();
    check.snapshot = frozen_snapshot(&repo, &plan, &[check.clone()]).unwrap();
    let step = StepSpec {
        run_id: "r".into(),
        step_id: "u1".into(),
        purpose: Purpose::Engineer,
        brief_ref: brief,
        scope: vec!["one.txt".into()],
        dependency_ids: vec![],
        check_ids: vec!["test".into()],
    };
    rev = install_plan(&store, "r", rev, &plan, &[step], &[check.clone()]).unwrap();
    assert!(advance_phase(&store, "r", rev, Phase::Engineering)
        .unwrap_err()
        .message
        .contains("phase transition missing evidence"));
    let result = CheckResult {
        check_id: "test".into(),
        snapshot: check.snapshot.clone(),
        spec_digest: check.spec_digest.clone(),
        exit_code: Some(1),
        policy_applied: true,
        stdout: reference(&store.join("stdout"), b"out"),
        stderr: reference(&store.join("stderr"), b"err"),
        executor: ExecutorIdentity {
            attempt_id: "a".into(),
            thread_id: "t".into(),
            model: "m".into(),
            purpose: Purpose::Verifier,
            policy: ExecutionPolicy::ScratchCheck,
        },
    };
    let baseline = reference(
        &store.join("baseline"),
        &serde_json::to_vec(&result).unwrap(),
    );
    rev = record_receipt(&store, "r", rev, "baseline-test", &baseline).unwrap();
    assert!(
        advance_phase(&store, "r", rev, Phase::Engineering).is_err(),
        "baseline receipts do not classify the infrastructure"
    );
    let gate = reference(&store.join("gate"), b"ready");
    rev = record_receipt(&store, "r", rev, BASELINE_GATE, &gate).unwrap();
    advance_phase(&store, "r", rev, Phase::Engineering).unwrap();
    assert_eq!(load_run(&store, "r").unwrap().phase, Phase::Engineering);
}
