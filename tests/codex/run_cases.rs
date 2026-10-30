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
        json!({"sections":["apps","goals","skills_catalog","personality","web_search"],"plugins":["keepme@personal","panel@personal"],"mcp_servers":["node_repl"]})
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
        json!({"apps":false,"goals":false,"multi_agent":false,"multi_agent_v2":false,"view_image":true})
    );
    assert_eq!(p["config"]["personality"], "none");
    assert_eq!(echo["config"]["skills"]["max_context_tokens"], 1);
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
    req.lean = false;
    let inherited = server.run(&req).unwrap();
    assert!(inherited["disabled"].is_null());
    let echo: Value = serde_json::from_str(inherited["final_message"].as_str().unwrap()).unwrap();
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
    assert_eq!(r["error"]["kind"], "codex_exited");
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
        "timeout"
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
