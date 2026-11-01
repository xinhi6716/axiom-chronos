use axiom::{
    base,
    bridge::{
        self,
        dispatch::{DispatchAction, DispatchOptions, DispatchRequest, RootContext},
    },
    session,
};
use std::sync::{atomic::AtomicBool, Arc};

#[test]
fn status_unknown_does_not_launch_codex() {
    let dir = tempfile::tempdir().unwrap();
    let root = RootContext {
        repo_root: dir.path().canonicalize().unwrap(),
        project: "test".into(),
    };
    let config = bridge::Config {
        data_home: Some(dir.path().into()),
        codex: Err(base::Failure::new("sentinel", "must not start")),
        agy: Err(base::Failure::new("sentinel", "unused")),
        wsl: None,
        on_windows: cfg!(windows),
    };
    let bridge = bridge::Bridge::new(config);
    let request = DispatchRequest {
        request_id: "request-1".into(),
        action: DispatchAction::Status {
            run_id: "missing".into(),
        },
    };
    let options = DispatchOptions {
        max_wait_ms: 0,
        cancel_grace_ms: 10000,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let mut out = Vec::new();
    assert!(session::dispatch_request(&bridge, &root, &request, &options, &mut out).is_err());
    assert!(!bridge.running());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&out).unwrap()["request_id"],
        "request-1"
    );
}

#[test]
fn status_reads_existing_checkpoint_without_starting_a_model() {
    use axiom::orch::checkpoint::{create_run, ArtifactRef, RepoIdentity, RunSeed};
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("orchestrate/test");
    std::fs::create_dir_all(&store).unwrap();
    let path = store.join("intent.txt");
    std::fs::write(&path, b"test intent").unwrap();
    let root = RootContext {
        repo_root: dir.path().canonicalize().unwrap(),
        project: "test".into(),
    };
    create_run(
        &store,
        &RunSeed {
            run_id: "run-1".into(),
            repo: RepoIdentity {
                canonical_root: root.repo_root.clone(),
                identity_digest: "repo".into(),
            },
            intent: ArtifactRef {
                path,
                digest: format!("{:x}", Sha256::digest(b"test intent")),
            },
        },
    )
    .unwrap();
    let bridge = bridge::Bridge::new(bridge::Config {
        data_home: Some(dir.path().into()),
        codex: Err(base::Failure::new("sentinel", "must not start")),
        agy: Err(base::Failure::new("sentinel", "unused")),
        wsl: None,
        on_windows: cfg!(windows),
    });
    let request = DispatchRequest {
        request_id: "status-1".into(),
        action: DispatchAction::Status {
            run_id: "run-1".into(),
        },
    };
    let mut out = Vec::new();
    assert_eq!(
        session::dispatch_request(
            &bridge,
            &root,
            &request,
            &DispatchOptions::default(),
            &mut out
        )
        .unwrap(),
        0
    );
    let text = String::from_utf8(out).unwrap();
    let rows: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows[1]["request_id"], "status-1");
    assert_eq!(rows[1]["result"]["run_id"], "run-1");
    assert!(!bridge.running());
}

#[test]
fn incomplete_recovery_is_rejected() {
    use axiom::bridge::dispatch::RecoveryDecision;
    assert!(serde_json::from_str::<RecoveryDecision>(
        r#"{"retry_after_repair":{"attempt_id":"a","reason":"try"}}"#
    )
    .is_err());
}

#[test]
fn unsupported_legacy_turn_limit_is_not_silently_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let config = session::Config::from_env();
    let mut out = Vec::new();
    let err = session::run(
        &config,
        "wish",
        "test",
        dir.path(),
        Some(7),
        Box::new(std::io::Cursor::new(Vec::<u8>::new())),
        &mut out,
    )
    .unwrap_err();
    assert_eq!(err.kind, "blocked_capability");
    assert!(out.is_empty());
}
