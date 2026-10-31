use axiom::orch::{checkpoint, commit_dispatch, wait_event, wait_state, Checkpoint, Event};

#[test]
fn missing_enrolled_registry_never_falls_back_to_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    commit_dispatch(p, "t").unwrap();
    let e = event(p, "build");
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    std::fs::write(
        p.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    std::fs::write(p.join("wait-required"), "1\n").unwrap();
    std::fs::remove_file(p.join("units.tsv")).unwrap();
    assert!(commit_dispatch(p, "t").is_err());
    assert!(wait_event(p, e).is_err());
    assert!(axiom::orch::complete_round(p, "t", "verification").is_err());
    assert!(axiom::orch::release_gate(p).is_err());
    assert!(checkpoint(p, checkpoint_value()).is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
}

#[test]
fn malformed_enrolled_registry_is_not_empty() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    std::fs::write(p.join("wait-required"), "1\n").unwrap();
    std::fs::write(p.join("units.tsv"), "broken header\n").unwrap();
    assert!(wait_state(p).is_err());
}

#[test]
fn enrollment_commits_journal_and_missing_or_unreadable_journal_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n").unwrap();
    let args = vec!["wait".into(), "enroll".into()];
    let out = axiom::orch::run(p, &args, true, false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(wait_state(p).unwrap().engineering_enrolled);
    std::fs::remove_file(p.join("core-wait.json")).unwrap();
    assert!(wait_state(p).unwrap_err().message.contains("journal is missing"));
    // Re-enrollment cannot silently replace a lost enrolled history.
    assert_ne!(axiom::orch::run(p, &args, true, false).exit_code, 0);
    std::fs::create_dir(p.join("core-wait.json")).unwrap();
    assert_eq!(wait_state(p).unwrap_err().kind, "io");
}

// Synthetic host records exercise replay and the real registered release API;
// they are fixtures, not evidence that live model dispatch was performed.
fn registered_continuation_fixture(p: &std::path::Path) -> (Event, Vec<axiom::orch::Evidence>) {
    use axiom::orch::*;
    use serde_json::json;
    use sha2::Digest;
    let repo = p.join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    std::fs::write(repo.join("out"), "artifact").unwrap();
    let mut c = checkpoint_value();
    c.severity = "normal".into();
    for round in 1..5 {
        c.round = round;
        c.evidence_version = round.into();
        checkpoint(p, c.clone()).unwrap();
        commit_dispatch(p, "t").unwrap();
        for kind in [
            "build",
            "assembly",
            "verification",
            "core_approve",
            "completion",
            "release",
        ] {
            advance(p, &c, kind);
        }
    }
    std::fs::write(p.join("brief"), "GOAL test\nSCOPE out\nVERIFY echo ok\nDELIVERY verification\nOUTPUTS out\n").unwrap();
    std::fs::write(
        p.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n",
            p.join("brief").display()
        ),
    )
    .unwrap();
    std::fs::write(p.join("modules.md"), "## m\nPURPOSE test\nPATHS out\nDEPENDS none\nCONTRACT test\nACCEPT test `echo ok`\nFULL TEST `echo ok`\n").unwrap();
    c.round = 5;
    c.evidence_version = 5;
    c.contract = contract_identity(p, "t").unwrap();
    c.repo = repo.to_string_lossy().into_owned();
    checkpoint(p, c.clone()).unwrap();
    commit_dispatch(p, "t").unwrap();
    let mut state = wait_state(p).unwrap();
    let dispatch = state.events.last().unwrap().clone();
    let artifact = snapshot(&repo, &["out".into()]);
    let source = snapshot(&repo, &[".".into()]);
    assert!(!artifact.is_empty());
    let package_path = p.join("package");
    std::fs::write(&package_path, "package bytes").unwrap();
    let hash = format!("sha256:{:x}", sha2::Sha256::digest(b"package bytes"));
    for (kind, actor, model) in [
        ("build", "worker", "gpt-6.1-terra"),
        ("assembly_intent", "lifecycle", ""),
        ("assembly", "sol", "gpt-6.1-sol"),
        ("package_intent", "lifecycle", ""),
        ("package", "lifecycle", ""),
        ("verification_intent", "lifecycle", ""),
        ("verification", "astra-verify", "gpt-6-astra"),
        ("notification", "lifecycle", ""),
        ("core_approve", "core", ""),
        ("completion", "core", ""),
    ] {
        let mut e = dispatch.clone();
        e.id = format!("round5-{kind}");
        e.kind = kind.into();
        e.actor = actor.into();
        e.artifact = artifact.clone();
        let mut report = json!({
            "status":"completed", "actual_effort":"high", "model":model,
            "actual_sandbox": if kind == "verification" {"readOnly"} else {"workspaceWrite"},
            "thread_id":"fixture-thread", "turn_id":kind, "source_identity":source,
            "binding":{"task":c.task,"round":c.round,"attempt":dispatch.id,
                "artifact":artifact,"issue":c.issue,"evidence_version":c.evidence_version,"contract":c.contract},
            "package_path":package_path, "package_hash":hash,
            "structured_output":{"issues":["root-a","root-b","root-c"],"severity":"normal"}
        });
        if kind == "core_approve" {
            report = json!({"valid_issues":["root-a","root-b","root-c"],"severity":"normal","decision":"continue"});
            e.evidence = report.to_string();
        } else {
            e.evidence = format!("host:{report}");
        }
        state.events.push(e);
    }
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    wait_state(p).unwrap(); // Replay must validate the full fixture before use.
    let receipt_path = p.join("verification-receipt.json");
    let receipt = serde_json::to_vec(&json!({"command":"echo ok","source_before":source,
        "source_after":source,"exit_code":0,"timed_out":false,"error":null,"stdout":"ok","stderr":""})).unwrap();
    std::fs::write(&receipt_path, &receipt).unwrap();
    let mut rows: Vec<Evidence> = [
        ("run", "gpt-6.1-terra"),
        ("scope", "scope"),
        ("contract", c.contract.as_str()),
        ("dispatch", dispatch.id.as_str()),
        ("verify", "echo ok"),
        ("accept", "echo ok"),
        ("review", "gpt-6-astra"),
    ]
    .into_iter()
    .map(|(kind, command)| Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: 0,
        commit: artifact.clone(),
        ts: "fixture".into(),
    })
    .collect();
    rows.push(Evidence {
        kind: "verification_receipt".into(),
        command: receipt_path.to_string_lossy().into_owned(),
        exit_code: 0,
        commit: format!("sha256:{:x}", sha2::Sha256::digest(&receipt)),
        ts: "fixture".into(),
    });
    let mut release = dispatch;
    release.id = "round5-release".into();
    release.kind = "release".into();
    release.actor = "core".into();
    release.artifact = artifact;
    release.evidence = "explicit approved round completion".into();
    (release, rows)
}

#[test]
fn continuation_never_bypasses_registered_engineering_acceptance() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (release, baseline) = registered_continuation_fixture(p);
    assert_eq!(derive_state(&baseline, true), "verified");
    for failure in [
        "run",
        "scope",
        "verify",
        "accept",
        "review",
        "missing-accept",
        "missing-review",
        "same-reviewer",
        "stale-accept",
        "stale-contract",
        "wrong-dispatch",
    ] {
        let mut rows = baseline.clone();
        match failure {
            "missing-accept" => rows.retain(|r| r.kind != "accept"),
            "missing-review" => rows.retain(|r| r.kind != "review"),
            "same-reviewer" => {
                rows.iter_mut()
                    .find(|r| r.kind == "review")
                    .unwrap()
                    .command = "gpt-6.1-terra".into()
            }
            "stale-accept" => {
                rows.iter_mut().find(|r| r.kind == "accept").unwrap().commit = "old".into()
            }
            "stale-contract" => {
                rows.iter_mut()
                    .find(|r| r.kind == "contract")
                    .unwrap()
                    .command = "old".into()
            }
            "wrong-dispatch" => {
                rows.iter_mut()
                    .find(|r| r.kind == "dispatch")
                    .unwrap()
                    .command = "other".into()
            }
            kind => rows.iter_mut().find(|r| r.kind == kind).unwrap().exit_code = 1,
        }
        write_fixture_evidence(p, &rows);
        let before = std::fs::read(p.join("core-wait.json")).unwrap();
        let error = wait_event(p, release.clone()).unwrap_err();
        assert!(
            error
                .message
                .contains("release requires independent review and acceptance")
                || error.message.contains("matching actual verification")
                || error.message.contains("another dispatch"),
            "{failure}: {error:?}"
        );
        assert_eq!(
            std::fs::read(p.join("core-wait.json")).unwrap(),
            before,
            "{failure}"
        );
        assert!(!task_released(&wait_state(p).unwrap(), "t"));
        assert!(release_gate(p).is_err(), "{failure}");
    }
    // Positive control: the same continuation is released only after all
    // engineering evidence passes. Unit completion is independent of product policy.
    write_fixture_evidence(p, &baseline);
    let released = wait_event(p, release).unwrap();
    assert!(released
        .events
        .iter()
        .any(|e| e.round == 5 && e.kind == "release"));
    assert!(may_finish(&released, "t"));
    assert!(release_gate(p).is_ok());
    assert!(released.product_runs.is_empty());
}

#[test]
fn core_continuation_assessments_are_validated_on_write_and_replay() {
    use axiom::orch::*;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (_, baseline) = registered_continuation_fixture(p);
    write_fixture_evidence(p, &baseline);
    let mut before_verdict = wait_state(p).unwrap();
    let candidate = before_verdict
        .events
        .iter()
        .find(|e| e.round == 5 && e.kind == "core_approve")
        .unwrap()
        .clone();
    before_verdict
        .events
        .retain(|e| e.round != 5 || !["core_approve", "completion"].contains(&e.kind.as_str()));
    for (name, issues, valid, severity, core_severity, proof, decision, accepted) in [
        (
            "two roots cannot continue",
            vec!["a", "b"],
            vec!["a", "b"],
            "normal",
            "normal",
            "proof",
            "continue",
            false,
        ),
        (
            "three roots continue",
            vec!["a", "b", "c"],
            vec!["a", "b", "c"],
            "normal",
            "normal",
            "proof",
            "continue",
            true,
        ),
        (
            "duplicate",
            vec!["a", "b"],
            vec!["a", "a", "b"],
            "normal",
            "normal",
            "proof",
            "continue",
            false,
        ),
        (
            "unknown",
            vec!["a", "b"],
            vec!["a", "b", "unknown"],
            "normal",
            "normal",
            "proof",
            "continue",
            false,
        ),
        (
            "resolved excluded",
            vec!["a", "b", "resolved"],
            vec!["a", "b"],
            "normal",
            "normal",
            "proof",
            "finish",
            true,
        ),
        (
            "severe",
            vec![],
            vec![],
            "severe",
            "severe",
            "proof",
            "continue",
            true,
        ),
        (
            "exceptional",
            vec![],
            vec![],
            "excellent",
            "excellent",
            "proof",
            "continue",
            true,
        ),
        (
            "unsupported core severity",
            vec![],
            vec![],
            "normal",
            "severe",
            "proof",
            "continue",
            false,
        ),
        (
            "unsupported exceptional",
            vec![],
            vec![],
            "excellent",
            "excellent",
            "",
            "continue",
            false,
        ),
    ] {
        let mut state = before_verdict.clone();
        let verification = state
            .events
            .iter_mut()
            .find(|e| e.round == 5 && e.kind == "verification")
            .unwrap();
        let mut report: serde_json::Value =
            serde_json::from_str(verification.evidence.strip_prefix("host:").unwrap()).unwrap();
        report["structured_output"] = json!({"issues":issues,"severity":severity,"evidence":proof});
        verification.evidence = format!("host:{report}");
        std::fs::write(
            p.join("core-wait.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let original = std::fs::read(p.join("core-wait.json")).unwrap();
        let mut verdict = candidate.clone();
        verdict.evidence =
            json!({"valid_issues":valid,"severity":core_severity,"decision":decision}).to_string();
        assert_eq!(
            wait_event(p, verdict.clone()).is_ok(),
            accepted,
            "write: {name}"
        );
        if !accepted {
            assert_eq!(
                std::fs::read(p.join("core-wait.json")).unwrap(),
                original,
                "{name}"
            );
        }
        // A corrupted persisted verdict must not gain authority through replay.
        state.events.push(verdict);
        std::fs::write(
            p.join("core-wait.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert_eq!(wait_state(p).is_ok(), accepted, "replay: {name}");
    }
}

fn write_fixture_evidence(p: &std::path::Path, rows: &[axiom::orch::Evidence]) {
    let mut text = "kind\tcommand\texit_code\tcommit\tts\n".to_string();
    for row in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            row.kind, row.command, row.exit_code, row.commit, row.ts
        ));
    }
    std::fs::create_dir_all(p.join("reports")).unwrap();
    std::fs::write(p.join("reports/t.evidence.tsv"), text).unwrap();
}
#[test]
fn prepare_refreshes_only_an_unclaimed_current_round() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let repo = p.join("repo");
    std::fs::create_dir(&repo).unwrap();
    let mut first = checkpoint_value();
    first.min_rounds = 3;
    checkpoint(p, first.clone()).unwrap();
    commit_dispatch(p, "t").unwrap();
    for kind in [
        "build",
        "assembly",
        "verification",
        "core_approve",
        "completion",
        "release",
    ] {
        advance(p, &first, kind);
    }
    let brief = p.join("brief");
    std::fs::write(&brief, "GOAL test\nSCOPE a.txt\nVERIFY echo ok\nDELIVERY implementation\nOUTPUTS a.txt\n").unwrap();
    std::fs::write(
        p.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let prepare = |minimum: &str| {
        axiom::orch::run(
            p,
            &[
                "wait".into(),
                "prepare".into(),
                "--task".into(),
                "t".into(),
                "--repo".into(),
                repo.to_string_lossy().into_owned(),
                "--issue".into(),
                "i".into(),
                "--proposal".into(),
                "p".into(),
                "--min-rounds".into(),
                minimum.into(),
            ],
            true,
            false,
        )
    };
    let result = prepare("3");
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    assert_eq!(prepare("3").exit_code, 0);
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    assert_ne!(prepare("5").exit_code, 0);
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    std::fs::write(&brief, "GOAL changed\nSCOPE b.txt\nVERIFY echo ok\nDELIVERY implementation\nOUTPUTS b.txt\n").unwrap();
    assert!(commit_dispatch(p, "t").is_err());
    std::fs::write(p.join(".worker.claim"), "unknown").unwrap();
    assert_ne!(prepare("3").exit_code, 0);
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    std::fs::remove_file(p.join(".worker.claim")).unwrap();
    let result = prepare("3");
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let state = wait_state(p).unwrap();
    assert_eq!(state.checkpoints.len(), 2);
    let current = &state.checkpoints[1];
    assert_eq!(
        (current.round, current.min_rounds, current.evidence_version),
        (2, 3, 3)
    );
    assert_eq!(current.outputs, ["b.txt"]);
    let mut stale = event(p, "build");
    stale.round = 2;
    stale.evidence_version = 2;
    assert!(wait_event(p, stale).is_err());
    assert_eq!(prepare("3").exit_code, 0);
    commit_dispatch(p, "t").unwrap();
    let after = std::fs::read(p.join("core-wait.json")).unwrap();
    std::fs::write(&brief, "GOAL later\nSCOPE c.txt\nVERIFY echo ok\n").unwrap();
    assert_ne!(prepare("3").exit_code, 0);
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), after);
    assert_eq!(wait_state(p).unwrap().events.last().unwrap().round, 2);
}

fn checkpoint_value() -> Checkpoint {
    Checkpoint {
        task: "t".into(),
        module: "m".into(),
        round: 1,
        min_rounds: 5,
        owner: "o".into(),
        dependencies: vec![],
        dependency_versions: Default::default(),
        outputs: vec!["out".into()],
        evidence: vec!["e".into()],
        next_action: "build".into(),
        issue: "i".into(),
        proposal: "p".into(),
        evidence_version: 1,
        contract: "contract".into(),
        repo: String::new(),
        severity: "severe".into(),
        findings: vec![],
    }
}
fn event(p: &std::path::Path, kind: &str) -> Event {
    Event {
        id: kind.into(),
        task: "t".into(),
        round: 1,
        issue: "i".into(),
        proposal: "p".into(),
        evidence_version: 1,
        contract: "contract".into(),
        kind: kind.into(),
        evidence: "proof".into(),
        actor: match kind {
            "build" => "worker",
            "assembly" => "sol",
            "verification" => "astra-verify",
            "start_review" => "reviewer",
            _ => "core",
        }
        .into(),
        dispatch_id: wait_state(p)
            .unwrap()
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "dispatch")
            .map(|e| e.id.clone())
            .unwrap_or_default(),
        artifact: "sha256:artifact".into(),
    }
}
#[test]
fn bindings_order_restart_and_duplicate_controls() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    assert!(wait_event(p, event(p, "completion")).is_err());
    assert!(wait_event(p, event(p, "build")).is_err());
    commit_dispatch(p, "t").unwrap();
    assert!(commit_dispatch(p, "t").is_err());
    let mut stale = event(p, "build");
    stale.evidence_version = 2;
    assert!(wait_event(p, stale).is_err());
    for k in ["build", "assembly", "verification"] {
        wait_event(p, event(p, k)).unwrap();
    }
    assert!(wait_event(p, event(p, "completion")).is_err());
    wait_event(p, event(p, "core_approve")).unwrap();
    assert_eq!(wait_state(p).unwrap().events.len(), 5);
    wait_event(p, event(p, "completion")).unwrap();
    wait_event(p, event(p, "release")).unwrap();
    assert!(wait_event(p, event(p, "release")).is_err());
    assert!(axiom::orch::may_finish(&wait_state(p).unwrap(), "t"));
}

#[test]
fn dispatch_is_committed_before_lifecycle_runs_and_not_repeated() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store");
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let brief = store.join("brief");
    std::fs::write(&brief, "GOAL test\nSCOPE out\nVERIFY echo ok\nDELIVERY verification\nOUTPUTS out\n").unwrap();
    std::fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let mut c = checkpoint_value();
    c.contract = axiom::orch::contract_identity(&store, "t").unwrap();
    c.repo = repo.to_string_lossy().into_owned();
    checkpoint(&store, c).unwrap();
    assert_eq!(axiom::orch::ready_units(&store).unwrap(), ["t"]);
    assert!(matches!(
        axiom::orch::begin_unit(&store, "t", "worker", &repo, false).unwrap(),
        axiom::orch::Begin::Run { .. }
    ));
    assert_eq!(wait_state(&store).unwrap().events[0].kind, "dispatch");
    axiom::orch::abort_unit(&store, "t");
    assert!(axiom::orch::ready_units(&store).unwrap().is_empty());
    assert!(axiom::orch::begin_unit(&store, "t", "worker", &repo, false).is_err());
}
#[test]
fn unreadable_checkpoint_does_not_commit() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let build = event(p, "build");
    // Make the checkpoint unreadable without altering the saved committed bytes.
    std::fs::rename(p.join("core-wait.json"), p.join("saved.json")).unwrap();
    std::fs::create_dir(p.join("core-wait.json")).unwrap();
    assert!(wait_event(p, build).is_err());
    std::fs::remove_dir(p.join("core-wait.json")).unwrap();
    std::fs::rename(p.join("saved.json"), p.join("core-wait.json")).unwrap();
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
}

#[test]
fn dispatch_child_process() {
    if let Some(path) = std::env::var_os("CHRONOS_WAIT_CHILD_STORE") {
        commit_dispatch(std::path::Path::new(&path), "t").unwrap();
    }
}

#[test]
fn dispatch_survives_process_exit() {
    let dir = tempfile::tempdir().unwrap();
    checkpoint(dir.path(), checkpoint_value()).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "wait_cases::dispatch_child_process"])
        .env("CHRONOS_WAIT_CHILD_STORE", dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(wait_state(dir.path()).unwrap().events[0].kind, "dispatch");
    assert!(commit_dispatch(dir.path(), "t").is_err());
}

#[test]
fn concurrent_dispatch_has_at_most_one_commit() {
    let dir = tempfile::tempdir().unwrap();
    checkpoint(dir.path(), checkpoint_value()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let path = dir.path().to_owned();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            commit_dispatch(&path, "t").is_ok()
        }));
    }
    barrier.wait();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|ok| *ok)
            .count(),
        1
    );
    assert_eq!(wait_state(dir.path()).unwrap().events.len(), 1);
}

fn advance(p: &std::path::Path, c: &Checkpoint, kind: &str) {
    let mut e = event(p, kind);
    e.id = format!("{}-{}-{kind}", c.task, c.round);
    e.task = c.task.clone();
    e.round = c.round;
    e.evidence_version = c.evidence_version;
    e.contract = c.contract.clone();
    wait_event(p, e).unwrap();
}
#[test]
fn legacy_task_minimum_is_preserved_but_does_not_control_unit_completion() {
    use axiom::orch::may_finish;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let mut fast = checkpoint_value();
    fast.min_rounds = 3;
    let mut legacy = checkpoint_value();
    legacy.task = "legacy".into();
    checkpoint(p, legacy.clone()).unwrap();
    // Simulate a state written before min_rounds existed.
    let path = p.join("core-wait.json");
    let mut data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    data["checkpoints"][0]
        .as_object_mut()
        .unwrap()
        .remove("min_rounds");
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert_eq!(wait_state(p).unwrap().checkpoints[0].min_rounds, 5);
    for round in 1..=3 {
        fast.round = round;
        fast.evidence_version = round.into();
        checkpoint(p, fast.clone()).unwrap();
        commit_dispatch(p, "t").unwrap();
        for kind in ["build", "assembly", "verification"] {
            advance(p, &fast, kind);
        }
        assert!(!may_finish(&wait_state(p).unwrap(), "t"));
        for kind in ["core_approve", "completion", "release"] {
            advance(p, &fast, kind);
        }
        assert!(may_finish(&wait_state(p).unwrap(), "t"));
        let mut changed = fast.clone();
        changed.round += 1;
        changed.evidence_version += 1;
        for value in [0, 5] {
            changed.min_rounds = value;
            assert!(checkpoint(p, changed.clone()).is_err());
        }
    }
    assert!(!may_finish(&wait_state(p).unwrap(), "legacy"));
    let original = std::fs::read(&path).unwrap();
    let mut data: serde_json::Value = serde_json::from_slice(&original).unwrap();
    data["checkpoints"][2]["min_rounds"] = 5.into();
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(wait_state(p).is_err());
    data["checkpoints"][2]["min_rounds"] = 0.into();
    std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
    assert!(wait_state(p).is_err());
    std::fs::write(&path, original).unwrap();
    assert!(may_finish(&wait_state(p).unwrap(), "t"));
}
#[test]
fn rounds_versions_findings_rejections_and_latest_dependencies_fail_closed() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let mut c = checkpoint_value();
    c.round = 5;
    assert!(checkpoint(p, c.clone()).is_err()); // Cannot skip mandatory rounds.
    c.round = 1;
    c.severity = "normal".into();
    for round in 1..=7 {
        c.round = round;
        c.evidence_version = u64::from(round);
        c.findings = if round == 6 {
            vec!["a".into(), "b".into(), "c".into(), "d".into()]
        } else {
            vec![]
        };
        checkpoint(p, c.clone()).unwrap();
        assert!(!may_finish(&wait_state(p).unwrap(), "t"));
        commit_dispatch(p, "t").unwrap();
        if round >= 6 {
            let mut premature = event(p, "build");
            premature.round = round;
            premature.evidence_version = u64::from(round);
            assert!(wait_event(p, premature).is_err());
            advance(p, &c, "start_review");
        }
        for kind in ["build", "assembly", "verification"] {
            advance(p, &c, kind);
        }
        let mut early = event(p, "completion");
        early.round = round;
        early.evidence_version = u64::from(round);
        assert!(wait_event(p, early).is_err()); // Normal severity also needs core.
        advance(p, &c, "core_approve");
        advance(p, &c, "completion");
        advance(p, &c, "release");
        assert_eq!(may_finish(&wait_state(p).unwrap(), "t"), true);
        let mut stale = c.clone();
        stale.round += 1;
        assert!(checkpoint(p, stale).is_err()); // Evidence version must increase.
        let mut dependent = checkpoint_value();
        dependent.task = format!("dep{round}");
        dependent.dependencies = vec!["t".into()];
        dependent
            .dependency_versions
            .insert("t".into(), u64::from(round));
        assert!(checkpoint(p, dependent).is_ok());
    }
    assert!(!dispatch_ready(&wait_state(p).unwrap(), "dep5"));
    assert!(commit_dispatch(p, "dep5").is_err()); // A newer released dependency is not the bound version.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let c = checkpoint_value();
    checkpoint(p, c.clone()).unwrap();
    commit_dispatch(p, "t").unwrap();
    for kind in ["build", "assembly", "verification", "core_reject"] {
        advance(p, &c, kind);
    }
    assert!(wait_event(p, event(p, "core_approve")).is_err());
    assert!(wait_event(p, event(p, "completion")).is_err());
    assert!(!task_released(&wait_state(p).unwrap(), "t"));
}
#[test]
fn recovery_is_explicit_bound_audited_and_old_attempt_events_never_apply() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    commit_dispatch(p, "t").unwrap();
    let old = event(p, "build");
    let recovery = event(p, "recovery");
    assert!(wait_event(p, recovery.clone()).is_err());
    assert!(recover_dispatch(p, recovery.clone(), "maybe").is_err());
    let mut wrong = recovery.clone();
    wrong.dispatch_id = "other-attempt".into();
    assert!(recover_dispatch(p, wrong, "stopped").is_err());
    std::fs::write(
        p.join(".worker.claim"),
        format!("{}:live", std::process::id()),
    )
    .unwrap();
    assert!(recover_dispatch(p, recovery.clone(), "not_started").is_err());
    std::fs::remove_file(p.join(".worker.claim")).unwrap();
    recover_dispatch(p, recovery.clone(), "not_started").unwrap();
    // Simulate death after the recovery commit but before stale-lease cleanup.
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let dead = child.id();
    child.wait().unwrap();
    std::fs::write(p.join(".worker.claim"), format!("{dead}:abandoned")).unwrap();
    let count = wait_state(p).unwrap().events.len();
    recover_dispatch(p, recovery.clone(), "not_started").unwrap();
    assert!(!p.join(".worker.claim").exists());
    assert_eq!(wait_state(p).unwrap().events.len(), count);
    assert!(wait_event(p, old.clone()).is_err());
    commit_dispatch(p, "t").unwrap();
    assert!(wait_event(p, old).is_err());
    assert!(recover_dispatch(p, recovery, "stopped").is_err());
    wait_event(p, event(p, "build")).unwrap();
    let state = wait_state(p).unwrap();
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "dispatch").count(),
        2
    );
    assert!(state
        .events
        .iter()
        .any(|e| e.kind == "recovery" && e.evidence.starts_with("not_started:")));
}
#[test]
fn corrupt_and_unknown_schema_or_history_never_enable_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    commit_dispatch(p, "t").unwrap();
    let good = std::fs::read(p.join("core-wait.json")).unwrap();
    for mutate in [
        "schema",
        "version",
        "contract",
        "actor",
        "duplicate",
        "unknown",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&good).unwrap();
        match mutate {
            "schema" => value["schema_version"] = 99.into(),
            "version" => value["events"][0]["evidence_version"] = 99.into(),
            "contract" => value["events"][0]["contract"] = "other".into(),
            "actor" => value["events"][0]["actor"] = "core".into(),
            "duplicate" => {
                let e = value["events"][0].clone();
                value["events"].as_array_mut().unwrap().push(e);
            }
            _ => value["unexpected"] = true.into(),
        }
        std::fs::write(
            p.join("core-wait.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(wait_state(p).is_err(), "{mutate}");
        assert!(commit_dispatch(p, "t").is_err(), "{mutate}");
    }
    std::fs::write(p.join("core-wait.json"), b"{partial").unwrap();
    assert!(wait_state(p).is_err());
}
#[test]
fn crash_and_concurrency_child() {
    let Some(p) = std::env::var_os("CHRONOS_FAULT_STORE") else {
        return;
    };
    let p = std::path::PathBuf::from(p);
    let mode = std::env::var("CHRONOS_FAULT_MODE").unwrap();
    if mode == "lock" {
        axiom::orch::with_lock(&p, false, || {
            std::fs::write(p.join("locked"), "ready").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            #[allow(unreachable_code)]
            Ok(())
        })
        .unwrap();
    } else if mode == "crash" {
        let repo = p.join("repo");
        axiom::orch::begin_unit(&p, "t", "worker", &repo, false).unwrap();
        std::fs::write(p.join("committed"), "ready").unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    } else {
        let n = std::env::var("CHRONOS_FAULT_N").unwrap();
        std::fs::write(p.join(format!("ready{n}")), "ready").unwrap();
        await_file(&p.join("go"));
        let ok = commit_dispatch(&p, "t").is_ok();
        std::fs::write(p.join(format!("result{n}")), if ok { "yes" } else { "no" }).unwrap();
    }
}
fn await_file(p: &std::path::Path) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !p.exists() {
        assert!(
            std::time::Instant::now() < until,
            "timeout waiting {}",
            p.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn fault_child(p: &std::path::Path, mode: &str, n: usize) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "wait_cases::crash_and_concurrency_child"])
        .env("CHRONOS_FAULT_STORE", p)
        .env("CHRONOS_FAULT_MODE", mode)
        .env("CHRONOS_FAULT_N", n.to_string())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap()
}
#[test]
fn process_killed_after_commit_before_launch_requires_confirmed_recovery() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let repo = p.join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let brief = p.join("brief");
    std::fs::write(&brief, "GOAL test\nSCOPE out\nVERIFY echo ok\nDELIVERY verification\nOUTPUTS out\n").unwrap();
    std::fs::write(
        p.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let mut c = checkpoint_value();
    c.contract = contract_identity(p, "t").unwrap();
    c.repo = repo.to_string_lossy().into_owned();
    checkpoint(p, c.clone()).unwrap();
    let mut child = fault_child(p, "crash", 0);
    await_file(&p.join("committed"));
    assert!(begin_unit(p, "t", "worker", &repo, false).is_err());
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(ready_units(p).unwrap().is_empty());
    assert!(begin_unit(p, "t", "worker", &repo, false).is_err());
    let mut recovery = event(p, "recovery");
    recovery.contract = c.contract;
    recover_dispatch(p, recovery, "not_started").unwrap();
    assert_eq!(ready_units(p).unwrap(), ["t"]);
    assert!(matches!(
        begin_unit(p, "t", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(p, "t");
    assert!(begin_unit(p, "t", "worker", &repo, false).is_err());
}
#[test]
fn independent_processes_compete_for_one_durable_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    let mut children: Vec<_> = (0..4).map(|n| fault_child(p, "concurrent", n)).collect();
    for n in 0..4 {
        await_file(&p.join(format!("ready{n}")));
    }
    std::fs::write(p.join("go"), "go").unwrap();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(
        (0..4)
            .filter(|n| std::fs::read_to_string(p.join(format!("result{n}"))).unwrap() == "yes")
            .count(),
        1
    );
    assert_eq!(wait_state(p).unwrap().events.len(), 1);
}

#[test]
fn force_cannot_steal_live_os_lock_and_killed_owner_unlocks_without_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    let mut child = fault_child(p, "lock", 0);
    await_file(&p.join("locked"));
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let forced = axiom::orch::run(
        p,
        &["standing".into(), "add".into(), "must-not-write".into()],
        true,
        true,
    );
    assert_ne!(forced.exit_code, 0);
    assert!(forced.stderr.contains("OS lock"), "{}", forced.stderr);
    assert!(commit_dispatch(p, "t").is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    commit_dispatch(p, "t").unwrap(); // Stale PID metadata recovered under released OS mutex.
    assert!(commit_dispatch(p, "t").is_err());
}
#[cfg(windows)]
#[test]
fn failed_checkpoint_replace_does_not_dispatch_and_can_restart() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    let path = p.join("core-wait.json");
    let before = std::fs::read(&path).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let result = commit_dispatch(p, "t");
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(&path, permissions).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(wait_state(p).unwrap().events.is_empty());
    commit_dispatch(p, "t").unwrap();
    assert_eq!(wait_state(p).unwrap().events.len(), 1);
}

fn assert_rejected_without_persisting(p: &std::path::Path, candidate: Event) {
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let prior = wait_state(p).unwrap().events;
    assert!(
        wait_event(p, candidate.clone()).is_err(),
        "accepted {candidate:?}"
    );
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    let reloaded = wait_state(p).unwrap();
    assert_eq!(reloaded.events, prior);
    assert!(!axiom::orch::task_released(&reloaded, "t"));
    assert!(!reloaded
        .events
        .iter()
        .any(|e| e.task == "t" && e.kind == "release"));
}

#[test]
fn invalid_bindings_and_transitions_leave_durable_state_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    assert_rejected_without_persisting(p, event(p, "build"));
    commit_dispatch(p, "t").unwrap();

    let valid = event(p, "build");
    for field in [
        "task", "round", "issue", "proposal", "version", "contract", "dispatch",
    ] {
        let mut bad = valid.clone();
        match field {
            "task" => bad.task = "other".into(),
            "round" => bad.round += 1,
            "issue" => bad.issue = "other".into(),
            "proposal" => bad.proposal = "other".into(),
            "version" => bad.evidence_version += 1,
            "contract" => bad.contract = "other".into(),
            _ => bad.dispatch_id = "other-attempt".into(),
        }
        assert_rejected_without_persisting(p, bad);
    }
    for kind in [
        "assembly",
        "verification",
        "core_approve",
        "completion",
        "release",
    ] {
        assert_rejected_without_persisting(p, event(p, kind));
    }
    wait_event(p, valid.clone()).unwrap();
    assert_rejected_without_persisting(p, valid.clone()); // Same event ID.
    let mut duplicate_kind = valid;
    duplicate_kind.id = "another-build-id".into();
    assert_rejected_without_persisting(p, duplicate_kind);
    for kind in ["assembly", "verification", "core_approve", "completion"] {
        wait_event(p, event(p, kind)).unwrap();
    }
    wait_event(p, event(p, "release")).unwrap();
    let state = wait_state(p).unwrap();
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "release").count(),
        1
    );
}

#[test]
fn terminal_event_binding_negatives_after_prerequisites() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    commit_dispatch(p, "t").unwrap();
    for kind in ["build", "assembly", "verification", "core_approve"] {
        wait_event(p, event(p, kind)).unwrap();
    }

    for kind in ["completion", "release"] {
        let valid = event(p, kind);
        for field in [
            "task",
            "round",
            "issue",
            "proposal",
            "evidence_version",
            "contract",
            "dispatch_id",
        ] {
            let mut bad = valid.clone();
            match field {
                "task" => bad.task = "other".into(),
                "round" => bad.round += 1,
                "issue" => bad.issue = "other".into(),
                "proposal" => bad.proposal = "other".into(),
                "evidence_version" => bad.evidence_version += 1,
                "contract" => bad.contract = "other".into(),
                "dispatch_id" => bad.dispatch_id = "other-attempt".into(),
                _ => unreachable!(),
            }
            assert_rejected_without_persisting(p, bad);
        }
        wait_event(p, valid).unwrap();
    }
    let state = wait_state(p).unwrap();
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "release").count(),
        1
    );
}

#[test]
fn recovery_restart_child() {
    let Some(path) = std::env::var_os("CHRONOS_RECOVERY_RESTART_STORE") else {
        return;
    };
    let p = std::path::Path::new(&path);
    assert!(!axiom::orch::dispatch_ready(&wait_state(p).unwrap(), "t"));
    assert!(commit_dispatch(p, "t").is_err());
    axiom::orch::recover_dispatch(p, event(p, "recovery"), "stopped").unwrap();
}

#[test]
fn recovery_persists_across_process_restart_and_quarantines_old_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    checkpoint(p, checkpoint_value()).unwrap();
    commit_dispatch(p, "t").unwrap();
    let old = event(p, "build");
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    assert!(commit_dispatch(p, "t").is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "wait_cases::recovery_restart_child"])
        .env("CHRONOS_RECOVERY_RESTART_STORE", p)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = wait_state(p).unwrap();
    assert!(axiom::orch::dispatch_ready(&state, "t"));
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "recovery").count(),
        1
    );
    assert_rejected_without_persisting(p, old.clone());
    commit_dispatch(p, "t").unwrap();
    assert_ne!(event(p, "build").dispatch_id, old.dispatch_id);
    assert_rejected_without_persisting(p, old);
    wait_event(p, event(p, "build")).unwrap();
    let state = wait_state(p).unwrap();
    assert_eq!(
        state.events.iter().filter(|e| e.kind == "dispatch").count(),
        2
    );
    assert!(!axiom::orch::task_released(&state, "t"));
}

// Synthetic actual-host journals are fixtures, not claims of live model runs.
// Progress is replayed, but core verdict/completion/product transitions use the
// production write APIs and real on-disk registered engineering evidence.
fn registered_product_attempt(
    p: &std::path::Path,
    template: &axiom::orch::WaitState,
    baseline: &[axiom::orch::Evidence],
    issues: &[&str],
    severity: &str,
    proof: &str,
) -> String {
    use axiom::orch::*;
    use serde_json::json;
    let mut state = wait_state(p).unwrap();
    let mut c = current_checkpoint(&state, "t").unwrap();
    if may_finish(&state, "t") {
        c.round += 1;
        c.evidence_version += 1;
        checkpoint(p, c.clone()).unwrap();
        // Preserve the existing engineering MAX pre-review gate for repeated
        // task attempts; it is not a product continuation decision.
        state = wait_state(p).unwrap();
        let attempt = format!("product-fixture-dispatch-{}", c.round);
        let source = template
            .events
            .iter()
            .find(|e| e.round == 5 && e.kind == "build")
            .unwrap();
        let mut intent = source.clone();
        intent.id = format!("review-intent-{}", c.round);
        intent.round = c.round;
        intent.evidence_version = c.evidence_version;
        intent.kind = "review_intent".into();
        intent.actor = "lifecycle".into();
        intent.dispatch_id = attempt;
        intent.evidence = "fixture durable review intent".into();
        state.events.push(intent.clone());
        let mut review = intent;
        review.id = format!("pre-review-{}", c.round);
        review.kind = "start_review".into();
        review.actor = "reviewer".into();
        let mut report: serde_json::Value =
            serde_json::from_str(source.evidence.trim_start_matches("host:")).unwrap();
        report["actual_effort"] = json!("max");
        report["actual_sandbox"] = json!("readOnly");
        report["model"] = json!("gpt-6-astra");
        report["binding"]["round"] = json!(c.round);
        report["binding"]["evidence_version"] = json!(c.evidence_version);
        report["binding"]["attempt"] = json!(review.dispatch_id);
        review.evidence = format!("host:{report}");
        state.events.push(review);
        std::fs::write(
            p.join("core-wait.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        wait_state(p).unwrap();
        commit_dispatch(p, "t").unwrap();
        state = wait_state(p).unwrap();
    } else {
        // The initial fixture has a real dispatch and a synthetic completed
        // progress prefix. Remove the prefix to exercise actual verdict writes.
        state
            .events
            .retain(|e| e.round != c.round || e.kind == "dispatch");
    }
    let dispatch = state
        .events
        .iter()
        .rev()
        .find(|e| e.task == "t" && e.kind == "dispatch")
        .unwrap()
        .clone();
    for source in template.events.iter().filter(|e| {
        e.round == 5
            && !["dispatch", "core_approve", "completion", "release"].contains(&e.kind.as_str())
    }) {
        let mut e = source.clone();
        e.id = format!("attempt-{}-{}", c.round, e.kind);
        e.round = c.round;
        e.evidence_version = c.evidence_version;
        e.dispatch_id = dispatch.id.clone();
        let mut report: serde_json::Value =
            serde_json::from_str(e.evidence.trim_start_matches("host:")).unwrap();
        report["binding"]["round"] = json!(c.round);
        report["binding"]["evidence_version"] = json!(c.evidence_version);
        report["binding"]["attempt"] = json!(dispatch.id);
        if e.kind == "verification" {
            report["structured_output"] =
                json!({"issues":issues,"severity":severity,"evidence":proof});
        }
        e.evidence = format!("host:{report}");
        state.events.push(e);
    }
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    wait_state(p).unwrap();
    let mut rows = baseline.to_vec();
    rows.iter_mut()
        .find(|e| e.kind == "dispatch")
        .unwrap()
        .command = dispatch.id;
    write_fixture_evidence(p, &rows);
    let verification = format!("attempt-{}-verification", c.round);
    core_verdict(
        p,
        "t",
        &verification,
        true,
        &json!({"valid_issues":issues.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            "severity":"normal","decision":"finish"})
        .to_string(),
    )
    .unwrap();
    complete_round(p, "t", &verification).unwrap();
    assert!(may_finish(&wait_state(p).unwrap(), "t"));
    verification
}

#[test]
fn durable_product_rounds_one_to_seven_and_distinct_counts_zero_to_four() {
    use axiom::orch::*;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "product-a").unwrap();
    let (_, baseline) = registered_continuation_fixture(p);
    let template = wait_state(p).unwrap();
    let roots = ["a", "b", "c", "d"];
    for round in 1..=7 {
        if round > 1 {
            product_checkpoint(p, "product-a").unwrap();
        }
        let before_attempt = std::fs::read(p.join("core-wait.json")).unwrap();
        let mut continuing = vec![];
        for count in 0..=4 {
            // Branch the same persisted prefix rather than fabricate policy state.
            std::fs::write(p.join("core-wait.json"), &before_attempt).unwrap();
            let issues = &roots[..count];
            let verification =
                registered_product_attempt(p, &template, &baseline, issues, "normal", "proof");
            let assessment =
                json!({"valid_issues":issues,"severity":"normal","decision":"finish"}).to_string();
            let required = round < 5 || count >= 3;
            let action = if required { "continue" } else { "finish" };
            let before = std::fs::read(p.join("core-wait.json")).unwrap();
            assert!(product_transition(
                p,
                "product-a",
                round,
                vec![verification.clone()],
                &assessment,
                if required { "finish" } else { "continue" }
            )
            .is_err());
            assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
            let committed = product_transition(
                p,
                "product-a",
                round,
                vec![verification.clone()],
                &assessment,
                action,
            )
            .unwrap();
            let replay = wait_state(p).unwrap();
            assert_eq!(committed.product_runs, replay.product_runs);
            assert_eq!(
                product_policy_finished(&replay, "product-a"),
                !required,
                "product round {round}; roots {count}"
            );
            assert_eq!(current_checkpoint(&replay, "t").unwrap().round, round + 4);
            product_transition(
                p,
                "product-a",
                round,
                vec![verification],
                &assessment,
                action,
            )
            .unwrap();
            assert_eq!(
                wait_state(p).unwrap().product_runs[0].rounds.len(),
                round as usize
            );
            if count == 3 {
                continuing = std::fs::read(p.join("core-wait.json")).unwrap();
            }
            if required {
                product_checkpoint(p, "product-a").unwrap();
                let next = wait_state(p).unwrap();
                assert_eq!(next.product_runs[0].rounds.last().unwrap().round, round + 1);
                assert!(!product_policy_finished(&next, "product-a"));
                let last = replay.product_runs[0]
                    .rounds
                    .last()
                    .unwrap()
                    .transition
                    .as_ref()
                    .unwrap();
                assert!(product_transition(
                    p,
                    "product-a",
                    round + 1,
                    last.verification_ids.clone(),
                    &last.assessment,
                    "continue"
                )
                .is_err());
            } else {
                assert!(product_checkpoint(p, "product-a").is_err());
            }
        }
        std::fs::write(p.join("core-wait.json"), continuing).unwrap();
    }
}

#[test]
fn finished_product_requires_fresh_round_after_engineering_changes() {
    use axiom::orch::*;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "run").unwrap();
    let (_, baseline) = registered_continuation_fixture(p);
    let template = wait_state(p).unwrap();
    let assessment = json!({"valid_issues":[],"severity":"normal","decision":"finish"}).to_string();
    for round in 1..=5 {
        if round > 1 {
            product_checkpoint(p, "run").unwrap();
        }
        let id = registered_product_attempt(p, &template, &baseline, &[], "normal", "proof");
        product_transition(
            p,
            "run",
            round,
            vec![id],
            &assessment,
            if round < 5 { "continue" } else { "finish" },
        )
        .unwrap();
    }
    assert!(product_policy_finished(&wait_state(p).unwrap(), "run"));
    assert!(product_checkpoint(p, "run").is_err());
    let journal = p.join("core-wait.json");
    let finished = std::fs::read(&journal).unwrap();
    let evidence_path = p.join("reports/t.evidence.tsv");
    let saved_evidence = std::fs::read(&evidence_path).unwrap();
    let review = evidence(p, "t")
        .unwrap()
        .into_iter()
        .find(|r| r.kind == "review")
        .unwrap();
    append_evidence(p, "t", &review).unwrap();
    assert!(done_gate_in_repo(p, true, Some(&p.join("repo")))
        .unwrap_err()
        .message
        .contains("product rounds"));
    product_checkpoint(p, "run").unwrap();
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
    std::fs::write(&journal, &finished).unwrap();
    std::fs::write(evidence_path, saved_evidence).unwrap();
    let mut c = current_checkpoint(&wait_state(p).unwrap(), "t").unwrap();
    c.round += 1;
    c.evidence_version += 1;
    checkpoint(p, c).unwrap();
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
    product_checkpoint(p, "run").unwrap();
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
    std::fs::write(&journal, finished).unwrap();

    // A complete new unit attempt is not retroactively a product round.
    let old = registered_product_attempt(p, &template, &baseline, &[], "normal", "proof");
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
    assert!(done_gate_in_repo(p, true, Some(&p.join("repo")))
        .unwrap_err()
        .message
        .contains("product rounds"));
    product_checkpoint(p, "run").unwrap();
    assert!(product_transition(p, "run", 6, vec![old], &assessment, "finish").is_err());
    let roots = ["a", "b", "c"];
    let id = registered_product_attempt(p, &template, &baseline, &roots, "normal", "proof");
    let continuation =
        json!({"valid_issues":roots,"severity":"normal","decision":"finish"}).to_string();
    assert!(product_transition(p, "run", 6, vec![id.clone()], &continuation, "finish").is_err());
    product_transition(p, "run", 6, vec![id], &continuation, "continue").unwrap();
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
    product_checkpoint(p, "run").unwrap();
    let id = registered_product_attempt(p, &template, &baseline, &[], "normal", "proof");
    product_transition(p, "run", 7, vec![id], &assessment, "finish").unwrap();
    assert!(product_policy_finished(&wait_state(p).unwrap(), "run"));
}

#[test]
fn durable_product_severe_excellent_finish_and_unsupported_controls() {
    use axiom::orch::*;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "run").unwrap();
    let (_, baseline) = registered_continuation_fixture(p);
    let template = wait_state(p).unwrap();
    for round in 1..5 {
        if round > 1 {
            product_checkpoint(p, "run").unwrap();
        }
        let roots = ["a", "b", "c"];
        let verification =
            registered_product_attempt(p, &template, &baseline, &roots, "normal", "proof");
        product_transition(
            p,
            "run",
            round,
            vec![verification],
            &json!({"valid_issues":roots,"severity":"normal","decision":"finish"}).to_string(),
            "continue",
        )
        .unwrap();
    }
    product_checkpoint(p, "run").unwrap();
    let prefix = std::fs::read(p.join("core-wait.json")).unwrap();
    for (name, issues, valid, observed, assessed, proof, decision, accepted, required) in [
        (
            "severe supported finish",
            vec![],
            vec![],
            "severe",
            "severe",
            "proof",
            "finish",
            true,
            true,
        ),
        (
            "excellent supported finish",
            vec![],
            vec![],
            "excellent",
            "excellent",
            "proof",
            "finish",
            true,
            true,
        ),
        (
            "severe supported continue",
            vec![],
            vec![],
            "severe",
            "severe",
            "proof",
            "continue",
            true,
            true,
        ),
        (
            "excellent supported continue",
            vec![],
            vec![],
            "excellent",
            "excellent",
            "proof",
            "continue",
            true,
            true,
        ),
        (
            "no severe proof",
            vec![],
            vec![],
            "severe",
            "severe",
            "",
            "finish",
            true,
            false,
        ),
        (
            "no excellent proof",
            vec![],
            vec![],
            "excellent",
            "excellent",
            "  ",
            "finish",
            true,
            false,
        ),
        (
            "no core confirmation",
            vec![],
            vec![],
            "severe",
            "normal",
            "proof",
            "finish",
            true,
            false,
        ),
        (
            "severity mismatch",
            vec![],
            vec![],
            "normal",
            "severe",
            "proof",
            "continue",
            false,
            false,
        ),
        (
            "unsupported continue",
            vec!["a", "b"],
            vec!["a", "b"],
            "normal",
            "normal",
            "proof",
            "continue",
            false,
            false,
        ),
        (
            "duplicate report roots",
            vec!["a", "a", "b"],
            vec!["a", "b"],
            "normal",
            "normal",
            "proof",
            "finish",
            true,
            false,
        ),
        (
            "duplicate core roots",
            vec!["a", "b"],
            vec!["a", "a", "b"],
            "normal",
            "normal",
            "proof",
            "finish",
            false,
            false,
        ),
        (
            "unknown core root",
            vec!["a", "b"],
            vec!["a", "b", "unknown"],
            "normal",
            "normal",
            "proof",
            "finish",
            false,
            false,
        ),
        (
            "resolved excluded",
            vec!["a", "b", "resolved"],
            vec!["a", "b"],
            "normal",
            "normal",
            "proof",
            "finish",
            true,
            false,
        ),
        (
            "three roots cannot be waived",
            vec!["a", "b", "c"],
            vec!["a", "b", "c"],
            "normal",
            "normal",
            "proof",
            "finish",
            true,
            true,
        ),
        (
            "unknown decision",
            vec![],
            vec![],
            "normal",
            "normal",
            "proof",
            "maybe",
            false,
            false,
        ),
    ] {
        std::fs::write(p.join("core-wait.json"), &prefix).unwrap();
        let verification =
            registered_product_attempt(p, &template, &baseline, &issues, observed, proof);
        let assessment =
            json!({"valid_issues":valid,"severity":assessed,"decision":decision}).to_string();
        let original = std::fs::read(p.join("core-wait.json")).unwrap();
        let action = if required { "continue" } else { "finish" };
        if accepted && required {
            assert!(
                product_transition(
                    p,
                    "run",
                    5,
                    vec![verification.clone()],
                    &assessment,
                    "finish"
                )
                .is_err(),
                "supported continuation cannot be waived on write: {name}"
            );
            assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), original);
        }
        assert_eq!(
            product_transition(p, "run", 5, vec![verification.clone()], &assessment, action)
                .is_ok(),
            accepted,
            "write: {name}"
        );
        if accepted {
            let state = wait_state(p).unwrap();
            assert_eq!(product_policy_finished(&state, "run"), !required, "{name}");
            if required {
                product_checkpoint(p, "run").unwrap();
                assert_eq!(
                    wait_state(p).unwrap().product_runs[0]
                        .rounds
                        .last()
                        .unwrap()
                        .round,
                    6
                );
            } else {
                assert!(product_checkpoint(p, "run").is_err());
            }
        } else {
            assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), original);
        }
        // Even a syntactically valid injected transition must fail replay
        // for unsupported assessments or a policy-violating finish.
        let mut state: WaitState = serde_json::from_slice(&original).unwrap();
        let end = state.events.len();
        state.product_runs[0].rounds[4].transition = Some(ProductTransition {
            verification_ids: vec![verification],
            assessment,
            action: action.into(),
            event_end: end,
            checkpoint_end: Some(state.checkpoints.clone()),
            engineering_end: None,
        });
        std::fs::write(
            p.join("core-wait.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert_eq!(wait_state(p).is_ok(), accepted, "replay: {name}");
        if accepted && required {
            state.product_runs[0].rounds[4]
                .transition
                .as_mut()
                .unwrap()
                .action = "finish".into();
            std::fs::write(
                p.join("core-wait.json"),
                serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
            assert!(wait_state(p).is_err(), "cannot waive: {name}");
        }
    }
}

#[test]
fn explicit_legacy_migration_preserves_history_without_inventing_product_rounds() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (release, rows) = registered_continuation_fixture(p);
    write_fixture_evidence(p, &rows);
    wait_event(p, release).unwrap();
    let mut legacy: serde_json::Value = serde_json::to_value(wait_state(p).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("product_runs");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("engineering_enrolled");
    legacy["checkpoints"][0]
        .as_object_mut()
        .unwrap()
        .remove("min_rounds");
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(p.join("core-wait.json"), &bytes).unwrap();
    assert!(wait_state(p).is_err());
    assert!(commit_dispatch(p, "t").is_err());
    assert!(release_gate(p).is_err());
    assert!(product_checkpoint(p, "run").is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), bytes);
    let migrated = migrate_wait(p).unwrap();
    assert_eq!(migrated.schema_version, 2);
    assert_eq!(std::fs::read(p.join("core-wait.v1.json")).unwrap(), bytes);
    assert_eq!(
        serde_json::to_value(&migrated.events).unwrap(),
        legacy["events"]
    );
    assert_eq!(migrated.checkpoints[0].min_rounds, 5);
    assert_eq!(migrated.checkpoints.len(), 5);
    assert!(migrated.product_runs.is_empty());
    assert!(!product_policy_finished(&migrated, "run"));
    let after = std::fs::read(p.join("core-wait.json")).unwrap();
    migrate_wait(p).unwrap();
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), after);
    product_checkpoint(p, "run").unwrap();
    assert!(product_transition(
        p,
        "run",
        1,
        vec!["round5-verification".into()],
        r#"{"valid_issues":[],"severity":"normal","decision":"finish"}"#,
        "continue"
    )
    .is_err());
    let started = wait_state(p).unwrap();
    assert_eq!(started.product_runs[0].rounds[0].round, 1);
    assert!(started.product_runs[0].rounds[0].transition.is_none());
    // Unknown schema and archive conflicts fail without destroying either file.
    std::fs::write(p.join("core-wait.json"), &bytes).unwrap();
    std::fs::write(p.join("core-wait.v1.json"), b"historical evidence").unwrap();
    assert!(migrate_wait(p).is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), bytes);
    assert_eq!(
        std::fs::read(p.join("core-wait.v1.json")).unwrap(),
        b"historical evidence"
    );
    let mut unknown = legacy.clone();
    unknown["schema_version"] = 99.into();
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&unknown).unwrap(),
    )
    .unwrap();
    assert!(migrate_wait(p).is_err());
}

#[test]
fn missing_enrolled_registry_blocks_current_registered_evidence_and_product_paths() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "run").unwrap();
    let (_, baseline) = registered_continuation_fixture(p);
    let template = wait_state(p).unwrap();
    let verification =
        registered_product_attempt(p, &template, &baseline, &["a", "b", "c"], "normal", "proof");
    std::fs::write(p.join("wait-required"), "1\n").unwrap();
    let state = wait_state(p).unwrap();
    let repo = p.join("repo");
    assert!(current_verified_unit(p, "t", &repo).unwrap());
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let evidence_before = std::fs::read(p.join("reports/t.evidence.tsv")).unwrap();
    std::fs::remove_file(p.join("units.tsv")).unwrap();
    assert!(wait_state(p).is_err());
    assert!(commit_dispatch(p, "t").is_err());
    assert!(wait_event(p, state.events.last().unwrap().clone()).is_err());
    assert!(complete_round(p, "t", &verification).is_err());
    assert!(core_verdict(p, "t", &verification, true, "{}").is_err());
    assert!(release_gate(p).is_err());
    assert!(reconcile_release(p, &state).is_err());
    assert!(done_gate(p, false).is_err());
    assert!(done_gate_in_repo(p, false, Some(&repo)).is_err());
    assert!(current_verified_unit(p, "t", &repo).is_err());
    assert!(checkpoint(p, checkpoint_value()).is_err());
    assert!(product_checkpoint(p, "another").is_err());
    assert!(product_transition(
        p,
        "run",
        1,
        vec![verification],
        r#"{"valid_issues":["a","b","c"],"severity":"normal","decision":"finish"}"#,
        "continue"
    )
    .is_err());
    assert!(migrate_wait(p).is_err());
    let enroll = run(p, &["wait".into(), "enroll".into()], true, false);
    assert_ne!(enroll.exit_code, 0);
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    assert_eq!(
        std::fs::read(p.join("reports/t.evidence.tsv")).unwrap(),
        evidence_before
    );
    assert_eq!(std::fs::read(p.join("wait-required")).unwrap(), b"1\n");
    assert!(state.engineering_enrolled);
    // Registered checkpoint authority is sticky, even without the enrollment
    // marker; losing files must not resurrect the legacy event path.
    std::fs::remove_file(p.join("wait-required")).unwrap();
    assert!(wait_state(p).is_err());
    assert!(wait_event(p, state.events.last().unwrap().clone()).is_err());
    assert!(release_gate(p).is_err());
    assert!(reconcile_release(p, &state).is_err());
    assert!(current_verified_unit(p, "t", &repo).is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
}

#[test]
fn independent_task_attempts_never_advance_product_rounds() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "product").unwrap();
    for (task, rounds) in [("t", 2), ("independent", 3)] {
        for round in 1..=rounds {
            let mut c = checkpoint_value();
            c.task = task.into();
            c.round = round;
            c.evidence_version = round.into();
            checkpoint(p, c.clone()).unwrap();
            commit_dispatch(p, task).unwrap();
            for kind in [
                "build",
                "assembly",
                "verification",
                "core_approve",
                "completion",
                "release",
            ] {
                advance(p, &c, kind);
            }
            let state = wait_state(p).unwrap();
            assert!(may_finish(&state, task));
            assert_eq!(state.product_runs[0].rounds.len(), 1);
            assert!(state.product_runs[0].rounds[0].transition.is_none());
        }
    }
    assert!(product_checkpoint(p, "product").is_err());
    // Multiple explicit identities remain distinct; no round counter aliasing.
    product_checkpoint(p, "other-product").unwrap();
    assert_eq!(wait_state(p).unwrap().product_runs.len(), 2);
}

#[test]
fn product_continuation_rechecks_registered_engineering_acceptance() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    product_checkpoint(p, "run").unwrap();
    let (_, baseline) = registered_continuation_fixture(p);
    let template = wait_state(p).unwrap();
    let verification =
        registered_product_attempt(p, &template, &baseline, &["a", "b", "c"], "normal", "proof");
    let original = std::fs::read(p.join("core-wait.json")).unwrap();
    for kind in [
        "run", "scope", "verify", "accept", "review", "contract", "dispatch",
    ] {
        let mut rows = baseline.clone();
        if ["contract", "dispatch"].contains(&kind) {
            rows.iter_mut().find(|r| r.kind == kind).unwrap().command = "stale".into();
        } else {
            rows.iter_mut().find(|r| r.kind == kind).unwrap().exit_code = 1;
        }
        write_fixture_evidence(p, &rows);
        assert!(
            product_transition(
                p,
                "run",
                1,
                vec![verification.clone()],
                r#"{"valid_issues":["a","b","c"],"severity":"normal","decision":"finish"}"#,
                "continue"
            )
            .is_err(),
            "{kind}"
        );
        assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), original);
    }
    write_fixture_evidence(p, &baseline);
    product_transition(
        p,
        "run",
        1,
        vec![verification],
        r#"{"valid_issues":["a","b","c"],"severity":"normal","decision":"finish"}"#,
        "continue",
    )
    .unwrap();
    assert!(!product_policy_finished(&wait_state(p).unwrap(), "run"));
}

#[test]
fn legacy_host_verdicts_replay_only_as_archived_history() {
    use axiom::orch::*;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (_release, _) = registered_continuation_fixture(p);
    let mut legacy = serde_json::to_value(wait_state(p).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("product_runs");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("engineering_enrolled");
    // Actual history used independent round-1 units with explicit min_rounds=5,
    // not five released attempts. Retain the fixture's bound host evidence.
    legacy["checkpoints"].as_array_mut().unwrap().drain(..4);
    legacy["checkpoints"][0]["round"] = 1.into();
    legacy["checkpoints"][0]["evidence_version"] = 1.into();
    let events = legacy["events"].as_array_mut().unwrap();
    events.retain(|e| e["round"] == 5);
    for e in events.iter_mut() {
        e["round"] = 1.into();
        e["evidence_version"] = 1.into();
        if let Some(report) = e["evidence"].as_str().and_then(|s| s.strip_prefix("host:")) {
            let mut report: serde_json::Value = serde_json::from_str(report).unwrap();
            report["binding"]["round"] = 1.into();
            report["binding"]["evidence_version"] = 1.into();
            e["evidence"] = format!("host:{report}").into();
        }
    }
    let verdict = events
        .iter_mut()
        .find(|e| e["id"] == "round5-core_approve")
        .unwrap();
    verdict["evidence"] = json!(r#"{"valid_issues":[],"severity":"normal","decision":"continue"}"#);
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    std::fs::write(p.join("core-wait.json"), &bytes).unwrap();
    // A schema-2 journal has no blanket exemption for the same unsupported verdict.
    let mut modern = legacy.clone();
    modern["schema_version"] = 2.into();
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&modern).unwrap(),
    )
    .unwrap();
    assert!(wait_state(p).is_err());
    std::fs::write(p.join("core-wait.json"), &bytes).unwrap();
    let migrated = migrate_wait(p).unwrap();
    assert_eq!(migrated.checkpoints[0].round, 1);
    assert_eq!(migrated.checkpoints[0].min_rounds, 5);
    assert_eq!(
        serde_json::to_value(&migrated.events).unwrap(),
        legacy["events"]
    );
    assert!(migrated.product_runs.is_empty());
    assert_eq!(std::fs::read(p.join("core-wait.v1.json")).unwrap(), bytes);
    let after = std::fs::read(p.join("core-wait.json")).unwrap();
    assert!(wait_state(p).is_ok());
    migrate_wait(p).unwrap();
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), after);
    product_checkpoint(p, "new-product").unwrap();
    assert!(!product_policy_finished(
        &wait_state(p).unwrap(),
        "new-product"
    ));
    // A modified verdict or archive cannot inherit historical treatment.
    let valid = std::fs::read(p.join("core-wait.json")).unwrap();
    let mut changed: serde_json::Value = serde_json::from_slice(&valid).unwrap();
    let e = changed["events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == "round5-core_approve")
        .unwrap();
    e["evidence"] =
        json!(r#"{"valid_issues":["invented"],"severity":"normal","decision":"continue"}"#);
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(wait_state(p).is_err());
    std::fs::write(p.join("core-wait.json"), &valid).unwrap();
    std::fs::remove_file(p.join("core-wait.v1.json")).unwrap();
    assert_eq!(wait_state(p).unwrap_err().message, "missing legacy archive");
    std::fs::write(p.join("core-wait.v1.json"), b"tampered").unwrap();
    assert!(wait_state(p).is_err());
    // Legacy semantics still reject invented issues and invalid authority.
    // Remove the archive so archive conflict cannot mask a validation failure.
    std::fs::remove_file(p.join("core-wait.v1.json")).unwrap();
    for (mutation, expected) in [
        ("issue", "core issue is not in actual verification evidence"),
        ("actor", "invalid authority or early transition"),
        ("duplicate", "invalid core verdict assessment"),
        ("decision", "invalid core verdict assessment"),
        ("binding", "event binding mismatch"),
    ] {
        let mut bad = legacy.clone();
        let e = bad["events"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|e| e["id"] == "round5-core_approve")
            .unwrap();
        match mutation {
            "issue" => {
                e["evidence"] = json!(
                    r#"{"valid_issues":["invented"],"severity":"normal","decision":"continue"}"#
                )
            }
            "actor" => e["actor"] = json!("worker"),
            "duplicate" => {
                e["evidence"] = json!(
                    r#"{"valid_issues":["root-a","root-a"],"severity":"normal","decision":"continue"}"#
                )
            }
            "decision" => {
                e["evidence"] =
                    json!(r#"{"valid_issues":[],"severity":"normal","decision":"skip"}"#)
            }
            _ => e["contract"] = json!("unbound"),
        }
        let bad = serde_json::to_vec(&bad).unwrap();
        std::fs::write(p.join("core-wait.json"), &bad).unwrap();
        assert_eq!(migrate_wait(p).unwrap_err().message, expected, "{mutation}");
        assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), bad);
        assert!(!p.join("core-wait.v1.json").exists());
    }
    // The other observed normal/continue case had one independently listed ID.
    let e = legacy["events"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == "round5-core_approve")
        .unwrap();
    e["evidence"] =
        json!(r#"{"valid_issues":["root-a"],"severity":"normal","decision":"continue"}"#);
    let mut modern = legacy.clone();
    modern["schema_version"] = 2.into();
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&modern).unwrap(),
    )
    .unwrap();
    assert_eq!(
        wait_state(p).unwrap_err().message,
        "continuation lacks independent supporting evidence"
    );
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    assert!(migrate_wait(p).unwrap().product_runs.is_empty());
    assert!(wait_state(p).is_ok());
}

#[test]
fn legacy_boundary_does_not_exempt_future_verdicts() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (_, rows) = registered_continuation_fixture(p);
    let mut legacy = serde_json::to_value(wait_state(p).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    let events = legacy["events"].as_array_mut().unwrap();
    let index = events
        .iter()
        .position(|e| e["id"] == "round5-core_approve")
        .unwrap();
    let mut verdict = events[index].clone();
    verdict["evidence"] =
        serde_json::json!(r#"{"valid_issues":[],"severity":"normal","decision":"continue"}"#);
    events.truncate(index);
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    migrate_wait(p).unwrap();
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let event: Event = serde_json::from_value(verdict.clone()).unwrap();
    assert!(wait_event(p, event).is_err());
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    let mut injected: serde_json::Value = serde_json::from_slice(&before).unwrap();
    injected["events"].as_array_mut().unwrap().push(verdict);
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&injected).unwrap(),
    )
    .unwrap();
    assert!(wait_state(p).is_err());
    // Supported new events still work and retain strict replay after restart.
    std::fs::write(p.join("core-wait.json"), &before).unwrap();
    let mut supported: Event = serde_json::from_value(
        injected["events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone(),
    )
    .unwrap();
    supported.evidence = r#"{"valid_issues":["root-a","root-b","root-c"],"severity":"normal","decision":"continue"}"#.into();
    write_fixture_evidence(p, &rows);
    wait_event(p, supported).unwrap();
    assert!(wait_state(p).is_ok());
}

#[test]
fn migrated_prepared_checkpoint_cannot_rewrite_archived_history() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let (release, rows) = registered_continuation_fixture(p);
    write_fixture_evidence(p, &rows);
    wait_event(p, release).unwrap();
    let prepare = || {
        run(
            p,
            &[
                "wait".into(),
                "prepare".into(),
                "--task".into(),
                "t".into(),
                "--repo".into(),
                p.join("repo").to_string_lossy().into_owned(),
                "--issue".into(),
                "i".into(),
                "--proposal".into(),
                "p".into(),
            ],
            true,
            false,
        )
    };
    let result = prepare();
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let mut legacy = serde_json::to_value(wait_state(p).unwrap()).unwrap();
    legacy["schema_version"] = 1.into();
    std::fs::write(
        p.join("core-wait.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    migrate_wait(p).unwrap();
    let before = std::fs::read(p.join("core-wait.json")).unwrap();
    let archive = std::fs::read(p.join("core-wait.v1.json")).unwrap();
    assert_eq!(prepare().exit_code, 0); // An unchanged checkpoint is idempotent.
    std::fs::write(
        p.join("brief"),
        "GOAL changed\nSCOPE changed\nVERIFY echo ok\nDELIVERY verification\nOUTPUTS changed\n",
    )
    .unwrap();
    let result = prepare();
    assert_ne!(result.exit_code, 0);
    assert!(
        result.stderr.contains("legacy archive history mismatch"),
        "{}",
        result.stderr
    );
    assert_eq!(std::fs::read(p.join("core-wait.json")).unwrap(), before);
    assert_eq!(std::fs::read(p.join("core-wait.v1.json")).unwrap(), archive);
    assert!(wait_state(p).is_ok());
}

#[test]
fn delivery_is_optional_for_historical_parse_but_new_prepare_requires_it() {
    use axiom::orch::*;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let repo = p.join("repo"); std::fs::create_dir(&repo).unwrap();
    let brief = p.join("brief");
    std::fs::write(&brief, "GOAL old\nSCOPE out\nVERIFY echo ok\n").unwrap();
    assert!(parse_brief(&std::fs::read_to_string(&brief).unwrap()).unwrap().delivery.is_none());
    std::fs::write(p.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nt\tm\tpending\t\t\t\t{}\n", brief.display())).unwrap();
    let args: Vec<String> = vec!["wait".into(),"prepare".into(),"--task".into(),"t".into(),"--repo".into(),repo.to_string_lossy().into_owned(),"--issue".into(),"i".into(),"--proposal".into(),"p".into()];
    assert_ne!(run(p, &args, true, false).exit_code, 0);
    std::fs::write(&brief, "GOAL new\nSCOPE out\nVERIFY echo ok\nDELIVERY implementation\n").unwrap();
    assert_ne!(run(p, &args, true, false).exit_code, 0);
    assert!(!p.join("core-wait.json").exists());
    std::fs::write(&brief, "GOAL new\nSCOPE out other\nVERIFY echo ok\nDELIVERY implementation\nOUTPUTS out\n").unwrap();
    let mut overridden = args.clone(); overridden.extend(["--outputs".into(), "out".into()]);
    assert_ne!(run(p, &overridden, true, false).exit_code, 0);
    assert_eq!(run(p, &args, true, false).exit_code, 0);
    let original = contract_identity(p, "t").unwrap();
    std::fs::write(&brief, "GOAL new\nSCOPE out other\nVERIFY echo ok\nDELIVERY implementation\nOUTPUTS other\n").unwrap();
    assert_ne!(contract_identity(p, "t").unwrap(), original);
    assert!(commit_dispatch(p, "t").is_err());
}
