use axiom::bridge::{
    dispatch::{DispatchAction, DispatchOptions, DispatchRequest, RootContext},
    Bridge, Config,
};
use axiom::orch::checkpoint::{self, ArtifactRef, Phase, RepoIdentity, RunSeed, RunStatus};
use sha2::{Digest, Sha256};
#[path = "../common/mod.rs"]
mod common;

struct Wire(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
impl std::io::Write for Wire {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn flow(mode: &str, approve: bool) {
    use serde_json::{json, Value};
    let tmp = common::temp();
    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let home = tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let log = tmp.path().join("rpc.jsonl");
    let config = || Config {
        data_home: Some(home.clone()),
        codex: Ok(vec![
            common::python(),
            "-I".into(),
            common::repo_root()
                .join("tests/fakes/fake_flow.py")
                .to_string_lossy()
                .into_owned(),
            log.to_string_lossy().into_owned(),
            mode.into(),
        ]),
        agy: Err(axiom::base::Failure::new("forbidden", "no agy")),
        wsl: None,
        on_windows: false,
    };
    let invoke = |request_id: &str, action: Value| {
        let capture = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":request_id,"method":"tools/call","params":{"name":"dispatch","arguments":{"project":"demo","repo_root":repo,"request_id":request_id,"action":action}}})
        );
        assert_eq!(
            axiom::bridge::serve(config(), std::io::Cursor::new(input), Wire(capture.clone())),
            0
        );
        let response: Value = serde_json::from_slice(&capture.lock().unwrap()).unwrap();
        assert_eq!(
            response["id"], request_id,
            "original tools/call response must await the boundary"
        );
        response["result"]["structuredContent"].clone()
    };
    let start = invoke(
        "start",
        json!({"type":"start","intent":"build two independent text units"}),
    );
    if mode == "spoof" {
        assert_ne!(start["status"], "needs_intent_review", "{start}");
        return;
    }
    assert_eq!(start["status"], "needs_intent_review", "{start}");
    assert!(!repo.join("one.txt").exists());
    assert_eq!(
        invoke(
            "start",
            json!({"type":"start","intent":"build two independent text units"})
        ),
        start
    );
    let run = start["run_id"].as_str().unwrap();
    let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
    assert_eq!(
        start["research"]["digest"],
        c.research.as_ref().unwrap().digest
    );
    assert_eq!(c.attempts.len(), 2);
    assert!(c
        .attempts
        .values()
        .all(|a| a.state == checkpoint::AttemptState::Terminal));
    let mut result = invoke(
        "decide",
        json!({"type":"decide","run_id":run,"research_digest":c.research.unwrap().digest,"pass":approve,"reason":"core intent decision"}),
    );
    let rows = || {
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .collect::<Vec<_>>()
    };
    if !approve {
        assert_eq!(result["status"], "needs_intent_review", "{result}");
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        assert!(!repo.join("one.txt").exists());
        let revised = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(revised.receipts.contains_key("intent-rejection-1"));
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 2);
        assert!(rows()
            .iter()
            .filter(|r| r["kind"] == "research")
            .all(|r| r["effort"] == "max"));
        assert!(rows().iter().any(|r| r["kind"] == "research"
            && r["brief"]
                .as_str()
                .unwrap()
                .contains("Revise the previous research")));
        let approved = invoke(
            "approve-revision",
            json!({"type":"decide","run_id":run,"research_digest":revised.research.unwrap().digest,"pass":true,"reason":"revision matches intent"}),
        );
        assert_eq!(approved["status"], "ready_to_commit", "{approved}");
        return;
    }
    if mode == "planning-retry" {
        assert_eq!(result["status"], "blocked");
        assert!(
            result["short_reason"]
                .as_str()
                .unwrap()
                .contains("implementable units"),
            "{result}"
        );
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        result = invoke(
            "retry-planning",
            json!({"type":"resume","run_id":run,
            "recovery":{"retry_planning":{"reason":"runtime now prepares independent evaluation"}}}),
        );
        let recovered = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(checkpoint::planning_epoch(&recovered), 1);
        assert!(recovered
            .attempts
            .values()
            .any(|a| a.outcome == Some(checkpoint::AttemptOutcome::Failed)));
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 1);
    }
    if mode == "evaluation-recovered" {
        // A preparation failure is a typed, recoverable planning boundary.
        assert_eq!(result["status"], "blocked", "{result}");
        assert!(
            result["short_reason"]
                .as_str()
                .unwrap()
                .contains("independent evaluation preparation failed"),
            "{result}"
        );
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        assert!(!repo.join("one.txt").exists());
        let failed = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(failed.phase, Phase::Planning);
        assert!(failed.steps.get("baseline").is_none());
        assert!(failed.checks.is_empty());
        assert!(!failed.receipts.keys().any(|k| k.starts_with("evaluation-")));
        result = invoke(
            "retry-evaluation",
            json!({"type":"resume","run_id":run,
            "recovery":{"retry_planning":{"reason":"the tester can now return inline held-out checks"}}}),
        );
        assert_eq!(result["status"], "ready_to_commit", "{result}");
        let recovered = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(checkpoint::planning_epoch(&recovered), 1);
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 1);
        assert!(recovered
            .receipts
            .keys()
            .any(|k| k.starts_with("planning-retry-")));
    }
    if ["evaluation-unprepared", "evaluation-fake-ready"].contains(&mode) {
        // The dev.8 shape and its relabelled-public-check variant: preparation
        // must fail before any check is installed or any writer starts.
        assert_eq!(result["status"], "blocked", "{result}");
        let reason = result["short_reason"].as_str().unwrap();
        assert!(
            reason.contains("independent evaluation preparation failed")
                || reason.contains("repeats a public regression command"),
            "{reason}"
        );
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        assert!(!repo.join("one.txt").exists());
        let failed = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(failed.phase, Phase::Planning);
        assert!(failed.steps.get("baseline").is_none());
        assert!(failed.checks.is_empty(), "no acceptance check is installed");
        assert!(
            !failed.receipts.keys().any(|k| k.starts_with("evaluation-")),
            "a failed preparation is not saved as an evaluation receipt"
        );
        assert!(
            !failed.receipts.keys().any(|k| k.starts_with("baseline-")),
            "the baseline must not run on an uninstalled plan"
        );
        // Research and the typed failure stay; a new planning epoch is legal.
        let retried = invoke(
            "retry-unprepared",
            json!({"type":"resume","run_id":run,
            "recovery":{"retry_planning":{"reason":"runtime now offers an inline tester contract"}}}),
        );
        assert_ne!(retried["status"], "ready_to_commit", "{retried}");
        let retried_run = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(checkpoint::planning_epoch(&retried_run), 1);
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 1);
        return;
    }
    if mode == "baseline-infrastructure" {
        // The checks are genuine but the baseline says the infrastructure
        // cannot judge the product: engineering must stay closed.
        assert_eq!(result["status"], "blocked", "{result}");
        assert!(
            result["short_reason"]
                .as_str()
                .unwrap()
                .contains("acceptance infrastructure not ready"),
            "{result}"
        );
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        assert!(!repo.join("one.txt").exists());
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(c.phase, Phase::Planning);
        assert!(c.plan.is_some());
        assert!(
            c.checks
                .keys()
                .all(|id| c.receipts.contains_key(&format!("baseline-{id}"))),
            "the baseline ran every frozen check"
        );
        assert!(
            c.receipts.get(checkpoint::BASELINE_GATE).is_none(),
            "no gate receipt may authorize writers"
        );
        let retried = invoke(
            "retry-frozen-baseline",
            json!({"type":"resume","run_id":run,
            "recovery":{"retry_planning":{"reason":"must not unfreeze an installed plan"}}}),
        );
        assert_ne!(retried["status"], "ready_to_commit", "{retried}");
        assert_eq!(
            checkpoint::load_run(&home.join("orchestrate/demo"), run)
                .unwrap()
                .phase,
            Phase::Planning
        );
        return;
    }
    if mode == "writer-blocked" {
        // A normally terminating turn that reports a blocked unit must not
        // satisfy dependents, assembly or verification.
        assert_eq!(result["status"], "blocked", "{result}");
        let reason = result["short_reason"].as_str().unwrap();
        assert!(reason.contains("reported blocked"), "{reason}");
        assert!(reason.contains("未開始工程"), "{reason}");
        assert!(!rows().iter().any(|r| r["kind"] == "assembly"));
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(c.phase, Phase::Engineering);
        assert!(c
            .attempts
            .values()
            .any(|a| a.outcome == Some(checkpoint::AttemptOutcome::Blocked)));
        assert!(!c.receipts.keys().any(|k| k.starts_with("review-")));
        assert!(!c.receipts.contains_key("assembly"));
        assert!(!repo.join("one.txt").exists());
        let saved = c
            .attempts
            .values()
            .find(|a| a.outcome == Some(checkpoint::AttemptOutcome::Blocked))
            .and_then(|a| a.raw_result.as_ref())
            .map(|r| std::fs::read_to_string(&r.path).unwrap())
            .unwrap();
        assert!(
            saved.contains("未開始工程"),
            "the refusal report is retained"
        );
        return;
    }
    if ["verifier-upstream", "verifier-no-owner"].contains(&mode) {
        // Recovery follows the failure's origin: an upstream or unattributed
        // failure never consumes a SOL rework.
        assert_eq!(result["status"], "blocked", "{result}");
        let reason = result["short_reason"].as_str().unwrap();
        if mode == "verifier-upstream" {
            assert!(reason.contains("no rework was scheduled"), "{reason}");
            assert!(reason.contains("upstream"), "{reason}");
        } else {
            assert!(reason.contains("defect_owner=none"), "{reason}");
        }
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(!c.receipts.keys().any(|k| k.starts_with("rework-")));
        assert_eq!(rows().iter().filter(|r| r["kind"] == "assembly").count(), 1);
        return;
    }
    if ![
        "normal",
        "content",
        "repairable-review",
        "evaluation",
        "planning-retry",
        "evaluation-recovered",
    ]
    .contains(&mode)
    {
        assert_ne!(result["status"], "ready_to_commit", "{result}");
        if mode == "failed-peer" {
            assert!(
                rows().iter().any(|r| r["kind"] == "interrupted")
                    || rows()
                        .iter()
                        .filter(|r| r["kind"] == "engineer-start")
                        .count()
                        == 1
            );
        }
        if mode == "rejected-review" {
            assert!(axiom::orch::read_units(&home.join("orchestrate/demo"))
                .unwrap()
                .iter()
                .any(|u| u.state == "review-failed"));
        }
        let before = rows()
            .iter()
            .filter(|r| r["kind"] == "engineer-start")
            .count();
        let resumed = invoke(
            "resume",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        assert_ne!(resumed["status"], "ready_to_commit", "{resumed}");
        let invalid_retry = invoke(
            "reject-planning-retry",
            json!({"type":"resume","run_id":run,
            "recovery":{"retry_planning":{"reason":"must not bypass frozen plan or unknown writer"}}}),
        );
        assert_ne!(
            invalid_retry["status"], "ready_to_commit",
            "{invalid_retry}"
        );
        assert_eq!(
            checkpoint::planning_epoch(
                &checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap()
            ),
            0
        );
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "engineer-start")
                .count(),
            before
        );
        return;
    }
    assert_eq!(result["status"], "ready_to_commit", "{result}");
    let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
    assert!(checkpoint::verified_outcome(&c));
    assert!(
        axiom::orch::done_gate(&home.join("orchestrate/demo"), true).is_ok(),
        "local submission needs no remote or package release"
    );
    assert_eq!(
        axiom::orch::read_units(&home.join("orchestrate/demo"))
            .unwrap()
            .iter()
            .filter(|u| u.state == "verified")
            .count(),
        if mode == "content" { 2 } else { 3 }
    );
    assert!(c.receipts.contains_key("memory"));
    let records = rows();
    if ["evaluation", "planning-retry", "evaluation-recovered"].contains(&mode) {
        let prep = records
            .iter()
            .find(|r| r["kind"] == "evaluation-prepared")
            .unwrap();
        let first_write = records
            .iter()
            .find(|r| r["kind"] == "engineer-start")
            .unwrap();
        assert!(prep["time"].as_f64().unwrap() < first_write["time"].as_f64().unwrap());
        assert!(c.checks.contains_key("independent-output"));
        let public_plan = std::fs::read_to_string(&c.plan.as_ref().unwrap().path).unwrap();
        assert!(!public_plan.contains("PRIVATE_EVALUATION_CANARY"));
        let evaluation = c
            .receipts
            .get(&format!("evaluation-{}", checkpoint::planning_epoch(&c)))
            .unwrap();
        for row in records.iter().filter(|r| r["kind"] == "turn") {
            let brief = row["params"]["input"][0]["text"].as_str().unwrap();
            if brief.starts_with("ROLE Engineer") || brief.starts_with("ROLE Assembler") {
                assert!(!brief.contains("PRIVATE_EVALUATION_CANARY"));
                assert!(!brief.contains(&evaluation.path.to_string_lossy().to_string()));
                // Non-leaking prerequisite proof: the writer is told Rust
                // already froze and classified the checks before it started.
                assert!(
                    brief.contains("acceptance infrastructure was classified ready"),
                    "{brief}"
                );
                assert!(brief.contains("private held-out checks"), "{brief}");
            }
        }
        assert!(checkpoint::resolve_attempt(
            &home.join("orchestrate/demo"),
            run,
            c.revision,
            &checkpoint::RecoveryDecision::RetryPlanning {
                reason: "must not unfreeze completed plan".into()
            }
        )
        .is_err());
    }
    assert!(records
        .iter()
        .filter(|r| r["kind"] == "research")
        .all(|r| r["effort"] == "max"));
    if mode == "content" {
        let content = records.iter().find(|r| r["kind"] == "content").unwrap();
        let thread = records
            .iter()
            .find(|r| r["kind"] == "thread" && r["thread"] == content["thread"])
            .unwrap();
        assert!(thread["params"]["model"]
            .as_str()
            .unwrap()
            .ends_with("-luna"));
        assert_eq!(thread["params"]["sandbox"], "read-only");
    }
    let starts = records
        .iter()
        .filter(|r| r["kind"] == "engineer-start")
        .map(|r| r["time"].as_f64().unwrap())
        .collect::<Vec<_>>();
    let ends = records
        .iter()
        .filter(|r| r["kind"] == "engineer-finish")
        .map(|r| r["time"].as_f64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), if mode == "content" { 1 } else { 2 });
    assert_eq!(ends.len(), starts.len());
    assert!(
        starts.iter().copied().fold(f64::MIN, f64::max)
            < ends.iter().copied().fold(f64::MAX, f64::min)
    );
    assert_eq!(
        records.iter().filter(|r| r["kind"] == "assembly").count(),
        if mode == "repairable-review" { 2 } else { 1 }
    );
    let command = records.iter().find(|r| r["kind"] == "command").unwrap();
    assert_eq!(
        Path::new(command["process_cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        Path::new(command["params"]["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    );
    assert!(!repo.join("scratch-proof").exists());
    assert!(
        std::fs::read_to_string(&c.receipts["check-frozen-python"].path)
            .unwrap()
            .contains("scratch_check")
    );
    let attempts = c.attempts.len();
    assert_eq!(
        invoke(
            "resume",
            json!({"type":"resume","run_id":run,"recovery":null})
        )["status"],
        "ready_to_commit"
    );
    assert_eq!(
        checkpoint::load_run(&home.join("orchestrate/demo"), run)
            .unwrap()
            .attempts
            .len(),
        attempts
    );
    use std::path::Path;
}

#[test]
fn independent_evaluation_is_created_before_writes_and_omitted_from_writer_context() {
    flow("evaluation", true);
}
#[test]
fn failed_read_only_planning_can_resume_without_repeating_research_or_erasing_failure() {
    flow("planning-retry", true);
}
#[test]
fn batch_failure_cancels_pending_peer_instead_of_waiting_for_its_execution_budget() {
    let started = std::time::Instant::now();
    flow("failed-peer", true);
    assert!(
        started.elapsed().as_secs() < 60,
        "must not consume the 1800-second leaf budget"
    );
}
#[test]
fn rpc_research_reject_has_zero_engineering() {
    flow("normal", false);
}
#[test]
fn rpc_rejected_verification_returns_to_sol_then_fresh_verifier() {
    flow("repairable-review", true);
}
#[test]
fn rpc_approve_parallel_units_assembly_real_check_memory_boundary() {
    flow("normal", true);
}
#[test]
fn rpc_role_spoof_check_claim_and_unknown_write_negative_controls() {
    for mode in [
        "spoof",
        "no-check",
        "failed-check",
        "rejected-review",
        "unknown-check",
        "unknown-write",
    ] {
        flow(mode, true);
    }
}

fn artifact(path: &std::path::Path, bytes: &[u8]) -> ArtifactRef {
    std::fs::write(path, bytes).unwrap();
    ArtifactRef {
        path: path.into(),
        digest: format!("{:x}", Sha256::digest(bytes)),
    }
}

#[test]
fn blocked_managed_run_rejects_legacy_writes_even_with_a_different_project() {
    use serde_json::json;
    let tmp = common::temp();
    let repo = std::fs::canonicalize(tmp.path()).unwrap();
    let store = repo.join("orchestrate/demo");
    std::fs::create_dir_all(&store).unwrap();
    let c = checkpoint::create_run(
        &store,
        &RunSeed {
            run_id: "run".into(),
            repo: RepoIdentity {
                canonical_root: repo.clone(),
                identity_digest: "repo".into(),
            },
            intent: artifact(&store.join("intent"), b"intent"),
        },
    )
    .unwrap();
    checkpoint::record_dispatch_failure(&store, "run", c.revision, false).unwrap();
    let b = Bridge::new(Config {
        data_home: Some(repo.clone()),
        codex: Err(axiom::base::Failure::new("missing", "must not launch")),
        agy: Err(axiom::base::Failure::new("missing", "must not launch")),
        wsl: None,
        on_windows: false,
    });
    for project in ["demo", "different-slug"] {
        let error = axiom::bridge::tools::dispatch(
            &b,
            "codex_run",
            &json!({"project":project,"unit":"bypass","role":"terra","cwd":repo}),
        )
        .unwrap_err();
        assert_eq!(error.kind, "managed_run_owned");
    }
    let error = axiom::bridge::tools::dispatch(
        &b,
        "orch",
        &json!({"project":"demo","args":["unit","add","bypass"]}),
    )
    .unwrap_err();
    assert_eq!(error.kind, "managed_run_owned");
    assert!(axiom::bridge::tools::dispatch(
        &b,
        "orch",
        &json!({"project":"demo","args":["status"]})
    )
    .is_ok());
    let c = checkpoint::load_run(&store, "run").unwrap();
    checkpoint::advance_phase(&store, "run", c.revision, Phase::Cancelled).unwrap();
    let result = axiom::bridge::tools::dispatch(
        &b,
        "codex_run",
        &json!({"cwd":repo,"role":"terra","brief":"unbound write"}),
    )
    .unwrap();
    assert_eq!(
        result["error"]["kind"], "unbound_write",
        "legacy standalone validation is restored after cancellation"
    );
}

#[test]
fn rejection_requires_matching_digest_and_cancel_requires_reason() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let store = repo.join("orchestrate/demo");
    std::fs::create_dir_all(&store).unwrap();
    let intent = artifact(&store.join("intent"), b"intent");
    let research = artifact(&store.join("research"), b"research");
    let c = checkpoint::create_run(
        &store,
        &RunSeed {
            run_id: "run".into(),
            repo: RepoIdentity {
                canonical_root: repo.clone(),
                identity_digest: "repo".into(),
            },
            intent,
        },
    )
    .unwrap();
    let revision =
        checkpoint::record_receipt(&store, "run", c.revision, "research", &research).unwrap();
    checkpoint::advance_phase(&store, "run", revision, Phase::IntentReview).unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(repo.clone()),
        codex: Err(axiom::base::Failure::new("missing", "missing")),
        agy: Err(axiom::base::Failure::new("missing", "missing")),
        wsl: None,
        on_windows: false,
    });
    let root = RootContext {
        repo_root: repo,
        project: "demo".into(),
    };
    let decide = |digest: String| DispatchRequest {
        request_id: "id".into(),
        action: DispatchAction::Decide {
            run_id: "run".into(),
            research_digest: digest,
            pass: false,
            reason: "no".into(),
        },
    };
    assert!(bridge
        .dispatch(
            &root,
            &decide("wrong".into()),
            &DispatchOptions::default(),
            None
        )
        .is_err());
    let result = bridge.dispatch(
        &root,
        &decide(research.digest),
        &DispatchOptions::default(),
        None,
    );
    assert!(
        result.is_err(),
        "missing model transport cannot complete revision"
    );
    let saved = checkpoint::load_run(&store, "run").unwrap();
    assert_eq!(saved.phase, Phase::Research);
    assert!(saved.receipts.contains_key("intent-rejection-1"));
    let cancel = |reason: &str| DispatchRequest {
        request_id: "cancel".into(),
        action: DispatchAction::Cancel {
            run_id: "run".into(),
            reason: reason.into(),
        },
    };
    assert!(bridge
        .dispatch(&root, &cancel(""), &DispatchOptions::default(), None)
        .is_err());
    assert_eq!(
        bridge
            .dispatch(&root, &cancel("stop"), &DispatchOptions::default(), None)
            .unwrap()
            .status,
        RunStatus::Cancelled
    );
    assert!(!bridge.running());
}

#[test]
fn luna_content_is_read_only_and_sol_integrates_the_artifact() {
    flow("content", true);
}

#[test]
fn prepared_evaluation_that_reports_failure_blocks_before_baseline() {
    flow("evaluation-unprepared", true);
}

#[test]
fn relabelled_public_check_is_not_held_out_acceptance() {
    flow("evaluation-fake-ready", true);
}

#[test]
fn preparation_failure_recovers_through_a_new_planning_epoch() {
    flow("evaluation-recovered", true);
}

#[test]
fn baseline_infrastructure_failure_keeps_engineering_closed() {
    flow("baseline-infrastructure", true);
}

#[test]
fn writer_refusal_after_a_normal_turn_does_not_unlock_dependents() {
    flow("writer-blocked", true);
}

#[test]
fn upstream_and_unattributed_verifier_failures_do_not_consume_sol_rework() {
    flow("verifier-upstream", true);
    flow("verifier-no-owner", true);
}
