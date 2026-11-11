use crate::common;
use axiom::{
    base::Failure,
    codex::{role_matches, AppServer, TESTED_CODEX},
};
use std::{sync::mpsc::RecvTimeoutError, time::Duration};

const NEWER: &str = "--fake-version=codex-cli 999.1.2";

fn account_update_server(dir: &std::path::Path, mode: &str) -> AppServer {
    let wrapper = dir.join("account_wrapper.py");
    std::fs::write(
        &wrapper,
        "import os, runpy, sys\nos.environ['FAKE_CODEX_ACCOUNT_UPDATE'] = sys.argv[1]\nrunpy.run_path(sys.argv[2], run_name='__main__')\n",
    )
    .unwrap();
    AppServer::new(vec![
        common::python(),
        "-I".into(),
        wrapper.to_string_lossy().into_owned(),
        mode.into(),
        common::fake("fake_codex")[2].clone(),
    ])
}

#[test]
fn rate_limits_returns_the_entire_result() {
    let server = AppServer::new(common::fake("fake_codex"));
    let limits = serde_json::json!({
        "limitId": "codex",
        "primary": {"usedPercent": 38, "windowDurationMins": 300, "resetsAt": 1900000000},
        "secondary": {"usedPercent": 59, "windowDurationMins": 10080, "resetsAt": 1900500000}
    });
    assert_eq!(
        server.read_rate_limits().unwrap(),
        serde_json::json!({
            "rateLimits": limits, "rateLimitsByLimitId": {"codex": limits}
        })
    );
}

#[test]
fn account_events_include_notifications_without_thread_id_only_for_account_methods() {
    let dir = common::temp();
    let server = account_update_server(dir.path(), "two");
    let events = server.account_events();
    assert!(!server.alive());
    assert_eq!(server.generation(), 0);
    server.read_rate_limits().unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        serde_json::json!({
            "method": "account/other", "params": {}
        })
    );
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        serde_json::json!({
            "method": "account/rateLimits/updated",
            "params": {"rateLimits": {"primary": {"usedPercent": 41, "windowDurationMins": 300, "resetsAt": 1900000000}}}
        })
    );
    assert_eq!(
        events.recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Timeout)
    );
}

#[test]
fn new_account_receiver_replaces_old_one() {
    let dir = common::temp();
    let server = account_update_server(dir.path(), "one");
    let old = server.account_events();
    let new = server.account_events();
    assert_eq!(
        old.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Disconnected)
    );
    server.read_rate_limits().unwrap();
    assert_eq!(
        new.recv_timeout(Duration::from_secs(5)).unwrap()["method"],
        "account/rateLimits/updated"
    );
}

#[test]
fn rate_limits_uses_null_params_and_routes_account_events_with_thread_id() {
    let dir = common::temp();
    let script = dir.path().join("account_server.py");
    std::fs::write(
        &script,
        r#"
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get('method') == 'initialize':
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
    elif msg.get('method') == 'account/rateLimits/read':
        assert 'params' in msg and msg['params'] is None
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
        print(json.dumps({'method': 'account/other', 'params': {'threadId': 't'}}), flush=True)
"#,
    )
    .unwrap();
    let server = AppServer::new(vec![
        common::python(),
        script.to_string_lossy().into_owned(),
    ]);
    let events = server.account_events();
    server.read_rate_limits().unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap(),
        serde_json::json!({"method": "account/other", "params": {"threadId": "t"}})
    );
}

#[test]
fn generation_counts_successful_starts() {
    let server = AppServer::new(common::fake("fake_codex"));
    assert_eq!(server.generation(), 0);
    server.read_rate_limits().unwrap();
    assert_eq!(server.generation(), 1);
    server.read_rate_limits().unwrap();
    assert_eq!(server.generation(), 1);
    server.close();
    assert_eq!(server.generation(), 1);
    server.read_rate_limits().unwrap();
    assert_eq!(server.generation(), 2);
    server.read_rate_limits().unwrap();
    assert_eq!(server.generation(), 2);

    let broken = AppServer::new(common::fake_codex(&["--fake-break=initialize:exit"]));
    assert!(broken.read_rate_limits().is_err());
    assert_eq!(broken.generation(), 0);
}

#[test]
fn account_receiver_survives_restart() {
    let dir = common::temp();
    let server = account_update_server(dir.path(), "one");
    let events = server.account_events();
    server.read_rate_limits().unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap()["method"],
        "account/rateLimits/updated"
    );
    server.close();
    server.read_rate_limits().unwrap();
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5)).unwrap()["method"],
        "account/rateLimits/updated"
    );
}

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
fn a_tested_version_is_not_blamed_for_a_protocol_error() {
    let tested = common::tested_codex_flag();
    let failure = AppServer::new(common::fake_codex(&[
        &tested,
        "--fake-break=initialize:-32601:Method not found",
    ]))
    .list_models()
    .unwrap_err();
    assert_eq!(failure.kind, "codex_protocol", "{failure}");
    assert!(
        failure
            .message
            .starts_with("Codex app-server protocol error"),
        "{failure}"
    );
    assert!(
        failure.message.contains(&format!(
            "installed codex-cli {}, last tested with Chronos {}",
            TESTED_CODEX.last_tested, TESTED_CODEX.last_tested
        )),
        "{failure}"
    );
    assert!(!failure.message.contains("mismatch"), "{failure}");
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
        if named {
            assert!(failure.message.contains("newer than"), "{failure}");
        } else {
            assert_eq!(failure.message, "model/list timed out");
        }
    }
}
