use axiom::bridge::{
    dispatch::{DispatchAction, DispatchOptions, DispatchRequest, RootContext},
    Bridge, Config,
};
use axiom::orch::checkpoint::{self, ArtifactRef, Phase, RepoIdentity, RunSeed, RunStatus};
use sha2::{Digest, Sha256};
#[path = "../common/mod.rs"]
mod common;

/// Every flow drives a real subprocess timeline. Serializing the flows keeps a
/// small CI runner from making timing-sensitive runs flaky, while the rest of
/// the suite stays parallel.
static FLOW: std::sync::Mutex<()> = std::sync::Mutex::new(());

// dev.9 rejects unknown fields in a persisted request's result.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyBoundaryResult {
    run_id: String,
    phase: serde_json::Value,
    status: serde_json::Value,
    short_reason: String,
    research: Option<serde_json::Value>,
    artifact_refs: Vec<serde_json::Value>,
    next_action: Option<String>,
}

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
    let _serial = FLOW.lock().unwrap_or_else(|e| e.into_inner());
    // Queueing behind unrelated flows is outside this execution's deadline.
    // Keep the full flow (including cleanup) inside the original time budget.
    let started = std::time::Instant::now();
    run_flow(mode, approve);
    if mode == "failed-peer" {
        assert!(
            started.elapsed().as_secs() < 60,
            "must not consume the 1800-second leaf budget"
        );
    }
}

fn run_flow(mode: &str, approve: bool) {
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
    let research_digest = c.research.as_ref().unwrap().digest.clone();
    let decide = json!({"type":"decide","run_id":run,"research_digest":research_digest,"pass":approve,"reason":"core intent decision"});
    let mut result = invoke("decide", decide.clone());
    let rows = || {
        std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .collect::<Vec<_>>()
    };
    // A fixture crash looks like a lost connection unless its traceback is
    // surfaced, so failed assertions carry the replay trail.
    let trail = || {
        let all = rows();
        let mut parts = vec![all
            .iter()
            .rev()
            .take(14)
            .rev()
            .map(|r| r["kind"].as_str().unwrap_or("?").to_owned())
            .collect::<Vec<_>>()
            .join(" | ")];
        for r in all.iter().filter(|r| {
            matches!(
                r["kind"].as_str(),
                Some("fixture-error" | "fixture-thread-error")
            )
        }) {
            parts.push(format!(
                "{}: {}",
                r["kind"].as_str().unwrap_or("fixture-error"),
                r["traceback"].as_str().unwrap_or("")
            ));
        }
        parts.join("\n")
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
    if mode == "verifier-history-null" {
        assert_eq!(result["status"], "outcome_unknown", "{result}\n{}", trail());
        result = invoke(
            "recover-model-history",
            json!({"type":"resume","run_id":run}),
        );
        assert_eq!(result["status"], "ready_to_commit", "{result}\n{}", trail());
        let saved = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        let verifier = saved
            .attempts
            .values()
            .find(|a| a.step_id == "verification")
            .unwrap();
        let (raw, _, model) = checkpoint::terminal_attempt_report(
            &home.join("orchestrate/demo"),
            &saved,
            &verifier.attempt_id,
        )
        .unwrap();
        assert!(raw["model"].is_null());
        assert_eq!(raw["model_source"], "thread/read");
        assert_eq!(model.configured_model, "gpt-6.1-astra");
        assert!(checkpoint::verified_review(&saved).is_some());
        assert!(rows().iter().any(|r| r["kind"] == "model-list"
            && !r["roles"].as_array().unwrap().contains(&json!("astra"))));
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "turn"
                    && r["thread"] == verifier.thread_id.as_deref().unwrap())
                .count(),
            1
        );
        return;
    }
    if mode.starts_with("engineer-config-") {
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
        let store = home.join("orchestrate/demo");
        let before = checkpoint::load_run(&store, run).unwrap();
        let prepared = before
            .attempts
            .values()
            .filter(|a| before.steps[&a.step_id].purpose == checkpoint::Purpose::Engineer)
            .map(|a| a.attempt_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(prepared.len(), 2);
        for id in &prepared {
            let a = &before.attempts[id];
            assert_eq!(a.state, checkpoint::AttemptState::Prepared);
            assert!(a.generation.is_none() && a.thread_id.is_none() && a.turn_id.is_none());
            assert!(!before.receipts.contains_key(&format!("attempt-model-{id}")));
        }
        assert_eq!(before.reservations.len(), 2);
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        if mode == "engineer-config-cancel" {
            let cancelled = invoke(
                "cancel-prepared",
                json!({"type":"cancel","run_id":run,"reason":"stop before any writer turn was sent"}),
            );
            assert_eq!(cancelled["status"], "cancelled", "{cancelled}");
            let saved = checkpoint::load_run(&store, run).unwrap();
            assert!(saved.reservations.is_empty());
            for id in &prepared {
                assert_eq!(
                    saved.attempts[id].outcome,
                    Some(checkpoint::AttemptOutcome::Cancelled)
                );
                assert!(saved
                    .receipts
                    .contains_key(&format!("cancel-prepared-{id}")));
            }
        } else {
            // Configuration repair is external. An explicit Resume reuses the
            // original never-dispatched attempts; it cannot retry uncertain work.
            std::fs::write(
                log.with_extension("fixed-model"),
                "configured model repaired",
            )
            .unwrap();
            let recovered = invoke("resume-prepared", json!({"type":"resume","run_id":run}));
            assert_eq!(
                recovered["status"],
                "ready_to_commit",
                "{recovered}\n{}",
                trail()
            );
            let saved = checkpoint::load_run(&store, run).unwrap();
            assert_eq!(
                saved
                    .attempts
                    .values()
                    .filter(|a| saved.steps[&a.step_id].purpose == checkpoint::Purpose::Engineer)
                    .count(),
                2
            );
            for id in &prepared {
                assert_eq!(
                    saved.attempts[id].outcome,
                    Some(checkpoint::AttemptOutcome::Completed)
                );
            }
            assert!(saved
                .receipts
                .keys()
                .any(|key| key.starts_with("retry-prepared-")));
            assert!(!saved
                .attempts
                .values()
                .any(|a| checkpoint::failed_attempt_needs_repair(&saved, a)));
            let stable = invoke("resume-completed", json!({"type":"resume","run_id":run}));
            assert_eq!(stable["status"], "ready_to_commit", "{stable}");
        }
        return;
    }
    if [
        "planning-retry",
        "retry-missing",
        "retry-tampered",
        "retry-wrong-epoch",
    ]
    .contains(&mode)
    {
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
        if mode != "planning-retry" {
            assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
            assert!(recovered.plan.is_none());
            assert!(recovered.checks.is_empty());
            assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
            assert!(!rows().iter().any(|r| r["kind"] == "evaluation-prepared"));
            assert!(!repo.join("one.txt").exists());
            return;
        }
        assert_eq!(checkpoint::planning_epoch(&recovered), 1);
        assert!(recovered
            .attempts
            .values()
            .any(|a| a.outcome == Some(checkpoint::AttemptOutcome::Failed)));
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 1);
        let turns = rows();
        let planners: Vec<_> = turns
            .iter()
            .filter(|row| {
                row["kind"] == "turn"
                    && row["params"]["input"][0]["text"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("ROLE Planner"))
            })
            .collect();
        assert_eq!(planners.len(), 2);
        let first = planners[0]["params"]["input"][0]["text"].as_str().unwrap();
        assert!(
            first.contains("Public check IDs must not begin independent-"),
            "{first}"
        );
        assert!(first.contains("independent-h"), "{first}");
        assert!(!first.contains("runtime now prepares independent evaluation"));
        let retry = planners[1]["params"]["input"][0]["text"].as_str().unwrap();
        assert!(
            retry.contains("runtime now prepares independent evaluation"),
            "{retry}"
        );
        let prep = turns
            .iter()
            .find(|row| row["kind"] == "evaluation-prepared")
            .unwrap();
        assert!(prep["brief"]
            .as_str()
            .unwrap()
            .contains("runtime now prepares independent evaluation"));
    }
    if mode == "retry-second-epoch" {
        assert_eq!(result["status"], "blocked", "{result}");
        let first = invoke(
            "retry-first",
            json!({"type":"resume","run_id":run,
                "recovery":{"retry_planning":{"reason":"first correction: use independent-h for the private ID"}}}),
        );
        assert_eq!(first["status"], "blocked", "{first}\n{}", trail());
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        result = invoke(
            "retry-second",
            json!({"type":"resume","run_id":run,
                "recovery":{"retry_planning":{"reason":"second correction: prepare an inline acceptance check"}}}),
        );
        assert_eq!(result["status"], "ready_to_commit", "{result}\n{}", trail());
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(checkpoint::planning_epoch(&c), 2);
        assert!(c.receipts.contains_key("planning-retry-1"));
        assert!(c.receipts.contains_key("planning-retry-2"));
        let records = rows();
        assert_eq!(
            records.iter().filter(|r| r["kind"] == "research").count(),
            1
        );
        let prep = records
            .iter()
            .find(|r| r["kind"] == "evaluation-prepared")
            .unwrap();
        let planners: Vec<_> = records
            .iter()
            .filter(|row| {
                row["kind"] == "turn"
                    && row["params"]["input"][0]["text"]
                        .as_str()
                        .is_some_and(|s| s.starts_with("ROLE Planner"))
            })
            .collect();
        assert_eq!(planners.len(), 3);
        let planner = planners[2]["params"]["input"][0]["text"].as_str().unwrap();
        let prep = prep["brief"].as_str().unwrap();
        for brief in [planner, prep] {
            let first = brief
                .find("first correction: use independent-h for the private ID")
                .unwrap();
            let second = brief
                .find("second correction: prepare an inline acceptance check")
                .unwrap();
            assert!(first < second, "{brief}");
            assert!(
                brief
                    .contains("Later reasons supplement earlier ones, or explicitly override them"),
                "{brief}"
            );
        }
    }
    if cfg!(windows)
        && [
            "evaluation-case-variant",
            "evaluation-unicode-case-variant",
            "public-duplicate-case-env",
        ]
        .contains(&mode)
    {
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
        let expected = if mode == "public-duplicate-case-env" {
            "empty or duplicate check environment name"
        } else {
            "repeats a public regression command"
        };
        assert!(
            result["short_reason"].as_str().unwrap().contains(expected),
            "{result}"
        );
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(c.plan.is_none() && c.checks.is_empty());
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        return;
    }
    if [
        "public-private-id",
        "public-duplicate-env",
        "private-duplicate-env",
    ]
    .contains(&mode)
    {
        assert_eq!(result["status"], "blocked", "{result}");
        let expected = if mode == "public-private-id" {
            "public check ID uses reserved independent- prefix"
        } else {
            "empty or duplicate check environment name"
        };
        assert!(result["short_reason"].as_str().unwrap().contains(expected));
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(c.plan.is_none() && c.checks.is_empty());
        if mode != "private-duplicate-env" {
            assert!(!rows().iter().any(|r| r["kind"] == "evaluation-prepared"));
        }
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        return;
    }
    if mode == "evaluation-recovered" {
        // A preparation failure is a typed, recoverable planning boundary.
        assert_eq!(result["status"], "blocked", "{result}");
        assert_eq!(result["error"]["kind"], "evaluation_unprepared", "{result}");
        let status = invoke(
            "status-before-recovery",
            json!({"type":"status","run_id":run}),
        );
        assert_eq!(status["error"], result["error"], "{status}");
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
        assert_eq!(result["status"], "ready_to_commit", "{result}\n{}", trail());
        let status = invoke(
            "status-after-recovery",
            json!({"type":"status","run_id":run}),
        );
        assert_eq!(status["status"], "ready_to_commit", "{status}");
        assert!(status.get("error").is_none(), "{status}");
        let recovered = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(checkpoint::planning_epoch(&recovered), 1);
        assert_eq!(rows().iter().filter(|r| r["kind"] == "research").count(), 1);
        assert!(recovered
            .receipts
            .keys()
            .any(|k| k.starts_with("planning-retry-")));
    }
    if [
        "evaluation-unprepared",
        "evaluation-fake-ready",
        "evaluation-reordered-env",
    ]
    .contains(&mode)
    {
        // The dev.8 shape and its relabelled-public-check variant: preparation
        // must fail before any check is installed or any writer starts.
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
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
    if [
        "runner-fail-fallback",
        "runner-forged",
        "runner-unknown",
        "runner-public-forged",
    ]
    .contains(&mode)
    {
        assert!(
            matches!(
                result["status"].as_str(),
                Some("blocked" | "outcome_unknown")
            ),
            "{result}\n{}",
            trail()
        );
        if mode != "runner-unknown" {
            assert_eq!(result["error"]["kind"], "evaluation_unprepared", "{result}");
        }
        let failed = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(failed.plan.is_none());
        assert!(failed.checks.is_empty());
        assert!(!rows().iter().any(|r| r["kind"] == "engineer-start"));
        if mode == "runner-fail-fallback" {
            assert!(failed.receipts.contains_key("runner-claim-0"));
            assert!(failed.receipts.contains_key("runner-result-0"));
        } else if ["runner-forged", "runner-public-forged"].contains(&mode) {
            assert!(!failed.receipts.contains_key("runner-claim-0"));
        } else {
            assert!(failed.receipts.contains_key("runner-claim-0"));
            assert!(!failed.receipts.contains_key("runner-result-0"));
            let before = rows().iter().filter(|r| r["kind"] == "command").count();
            let resumed = invoke(
                "runner-unknown-resume",
                json!({"type":"resume","run_id":run,"recovery":null}),
            );
            assert_ne!(resumed["status"], "ready_to_commit");
            assert_eq!(
                rows().iter().filter(|r| r["kind"] == "command").count(),
                before
            );
        }
        return;
    }
    if mode == "baseline-infrastructure" {
        // The checks are genuine but the baseline says the infrastructure
        // cannot judge the product: engineering must stay closed.
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
        assert_eq!(
            result["error"]["kind"], "evaluation_infrastructure",
            "{result}"
        );
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
        // `invoke` opens a fresh bridge each time. Both status after reload and
        // idempotent request replay retain the exact typed cause.
        let status = invoke("status-baseline", json!({"type":"status","run_id":run}));
        assert_eq!(status["error"], result["error"], "{status}");
        assert_eq!(status["short_reason"], result["short_reason"], "{status}");
        assert_eq!(invoke("decide", decide.clone()), result);
        let request_file = home
            .join("orchestrate/demo/requests")
            .join(format!("{:x}.json", Sha256::digest(b"decide")));
        let request: Value =
            serde_json::from_slice(&std::fs::read(&request_file).unwrap()).unwrap();
        let _: LegacyBoundaryResult = serde_json::from_value(request["result"].clone()).unwrap();
        assert_eq!(request["boundary_error"], result["error"]);
        let mut legacy_request = request.clone();
        legacy_request
            .as_object_mut()
            .unwrap()
            .remove("boundary_error");
        std::fs::write(&request_file, serde_json::to_vec(&legacy_request).unwrap()).unwrap();
        let legacy_replay = invoke("decide", decide.clone());
        assert_eq!(legacy_replay["error"]["kind"], "unknown", "{legacy_replay}");
        assert_eq!(legacy_replay["short_reason"], result["short_reason"]);

        // A corrupted receipt must not be presented as a valid diagnosis.
        let receipt = c.receipts.get(checkpoint::DISPATCH_FAILURE).unwrap();
        let original = std::fs::read(&receipt.path).unwrap();
        std::fs::write(&receipt.path, b"changed").unwrap();
        let corrupt = invoke("corrupt-status", json!({"type":"status","run_id":run}));
        assert_eq!(corrupt["status"], "bridge_error", "{corrupt}");
        assert_eq!(corrupt["error"]["kind"], "checkpoint", "{corrupt}");
        std::fs::write(&receipt.path, original).unwrap();

        let repeated = invoke(
            "same-block",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        assert_eq!(repeated["error"], result["error"], "{repeated}");
        let status = invoke("status-repeated", json!({"type":"status","run_id":run}));
        assert_eq!(status["error"], result["error"], "{status}");
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
        // A later mutation invalidates the earlier revision-bound diagnostic;
        // do not claim it explains a different blocked boundary.
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        checkpoint::record_dispatch_failure(&home.join("orchestrate/demo"), run, c.revision, false)
            .unwrap();
        let status = invoke("stale-status", json!({"type":"status","run_id":run}));
        assert_eq!(status["error"]["kind"], "unknown", "{status}");
        return;
    }
    if mode == "writer-blocked" {
        // A normally terminating turn that reports a blocked unit must not
        // satisfy dependents, assembly or verification.
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
        assert_eq!(result["error"]["kind"], "work_blocked", "{result}");
        let status = invoke("status-writer", json!({"type":"status","run_id":run}));
        assert_eq!(status["error"], result["error"], "{status}");
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
        assert_eq!(result["status"], "blocked", "{result}\n{}", trail());
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
    if mode == "writer-cancelled" {
        assert_eq!(result["status"], "blocked", "{result}");
        let store = home.join("orchestrate/demo");
        let before = checkpoint::load_run(&store, run).unwrap();
        let attempt = before
            .attempts
            .values()
            .find(|a| a.step_id == "u1")
            .unwrap_or_else(|| panic!("missing writer: {result}\n{}", trail()));
        assert_eq!(attempt.outcome, Some(checkpoint::AttemptOutcome::Cancelled));
        let report: Value = serde_json::from_slice(
            &std::fs::read(&attempt.raw_result.as_ref().unwrap().path).unwrap(),
        )
        .unwrap();
        let audit: ArtifactRef = serde_json::from_value(report["scope_audit"].clone()).unwrap();
        let saved_audit: Value =
            serde_json::from_slice(&std::fs::read(&audit.path).unwrap()).unwrap();
        assert_eq!(saved_audit["attempt_id"], attempt.attempt_id);
        assert!(saved_audit["after"]["one.txt"].is_string());
        let started = rows()
            .iter()
            .filter(|r| r["kind"] == "engineer-start")
            .count();
        let resumed = invoke(
            "resume-cancelled",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        assert_eq!(resumed["status"], "blocked", "{resumed}");
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "engineer-start")
                .count(),
            started
        );
        let current = checkpoint::load_run(&store, run).unwrap();
        let step = current.steps["u1"].clone();
        assert!(
            checkpoint::claim_step(&store, current.revision, &step, &step.brief_ref.digest)
                .is_err()
        );
        let stop = artifact(
            &store.join("stopped.json"),
            &serde_json::to_vec(&json!({
                "attempt_id":attempt.attempt_id,"terminal":true,"thread_id":attempt.thread_id,
                "turn_id":attempt.turn_id,"generation":attempt.generation
            }))
            .unwrap(),
        );
        let repair = artifact(
            &store.join("repair.json"),
            &serde_json::to_vec(&json!({
                "attempt_id":attempt.attempt_id,"repaired":true
            }))
            .unwrap(),
        );
        let rev = checkpoint::resolve_attempt(
            &store,
            run,
            current.revision,
            &checkpoint::RecoveryDecision::RetryAfterRepair {
                attempt_id: attempt.attempt_id.clone(),
                stopped_evidence: stop,
                repair_evidence: repair,
                reason: "stopped and repaired".into(),
            },
        )
        .unwrap();
        let repaired = checkpoint::load_run(&store, run).unwrap();
        assert_eq!(
            repaired.attempts[&attempt.attempt_id].raw_result,
            attempt.raw_result
        );
        assert_eq!(
            checkpoint::claim_step(&store, rev, &step, &step.brief_ref.digest)
                .unwrap()
                .disposition,
            checkpoint::ClaimDisposition::New
        );
        return;
    }
    if [
        "unknown-write",
        "unknown-then-failed",
        "unknown-partial",
        "unknown-completed-inside",
        "unknown-completed-outside",
        "unknown-history-cancelled",
        "unknown-history-failed",
        "unknown-no-turn",
    ]
    .contains(&mode)
    {
        assert_eq!(result["status"], "outcome_unknown", "{result}\n{}", trail());
        assert_eq!(result["error"]["kind"], "outcome_unknown", "{result}");
        let original_reason = result["short_reason"].as_str().unwrap();
        assert!(original_reason.contains("step "), "{result}");
        assert!(original_reason.contains("thread "), "{result}");
        assert!(original_reason.contains("status "), "{result}");
        let mut before = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        if mode == "unknown-no-turn" {
            // Model a turn/start response lost before its ID was persisted.
            for a in before.attempts.values_mut().filter(|a| {
                !matches!(
                    a.state,
                    checkpoint::AttemptState::Prepared | checkpoint::AttemptState::Terminal
                )
            }) {
                a.turn_id = None;
            }
            std::fs::write(
                home.join("orchestrate/demo/runs")
                    .join(run)
                    .join("checkpoint.json"),
                serde_json::to_vec(&before).unwrap(),
            )
            .unwrap();
        }
        let active_before = before
            .attempts
            .values()
            .filter(|a| {
                !matches!(
                    a.state,
                    checkpoint::AttemptState::Prepared | checkpoint::AttemptState::Terminal
                ) && before.steps.get(&a.step_id).is_some_and(|s| {
                    matches!(
                        s.purpose,
                        checkpoint::Purpose::Engineer | checkpoint::Purpose::Assembler
                    )
                })
            })
            .count();
        assert!(active_before > 0, "{result}");
        let writes_before = rows()
            .iter()
            .filter(|r| r["kind"] == "engineer-start")
            .count();

        let resumed = invoke(
            "resume-unknown",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        let after = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        let active_after = after
            .attempts
            .values()
            .filter(|a| {
                !matches!(
                    a.state,
                    checkpoint::AttemptState::Prepared | checkpoint::AttemptState::Terminal
                ) && after.steps.get(&a.step_id).is_some_and(|s| {
                    matches!(
                        s.purpose,
                        checkpoint::Purpose::Engineer | checkpoint::Purpose::Assembler
                    )
                })
            })
            .count();
        assert_eq!(active_after, active_before, "{resumed}");
        for attempt in before.attempts.values().filter(|a| {
            before.steps.get(&a.step_id).is_some_and(|s| {
                matches!(
                    s.purpose,
                    checkpoint::Purpose::Engineer | checkpoint::Purpose::Assembler
                )
            })
        }) {
            assert_eq!(
                after.attempts.get(&attempt.attempt_id),
                Some(attempt),
                "{resumed}"
            );
        }
        assert_eq!(after.reservations, before.reservations, "{resumed}");
        assert_eq!(resumed["status"], "outcome_unknown", "{resumed}");
        assert_eq!(resumed["error"]["kind"], "outcome_unknown", "{resumed}");
        assert!(
            resumed["short_reason"]
                .as_str()
                .unwrap()
                .contains("stop and repair"),
            "{resumed}"
        );
        let writer_threads = before
            .attempts
            .values()
            .filter(|a| {
                before.steps.get(&a.step_id).is_some_and(|s| {
                    matches!(
                        s.purpose,
                        checkpoint::Purpose::Engineer | checkpoint::Purpose::Assembler
                    )
                })
            })
            .filter_map(|a| a.thread_id.as_deref())
            .collect::<Vec<_>>();
        if mode == "unknown-no-turn" {
            assert!(
                !rows().iter().any(|r| r["kind"] == "history-failed"
                    && r["thread"]
                        .as_str()
                        .is_some_and(|t| writer_threads.contains(&t))),
                "a missing turn ID must not select the latest history turn"
            );
        } else if mode != "unknown-write" {
            assert!(
                rows().iter().any(|r| r["kind"] == "history-failed"
                    && r["thread"]
                        .as_str()
                        .is_some_and(|t| writer_threads.contains(&t))),
                "mode={mode}; writer history terminal was not observed: {}",
                trail()
            );
        }
        let status = invoke(
            "status-after-unknown-resume",
            json!({"type":"status","run_id":run}),
        );
        assert_eq!(status["status"], resumed["status"], "{status}");
        assert_eq!(status["error"], resumed["error"], "{status}");
        assert_eq!(status["short_reason"], resumed["short_reason"], "{status}");
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "engineer-start")
                .count(),
            writes_before
        );
        return;
    }
    if ["memory-forged", "memory-stale"].contains(&mode) {
        assert_eq!(result["status"], "blocked", "{result}");
        assert!(
            result["short_reason"]
                .as_str()
                .unwrap()
                .contains("current verified review"),
            "{result}"
        );
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert_eq!(c.phase, Phase::Memory);
        assert!(!c.receipts.contains_key("memory"));
        return;
    }
    if mode == "unready-peer-write" {
        assert_eq!(result["status"], "blocked", "{result}");
        let c = checkpoint::load_run(&home.join("orchestrate/demo"), run).unwrap();
        assert!(!c.receipts.contains_key("assembly"));
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "engineer-start")
                .count(),
            1
        );
        let attempt = c.attempts.values().find(|a| a.step_id == "u1").unwrap();
        assert_eq!(attempt.outcome, Some(checkpoint::AttemptOutcome::Failed));
        let report: Value = serde_json::from_slice(
            &std::fs::read(&attempt.raw_result.as_ref().unwrap().path).unwrap(),
        )
        .unwrap();
        assert_eq!(report["error"]["kind"], "scope_violation");
        assert!(!c.attempts.values().any(|a| a.step_id == "u2"));
        let _ = invoke(
            "scope-resume",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        assert_eq!(
            rows()
                .iter()
                .filter(|r| r["kind"] == "engineer-start")
                .count(),
            1
        );
        return;
    }
    if ![
        "normal",
        "directory-peers",
        "content",
        "repairable-review",
        "repairable-review-twice",
        "evaluation-repairable-review",
        "evaluation",
        "evaluation-distinct-env",
        "evaluation-case-variant",
        "evaluation-unicode-case-variant",
        "evaluation-lowercase-default",
        "public-duplicate-case-env",
        "planning-retry",
        "retry-second-epoch",
        "evaluation-recovered",
        "runner-ready",
    ]
    .contains(&mode)
    {
        assert_ne!(result["status"], "ready_to_commit", "{result}");
        if matches!(
            result["status"].as_str(),
            Some("blocked" | "outcome_unknown")
        ) {
            let status = invoke("status-failed-flow", json!({"type":"status","run_id":run}));
            assert_eq!(status["error"], result["error"], "{status}");
            assert_ne!(status["error"]["kind"], "tool_failed", "{status}");
        }
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
    assert_eq!(result["status"], "ready_to_commit", "{result}\n{}", trail());
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
    let selected = checkpoint::verified_review(&c).expect("passing review");
    let memory_receipt: Value =
        serde_json::from_slice(&std::fs::read(&c.receipts["memory"].path).unwrap()).unwrap();
    assert_eq!(memory_receipt["review_digest"], selected.digest);
    let memory_turn = records
        .iter()
        .find(|r| {
            r["kind"] == "turn"
                && r["params"]["input"][0]["text"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("ROLE Memory"))
        })
        .expect("memory role");
    let memory_thread = records
        .iter()
        .find(|r| r["kind"] == "thread" && r["thread"] == memory_turn["params"]["threadId"])
        .expect("memory thread");
    assert_eq!(
        memory_thread["params"]["dynamicTools"][0]["inputSchema"]["properties"]["evidence_id"]
            ["enum"][0],
        selected.digest
    );
    let memory_brief = memory_turn["params"]["input"][0]["text"].as_str().unwrap();
    assert!(memory_brief.contains(&selected.digest));
    let memory = axiom::memory::Memory::open(&home).unwrap();
    assert_eq!(
        memory
            .record_run_with_evidence("demo", run, "forged", "not-a-review-digest")
            .unwrap_err()
            .kind,
        "not_verified"
    );
    assert_eq!(
        memory
            .record_run_with_evidence("demo", run, "verified", &selected.digest)
            .unwrap()["id"],
        memory_receipt["id"]
    );
    if mode == "repairable-review" {
        // freeze_verification clears old review receipts on the normal rework
        // path; historical failure stays auditable in the rework artifact.
        assert_eq!(
            c.receipts
                .keys()
                .filter(|k| k.starts_with("review-"))
                .count(),
            1
        );
        let feedback: Value =
            serde_json::from_slice(&std::fs::read(&c.receipts["rework-1"].path).unwrap()).unwrap();
        let old: ArtifactRef = serde_json::from_value(
            feedback
                .as_object()
                .unwrap()
                .iter()
                .find(|(key, _)| key.starts_with("review-"))
                .unwrap()
                .1
                .clone(),
        )
        .unwrap();
        let old_review: checkpoint::IndependentReview =
            serde_json::from_slice(&std::fs::read(&old.path).unwrap()).unwrap();
        assert!(!old_review.pass);
        assert!(!memory_brief.contains(&old.digest));
        assert_eq!(
            memory
                .record_run_with_evidence("demo", run, "failed review", &old.digest)
                .unwrap_err()
                .kind,
            "not_verified"
        );
    }
    if [
        "evaluation",
        "evaluation-repairable-review",
        "evaluation-distinct-env",
        "evaluation-case-variant",
        "evaluation-unicode-case-variant",
        "evaluation-lowercase-default",
        "public-duplicate-case-env",
        "planning-retry",
        "retry-second-epoch",
        "evaluation-recovered",
        "runner-ready",
    ]
    .contains(&mode)
    {
        let prep = records
            .iter()
            .find(|r| r["kind"] == "evaluation-prepared")
            .unwrap();
        let first_write = records
            .iter()
            .find(|r| r["kind"] == "engineer-start")
            .unwrap();
        assert!(prep["time"].as_f64().unwrap() < first_write["time"].as_f64().unwrap());
        assert!(c.checks.contains_key(
            if ["planning-retry", "retry-second-epoch"].contains(&mode) {
                "independent-h"
            } else {
                "independent-output"
            }
        ));
        let public_plan = std::fs::read_to_string(&c.plan.as_ref().unwrap().path).unwrap();
        assert!(!public_plan.contains("PRIVATE_EVALUATION_CANARY"));
        let evaluation = c
            .receipts
            .get(&format!("evaluation-{}", checkpoint::planning_epoch(&c)))
            .unwrap();
        if mode == "runner-ready" {
            assert!(c.receipts.contains_key("runner-claim-0"));
            assert!(c.receipts.contains_key("runner-result-0"));
            let private = &c.checks["independent-output"];
            assert!(private.program.starts_with("@axiom:rust:v1:"));
            let commands: Vec<_> = records.iter().filter(|r| r["kind"] == "command").collect();
            let compiled = commands
                .iter()
                .filter(|r| {
                    r["params"]["command"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|x| x == "--emit=link"))
                })
                .count();
            assert_eq!(compiled, 1, "runner must be compiled exactly once");
            let executions: Vec<_> = commands
                .iter()
                .filter(|r| {
                    r["params"]["command"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|x| x.as_str())
                        .is_some_and(|program| program.contains("prepared-runner-exec"))
                })
                .collect();
            assert_eq!(
                executions.len(),
                2,
                "baseline and final both use the materialized runner"
            );
            let receipt: axiom::orch::runner::RunnerReceipt = serde_json::from_slice(
                &std::fs::read(&c.receipts["runner-result-0"].path).unwrap(),
            )
            .unwrap();
            for execution in executions {
                let path = execution["params"]["command"][0].as_str().unwrap();
                let bytes = std::fs::read(path).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(bytes)),
                    receipt.binary.as_ref().unwrap().digest
                );
            }
            for row in records.iter().filter(|r| r["kind"] == "turn") {
                let brief = row["params"]["input"][0]["text"].as_str().unwrap();
                if brief.starts_with("ROLE Engineer") || brief.starts_with("ROLE Assembler") {
                    assert!(!brief.contains(&receipt.source.path.to_string_lossy().to_string()));
                    assert!(!brief.contains(
                        &receipt
                            .binary
                            .as_ref()
                            .unwrap()
                            .path
                            .to_string_lossy()
                            .to_string()
                    ));
                }
            }
        }
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
        if mode == "evaluation-repairable-review" {
            let feedback: Value =
                serde_json::from_slice(&std::fs::read(&c.receipts["rework-1"].path).unwrap())
                    .unwrap();
            assert!(!feedback["check-frozen-python"].is_null());
            assert!(feedback["check-independent-output"].is_null());
        }
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
        if mode == "repairable-review-twice" {
            3
        } else if ["repairable-review", "evaluation-repairable-review"].contains(&mode) {
            2
        } else {
            1
        }
    );
    let command = records
        .iter()
        .find(|r| {
            r["kind"] == "command"
                && r["params"]["command"][1] == "-I"
                && r["params"]["command"][2] == "-c"
        })
        .unwrap();
    if mode == "evaluation-lowercase-default" {
        let env = command["params"]["env"].as_object().unwrap();
        assert!(env.contains_key("cargo_target_dir"));
        assert!(!env.contains_key("CARGO_TARGET_DIR"));
    }
    assert_eq!(
        Path::new(command["process_cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        Path::new(command["params"]["cwd"].as_str().unwrap())
            .canonicalize()
            .unwrap()
    );
    let project_store = home.join("orchestrate/demo").canonicalize().unwrap();
    let command_scratch = Path::new(command["params"]["cwd"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert!(command_scratch.starts_with(project_store.join("s")));
    let writable = command["params"]["sandboxPolicy"]["writableRoots"]
        .as_array()
        .unwrap();
    assert_eq!(writable.len(), 1);
    assert_eq!(
        Path::new(writable[0].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        command_scratch
    );
    let result: checkpoint::CheckResult =
        serde_json::from_slice(&std::fs::read(&c.receipts["check-frozen-python"].path).unwrap())
            .unwrap();
    for output in [&result.stdout, &result.stderr] {
        assert!(output
            .path
            .canonicalize()
            .unwrap()
            .starts_with(project_store.join("s")));
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&output.path).unwrap())),
            output.digest
        );
    }
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
    if ["repairable-review", "repairable-review-twice"].contains(&mode) {
        let store = home.join("orchestrate/demo");
        let cp_path = store.join("runs").join(run).join("checkpoint.json");
        let saved_checkpoint = std::fs::read(&cp_path).unwrap();
        let current_step = if mode == "repairable-review" {
            "assembly-rework-1"
        } else {
            "assembly-rework-2"
        };
        let old_step = if mode == "repairable-review" {
            "assembly"
        } else {
            "assembly-rework-1"
        };
        let current = c
            .attempts
            .values()
            .find(|a| a.step_id == current_step)
            .unwrap();
        let old = c.attempts.values().find(|a| a.step_id == old_step).unwrap();
        let mut pending = c.clone();
        pending.phase = Phase::Assembling;
        pending.receipts.remove("assembly");
        std::fs::write(&cp_path, serde_json::to_vec(&pending).unwrap()).unwrap();
        assert!(
            checkpoint::advance_phase(&store, run, pending.revision, Phase::Verifying).is_err(),
            "a previous assembly cannot fill a missing receipt"
        );
        pending
            .receipts
            .insert("assembly".into(), old.raw_result.clone().unwrap());
        std::fs::write(&cp_path, serde_json::to_vec(&pending).unwrap()).unwrap();
        assert!(
            checkpoint::advance_phase(&store, run, pending.revision, Phase::Verifying).is_err(),
            "a previous assembly receipt cannot authorize this rework epoch"
        );
        pending
            .receipts
            .insert("assembly".into(), current.raw_result.clone().unwrap());
        std::fs::write(&cp_path, serde_json::to_vec(&pending).unwrap()).unwrap();
        checkpoint::advance_phase(&store, run, pending.revision, Phase::Verifying).unwrap();
        std::fs::write(&cp_path, &saved_checkpoint).unwrap();

        if mode == "repairable-review" {
            // This is the genuine audit created by the original live bridge
            // role, not a reconstructed or test-authored audit. Roll the
            // checkpoint back to simulate a crash after report persistence.
            let writer = c.attempts.values().find(|a| a.step_id == "u1").unwrap();
            let mut interrupted = c.clone();
            interrupted.phase = Phase::Engineering;
            interrupted.status = RunStatus::OutcomeUnknown;
            let unfinished = interrupted.attempts.get_mut(&writer.attempt_id).unwrap();
            unfinished.state = checkpoint::AttemptState::OutcomeUnknown;
            unfinished.outcome = None;
            unfinished.raw_result = None;
            interrupted
                .pending_evidence
                .retain(|p| p.attempt_id != writer.attempt_id);
            interrupted.reservations.push(checkpoint::ScopeReservation {
                attempt_id: writer.attempt_id.clone(),
                paths: interrupted.steps["u1"].scope.clone(),
            });
            std::fs::write(&cp_path, serde_json::to_vec(&interrupted).unwrap()).unwrap();
            checkpoint::reconcile_run(&store, run, interrupted.revision, &[]).unwrap();
            let recovered = checkpoint::load_run(&store, run).unwrap();
            assert_eq!(
                recovered.attempts[&writer.attempt_id].raw_result,
                writer.raw_result
            );
            assert_eq!(
                recovered.attempts[&writer.attempt_id].outcome,
                Some(checkpoint::AttemptOutcome::Completed)
            );
            assert!(recovered.reservations.is_empty());
            let writer_report: Value = serde_json::from_slice(
                &std::fs::read(&writer.raw_result.as_ref().unwrap().path).unwrap(),
            )
            .unwrap();
            let audit: ArtifactRef =
                serde_json::from_value(writer_report["scope_audit"].clone()).unwrap();
            let original_audit = std::fs::read(&audit.path).unwrap();
            std::fs::write(&audit.path, b"tampered audit").unwrap();
            std::fs::write(&cp_path, serde_json::to_vec(&interrupted).unwrap()).unwrap();
            let rejected =
                checkpoint::reconcile_run(&store, run, interrupted.revision, &[]).unwrap();
            assert!(rejected.actions.iter().any(|a| matches!(a, checkpoint::RecoveryAction::NeedsDecision { attempt_id, .. } if attempt_id == &writer.attempt_id)));
            let retained = checkpoint::load_run(&store, run).unwrap();
            assert_eq!(
                retained.attempts[&writer.attempt_id].state,
                checkpoint::AttemptState::OutcomeUnknown
            );
            assert_eq!(retained.reservations.len(), 1);
            std::fs::write(&audit.path, original_audit).unwrap();
            std::fs::write(&cp_path, &saved_checkpoint).unwrap();
        }
    }
    if mode == "repairable-review" {
        let review: checkpoint::IndependentReview =
            serde_json::from_slice(&std::fs::read(&selected.path).unwrap()).unwrap();
        let raw_report = std::fs::read(&review.report.path).unwrap();
        let checkpoint_path = home
            .join("orchestrate/demo/runs")
            .join(run)
            .join("checkpoint.json");
        let checkpoint_before = std::fs::read(&checkpoint_path).unwrap();
        let request_path = home
            .join("orchestrate/demo/requests")
            .join(format!("{:x}.json", Sha256::digest(b"decide")));
        let request_before = std::fs::read(&request_path).unwrap();
        std::fs::write(&review.report.path, b"forged report").unwrap();
        let status = invoke(
            "status-forged-review",
            json!({"type":"status","run_id":run}),
        );
        assert_eq!(status["status"], "blocked", "{status}");
        assert_eq!(status["error"]["kind"], "unverified_review", "{status}");
        let replay = invoke("decide", decide.clone());
        assert_eq!(replay["status"], "blocked", "{replay}");
        assert_eq!(replay["error"]["kind"], "unverified_review", "{replay}");
        let resume = invoke(
            "resume-forged-review",
            json!({"type":"resume","run_id":run,"recovery":null}),
        );
        assert_eq!(resume["status"], "blocked", "{resume}");
        assert_eq!(std::fs::read(&checkpoint_path).unwrap(), checkpoint_before);
        assert_eq!(std::fs::read(&request_path).unwrap(), request_before);
        std::fs::write(&review.report.path, raw_report).unwrap();
        let restored = invoke(
            "status-restored-review",
            json!({"type":"status","run_id":run}),
        );
        assert_eq!(restored["status"], "ready_to_commit", "{restored}");
        std::fs::write(repo.join("one.txt"), "changed after review").unwrap();
        assert_eq!(
            memory
                .record_run_with_evidence("demo", run, "stale", &selected.digest)
                .unwrap_err()
                .kind,
            "not_verified"
        );
    }
    use std::path::Path;
}

#[test]
fn independent_evaluation_is_created_before_writes_and_omitted_from_writer_context() {
    flow("evaluation", true);
}

#[test]
fn prepared_runner_links_once_and_failure_or_forgery_cannot_bypass_preparation() {
    flow("runner-ready", true);
    flow("runner-fail-fallback", true);
    flow("runner-forged", true);
    flow("runner-public-forged", true);
    flow("runner-unknown", true);
}
#[test]
fn failed_read_only_planning_can_resume_without_repeating_research_or_erasing_failure() {
    flow("planning-retry", true);
}

#[test]
fn planning_retry_rejects_missing_tampered_or_wrong_epoch_receipts_before_writers() {
    for mode in ["retry-missing", "retry-tampered", "retry-wrong-epoch"] {
        flow(mode, true);
    }
}

#[test]
fn second_planning_retry_passes_both_verified_corrections_in_order() {
    flow("retry-second-epoch", true);
}

#[test]
fn public_check_cannot_use_private_prefix() {
    flow("public-private-id", true);
}
#[test]
fn batch_failure_cancels_pending_peer_instead_of_waiting_for_its_execution_budget() {
    flow("failed-peer", true);
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
fn two_reworks_require_each_current_assembly_and_verifier() {
    flow("repairable-review-twice", true);
}
#[test]
fn memory_callback_rejects_forged_and_stale_review_evidence() {
    flow("memory-forged", true);
    flow("memory-stale", true);
}

#[test]
fn rework_keeps_public_evidence_and_withholds_private_check_evidence() {
    flow("evaluation-repairable-review", true);
}
#[test]
fn rpc_approve_parallel_units_assembly_real_check_memory_boundary() {
    flow("normal", true);
}
#[test]
fn managed_scope_rejects_unready_peer_writes_and_allows_new_parent_directories() {
    flow("unready-peer-write", true);
    flow("directory-peers", true);
}
#[test]
fn rpc_role_spoof_check_claim_and_unknown_write_negative_controls() {
    for mode in [
        "spoof",
        "no-check",
        "failed-check",
        "rejected-review",
        "unknown-check",
    ] {
        flow(mode, true);
    }
}

#[test]
fn unknown_execution_diagnostic_tracks_reconciliation_progress() {
    for mode in [
        "unknown-write",
        "unknown-then-failed",
        "unknown-partial",
        "unknown-completed-inside",
        "unknown-completed-outside",
        "unknown-history-cancelled",
        "unknown-history-failed",
        "unknown-no-turn",
    ] {
        flow(mode, true);
    }
}

#[test]
fn cancelled_writer_requires_explicit_stop_and_repair_before_retry() {
    flow("writer-cancelled", true);
}

fn artifact(path: &std::path::Path, bytes: &[u8]) -> ArtifactRef {
    std::fs::write(path, bytes).unwrap();
    ArtifactRef {
        path: path.into(),
        digest: format!("{:x}", Sha256::digest(bytes)),
    }
}

#[test]
fn historical_ready_status_and_replay_do_not_authorize_new_snapshot_policy() {
    use serde_json::json;
    let tmp = common::temp();
    let repo = tmp.path().join("repo");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&repo).unwrap();
    let repo = repo.canonicalize().unwrap();
    let store = home.join("orchestrate/demo");
    std::fs::create_dir_all(&store).unwrap();
    let mut c = checkpoint::create_run(
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
    c.plan = Some(artifact(&store.join("plan"), b"historical plan"));
    c.checks.insert(
        "check".into(),
        checkpoint::FrozenCheck {
            check_id: "check".into(),
            program: "verify".into(),
            args: vec![],
            env: Default::default(),
            timeout_s: 10,
            snapshot: checkpoint::Snapshot {
                source_digest: format!("tree:{}", "a".repeat(40)),
                plan_digest: c.plan.as_ref().unwrap().digest.clone(),
                checks_digest: "checks".into(),
            },
            spec_digest: "spec".into(),
        },
    );
    c.phase = Phase::ReadyToCommit;
    c.status = RunStatus::ReadyToCommit;
    let cp = store.join("runs/run/checkpoint.json");
    std::fs::write(&cp, serde_json::to_vec(&c).unwrap()).unwrap();
    let before = std::fs::read(&cp).unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home),
        codex: Err(axiom::base::Failure::new("missing", "must not launch")),
        agy: Err(axiom::base::Failure::new("missing", "must not launch")),
        wsl: None,
        on_windows: cfg!(windows),
    });
    let root = RootContext {
        repo_root: repo,
        project: "demo".into(),
    };
    let status = bridge
        .dispatch(
            &root,
            &DispatchRequest {
                request_id: "inspect".into(),
                action: DispatchAction::Status {
                    run_id: "run".into(),
                },
            },
            &DispatchOptions::default(),
            None,
        )
        .unwrap();
    assert_eq!(status.phase, Phase::ReadyToCommit);
    assert_eq!(status.status, RunStatus::Blocked);
    assert_eq!(status.error.unwrap().kind, "legacy_snapshot");
    assert!(status.short_reason.contains("ReadyToCommit"));
    assert_eq!(std::fs::read(&cp).unwrap(), before);

    let request = DispatchRequest {
        request_id: "original-decide".into(),
        action: DispatchAction::Decide {
            run_id: "run".into(),
            research_digest: "old-research".into(),
            pass: true,
            reason: "historical".into(),
        },
    };
    let input = serde_json::to_vec(&(&root, &request)).unwrap();
    let request_dir = store.join("requests");
    std::fs::create_dir_all(&request_dir).unwrap();
    let request_path = request_dir.join(format!(
        "{:x}.json",
        Sha256::digest(request.request_id.as_bytes())
    ));
    let saved = serde_json::to_vec(&json!({"input":format!("{:x}", Sha256::digest(&input)), "result":{
        "run_id":"run", "phase":"ready_to_commit", "status":"ready_to_commit",
        "short_reason":"historical ready", "research":null, "artifact_refs":[], "next_action":null
    }})).unwrap();
    std::fs::write(&request_path, &saved).unwrap();
    let replay = bridge
        .dispatch(&root, &request, &DispatchOptions::default(), None)
        .unwrap();
    assert_eq!(replay.status, RunStatus::Blocked);
    assert_eq!(replay.error.unwrap().kind, "legacy_snapshot");
    assert_eq!(std::fs::read(&cp).unwrap(), before);
    assert_eq!(std::fs::read(&request_path).unwrap(), saved);

    c.status = RunStatus::OutcomeUnknown;
    c.attempts.insert(
        "pending".into(),
        checkpoint::Attempt {
            attempt_id: "pending".into(),
            step_id: "old-writer".into(),
            input_digest: "input".into(),
            state: checkpoint::AttemptState::OutcomeUnknown,
            generation: Some(1),
            thread_id: Some("thread".into()),
            turn_id: Some("turn".into()),
            raw_result: None,
            outcome: None,
            last_event_sequence: 2,
        },
    );
    std::fs::write(&cp, serde_json::to_vec(&c).unwrap()).unwrap();
    let before = std::fs::read(&cp).unwrap();
    let unknown = bridge
        .dispatch(
            &root,
            &DispatchRequest {
                request_id: "inspect-active".into(),
                action: DispatchAction::Status {
                    run_id: "run".into(),
                },
            },
            &DispatchOptions::default(),
            None,
        )
        .unwrap();
    assert_eq!(unknown.status, RunStatus::OutcomeUnknown);
    assert_eq!(unknown.error.unwrap().kind, "outcome_unknown");
    assert_eq!(std::fs::read(&cp).unwrap(), before);
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
    let missing = ArtifactRef {
        path: store.join("missing-failure"),
        digest: "missing".into(),
    };
    assert!(checkpoint::record_dispatch_failure_with_receipt(
        &store, "run", c.revision, false, &missing
    )
    .is_err());
    let unchanged = checkpoint::load_run(&store, "run").unwrap();
    assert_eq!(unchanged.revision, c.revision);
    assert_eq!(unchanged.status, c.status);
    let fake = artifact(&store.join("fake-failure"), b"not a boundary");
    std::fs::create_dir_all(store.join("runs/run/artifacts")).unwrap();
    let error =
        checkpoint::record_dispatch_failure_with_receipt(&store, "run", c.revision, false, &fake)
            .unwrap_err();
    assert_eq!(error.kind, "checkpoint");
    assert_eq!(
        checkpoint::load_run(&store, "run").unwrap().revision,
        c.revision
    );
    assert!(checkpoint::record_receipt(
        &store,
        "run",
        c.revision,
        checkpoint::DISPATCH_FAILURE,
        &fake
    )
    .is_err());
    checkpoint::record_dispatch_failure(&store, "run", c.revision, false).unwrap();
    let b = Bridge::new(Config {
        data_home: Some(repo.clone()),
        codex: Err(axiom::base::Failure::new("missing", "must not launch")),
        agy: Err(axiom::base::Failure::new("missing", "must not launch")),
        wsl: None,
        on_windows: false,
    });
    let legacy_status = b
        .dispatch(
            &RootContext {
                repo_root: repo.clone(),
                project: "demo".into(),
            },
            &DispatchRequest {
                request_id: "legacy-status".into(),
                action: DispatchAction::Status {
                    run_id: "run".into(),
                },
            },
            &DispatchOptions::default(),
            None,
        )
        .unwrap();
    assert_eq!(legacy_status.error.unwrap().kind, "unknown");
    assert!(legacy_status
        .short_reason
        .contains("did not record its cause"));
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
fn verifier_history_null_uses_persisted_model_after_catalog_changes() {
    flow("verifier-history-null", true);
}

#[test]
fn pre_turn_model_failure_can_resume_original_prepared_writers_after_config_repair() {
    flow("engineer-config-missing", true);
    flow("engineer-config-mismatch", true);
}

#[test]
fn pre_turn_model_failure_can_cancel_without_fabricating_writer_completion() {
    flow("engineer-config-cancel", true);
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
fn reordered_identical_environment_is_not_held_out_but_distinct_environment_is_allowed() {
    flow("evaluation-reordered-env", true);
    flow("evaluation-distinct-env", true);
}

#[test]
fn duplicate_environment_names_are_rejected_before_any_writer() {
    flow("public-duplicate-env", true);
    flow("private-duplicate-env", true);
}

#[test]
fn environment_case_variants_follow_platform_semantics() {
    flow("evaluation-case-variant", true);
    flow("evaluation-unicode-case-variant", true);
    flow("public-duplicate-case-env", true);
}

#[cfg(windows)]
#[test]
fn windows_default_environment_does_not_override_frozen_lowercase_name() {
    flow("evaluation-lowercase-default", true);
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
