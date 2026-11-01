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
fn dispatch_model_receipt_is_atomic_bound_and_required_for_history_replay() {
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
    let step = StepSpec {
        run_id: "r".into(),
        step_id: "research".into(),
        purpose: Purpose::Research,
        brief_ref: intent,
        scope: vec![],
        dependency_ids: vec![],
        check_ids: vec![],
    };
    let claim = claim_step(&store, c.revision, &step, "input").unwrap();
    let aid = &claim.attempt.attempt_id;
    let mut event = AttemptEvent {
        event_id: "dispatch".into(),
        attempt_id: aid.clone(),
        sequence: 1,
        kind: AttemptEventKind::Dispatching {
            generation: 7,
            thread_id: "event-failed-thread".into(),
        },
    };
    let event_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&event).unwrap()));
    let event_path = store.join(format!("runs/r/evidence/event-{event_digest}.json"));
    fs::create_dir_all(event_path.parent().unwrap()).unwrap();
    fs::write(&event_path, b"immutable conflict").unwrap();
    assert!(record_attempt_dispatch(
        &store,
        "r",
        claim.revision,
        &event,
        "model",
        "model",
        Some("high")
    )
    .is_err());
    let after_failure = load_run(&store, "r").unwrap();
    assert_eq!(after_failure.revision, claim.revision);
    assert_eq!(after_failure.attempts[aid].state, AttemptState::Prepared);
    assert!(!after_failure
        .receipts
        .contains_key(&format!("attempt-model-{aid}")));
    assert!(!after_failure.receipts.contains_key("event-dispatch"));
    event.kind = AttemptEventKind::Dispatching {
        generation: 7,
        thread_id: "checkpoint-failed-thread".into(),
    };
    #[cfg(windows)]
    {
        let checkpoint = store.join("runs/r/checkpoint.json");
        let mut permissions = fs::metadata(&checkpoint).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&checkpoint, permissions).unwrap();
        let failed_write = record_attempt_dispatch(
            &store,
            "r",
            claim.revision,
            &event,
            "model",
            "model",
            Some("high"),
        );
        let mut permissions = fs::metadata(&checkpoint).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&checkpoint, permissions).unwrap();
        assert!(
            failed_write.is_err(),
            "checkpoint replacement must fail on a read-only destination"
        );
        assert_eq!(load_run(&store, "r").unwrap(), after_failure);
    }
    // Both the corrupt event orphan and any successful artifacts written
    // before the failed checkpoint replacement remain unreferenced.
    event.kind = AttemptEventKind::Dispatching {
        generation: 7,
        thread_id: "thread".into(),
    };
    assert_eq!(
        record_attempt_dispatch(
            &store,
            "r",
            claim.revision,
            &event,
            "model",
            "other",
            Some("high")
        )
        .unwrap_err()
        .kind,
        "model_identity"
    );
    let rev = record_attempt_dispatch(
        &store,
        "r",
        claim.revision,
        &event,
        "model",
        "model",
        Some("high"),
    )
    .unwrap();
    assert_eq!(rev, claim.revision + 1);
    let c = load_run(&store, "r").unwrap();
    let receipt = attempt_model(&store, &c, aid).unwrap();
    assert_eq!(receipt.requested_effort.as_deref(), Some("high"));
    assert!(c.receipts.contains_key("event-dispatch"));
    let key = format!("attempt-model-{aid}");
    assert!(record_receipt(&store, "r", rev, &key, &c.receipts[&key]).is_err());
    let original = fs::read(&c.receipts[&key].path).unwrap();
    let mut wrong_path = c.clone();
    let copied = reference(&store.join("copied-model-receipt.json"), &original);
    wrong_path.receipts.insert(key.clone(), copied);
    assert_eq!(
        attempt_model(&store, &wrong_path, aid).unwrap_err().kind,
        "model_identity"
    );
    fs::write(&c.receipts[&key].path, b"tampered").unwrap();
    assert_eq!(
        attempt_model(&store, &c, aid).unwrap_err().kind,
        "model_identity"
    );
    for (field, value) in [
        ("run_id", json!("wrong")),
        ("step_id", json!("wrong")),
        ("attempt_id", json!("wrong")),
        ("input_digest", json!("wrong")),
        ("generation", json!(8)),
        ("thread_id", json!("wrong")),
        ("configured_model", json!("other")),
        ("requested_model", json!("")),
        ("dispatch_event_id", json!("wrong")),
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
        changed[field] = value;
        let bytes = serde_json::to_vec(&changed).unwrap();
        let mut forged = c.clone();
        forged.receipts.get_mut(&key).unwrap().digest = format!("{:x}", Sha256::digest(&bytes));
        fs::write(&c.receipts[&key].path, &bytes).unwrap();
        assert_eq!(
            attempt_model(&store, &forged, aid).unwrap_err().kind,
            "model_identity",
            "{field}"
        );
    }
    fs::write(&c.receipts[&key].path, &original).unwrap();
    let mut missing = c.clone();
    missing.receipts.remove(&key);
    assert_eq!(
        attempt_model(&store, &missing, aid).unwrap_err().kind,
        "model_identity"
    );
    let rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "running".into(),
            attempt_id: aid.clone(),
            sequence: 2,
            kind: AttemptEventKind::Running {
                turn_id: "turn".into(),
            },
        },
    )
    .unwrap();
    let c = load_run(&store, "r").unwrap();
    let history = json!({"status":"completed","thread_id":"thread","turn_id":"turn","model":null,"model_source":"thread/read"});
    assert!(validate_attempt_report(&store, &c, aid, &history).is_ok());
    let mut contradictory = history.clone();
    contradictory["model"] = json!("other");
    assert_eq!(
        validate_attempt_report(&store, &c, aid, &contradictory)
            .unwrap_err()
            .kind,
        "model_identity"
    );
    contradictory["model"] = serde_json::Value::Null;
    contradictory
        .as_object_mut()
        .unwrap()
        .remove("model_source");
    assert!(validate_attempt_report(&store, &c, aid, &contradictory).is_err());
    let terminal =
        commit_result(&store, "r", rev, aid, AttemptOutcome::Completed, &history).unwrap();
    let c = load_run(&store, "r").unwrap();
    let (report, _, receipt) = terminal_attempt_report(&store, &c, aid).unwrap();
    assert!(report["model"].is_null());
    assert_eq!(receipt.configured_model, "model");
    fs::write(&terminal.value.path, b"report tampered").unwrap();
    assert!(terminal_attempt_report(&store, &c, aid).is_err());
}

fn baseline_step(brief: &ArtifactRef, check: &FrozenCheck) -> StepSpec {
    StepSpec {
        run_id: "r".into(),
        step_id: "baseline".into(),
        purpose: Purpose::Verifier,
        brief_ref: brief.clone(),
        scope: vec![],
        dependency_ids: vec![],
        check_ids: vec![check.check_id.clone()],
    }
}

/// Drive the actual checkpoint baseline protocol; no arbitrary gate marker.
fn authorize_baseline(store: &Path, mut rev: u64, check: &FrozenCheck) -> u64 {
    let phase = load_run(store, "r").unwrap().phase;
    if phase == Phase::Research {
        rev = advance_phase(store, "r", rev, Phase::IntentReview).unwrap();
    }
    if load_run(store, "r").unwrap().phase == Phase::IntentReview {
        rev = advance_phase(store, "r", rev, Phase::Planning).unwrap();
    }
    let step = load_run(store, "r").unwrap().steps["baseline"].clone();
    let claim = claim_step(store, rev, &step, "baseline-input").unwrap();
    rev = record_attempt_dispatch(
        store,
        "r",
        claim.revision,
        &AttemptEvent {
            event_id: "baseline-dispatch".into(),
            attempt_id: claim.attempt.attempt_id.clone(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: 1,
                thread_id: "baseline-thread".into(),
            },
        },
        "test-model",
        "test-model",
        Some("high"),
    )
    .unwrap();
    rev = record_attempt_event(
        store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "baseline-running".into(),
            attempt_id: claim.attempt.attempt_id.clone(),
            sequence: 2,
            kind: AttemptEventKind::Running {
                turn_id: "baseline-turn".into(),
            },
        },
    )
    .unwrap();
    let result = CheckResult {
        check_id: check.check_id.clone(),
        snapshot: check.snapshot.clone(),
        spec_digest: check.spec_digest.clone(),
        exit_code: Some(1),
        policy_applied: true,
        stdout: reference(&store.join("baseline-stdout"), b""),
        stderr: reference(&store.join("baseline-stderr"), b"product absent"),
        executor: ExecutorIdentity {
            attempt_id: claim.attempt.attempt_id.clone(),
            thread_id: "baseline-thread".into(),
            model: "test-model".into(),
            purpose: Purpose::Verifier,
            policy: ExecutionPolicy::ScratchCheck,
        },
    };
    rev = record_baseline_check(store, "r", rev, &result).unwrap();
    let report = json!({
        "status": "completed", "model": null, "model_source": "thread/read",
        "thread_id": "baseline-thread", "turn_id": "baseline-turn",
        "structured_output": {
            "infrastructure_ready": true, "summary": "product absent before engineering",
            "observations": [{"check_id": check.check_id, "classification": "product_missing"}]
        }
    });
    let committed = commit_result(
        store,
        "r",
        rev,
        &claim.attempt.attempt_id,
        AttemptOutcome::Completed,
        &report,
    )
    .unwrap();
    rev = record_baseline_gate(store, "r", committed.revision, &committed.value).unwrap();
    advance_phase(store, "r", rev, Phase::Engineering).unwrap()
}

#[test]
fn legacy_public_prefix_remains_classifiable_but_old_snapshot_cannot_dispatch_rework() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let store = tmp.path().join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    let intent = reference(&store.join("intent"), b"intent");
    let mut c = create_run(
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
    .unwrap();
    let plan_bytes = serde_json::to_vec(&json!({"checks":[{"id":"independent-public"}]})).unwrap();
    let plan = reference(&store.join("plan"), &plan_bytes);
    c.plan = Some(plan.clone());
    let snapshot = Snapshot {
        source_digest: "source".into(),
        plan_digest: plan.digest.clone(),
        checks_digest: "checks".into(),
    };
    for id in ["independent-public", "independent-private"] {
        c.checks.insert(
            id.into(),
            FrozenCheck {
                check_id: id.into(),
                program: "check".into(),
                args: vec![],
                env: Default::default(),
                timeout_s: 10,
                snapshot: snapshot.clone(),
                spec_digest: "spec".into(),
            },
        );
    }
    let prep_bytes = serde_json::to_vec(&json!({"structured_output":{
        "status":"ready", "exposure":"private case", "checks":[{"id":"independent-private"}]
    }}))
    .unwrap();
    let prep = reference(&store.join("prep"), &prep_bytes);
    c.receipts.insert("evaluation-0".into(), prep.clone());
    c.attempts.insert(
        "prep-attempt".into(),
        Attempt {
            attempt_id: "prep-attempt".into(),
            step_id: "evaluation-prep-0".into(),
            input_digest: "input".into(),
            state: AttemptState::Terminal,
            generation: Some(1),
            thread_id: Some("prep-thread".into()),
            turn_id: Some("prep-turn".into()),
            raw_result: Some(prep.clone()),
            outcome: Some(AttemptOutcome::Completed),
            last_event_sequence: 2,
        },
    );
    let public = public_check_ids(&store, &c).unwrap();
    assert_eq!(
        public.into_iter().collect::<Vec<_>>(),
        vec!["independent-public"]
    );
    assert_eq!(
        c.checks.len() - 1,
        1,
        "only the actual private check is counted"
    );
    fs::write(&plan.path, b"tampered").unwrap();
    assert!(public_check_ids(&store, &c).is_err());
    fs::write(&plan.path, &plan_bytes).unwrap();
    fs::write(&prep.path, b"tampered").unwrap();
    assert!(public_check_ids(&store, &c).is_err());
    fs::write(&prep.path, &prep_bytes).unwrap();
    let mut ambiguous = c.clone();
    ambiguous.receipts.remove("evaluation-0");
    assert!(public_check_ids(&store, &ambiguous).is_err());

    let brief = reference(&store.join("brief"), b"assemble");
    for (id, purpose) in [
        ("assembly", Purpose::Assembler),
        ("verification", Purpose::Verifier),
    ] {
        c.steps.insert(
            id.into(),
            StepSpec {
                run_id: "r".into(),
                step_id: id.into(),
                purpose,
                brief_ref: brief.clone(),
                scope: vec![],
                dependency_ids: vec![],
                check_ids: vec![],
            },
        );
    }
    let raw_report = reference(&store.join("raw-report"), b"report");
    let review = IndependentReview {
        review_id: "rejected".into(),
        snapshot,
        executor: ExecutorIdentity {
            attempt_id: "review".into(),
            thread_id: "review".into(),
            model: "test".into(),
            purpose: Purpose::Verifier,
            policy: ExecutionPolicy::ProductReadOnly,
        },
        check_ids: vec!["independent-public".into(), "independent-private".into()],
        pass: false,
        report: raw_report,
    };
    let review = reference(&store.join("review"), &serde_json::to_vec(&review).unwrap());
    c.receipts.insert("review-rejected".into(), review);
    let public_result = reference(&store.join("public-result"), b"public result");
    let private_result = reference(&store.join("private-result"), b"private result");
    c.receipts
        .insert("check-independent-public".into(), public_result);
    c.receipts
        .insert("check-independent-private".into(), private_result);
    c.phase = Phase::Verifying;
    c.status = RunStatus::Blocked;
    fs::write(
        store.join("runs/r/checkpoint.json"),
        serde_json::to_vec(&c).unwrap(),
    )
    .unwrap();
    // The public/private historical classification remains inspectable, but a
    // pre-v2 snapshot cannot be used to dispatch rework under the new policy.
    assert_eq!(
        prepare_rework(&store, "r", c.revision).unwrap_err().kind,
        "legacy_snapshot"
    );
    assert_eq!(load_run(&store, "r").unwrap(), c);
}
#[test]
fn recovery_keeps_unknown_reservation_and_replays_one_result() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    let repo = tmp.path().join("repo");
    fs::create_dir(&store).unwrap();
    fs::create_dir(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let repo = repo.canonicalize().unwrap();
    let intent = reference(&store.join("intent"), b"intent");
    let research = reference(&store.join("research"), b"research");
    let plan = reference(&store.join("plan"), b"plan");
    let brief = reference(&store.join("brief"), b"brief");
    let mut rev = create_run(
        &store,
        &RunSeed {
            run_id: "r".into(),
            repo: RepoIdentity {
                canonical_root: repo.clone(),
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
        brief_ref: brief.clone(),
        scope: vec!["src".into()],
        dependency_ids: vec![],
        check_ids: vec![],
    };
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
    let baseline = baseline_step(&brief, &check);
    rev = install_plan(
        &store,
        "r",
        rev,
        &plan,
        &[spec.clone(), baseline],
        &[check.clone()],
    )
    .unwrap();
    rev = authorize_baseline(&store, rev, &check);
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
        RecoveryAction::NeedsDecision { .. }
    ));
    assert_eq!(load_run(&store, "r").unwrap().reservations.len(), 1);
    assert!(claim_step(&store, rev, &spec, "different").is_err());
    // Bare report/outcome files are not an original role's scope audit.
    let reports = store.join("runs/r/reports");
    fs::create_dir_all(&reports).unwrap();
    fs::write(
        reports.join(format!("{}.json", claim.attempt.attempt_id)),
        br#"{"ok":true,"scope_checked":true}"#,
    )
    .unwrap();
    fs::write(
        reports.join(format!("{}.outcome.json", claim.attempt.attempt_id)),
        br#""completed""#,
    )
    .unwrap();
    let reconciled = reconcile_run(&store, "r", rev, &[]).unwrap();
    rev = reconciled.revision;
    assert!(matches!(
        reconciled.actions[0],
        RecoveryAction::NeedsDecision { .. }
    ));
    assert!(commit_result(
        &store,
        "r",
        rev,
        &claim.attempt.attempt_id,
        AttemptOutcome::Completed,
        &json!({"ok":true,"scope_checked":true})
    )
    .is_err());
    assert_eq!(load_run(&store, "r").unwrap().reservations.len(), 1);
    rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "unknown".into(),
            attempt_id: claim.attempt.attempt_id.clone(),
            sequence: 2,
            kind: AttemptEventKind::OutcomeUnknown {
                reason: "connection lost".into(),
            },
        },
    )
    .unwrap();
    let terminal = reference(&store.join("terminal"), &serde_json::to_vec(&json!({
        "attempt_id":claim.attempt.attempt_id,"thread_id":"t","generation":1,"turn_id":null,"outcome":"completed"
    })).unwrap());
    assert!(resolve_attempt(
        &store,
        "r",
        rev,
        &RecoveryDecision::AcceptTerminal {
            attempt_id: claim.attempt.attempt_id.clone(),
            report: terminal,
            reason: "history said completed".into(),
        }
    )
    .is_err());
    assert_eq!(load_run(&store, "r").unwrap().reservations.len(), 1);
    let stopped = reference(&store.join("stopped"), &serde_json::to_vec(&json!({
        "attempt_id":claim.attempt.attempt_id,"thread_id":"t","generation":1,"turn_id":null,"terminal":true
    })).unwrap());
    let repair = reference(
        &store.join("repair"),
        &serde_json::to_vec(&json!({
            "attempt_id":claim.attempt.attempt_id,"repaired":true
        }))
        .unwrap(),
    );
    rev = resolve_attempt(
        &store,
        "r",
        rev,
        &RecoveryDecision::RetryAfterRepair {
            attempt_id: claim.attempt.attempt_id.clone(),
            stopped_evidence: stopped,
            repair_evidence: repair,
            reason: "confirmed stopped and repaired".into(),
        },
    )
    .unwrap();
    let c = load_run(&store, "r").unwrap();
    assert!(c.reservations.is_empty());
    assert_eq!(
        c.attempts[&claim.attempt.attempt_id].outcome,
        Some(AttemptOutcome::Cancelled)
    );
    assert_eq!(
        claim_step(&store, rev, &spec, "new-after-repair")
            .unwrap()
            .disposition,
        ClaimDisposition::New
    );
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
    let forged_report = reference(&store.join("forged-review"), b"review");
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
    let step = |id: &str, purpose, scope: Vec<String>| StepSpec {
        run_id: "r".into(),
        step_id: id.into(),
        purpose,
        brief_ref: brief.clone(),
        scope,
        dependency_ids: vec![],
        check_ids: vec!["check".into()],
    };
    let engineer = step("content", Purpose::Content, vec![]);
    let assembler = step("assembly", Purpose::Assembler, vec!["file".into()]);
    let verifier = step("verification", Purpose::Verifier, vec![]);
    let baseline = baseline_step(&brief, &check);
    rev = install_plan(
        &store,
        "r",
        rev,
        &plan,
        &[
            engineer.clone(),
            assembler.clone(),
            verifier.clone(),
            baseline,
        ],
        &[check.clone()],
    )
    .unwrap();
    rev = authorize_baseline(&store, rev, &check);
    let before_verification = load_run(&store, "r").unwrap();
    let baseline_attempt = before_verification
        .attempts
        .values()
        .find(|a| a.step_id == "baseline")
        .unwrap();
    let mut result = CheckResult {
        check_id: "check".into(),
        snapshot: check.snapshot.clone(),
        spec_digest: check.spec_digest.clone(),
        exit_code: Some(0),
        policy_applied: true,
        stdout,
        stderr,
        executor: ExecutorIdentity {
            attempt_id: baseline_attempt.attempt_id.clone(),
            thread_id: "baseline-thread".into(),
            model: "test-model".into(),
            purpose: Purpose::Verifier,
            policy: ExecutionPolicy::ScratchCheck,
        },
    };
    // The old public API accepted a completed baseline verifier's result while
    // the run was still in Engineering; it must not create verification evidence.
    assert!(record_check(&store, "r", rev, &result).is_err());
    assert!(claim_step(&store, rev, &verifier, "too-early").is_err());

    let start_attempt =
        |mut revision: u64, attempt: &Attempt, prefix: &str, thread: &str, turn: &str| {
            revision = record_attempt_dispatch(
                &store,
                "r",
                revision,
                &AttemptEvent {
                    event_id: format!("{prefix}-dispatch"),
                    attempt_id: attempt.attempt_id.clone(),
                    sequence: 1,
                    kind: AttemptEventKind::Dispatching {
                        generation: 1,
                        thread_id: thread.into(),
                    },
                },
                "test-model",
                "test-model",
                Some("high"),
            )
            .unwrap();
            record_attempt_event(
                &store,
                "r",
                revision,
                &AttemptEvent {
                    event_id: format!("{prefix}-running"),
                    attempt_id: attempt.attempt_id.clone(),
                    sequence: 2,
                    kind: AttemptEventKind::Running {
                        turn_id: turn.into(),
                    },
                },
            )
            .unwrap()
        };
    let e = claim_step(&store, rev, &engineer, "e").unwrap();
    rev = start_attempt(e.revision, &e.attempt, "e", "worker-thread", "worker-turn");
    // Content is a read-only role and can complete without a writer audit.
    rev = commit_result(
        &store,
        "r",
        rev,
        &e.attempt.attempt_id,
        AttemptOutcome::Completed,
        &json!({"status":"completed","thread_id":"worker-thread","turn_id":"worker-turn","model":"test-model"}),
    )
    .unwrap()
    .revision;
    rev = advance_phase(&store, "r", rev, Phase::Assembling).unwrap();
    let a = claim_step(&store, rev, &assembler, "a").unwrap();
    rev = start_attempt(
        a.revision,
        &a.attempt,
        "a",
        "assembly-thread",
        "assembly-turn",
    );
    let assembly = reference(
        &store.join("historical-assembly"),
        br#"{"status":"completed"}"#,
    );
    let checkpoint_path = store.join("runs/r/checkpoint.json");
    let mut fixture = load_run(&store, "r").unwrap();
    let completed = fixture.attempts.get_mut(&a.attempt.attempt_id).unwrap();
    completed.state = AttemptState::Terminal;
    completed.outcome = Some(AttemptOutcome::Completed);
    completed.raw_result = Some(assembly.clone());
    fixture.reservations.clear();
    fs::write(&checkpoint_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    rev = record_receipt(&store, "r", rev, "assembly", &assembly).unwrap();
    assert!(
        advance_phase(&store, "r", rev, Phase::Verifying).is_err(),
        "historical unaudited writer cannot authorize verification"
    );
    // Exercise review gates on an already-verifying historical checkpoint;
    // this fixture does not manufacture an audit or authorize an assembly.
    fixture = load_run(&store, "r").unwrap();
    fixture.phase = Phase::Verifying;
    fs::write(&checkpoint_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    rev = freeze_verification(&store, "r", rev).unwrap();
    check = load_run(&store, "r").unwrap().checks["check"].clone();
    result.snapshot = check.snapshot.clone();
    assert!(
        record_check(&store, "r", rev, &result).is_err(),
        "baseline attempt is not the verification attempt"
    );
    let v = claim_step(&store, rev, &verifier, "v").unwrap();
    rev = v.revision;
    result.executor.attempt_id = v.attempt.attempt_id.clone();
    result.executor.thread_id = "review-thread".into();
    result.executor.model = "same-provider".into();
    assert!(
        record_check(&store, "r", rev, &result).is_err(),
        "prepared is not running"
    );
    rev = record_attempt_dispatch(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "v-dispatch".into(),
            attempt_id: v.attempt.attempt_id.clone(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: 1,
                thread_id: "review-thread".into(),
            },
        },
        "same-provider",
        "same-provider",
        Some("high"),
    )
    .unwrap();
    assert!(
        record_check(&store, "r", rev, &result).is_err(),
        "dispatching is not running"
    );
    rev = record_attempt_event(
        &store,
        "r",
        rev,
        &AttemptEvent {
            event_id: "v-running".into(),
            attempt_id: v.attempt.attempt_id.clone(),
            sequence: 2,
            kind: AttemptEventKind::Running {
                turn_id: "review-turn".into(),
            },
        },
    )
    .unwrap();
    result.policy_applied = false;
    assert!(record_check(&store, "r", rev, &result).is_err());
    result.policy_applied = true;
    let checkpoint_path = store.join("runs/r/checkpoint.json");
    let original_checkpoint = fs::read(&checkpoint_path).unwrap();
    let mut unauthorized_step = load_run(&store, "r").unwrap();
    unauthorized_step
        .steps
        .get_mut("verification")
        .unwrap()
        .check_ids
        .clear();
    fs::write(
        &checkpoint_path,
        serde_json::to_vec(&unauthorized_step).unwrap(),
    )
    .unwrap();
    assert!(
        record_check(&store, "r", rev, &result).is_err(),
        "frozen check must also be authorized for this verifier"
    );
    fs::write(&checkpoint_path, original_checkpoint).unwrap();
    let mut unauthorized = result.clone();
    unauthorized.check_id = "unfrozen".into();
    assert!(record_check(&store, "r", rev, &unauthorized).is_err());
    unauthorized = result.clone();
    unauthorized.executor.attempt_id = baseline_attempt.attempt_id.clone();
    unauthorized.executor.thread_id = "baseline-thread".into();
    assert!(record_check(&store, "r", rev, &unauthorized).is_err());
    rev = record_check(&store, "r", rev, &result).unwrap();
    let mut review = IndependentReview {
        review_id: v.attempt.attempt_id.clone(),
        snapshot: check.snapshot,
        executor: ExecutorIdentity {
            policy: ExecutionPolicy::ProductReadOnly,
            ..result.executor.clone()
        },
        check_ids: vec!["check".into()],
        pass: true,
        report: forged_report.clone(),
    };
    assert!(
        record_independent_review(&store, "r", rev, &review).is_err(),
        "running verifier is not a terminal report"
    );
    let terminal = commit_result(&store, "r", rev, &v.attempt.attempt_id, AttemptOutcome::Completed,
        &json!({"status":"completed", "model":null, "model_source":"thread/read", "thread_id":"review-thread", "turn_id":"review-turn",
            "structured_output":{"pass":true,"summary":"independently checked","defect_owner":"none"}})).unwrap();
    rev = terminal.revision;
    assert!(
        record_check(&store, "r", rev, &result).is_err(),
        "terminal verifier cannot add checks"
    );
    assert!(
        claim_step(&store, rev, &verifier, "different-input").is_err(),
        "completed verifier cannot be replaced in the same epoch"
    );
    assert!(
        record_independent_review(&store, "r", rev, &review).is_err(),
        "arbitrary review bytes are not the raw report"
    );
    review.report = terminal.value;
    let valid = review.clone();
    review.review_id = "forged".into();
    assert!(record_independent_review(&store, "r", rev, &review).is_err());
    review = valid.clone();
    review.pass = false;
    assert!(
        record_independent_review(&store, "r", rev, &review).is_err(),
        "decision must match report"
    );
    review = valid.clone();
    review.check_ids.clear();
    assert!(
        record_independent_review(&store, "r", rev, &review).is_err(),
        "partial review is not acceptance"
    );
    review = valid.clone();
    review.check_ids.push("check".into());
    assert!(
        record_independent_review(&store, "r", rev, &review).is_err(),
        "duplicate check is not complete coverage"
    );
    review = valid.clone();
    review.executor.model = "other-model".into();
    assert!(record_independent_review(&store, "r", rev, &review).is_err());
    review = valid.clone();
    review.executor.attempt_id = baseline_attempt.attempt_id.clone();
    review.executor.thread_id = "baseline-thread".into();
    assert!(record_independent_review(&store, "r", rev, &review).is_err());
    record_independent_review(&store, "r", rev, &valid).unwrap();
    let c = load_run(&store, "r").unwrap();
    let accepted = c.receipts[&format!("review-{}", v.attempt.attempt_id)].clone();
    assert_eq!(verified_review(&c), Some(&accepted));
    let mut nonindependent = c.clone();
    nonindependent
        .attempts
        .get_mut(&a.attempt.attempt_id)
        .unwrap()
        .thread_id = Some("review-thread".into());
    assert!(verified_review(&nonindependent).is_none());
    let original = fs::read(&accepted.path).unwrap();
    fs::write(&accepted.path, b"tampered").unwrap();
    assert!(verified_review(&c).is_none());
    fs::write(&accepted.path, original).unwrap();
    let raw = fs::read(&valid.report.path).unwrap();
    fs::write(&valid.report.path, b"forged terminal report").unwrap();
    assert!(verified_review(&c).is_none());
    fs::write(&valid.report.path, raw).unwrap();
    assert!(verified_review(&c).is_some());
    fs::write(repo.join("file"), "changed after review").unwrap();
    assert!(verified_review(&c).is_none());
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
        brief_ref: brief.clone(),
        scope: vec!["one.txt".into()],
        dependency_ids: vec![],
        check_ids: vec!["test".into()],
    };
    let baseline_step = baseline_step(&brief, &check);
    rev = install_plan(
        &store,
        "r",
        rev,
        &plan,
        &[step.clone(), baseline_step],
        &[check.clone()],
    )
    .unwrap();
    assert!(advance_phase(&store, "r", rev, Phase::Engineering).is_err());
    assert!(claim_step(&store, rev, &step, "without-gate").is_err());
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
    assert!(record_receipt(&store, "r", rev, "baseline-test", &baseline).is_err());
    assert!(
        record_baseline_check(&store, "r", rev, &result).is_err(),
        "no live bound baseline verifier exists"
    );
    assert!(
        advance_phase(&store, "r", rev, Phase::Engineering).is_err(),
        "baseline receipts do not classify the infrastructure"
    );
    let gate = reference(&store.join("gate"), b"ready");
    assert!(record_receipt(&store, "r", rev, BASELINE_GATE, &gate).is_err());
    assert!(record_baseline_gate(&store, "r", rev, &gate).is_err());
    rev = authorize_baseline(&store, rev, &check);
    assert_eq!(load_run(&store, "r").unwrap().phase, Phase::Engineering);
    let claim = claim_step(&store, rev, &step, "after-gate").unwrap();
    assert_eq!(claim.disposition, ClaimDisposition::New);

    // Simulate a historical checkpoint: its v1 tree has no ignored-input
    // evidence. Historical records stay intact and cannot authorize v2 work.
    let checkpoint_path = store.join("runs/r/checkpoint.json");
    let mut legacy = load_run(&store, "r").unwrap();
    legacy
        .checks
        .get_mut("test")
        .unwrap()
        .snapshot
        .source_digest = format!("tree:{}", "a".repeat(40));
    fs::write(&checkpoint_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let original = fs::read(&checkpoint_path).unwrap();
    assert_eq!(
        claim_step(&store, legacy.revision, &step, "old-write")
            .unwrap_err()
            .kind,
        "legacy_snapshot"
    );
    assert_eq!(
        advance_phase(&store, "r", legacy.revision, Phase::Assembling)
            .unwrap_err()
            .kind,
        "legacy_snapshot"
    );
    assert_eq!(
        record_check(&store, "r", legacy.revision, &result)
            .unwrap_err()
            .kind,
        "legacy_snapshot"
    );
    assert!(!verified_outcome(&legacy));
    assert_eq!(fs::read(&checkpoint_path).unwrap(), original);

    let recovered = reconcile_run(&store, "r", legacy.revision, &[]).unwrap();
    assert!(recovered.actions.iter().any(|action| matches!(action,
        RecoveryAction::NeedsDecision { attempt_id, .. } if attempt_id == &claim.attempt.attempt_id)));
    legacy = load_run(&store, "r").unwrap();
    legacy.phase = Phase::Verifying;
    fs::write(&checkpoint_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let original = fs::read(&checkpoint_path).unwrap();
    assert_eq!(
        freeze_verification(&store, "r", legacy.revision)
            .unwrap_err()
            .kind,
        "legacy_snapshot"
    );
    assert_eq!(fs::read(&checkpoint_path).unwrap(), original);
    legacy.phase = Phase::ReadyToCommit;
    legacy.status = RunStatus::ReadyToCommit;
    fs::write(&checkpoint_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let original = fs::read(&checkpoint_path).unwrap();
    assert_eq!(
        advance_phase(&store, "r", legacy.revision, Phase::Released)
            .unwrap_err()
            .kind,
        "legacy_snapshot"
    );
    assert_eq!(fs::read(&checkpoint_path).unwrap(), original);
}
