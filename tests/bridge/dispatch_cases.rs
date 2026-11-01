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
    assert_eq!(c.attempts.len(), 2);
    assert!(c
        .attempts
        .values()
        .all(|a| a.state == checkpoint::AttemptState::Terminal));
    let result = invoke(
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
    if !["normal", "content", "repairable-review"].contains(&mode) {
        assert_ne!(result["status"], "ready_to_commit", "{result}");
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
