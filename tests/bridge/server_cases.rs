#[path = "../common/mod.rs"]
mod common;
use axiom::bridge::{call, serve, tools, Bridge, Config};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Cursor, Write},
    sync::{Arc, Mutex},
};

fn config(home: &std::path::Path) -> Config {
    Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    }
}
#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn invoke(b: &Bridge, name: &str, args: Value) -> Value {
    call(b, name, &args)
}
fn check(r: &Value, status: &str) {
    assert_eq!(r["structuredContent"]["status"], status, "{r}");
    assert_eq!(r["isError"], status != "completed", "{r}");
    assert_eq!(
        serde_json::from_str::<Value>(r["content"][0]["text"].as_str().unwrap()).unwrap(),
        r["structuredContent"]
    );
    if status != "completed" {
        assert!(r["structuredContent"]["error"].is_object(), "{r}");
    }
}
fn fixture(home: &std::path::Path) {
    let store = home.join("orchestrate/demo");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("brief.md");
    fs::write(&brief, "GOAL test\nSCOPE file.txt\nVERIFY test\n").unwrap();
    fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tx\tverified\t\t\t\t{}\nu2\tx\tpending\t\t\t\t{}\nu3\tx\tbuilt\t\t\t\t{}\n", brief.display(), brief.display(), brief.display())).unwrap();
    fs::write(home.join("file.txt"), "content").unwrap();
}
#[test]
fn full_protocol_session() {
    let temp = common::temp();
    let out = Sink(Arc::new(Mutex::new(vec![])));
    let capture = out.0.clone();
    let input = concat!(
        "bad\n",
        "{\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"custom\"}}\n",
        "{\"method\":\"notifications/initialized\"}\n",
        "{\"id\":2,\"method\":\"tools/list\"}\n",
        "{\"id\":3,\"method\":\"ping\"}\n",
        "{\"id\":4,\"method\":\"unknown\"}\n",
        "{\"id\":5,\"method\":\"initialize\"}\n"
    );
    assert_eq!(serve(config(temp.path()), Cursor::new(input), out), 0);
    let bytes = capture.lock().unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0]["error"]["code"], -32700);
    assert_eq!(lines[1]["result"]["protocolVersion"], "custom");
    assert_eq!(lines[1]["result"]["serverInfo"]["name"], "chronos-bridge");
    assert_eq!(
        lines[2]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "codex_run",
            "orch",
            "run_on_linux",
            "memory_record",
            "memory_search",
            "memory_get",
            "memory_digest",
            "goal",
            "bridge_status"
        ]
    );
    assert_eq!(lines[3]["result"], json!({}));
    assert_eq!(lines[4]["error"]["code"], -32601);
    assert_eq!(lines[5]["result"]["protocolVersion"], "2025-06-18");
}
#[test]
fn descriptor_names_policy() {
    let t = tools();
    let descriptors = t.as_array().unwrap();
    let codex = descriptors
        .iter()
        .find(|v| v["name"] == "codex_run")
        .unwrap();
    let description = codex["description"].as_str().unwrap();
    let phrase = "${CLAUDE_PLUGIN_ROOT}/DISPATCH.md";
    assert!(description.contains(phrase));
    let stripped = description.replace(phrase, "");
    assert!(!stripped.contains(phrase));
    assert!(!descriptors.iter().any(|v| v["name"] == "gemini_preview"));
}
#[test]
fn memory_verified_round_trip_and_refusal() {
    let temp = common::temp();
    fixture(temp.path());
    let b = Bridge::new(config(temp.path()));
    let mut args = json!({"project":"demo","project_dir":temp.path(),"unit":"u1","title":"slugify","summary":"把標題轉成網址代稱","body":"slugify(text)","wish":"add slugify","files":["file.txt"]});
    let recorded = invoke(&b, "memory_record", args.clone());
    check(&recorded, "completed");
    let found = invoke(&b, "memory_search", json!({"query":"網址代稱"}));
    check(&found, "completed");
    assert_eq!(
        found["structuredContent"]["entries"][0]["id"],
        recorded["structuredContent"]["id"]
    );
    args["unit"] = json!("u2");
    let refused = invoke(&b, "memory_record", args);
    check(&refused, "bridge_error");
    assert_eq!(
        refused["structuredContent"]["error"]["kind"],
        "not_verified"
    );
    let absent = invoke(&b, "memory_search", json!({"query":"unmatchedword"}));
    assert_eq!(absent["structuredContent"]["entries"], json!([]));
}
#[test]
fn orch_status_and_failure() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    let init = invoke(&b, "orch", json!({"project":"demo","args":["init"]}));
    check(&init, "completed");
    let ok = invoke(&b, "orch", json!({"project":"demo","args":["status"]}));
    check(&ok, "completed");
    assert_eq!(
        std::path::Path::new(ok["structuredContent"]["store"].as_str().unwrap()),
        temp.path().join("orchestrate").join("demo")
    );
    let bad = invoke(
        &b,
        "orch",
        json!({"project":"demo","args":["not-a-command"]}),
    );
    check(&bad, "failed");
    assert_eq!(bad["structuredContent"]["error"]["kind"], "orch_failed");
}

#[test]
fn bridge_status_and_missing_command() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    let ok = invoke(&b, "bridge_status", json!({}));
    check(&ok, "completed");
    assert_eq!(
        ok["structuredContent"]["codex"]["version"],
        json!(["codex-cli fake"])
    );
    assert_eq!(ok["structuredContent"]["codex"]["exit_code"], 0);
    assert_eq!(
        ok["structuredContent"]["codex"]["version_check"]["verdict"],
        "fail"
    );
    assert_eq!(ok["structuredContent"]["codex_app_server_running"], false);
    assert!(ok["structuredContent"].get("agy").is_none());
    let mut c = config(temp.path());
    c.codex = Err(axiom::base::Failure::new("missing", "missing"));
    let missing = invoke(&Bridge::new(c), "bridge_status", json!({}));
    check(&missing, "completed");
    assert!(missing["structuredContent"].get("agy").is_none());
    assert_eq!(
        missing["structuredContent"]["codex"]["error"]["kind"],
        "missing"
    );
    let mut c = config(temp.path());
    c.codex = Ok(vec![temp
        .path()
        .join("no-such-codex")
        .to_string_lossy()
        .into_owned()]);
    let unstartable = invoke(&Bridge::new(c), "bridge_status", json!({}));
    check(&unstartable, "completed");
    assert_eq!(
        unstartable["structuredContent"]["codex"]["error"]["kind"],
        "probe_failed"
    );
}
#[test]
fn codex_run_failure_from_a_protocol_change_names_both_versions() {
    let temp = common::temp();
    let mut c = config(temp.path());
    c.codex = Ok(common::fake_codex(&[
        "--fake-version=codex-cli 999.1.2",
        "--fake-break=initialize:-32600:Invalid request: missing field `clientInfo`",
    ]));
    let result = invoke(
        &Bridge::new(c),
        "codex_run",
        json!({"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"hello"}),
    );
    check(&result, "bridge_error");
    let error = &result["structuredContent"]["error"];
    assert_eq!(error["kind"], "codex_protocol");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("installed codex-cli 999.1.2"), "{message}");
    assert!(
        message.contains(&axiom::codex::TESTED_CODEX.last_tested.to_string()),
        "{message}"
    );
    assert!(
        message.starts_with("Codex app-server protocol mismatch"),
        "{message}"
    );
}
#[test]
fn goal_operations_and_mode_lock() {
    let temp = common::temp();
    axiom::goal::open(temp.path(), "s", "wish", None).unwrap();
    let b = Bridge::new(config(temp.path()));
    for args in [
        json!({"session":"s"}),
        json!({"session":"s","mode":"change","status":"done"}),
    ] {
        let bad = invoke(&b, "goal", args);
        check(&bad, "bridge_error");
        assert_eq!(
            bad["structuredContent"]["error"]["kind"],
            "invalid_argument"
        );
    }
    let mode = invoke(&b, "goal", json!({"session":"s","mode":"change"}));
    check(&mode, "completed");
    assert_eq!(mode["structuredContent"]["goal"]["mode"], "change");
    let locked = invoke(&b, "goal", json!({"session":"s","mode":"review"}));
    check(&locked, "bridge_error");
    assert_eq!(locked["structuredContent"]["error"]["kind"], "mode_locked");
    axiom::goal::interject(temp.path(), "s", "stop now").unwrap();
    let stop = invoke(&b, "goal", json!({"session":"s","classify":"stop"}));
    check(&stop, "completed");
    assert_eq!(stop["structuredContent"]["goal"]["reason"], "user_stop");
    let status = invoke(
        &b,
        "goal",
        json!({"session":"s","status":"blocked","note":"waiting"}),
    );
    check(&status, "completed");
    assert_eq!(status["structuredContent"]["goal"]["status"], "blocked");
}
#[test]
fn failed_tools_are_errors() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    for (name, args) in [
        ("unknown", json!({})),
        ("memory_get", json!({"id":999})),
        ("run_on_linux", json!({"project_dir":"nope","args":[]})),
    ] {
        let r = invoke(&b, name, args);
        assert_ne!(r["structuredContent"]["status"], "completed");
        assert_eq!(r["isError"], true, "{r}");
        assert!(r["structuredContent"]["error"].is_object());
    }
}

#[test]
fn classify_stop_interrupts_inflight_codex() {
    use std::time::{Duration, Instant};
    let temp = common::temp();
    let log = temp.path().join("codex.log");
    let wrapper = temp.path().join("wrapper.py");
    fs::write(&wrapper,"import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nrunpy.run_path(sys.argv[2],run_name='__main__')\n").unwrap();
    let mut c = config(temp.path());
    c.codex = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        log.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    let b = Bridge::new(c);
    axiom::goal::open(temp.path(), "s", "wish", None).unwrap();
    let b = Arc::new(b);
    let worker = b.clone();
    let cwd = temp.path().to_path_buf();
    let handle = std::thread::spawn(move || {
        invoke(
            &worker,
            "codex_run",
            json!({"cwd":cwd,"role":"terra","sandbox":"read-only","brief":"SLOW"}),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fs::read_to_string(&log)
        .unwrap_or_default()
        .contains("turn/start")
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    axiom::goal::interject(temp.path(), "s", "stop").unwrap();
    let stopped = invoke(&b, "goal", json!({"session":"s","classify":"stop"}));
    check(&stopped, "completed");
    let result = handle.join().unwrap();
    assert_eq!(result["structuredContent"]["status"], "interrupted");
    assert!(fs::read_to_string(log).unwrap().contains("turn/interrupt"));
}
#[test]
fn serve_dispatches_tool_calls() {
    let temp = common::temp();
    let out = Sink(Arc::new(Mutex::new(vec![])));
    let capture = out.0.clone();
    let requests = format!(
        "{}\n{}\n{}\n",
        json!({"id":1,"method":"tools/call","params":{"name":"codex_run","arguments":{"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"hello"}}}),
        json!({"id":2,"method":"tools/call","params":{"name":"unknown","arguments":{}}}),
        json!({"method":"tools/call","params":{"name":"bridge_status"}})
    );
    assert_eq!(serve(config(temp.path()), Cursor::new(requests), out), 0);
    let output = capture.lock().unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    let success = lines.iter().find(|v| v["id"] == 1).unwrap();
    check(&success["result"], "completed");
    let failure = lines.iter().find(|v| v["id"] == 2).unwrap();
    check(&failure["result"], "bridge_error");
    assert_eq!(
        failure["result"]["structuredContent"]["error"]["kind"],
        "unknown_tool"
    );
}
#[test]
fn descriptors_expose_codex_unit_inputs() {
    let t = tools();
    let codex = t
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "codex_run")
        .unwrap();
    for key in ["project", "unit", "save_to", "stage"] {
        assert_eq!(codex["inputSchema"]["properties"][key]["type"], "string");
    }
    assert_eq!(codex["inputSchema"]["required"], json!(["cwd"]));
    let goal = t
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "goal")
        .unwrap();
    let description = goal["description"].as_str().unwrap();
    for phrase in ["classify", "mode"] {
        assert!(description.contains(phrase));
        let stripped = description.replace(phrase, "");
        assert!(!stripped.contains(phrase));
    }
}

#[test]
fn ping_precedes_slow_call_reply() {
    let temp = common::temp();
    let out = Sink(Arc::new(Mutex::new(vec![])));
    let capture = out.0.clone();
    let requests = format!(
        "{}\n{}\n",
        json!({"id":1,"method":"tools/call","params":{"name":"codex_run","arguments":{"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"SLOW"}}}),
        json!({"id":2,"method":"ping"})
    );
    assert_eq!(serve(config(temp.path()), Cursor::new(requests), out), 0);
    let bytes = capture.lock().unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
    assert_eq!(lines[0]["id"], 2);
    assert_eq!(lines[1]["id"], 1);
}
#[test]
fn serve_stops_codex_process() {
    let temp = common::temp();
    let pid = temp.path().join("codex.pid");
    let wrapper = temp.path().join("codex_pid.py");
    fs::write(&wrapper, "import os,runpy,sys\nopen(sys.argv[1], 'w').write(str(os.getpid()))\nrunpy.run_path(sys.argv[2],run_name='__main__')\n").unwrap();
    let mut c = config(temp.path());
    c.codex = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        pid.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    let request = format!(
        "{}\n",
        json!({"id":1,"method":"tools/call","params":{"name":"codex_run","arguments":{"cwd":temp.path(),"role":"terra","sandbox":"read-only","brief":"hello"}}})
    );
    assert_eq!(
        serve(c, Cursor::new(request), Sink(Arc::new(Mutex::new(vec![])))),
        0
    );
    let id = fs::read_to_string(pid).unwrap();
    let alive = if cfg!(windows) {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {id}"), "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{id}\""))
    } else {
        std::process::Command::new("kill")
            .args(["-0", &id])
            .output()
            .unwrap()
            .status
            .success()
    };
    assert!(!alive, "codex process {id} still running");
}

#[test]
fn goal_classify_amend_and_answer() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    for kind in ["amend", "answer"] {
        axiom::goal::open(temp.path(), kind, "wish", None).unwrap();
        let bad = invoke(&b, "goal", json!({"session":kind,"classify":kind}));
        check(&bad, "bridge_error");
        axiom::goal::interject(temp.path(), kind, "new words").unwrap();
        let result = invoke(&b, "goal", json!({"session":kind,"classify":kind}));
        check(&result, "completed");
        assert_eq!(result["structuredContent"]["goal"]["wish"], "wish");
        if kind == "amend" {
            assert_eq!(
                result["structuredContent"]["goal"]["amendments"][0],
                "new words"
            );
        } else {
            assert_eq!(result["structuredContent"]["goal"]["status"], "active");
        }
    }
}
#[test]
fn panic_in_tool_body_is_bridge_error() {
    let ok = axiom::bridge::call_body(|| Ok(json!({"status":"completed"})));
    check(&ok, "completed");
    let bad = axiom::bridge::call_body(|| panic!("test tool panic"));
    check(&bad, "bridge_error");
    assert_eq!(bad["structuredContent"]["error"]["kind"], "panic");
}

fn goal_call(b: &Bridge, mode: &str) -> Value {
    let result = invoke(b, "goal", json!({"session":"s","mode":mode}));
    check(&result, "completed");
    result["structuredContent"]["goal"].clone()
}
fn claude_client(b: &Bridge, version: &str) {
    b.note_client(&json!({"clientInfo":{"name":"claude-code","version":version}}));
}
fn unlock(home: &std::path::Path) {
    let file = home.join("goals/s.json");
    let mut stored: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    stored.as_object_mut().unwrap().remove("mode_locked");
    fs::write(file, stored.to_string()).unwrap();
}

#[test]
fn first_goal_call_records_the_run_once_with_the_handshake_version() {
    std::env::remove_var(axiom::provenance::ENV);
    let temp = common::temp();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    axiom::goal::open(temp.path(), "s", "wish", project.to_str()).unwrap();
    let b = Bridge::new(config(temp.path()));
    claude_client(&b, "2.1.311");
    let goal = goal_call(&b, "change");
    let entries = goal["provenance"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let block = &entries[0];
    assert_eq!(block["binary_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(block["engine_build"], axiom::engine::build_id());
    assert_eq!(block["claude_version"], "2.1.311");
    assert_eq!(block["codex_version"], "codex-cli fake");
    assert_eq!(block["plugin_version"], "unknown");
    assert_eq!(block["plugin_rules"], "unknown");
    assert_eq!(block["git_commit"], "unknown");
    assert_eq!(block["git_tree"], "unknown");
    assert_eq!(block["binary_build"].as_str().unwrap().len(), 12);
    unlock(temp.path());
    assert_eq!(
        goal_call(&b, "review")["provenance"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn only_claude_codes_handshake_is_believed_and_absence_stays_unknown() {
    std::env::remove_var(axiom::provenance::ENV);
    let temp = common::temp();
    axiom::goal::open(temp.path(), "s", "wish", None).unwrap();
    let b = Bridge::new(config(temp.path()));
    b.note_client(&json!({"clientInfo":{"name":"another-host","version":"9.9.9"}}));
    assert_eq!(
        goal_call(&b, "change")["provenance"][0]["claude_version"],
        "unknown"
    );
}

#[test]
fn a_new_run_on_a_goal_adds_a_record_only_when_it_differs() {
    std::env::remove_var(axiom::provenance::ENV);
    let temp = common::temp();
    axiom::goal::open(temp.path(), "s", "wish", None).unwrap();
    let first = Bridge::new(config(temp.path()));
    claude_client(&first, "2.1.311");
    goal_call(&first, "change");
    unlock(temp.path());
    let resumed_same = Bridge::new(config(temp.path()));
    claude_client(&resumed_same, "2.1.311");
    assert_eq!(
        goal_call(&resumed_same, "change")["provenance"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    unlock(temp.path());
    let resumed_newer = Bridge::new(config(temp.path()));
    claude_client(&resumed_newer, "2.1.320");
    let goal = goal_call(&resumed_newer, "change");
    let entries = goal["provenance"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["claude_version"], "2.1.311");
    assert_eq!(entries[1]["claude_version"], "2.1.320");
}

#[test]
fn a_second_wish_in_one_session_gets_its_own_record() {
    std::env::remove_var(axiom::provenance::ENV);
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    claude_client(&b, "2.1.311");
    axiom::goal::open(temp.path(), "s", "first", None).unwrap();
    goal_call(&b, "change");
    axiom::goal::open(temp.path(), "s", "second", None).unwrap();
    assert_eq!(
        goal_call(&b, "change")["provenance"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn goal_files_without_provenance_stay_readable_and_keep_their_shape() {
    let temp = common::temp();
    let goals = temp.path().join("goals");
    fs::create_dir_all(&goals).unwrap();
    fs::write(
        goals.join("s.json"),
        r#"{"wish":"old","status":"active","custom":1}"#,
    )
    .unwrap();
    let loaded = axiom::goal::load(temp.path(), "s").unwrap().unwrap();
    assert!(loaded.provenance.is_empty());
    axiom::goal::mark_asked(temp.path(), "s").unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(goals.join("s.json")).unwrap()).unwrap();
    assert!(saved.get("provenance").is_none());
    assert_eq!(saved["custom"], 1);
}
