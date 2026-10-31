use crate::common;
use axiom::codex::{AppServer, RunRequest};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn request(cwd: &std::path::Path, brief: &str) -> RunRequest {
    RunRequest {
        brief: brief.into(),
        cwd: cwd.to_string_lossy().into_owned(),
        model: "gpt-6.1-terra".into(),
        sandbox: "workspace-write".into(),
        effort: Some("low".into()),
        output_schema: None,
        developer_instructions: None,
        timeout_s: 10,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    }
}
fn server() -> AppServer {
    AppServer::new(common::fake("fake_codex"))
}
#[test]
fn worker_cargo_targets_are_external_per_thread_and_do_not_widen_readonly_roles() {
    let dir=common::temp();let server=server();
    let mut req=request(dir.path(),"inspect config");
    let mut targets=Vec::new();
    for _ in 0..2 {
        let report=server.run(&req).unwrap();
        let echo:Value=serde_json::from_str(report["final_message"].as_str().unwrap()).unwrap();
        let config=&echo["thread_params"]["config"];
        let target=config["shell_environment_policy"]["set"]["CARGO_TARGET_DIR"].as_str().unwrap();
        let path=std::path::Path::new(target);
        assert!(path.is_absolute()&&path.is_dir()&&!path.starts_with(dir.path()));
        assert_eq!(config["sandbox_workspace_write"]["writable_roots"],json!([target]));
        targets.push(target.to_owned());
    }
    assert_ne!(targets[0],targets[1]);
    req.sandbox="read-only".into();
    let report=server.run(&req).unwrap();
    let echo:Value=serde_json::from_str(report["final_message"].as_str().unwrap()).unwrap();
    assert!(echo["thread_params"]["config"]["sandbox_workspace_write"].is_null());
}

#[test]
fn actual_effort_uses_server_settings_evidence() {
    let dir = common::temp();
    let server = server();
    for (brief, expected) in [
        ("EFFORT_NESTED", json!("high")),
        ("EFFORT_MISSING", Value::Null),
        ("EFFORT_NULL", Value::Null),
        ("EFFORT_INVALID", Value::Null),
        ("EFFORT_EMPTY", Value::Null),
        ("EFFORT_LEGACY", json!("medium")),
        ("EFFORT_ROUTING", Value::Null),
    ] {
        let report = server.run(&request(dir.path(), brief)).unwrap();
        assert_eq!(report["status"], "completed", "{brief}");
        assert_eq!(report["actual_effort"], expected, "{brief}");
    }
}

#[test]
fn stderr_is_not_exposed_in_initialization_or_crash_errors() {
    const SECRET: &str = "synthetic-stderr-secret-83a4";
    let dir = common::temp();
    let script = dir.path().join("stderr_server.py");
    std::fs::write(
        &script,
        r#"
import json, sys
mode, marker, secret = sys.argv[1:4]
for line in sys.stdin:
    msg = json.loads(line)
    method = msg.get('method')
    if method == 'initialize' and mode == 'crash':
        print(json.dumps({'id': msg['id'], 'result': {}}), flush=True)
    elif method == ('initialize' if mode == 'init' else 'model/list'):
        sys.stderr.buffer.write(b'\xff' + secret.encode() + b'x' * (256 * 1024))
        sys.stderr.flush()
        with open(marker, 'w') as f:
            f.write('written')
        sys.exit(7)
"#,
    )
    .unwrap();
    for mode in ["init", "crash"] {
        let marker = dir.path().join(mode);
        let server = AppServer::with_request_timeout(
            vec![
                common::python(),
                script.to_string_lossy().into_owned(),
                mode.into(),
                marker.to_string_lossy().into_owned(),
                SECRET.into(),
            ],
            Some(Duration::from_secs(5)),
        );
        let error = server.list_models().unwrap_err();
        assert_eq!(error.kind, "codex_exited", "{mode}: {error}");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "written");
        assert!(
            !error.to_json().to_string().contains(SECRET),
            "{mode}: {error}"
        );
    }
}

#[test]
fn completed_and_lean_configuration() {
    let dir = common::temp();
    let server = server();
    let mut req = request(dir.path(), "GOAL 寫一個模組");
    req.developer_instructions = Some("instructions".into());
    let r = server.run(&req).unwrap();
    assert_eq!(r["status"], "completed");
    assert!(r["error"].is_null());
    assert_eq!(
        r["commands"],
        json!([{"command":"pytest -q","status":"completed","exit_code":0}])
    );
    assert_eq!(
        r["file_changes"],
        json!([{"path":"src/mod.py","kind":"add","status":"completed"}])
    );
    assert_eq!(
        r["disabled"],
        json!({"sections":["apps","goals","skills_catalog","personality","web_search"],"plugins":["keepme@personal","panel@personal"],"mcp_servers":["node_repl"],"skills":["/skills/ordinary/SKILL.md","/skills/workflow/SKILL.md"]})
    );
    assert_eq!(r["usage"]["totalTokens"], 1240);
    assert_eq!(r["started_mcp_servers"], json!([]));
    let echo: Value = serde_json::from_str(r["final_message"].as_str().unwrap()).unwrap();
    assert_eq!(echo["brief"], "GOAL 寫一個模組");
    assert_eq!(echo["effort"], "low");
    let p = &echo["thread_params"];
    assert_eq!(p["ephemeral"], true);
    assert_eq!(p["sandbox"], "workspace-write");
    assert_eq!(p["approvalPolicy"], "never");
    assert_eq!(p["cwd"], dir.path().to_string_lossy().as_ref());
    assert_eq!(p["developerInstructions"], "instructions");
    assert_eq!(
        p["config"]["features"],
        json!({"apps":false,"goals":false,"plugins":false,"remote_plugin":false,"multi_agent":false,"multi_agent_v2":false,"view_image":true})
    );
    assert_eq!(p["config"]["personality"], "none");
    assert_eq!(echo["config"]["skills"]["max_context_tokens"], 1);
    assert_eq!(
        echo["config"]["skills"]["config"].as_array().unwrap().len(),
        2
    );
    assert!(p["baseInstructions"]
        .as_str()
        .unwrap()
        .contains("project AGENTS"));
    assert_eq!(r["model"], "gpt-6.1-terra");
    assert_eq!(echo["config"]["web_search"], "disabled");
    assert_eq!(echo["disabled_plugin_ids"], r["disabled"]["plugins"]);
    let mut req = request(dir.path(), "search");
    req.web_search = true;
    req.keep_plugins.push("keepme@personal".into());
    let kept = server.run(&req).unwrap();
    assert_eq!(kept["started_mcp_servers"], json!(["keepme"]));
    assert!(!kept["disabled"]["sections"]
        .as_array()
        .unwrap()
        .contains(&json!("web_search")));
    let echo: Value = serde_json::from_str(kept["final_message"].as_str().unwrap()).unwrap();
    assert!(echo["config"].get("web_search").is_none());
    assert!(echo["config"]["features"].get("plugins").is_none());
    assert_eq!(
        echo["config"]["skills"]["config"],
        json!([{"path":"/skills/ordinary/SKILL.md","enabled":false}])
    );
    req.lean = false;
    let inherited = server.run(&req).unwrap();
    assert!(inherited["disabled"].is_null());
    let echo: Value = serde_json::from_str(inherited["final_message"].as_str().unwrap()).unwrap();
    assert!(echo["thread_params"].get("baseInstructions").is_none());
    assert_eq!(
        echo["thread_params"]["config"]["features"],
        json!({"multi_agent":false,"multi_agent_v2":false,"view_image":true})
    );
    assert_eq!(
        inherited["started_mcp_servers"].as_array().unwrap().len(),
        4
    );
    assert_ne!(r["thread_id"], inherited["thread_id"]);
}

#[test]
fn official_integrations_are_opt_in_per_thread_not_removed_from_global_config() {
    // Synthetic installed IDs exercise the same mechanism for each official service;
    // this is not a live OAuth/provider check.
    for service in [
        "github",
        "google-drive",
        "gmail",
        "google-calendar",
        "documents",
        "spreadsheets",
        "browser",
    ] {
        let dir = common::temp();
        let config = dir.path().join("config.toml");
        let original = format!(
            "# official integration sentinel\n[plugins.\"{service}@official\"]\nenabled = true\n"
        );
        std::fs::write(&config, &original).unwrap();
        let fake = dir.path().join("official.py");
        let id = format!("{service}@official");
        std::fs::write(&fake, format!("from pathlib import Path\np={}\ns=Path(p).read_text(encoding='utf-8').replace('keepme@personal',{})\nexec(compile(s,p,'exec'))\n", serde_json::to_string(&common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy()).unwrap(), serde_json::to_string(&id).unwrap())).unwrap();
        let server = AppServer::new(vec![
            common::python(),
            "-I".into(),
            fake.to_string_lossy().into_owned(),
        ]);
        let mut req = request(dir.path(), "inspect official integration routing");
        let lean = server.run(&req).unwrap();
        assert_eq!(lean["started_mcp_servers"], json!([]));
        req.keep_plugins = vec![id.clone()];
        let kept = server.run(&req).unwrap();
        assert_eq!(kept["started_mcp_servers"], json!(["keepme"]));
        let echo: Value = serde_json::from_str(kept["final_message"].as_str().unwrap()).unwrap();
        assert!(!kept["disabled"]["plugins"]
            .as_array()
            .unwrap()
            .contains(&json!(id)));
        assert_eq!(echo["config"]["features"]["view_image"], true);
        assert_eq!(
            echo["config"]["skills"]["config"],
            json!([{"path":"/skills/ordinary/SKILL.md","enabled":false}])
        );
        req.lean = false;
        let inherited = server.run(&req).unwrap();
        assert!(inherited["disabled"].is_null());
        assert_eq!(
            inherited["started_mcp_servers"].as_array().unwrap().len(),
            4
        );
        assert_eq!(std::fs::read_to_string(config).unwrap(), original);
    }
}

#[test]
fn structured_output_and_errors() {
    let dir = common::temp();
    let server = server();
    let mut req = request(dir.path(), "structured");
    req.output_schema = Some(json!({"type":"object","required":["answer"]}));
    let good = server.run(&req).unwrap();
    assert_eq!(good["structured_output"], json!({"answer":"ok"}));
    assert!(good["final_message"].is_null());
    for brief in ["BADJSON", "WRONGSHAPE"] {
        req.brief = brief.into();
        let r = server.run(&req).unwrap();
        assert_eq!(r["status"], "failed");
        assert_eq!(r["error"]["kind"], "invalid_structured_output");
    }
    req.output_schema = None;
    req.brief = "LIMIT".into();
    let r = server.run(&req).unwrap();
    assert_eq!(r["status"], "failed");
    assert_eq!(r["error"]["kind"], "usageLimitExceeded");
    assert_eq!(r["error"]["user_action"], true);
    req.brief = "AUTH".into();
    assert_eq!(server.run(&req).unwrap()["error"]["user_action"], true);
    req.brief = "BADMODEL".into();
    assert_eq!(
        server.run(&req).unwrap()["error"]["kind"],
        "model_not_found"
    );
    assert!(server.run(&req).unwrap()["error"]
        .get("user_action")
        .is_none());
    req.brief = "APPROVAL".into();
    let r = server.run(&req).unwrap();
    assert_eq!(r["status"], "completed");
    assert_eq!(
        r["refused_requests"],
        json!(["item/commandExecution/requestApproval"])
    );
}

#[test]
fn timeout_crash_and_restart() {
    let dir = common::temp();
    let (server, log) = logged_server(dir.path());
    let mut req = request(dir.path(), "HANG");
    req.timeout_s = 1;
    let r = server.run(&req).unwrap();
    assert_eq!(r["status"], "timeout");
    assert_eq!(r["error"]["kind"], "timeout");
    assert!(entries(&log)
        .iter()
        .any(|e| e["method"] == "turn/interrupt" && e["params"]["turnId"] == r["turn_id"]));
    req.brief = "CRASH".into();
    req.timeout_s = 10;
    let r = server.run(&req).unwrap();
    assert_eq!(r["status"], "bridge_error");
    assert_eq!(r["error"]["kind"], "turn_unconfirmed");
    req.brief = "after crash".into();
    assert_eq!(server.run(&req).unwrap()["status"], "completed");
}

#[test]
fn simultaneous_turns_overlap() {
    let dir = common::temp();
    let (server, log) = logged_server(dir.path());
    let server = Arc::new(server);
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let server = server.clone();
            let req = request(dir.path(), "SLOW");
            std::thread::spawn(move || server.run(&req).unwrap())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap()["status"], "completed");
    }
    let events = entries(&log);
    let starts: Vec<_> = events
        .iter()
        .filter(|e| e["method"] == "turn/start")
        .collect();
    assert_eq!(starts.len(), 2);
    assert!(starts[1]["time"].as_f64().unwrap() - starts[0]["time"].as_f64().unwrap() < 2.0);
}

#[test]
fn rejects_invalid_arguments() {
    let dir = common::temp();
    let server = server();
    for field in 0..4 {
        let mut req = request(dir.path(), "test");
        match field {
            0 => req.cwd = "relative".into(),
            1 => req.sandbox = "danger-full-access".into(),
            2 => req.brief = "  ".into(),
            _ => req.model.clear(),
        };
        assert_eq!(server.run(&req).unwrap_err().kind, "invalid_argument");
    }
}

fn logged_server(dir: &std::path::Path) -> (AppServer, std::path::PathBuf) {
    let log = dir.join("methods.jsonl");
    let wrapper = dir.join("wrapper.py");
    std::fs::write(&wrapper, "import os, runpy, sys\nos.environ['FAKE_CODEX_LOG'] = sys.argv[1]\nrunpy.run_path(sys.argv[2], run_name='__main__')\n").unwrap();
    let fake = common::repo_root().join("tests/fakes/fake_codex.py");
    (
        AppServer::new(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            fake.to_string_lossy().into_owned(),
        ]),
        log,
    )
}
fn entries(log: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
#[test]
fn hanging_rpc_times_out() {
    let dir = common::temp();
    let server = AppServer::with_request_timeout(
        common::fake("fake_codex"),
        Some(Duration::from_millis(300)),
    );
    let start = Instant::now();
    assert_eq!(
        server
            .run(&request(dir.path(), "HANGREQ"))
            .unwrap_err()
            .kind,
        "turn_unconfirmed"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}
#[test]
fn cancel_all_interrupts_single_and_concurrent_turns() {
    for count in [1, 2] {
        let dir = common::temp();
        let (server, log) = logged_server(dir.path());
        let server = Arc::new(server);
        let handles: Vec<_> = (0..count)
            .map(|_| {
                let server = server.clone();
                let req = request(dir.path(), "HANG");
                std::thread::spawn(move || server.run(&req).unwrap())
            })
            .collect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while entries(&log)
            .iter()
            .filter(|e| e["method"] == "turn/start")
            .count()
            < count
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        // Allow turn/start responses to be registered before cancellation.
        std::thread::sleep(Duration::from_millis(100));
        let start = Instant::now();
        server.cancel_all();
        for handle in handles {
            let report = handle.join().unwrap();
            assert_eq!(report["status"], "interrupted");
            assert_eq!(report["error"]["message"], "turn interrupted");
            assert!(entries(&log)
                .iter()
                .any(|e| e["method"] == "turn/interrupt"
                    && e["params"]["turnId"] == report["turn_id"]));
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}

#[test]
fn refused_interrupt_is_unconfirmed() {
    let dir = common::temp();
    let server = server();
    let mut req = request(dir.path(), "HANG REFUSE_INTERRUPT");
    req.timeout_s = 1;
    let report = server.run(&req).unwrap();
    assert_eq!(report["status"], "bridge_error");
    assert_eq!(report["error"]["kind"], "turn_unconfirmed");
}

#[test]
fn actual_model_and_unconfirmed_interrupt() {
    let dir = common::temp();
    let server = server();
    let mut req = request(dir.path(), "normal");
    req.model = "requested-model".into();
    let result = server.run(&req).unwrap();
    assert_eq!(result["model"], "actual-model");
    assert_eq!(result["requested_model"], "requested-model");
    req.brief = "NO_CONFIRM".into();
    req.timeout_s = 1;
    let start = Instant::now();
    let result = server.run(&req).unwrap();
    assert_eq!(result["error"]["kind"], "turn_unconfirmed");
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(6));
    req.brief = "normal".into();
    assert_eq!(server.run(&req).unwrap()["status"], "completed");
}

#[test]
fn confirmed_late_completion_is_terminal_but_not_cancelled_or_timed_out_success() {
    let dir = common::temp();
    let server = server();
    let mut req = request(dir.path(), "LATE_COMPLETE");
    req.timeout_s = 1;
    let result = server.run(&req).unwrap();
    assert_eq!(result["terminal_status"], "completed");
    assert_eq!(result["status"], "timeout");
    assert_eq!(result["error"]["kind"], "timeout");
    let (server, log) = logged_server(dir.path());
    let cancellation = axiom::base::proc::Cancellation::default();
    let worker_cancel = cancellation.clone();
    req.timeout_s = 10;
    let worker = std::thread::spawn(move || worker_cancel.scope(|| server.run(&req).unwrap()));
    let until = Instant::now() + Duration::from_secs(5);
    while !entries(&log).iter().any(|e| e["method"] == "turn/start") {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    // Unlike cancelling an unacknowledged start, target a registered running turn.
    std::thread::sleep(Duration::from_millis(100));
    cancellation.cancel();
    let result = worker.join().unwrap();
    assert_eq!(result["terminal_status"], "completed");
    assert_eq!(result["status"], "interrupted");
    assert_eq!(result["error"]["kind"], "interrupted");
}

#[test]
fn cancellation_during_turn_start_reports_shared_close() {
    let dir = common::temp();
    let (server, log) = logged_server(dir.path());
    let server = Arc::new(server);
    let cancellation = axiom::base::proc::Cancellation::default();
    let worker_cancel = cancellation.clone();
    let worker_server = server.clone();
    let req = request(dir.path(), "HANGREQ");
    let worker = std::thread::spawn(move || worker_cancel.scope(|| worker_server.run(&req)));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entries(&log).iter().any(|e| e["method"] == "turn/start") {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.kind, "turn_unconfirmed");
    assert!(error.message.contains("concurrent calls"));
}

#[test]
fn rejects_unconfirmed_verifier_policy_before_turn() {
    let dir = common::temp();
    for mode in [
        "FAKE_WRONG_MODEL",
        "FAKE_WRONG_SANDBOX",
        "FAKE_WRONG_APPROVAL",
    ] {
        let mut req = request(dir.path(), "must not execute");
        req.model = "gpt-6-astra".into();
        req.sandbox = "read-only".into();
        req.developer_instructions = Some(mode.into());
        assert_eq!(
            server().run(&req).unwrap_err().kind,
            "thread_policy",
            "{mode}"
        );
    }
}
