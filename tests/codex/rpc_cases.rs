use crate::common;
use axiom::codex::{role_matches, AppServer};

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
