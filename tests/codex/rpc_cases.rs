use crate::common;
use axiom::{
    base::Failure,
    codex::{role_matches, AppServer, TESTED_CODEX},
};
use std::time::Duration;

const NEWER: &str = "--fake-version=codex-cli 999.1.2";

fn assert_names_versions(failure: &Failure) {
    assert_eq!(failure.kind, "codex_protocol", "{failure}");
    assert!(
        failure.message.contains("installed codex-cli 999.1.2"),
        "{failure}"
    );
    assert!(
        failure.message.contains(&format!(
            "last tested with Chronos {}",
            TESTED_CODEX.last_tested
        )),
        "{failure}"
    );
}

#[test]
fn models_are_paginated_and_role_matching_is_exact() {
    let server = AppServer::new(common::fake("fake_codex"));
    assert!(!server.alive());
    let models = server.list_models().unwrap();
    assert!(server.alive());
    assert!(models.contains(&"gpt-6.9-terra".to_string()));
    assert_eq!(server.resolve_model("terra").unwrap(), "gpt-6.10-terra");
    assert_eq!(
        server.resolve_model("sol").unwrap_err().kind,
        "model_not_found"
    );
    assert!(!role_matches("terra", "gpt-6-terra-lite"));
    assert!(!role_matches("terra", "gpt-6-terran"));
    assert!(role_matches("terra", "gpt-6.10-terra"));
    server.close();
    assert!(!server.alive());
    assert_eq!(server.resolve_model("luna").unwrap(), "gpt-6-luna");
}

#[test]
fn missing_role_refreshes_exactly_once_and_reuses_process() {
    let dir = common::temp();
    let script = dir.path().join("server.py");
    let log = dir.path().join("log");
    std::fs::write(
        &script,
        r#"
import json, sys, os
log = sys.argv[1]
with open(log, 'a') as f: f.write('start\n')
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get('method')
    if method == 'initialize':
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
    elif method == 'model/list':
        with open(log, 'a') as f: f.write('page\n')
        cursor = msg['params'].get('cursor')
        result = {'data': []} if cursor else {'data': [], 'nextCursor': 'next'}
        print(json.dumps({'id': msg['id'], 'result': result}), flush=True)
"#,
    )
    .unwrap();
    let server = AppServer::new(vec![
        common::python(),
        script.to_string_lossy().into_owned(),
        log.to_string_lossy().into_owned(),
    ]);
    assert_eq!(
        server.resolve_model("sol").unwrap_err().kind,
        "model_not_found"
    );
    server.list_models().unwrap();
    server.close();
    let entries = std::fs::read_to_string(log).unwrap();
    assert_eq!(entries.lines().filter(|line| *line == "start").count(), 1);
    assert_eq!(entries.lines().filter(|line| *line == "page").count(), 6);
}

#[test]
fn crashed_server_is_restarted_for_next_request() {
    let dir = common::temp();
    let script = dir.path().join("server.py");
    let marker = dir.path().join("marker");
    std::fs::write(
        &script,
        r#"
import json, sys, os
marker = sys.argv[1]
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get('method') == 'initialize':
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
    elif msg.get('method') == 'model/list':
        if not os.path.exists(marker):
            open(marker, 'w').close()
            sys.exit(7)
        print(json.dumps({'id': msg['id'], 'result': {'data': [{'id': 'gpt-6-luna'}]}}), flush=True)
"#,
    )
    .unwrap();
    let server = AppServer::new(vec![
        common::python(),
        script.to_string_lossy().into_owned(),
        marker.to_string_lossy().into_owned(),
    ]);
    assert_eq!(server.list_models().unwrap_err().kind, "codex_exited");
    assert_eq!(server.resolve_model("luna").unwrap(), "gpt-6-luna");
}

#[test]
fn refuses_unsubscribed_server_requests_and_survives_invalid_utf8() {
    let dir = common::temp();
    let script = dir.path().join("server.py");
    std::fs::write(
        &script,
        r#"
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get('method')
    if method == 'initialize':
        sys.stdout.buffer.write(b'{"method":"noise","params":{"text":"\xff"}}\n')
        sys.stdout.buffer.flush()
        print(json.dumps({'id': 'server-request', 'method': 'approval', 'params': {}}), flush=True)
    elif msg.get('id') == 'server-request':
        assert msg['error']['code'] == -32601
        print(json.dumps({'id': 1, 'result': {}}), flush=True)
    elif method == 'model/list':
        print(json.dumps({'id': msg['id'], 'result': {'data': []}}), flush=True)
"#,
    )
    .unwrap();
    let server = AppServer::new(vec![
        common::python(),
        script.to_string_lossy().into_owned(),
    ]);
    assert!(server.list_models().unwrap().is_empty());
}

#[test]
fn handshake_failures_name_installed_and_last_tested_versions() {
    for (breakage, raw) in [
        ("--fake-break=start:exit", ""),
        ("--fake-break=initialize:exit", ""),
        (
            "--fake-break=initialize:no-result",
            "initialize: missing result",
        ),
        (
            "--fake-break=initialize:-32600:Invalid request: missing field `clientInfo`",
            "Invalid request: missing field `clientInfo`",
        ),
        (
            "--fake-break=initialize:-32601:Method not found",
            "Method not found",
        ),
        (
            "--fake-break=initialize:-32603:internal error",
            "internal error",
        ),
    ] {
        let server = AppServer::new(common::fake_codex(&[NEWER, breakage]));
        let failure = server.list_models().unwrap_err();
        assert_names_versions(&failure);
        assert!(failure.message.contains(raw), "{failure}");
        assert!(!server.alive(), "{breakage}");
    }
}

#[test]
fn handshake_that_never_answers_names_the_versions() {
    for breakage in [
        "--fake-break=initialize:silent",
        "--fake-break=initialize:not-json",
    ] {
        let server = AppServer::with_request_timeout(
            common::fake_codex(&[NEWER, breakage]),
            Some(Duration::from_secs(2)),
        );
        let failure = server.list_models().unwrap_err();
        assert_names_versions(&failure);
        assert!(
            failure.message.contains("initialize timed out"),
            "{failure}"
        );
    }
}

#[test]
fn replies_that_do_not_fit_the_protocol_name_the_versions() {
    for breakage in [
        "--fake-break=model/list:-32600:Invalid request: unknown variant `model/list`",
        "--fake-break=model/list:-32601:Method not found",
        "--fake-break=model/list:-32602:Invalid params",
        "--fake-break=model/list:-32700:Parse error",
        "--fake-break=model/list:no-result",
        "--fake-break=model/list:empty-result",
    ] {
        let server = AppServer::new(common::fake_codex(&[NEWER, breakage]));
        assert_names_versions(&server.list_models().unwrap_err());
    }
}

#[test]
fn ordinary_rpc_errors_are_not_called_protocol_mismatches() {
    for (breakage, raw) in [
        (
            "--fake-break=model/list:-32600:thread not found: x",
            "thread not found: x",
        ),
        (
            "--fake-break=model/list:-32603:internal error",
            "internal error",
        ),
    ] {
        let server = AppServer::new(common::fake_codex(&[NEWER, breakage]));
        let failure = server.list_models().unwrap_err();
        assert_eq!(failure.kind, "codex_rpc", "{failure}");
        assert!(failure.message.contains(raw), "{failure}");
        assert!(!failure.message.contains("999.1.2"), "{failure}");
    }
}

#[test]
fn raw_protocol_error_text_is_cut_short() {
    let long = format!(
        "--fake-break=model/list:-32600:Invalid request: unknown variant `x`, expected one of {}",
        "`thread/start`, ".repeat(300)
    );
    let server = AppServer::new(common::fake_codex(&[NEWER, &long]));
    let failure = server.list_models().unwrap_err();
    assert_names_versions(&failure);
    assert!(failure.message.contains("unknown variant `x`"), "{failure}");
    assert!(failure.message.len() < 600, "{}", failure.message.len());
}

#[test]
fn unknown_installed_version_is_said_plainly_alongside_last_tested() {
    for version in [
        &["--fake-version=codex-cli fake"][..],
        &["--fake-version-exit=3"][..],
    ] {
        let mut flags = vec!["--fake-break=initialize:exit"];
        flags.extend_from_slice(version);
        let failure = AppServer::new(common::fake_codex(&flags))
            .list_models()
            .unwrap_err();
        assert_eq!(failure.kind, "codex_protocol", "{failure}");
        assert!(
            failure.message.contains("installed version unknown"),
            "{failure}"
        );
        assert!(
            failure
                .message
                .contains(&TESTED_CODEX.last_tested.to_string()),
            "{failure}"
        );
    }
}

#[test]
fn request_timeout_mentions_the_version_only_outside_the_tested_range() {
    let tested = common::tested_codex_flag();
    for (flag, named) in [(NEWER, true), (tested.as_str(), false)] {
        let server = AppServer::with_request_timeout(
            common::fake_codex(&[flag, "--fake-break=model/list:silent"]),
            Some(Duration::from_secs(5)),
        );
        let failure = server.list_models().unwrap_err();
        assert_eq!(failure.kind, "timeout", "{failure}");
        assert_eq!(failure.message.contains("newer than"), named, "{failure}");
    }
}
