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
        codex: Ok(common::fake("fake_codex_review")),
        agy: Ok(common::fake("fake_agy")),
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
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(home)
        .status()
        .unwrap()
        .success());
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
            "gemini_preview",
            "orch",
            "review_code",
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
    assert!(t[0]["description"]
        .as_str()
        .unwrap()
        .contains("${CLAUDE_PLUGIN_ROOT}/DISPATCH.md"));
    assert!(!t[1]["description"]
        .as_str()
        .unwrap()
        .contains("DISPATCH.md"));
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
fn preview_echo_and_bad_cwd() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    let ok = invoke(&b, "gemini_preview", json!({"prompt":"hi"}));
    check(&ok, "completed");
    assert_eq!(ok["structuredContent"]["response"], "echo: hi");
    let bad = invoke(&b, "gemini_preview", json!({"prompt":"hi","cwd":"nope"}));
    check(&bad, "bridge_error");
    assert_eq!(
        bad["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
    let failed = invoke(&b, "gemini_preview", json!({"prompt":"AGY_EMPTY"}));
    check(&failed, "failed");
}
#[test]
fn review_records_state() {
    let temp = common::temp();
    fixture(temp.path());
    let b = Bridge::new(config(temp.path()));
    fs::create_dir_all(temp.path().join("orchestrate/demo/reports")).unwrap();
    let identity = axiom::orch::snapshot(temp.path(), &["file.txt".into()]);
    assert!(!identity.is_empty());
    fs::write(temp.path().join("orchestrate/demo/reports/u3.evidence.tsv"), format!("kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6-terra\t0\t\t1\nverify\ttest\t0\t{identity}\t2\n")).unwrap();
    let args = json!({"project":"demo","units":["u3"],"project_dir":temp.path(),"paths":["file.txt"],"contract":"contract"});
    let ok = invoke(&b, "review_code", args.clone());
    check(&ok, "completed");
    assert_eq!(ok["structuredContent"]["verdict"], "pass");
    assert_eq!(ok["structuredContent"]["states"]["u3"], "verified");
    let bad = invoke(
        &b,
        "review_code",
        json!({"project_dir":temp.path(),"paths":["missing"],"contract":"contract"}),
    );
    check(&bad, "bridge_error");
    assert_eq!(
        bad["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
}
#[test]
fn bridge_status_and_missing_command() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    let ok = invoke(&b, "bridge_status", json!({}));
    check(&ok, "completed");
    assert_eq!(ok["structuredContent"]["agy"]["status"], json!("disabled"));
    assert_eq!(ok["structuredContent"]["codex_app_server_running"], false);
    let mut c = config(temp.path());
    c.agy = Err(axiom::base::Failure::new("missing", "missing"));
    let missing = invoke(&Bridge::new(c), "bridge_status", json!({}));
    check(&missing, "completed");
    assert_eq!(missing["structuredContent"]["agy"]["status"], "disabled");
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
    let requests=concat!("{\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"gemini_preview\",\"arguments\":{\"prompt\":\"hi\"}}}\n", "{\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"unknown\",\"arguments\":{}}}\n", "{\"method\":\"tools/call\",\"params\":{\"name\":\"bridge_status\"}}\n");
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
    assert_eq!(
        success["result"]["structuredContent"]["response"],
        "echo: hi"
    );
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
    for key in ["project", "unit", "save_to", "stage"] {
        assert_eq!(t[0]["inputSchema"]["properties"][key]["type"], "string");
    }
    assert_eq!(t[0]["inputSchema"]["required"], json!(["cwd"]));
    assert!(t[9]["description"].as_str().unwrap().contains("classify"));
    assert!(t[9]["description"].as_str().unwrap().contains("mode"));
}
#[test]
fn preview_stage_and_absolute_save() {
    let temp = common::temp();
    let marker = temp.path().join("agy-marker");
    let wrapper = temp.path().join("agy_wrapper.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_AGY_PID']=sys.argv[1]\nrunpy.run_path(sys.argv[2],run_name='__main__')\n").unwrap();
    let mut c = config(temp.path());
    c.agy = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        marker.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_agy.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    let b = Bridge::new(c);
    let store = temp.path().join("orchestrate/demo");
    let dest = store.join("astra/review.json");
    let ok = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","project":"demo","stage":"second_review","save_to":dest,"json_schema":{"type":"object"}}),
    );
    check(&ok, "completed");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(dest).unwrap()).unwrap()["answer"],
        "hi"
    );
    let failed = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"AGY_EMPTY","project":"demo","stage":"rewrite"}),
    );
    check(&failed, "failed");
    let rows = fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap();
    assert!(rows
        .lines()
        .any(|r| r.starts_with("second_review\t") && r.contains("\t0\t")));
    assert!(rows
        .lines()
        .any(|r| r.starts_with("rewrite\t") && r.contains("\t1\t")));
    assert!(!marker.exists());
    let bad = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","project":"demo","stage":"rewrite","save_to":temp.path().join("outside")}),
    );
    check(&bad, "bridge_error");
    assert!(!marker.exists());
    assert_eq!(
        bad["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
    assert_eq!(
        fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap(),
        rows
    );
}
#[test]
fn preview_stage_schema_and_independent_inputs() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    assert_eq!(
        tools()[1]["inputSchema"]["properties"]["stage"]["enum"],
        json!(["second_review", "rewrite"])
    );
    for args in [
        json!({"prompt":"hi","project":"demo"}),
        json!({"prompt":"hi","stage":"rewrite"}),
    ] {
        check(&invoke(&b, "gemini_preview", args), "completed");
    }
    assert!(!temp
        .path()
        .join("orchestrate/demo/reports/_stages.evidence.tsv")
        .exists());
    let bad = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","stage":"invalid"}),
    );
    assert_eq!(
        bad["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
}
#[test]
fn preview_save_requires_structured_output() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    let r = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","project":"demo","stage":"rewrite","save_to":temp.path().join("orchestrate/demo/out.json")}),
    );
    check(&r, "failed");
    assert_eq!(
        r["structuredContent"]["error"]["kind"],
        "invalid_structured_output"
    );
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
fn preview_run_error_records_failure() {
    let temp = common::temp();
    let mut c = config(temp.path());
    c.codex = Err(axiom::base::Failure::new("missing", "missing"));
    let r = invoke(
        &Bridge::new(c),
        "gemini_preview",
        json!({"prompt":"hi","project":"demo","stage":"rewrite"}),
    );
    assert_eq!(r["structuredContent"]["error"]["kind"], "missing");
    let rows = fs::read_to_string(
        temp.path()
            .join("orchestrate/demo/reports/_stages.evidence.tsv"),
    )
    .unwrap();
    assert!(rows
        .lines()
        .any(|r| r.starts_with("rewrite\t") && r.contains("\t1\t")));
}

#[test]
fn review_blocker_and_units_without_project() {
    let temp = common::temp();
    fixture(temp.path());
    let b = Bridge::new(config(temp.path()));
    let store = temp.path().join("orchestrate/demo");
    let before = fs::read(store.join("units.tsv")).unwrap();
    let no_project = invoke(
        &b,
        "review_code",
        json!({"units":["u3"],"project_dir":temp.path(),"paths":["file.txt"],"contract":"NEEDS_FIX"}),
    );
    check(&no_project, "completed");
    assert_eq!(no_project["structuredContent"]["verdict"], "fail");
    assert!(no_project["structuredContent"].get("states").is_none());
    assert_eq!(fs::read(store.join("units.tsv")).unwrap(), before);
    assert!(!store.join("reports/u3.evidence.tsv").exists());
    fs::create_dir_all(store.join("reports")).unwrap();
    let identity = axiom::orch::snapshot(temp.path(), &["file.txt".into()]);
    assert!(!identity.is_empty());
    fs::write(store.join("reports/u3.evidence.tsv"), format!("kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6-terra\t0\t\t1\nverify\ttest\t0\t{identity}\t2\n")).unwrap();
    let failed = invoke(
        &b,
        "review_code",
        json!({"project":"demo","units":["u3"],"project_dir":temp.path(),"paths":["file.txt"],"contract":"NEEDS_FIX"}),
    );
    check(&failed, "completed");
    assert_eq!(failed["structuredContent"]["verdict"], "fail");
    assert_eq!(failed["structuredContent"]["states"]["u3"], "review-failed");
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

#[test]
fn codex_review_models_and_schema() {
    let temp = common::temp();
    fixture(temp.path());
    let b = Bridge::new(config(temp.path()));
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(store.join("reports")).unwrap();
    let review = |contract: &str| {
        invoke(
            &b,
            "review_code",
            json!({"project":"demo","units":["u3"],"project_dir":temp.path(),"paths":["file.txt"],"contract":contract,"model":"gemini-old"}),
        )
    };
    let identity = axiom::orch::snapshot(temp.path(), &["file.txt".into()]);
    assert!(!identity.is_empty());
    fs::write(
        store.join("reports/u3.evidence.tsv"),
        format!("kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6-sol\t0\t\t1\nverify\ttest\t0\t{identity}\t2\n"),
    )
    .unwrap();
    let r = review("contract");
    check(&r, "completed");
    assert_eq!(r["structuredContent"]["reviewer"], "gpt-6-astra");
    assert!(fs::read_to_string(store.join("reports/u3.evidence.tsv"))
        .unwrap()
        .contains("review\tgpt-6-astra\t"));
    let invalid = review("EXTRA_FIELD");
    check(&invalid, "failed");
    assert_eq!(
        invalid["structuredContent"]["error"]["kind"],
        "no_review_output"
    );
    let preview = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","stage":"second_review","model":"gemini-old"}),
    );
    assert_eq!(preview["structuredContent"]["model"], "gpt-6-sol");
    assert!(preview["structuredContent"]["thread_id"].is_string());
    assert_eq!(
        preview["structuredContent"]["conversation_id"],
        preview["structuredContent"]["thread_id"]
    );
    assert!(preview["structuredContent"].get("response").is_some());
    assert!(preview["structuredContent"].get("error").is_some());
    let rewrite = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"hi","stage":"rewrite"}),
    );
    assert_eq!(rewrite["structuredContent"]["model"], "gpt-6-luna");
    assert_eq!(
        rewrite["structuredContent"]["conversation_id"],
        rewrite["structuredContent"]["thread_id"]
    );
    let rejected = invoke(
        &b,
        "review_code",
        json!({"project_dir":temp.path(),"paths":["file.txt"],"contract":"x","model":"gpt-6-luna"}),
    );
    assert_eq!(
        rejected["structuredContent"]["error"]["kind"],
        "invalid_argument"
    );
}

#[test]
fn review_rejects_invalid_output_and_unsafe_or_oversized_files() {
    let temp = common::temp();
    let b = Bridge::new(config(temp.path()));
    fs::write(temp.path().join("one"), vec![b'x'; 30_001]).unwrap();
    fs::write(temp.path().join("two"), vec![b'x'; 30_000]).unwrap();
    let invoke_review = |paths: Vec<&str>, contract: &str| {
        invoke(
            &b,
            "review_code",
            json!({"project_dir":temp.path(),"paths":paths,"contract":contract}),
        )
    };
    for contract in ["BADJSON", "WRONGSHAPE", "AGY_EMPTY"] {
        let r = invoke_review(vec!["one"], contract);
        check(&r, "failed");
    }
    let r = invoke_review(vec!["one", "two"], "contract");
    assert_eq!(r["structuredContent"]["error"]["kind"], "too_large");
    fs::write(temp.path().join("large"), vec![b'x'; 60_001]).unwrap();
    let r = invoke_review(vec!["large"], "contract");
    assert_eq!(r["structuredContent"]["error"]["kind"], "too_large");
    let r = invoke_review(vec!["../outside"], "contract");
    assert_eq!(r["structuredContent"]["error"]["kind"], "invalid_argument");
}

#[test]
fn review_effort_and_forbidden_processes() {
    let temp = common::temp();
    let log = temp.path().join("calls.jsonl");
    let forbidden = temp.path().join("forbidden");
    let wrapper = temp.path().join("codex.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nrunpy.run_path(sys.argv[2],run_name='__main__')\n").unwrap();
    let forbidden_cmd = temp.path().join("forbidden.py");
    fs::write(
        &forbidden_cmd,
        format!(
            "open({:?},'w').write('invoked')\n",
            forbidden.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let mut c = config(temp.path());
    c.codex = Ok(vec![
        common::python(),
        wrapper.to_string_lossy().into_owned(),
        log.to_string_lossy().into_owned(),
        common::repo_root()
            .join("tests/fakes/fake_codex_review.py")
            .to_string_lossy()
            .into_owned(),
    ]);
    c.agy = Ok(vec![
        common::python(),
        forbidden_cmd.to_string_lossy().into_owned(),
    ]);
    let b = Bridge::new(c);
    fixture(temp.path());
    let store = temp.path().join("orchestrate/demo");
    fs::create_dir_all(store.join("reports")).unwrap();
    fs::write(
        store.join("reports/u3.evidence.tsv"),
        "kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6-sol\t0\t\t1\nverify\ttest\t0\t\t2\n",
    )
    .unwrap();
    check(&invoke(&b, "bridge_status", json!({})), "completed");
    check(
        &invoke(
            &b,
            "gemini_preview",
            json!({"prompt":"hi","stage":"second_review"}),
        ),
        "completed",
    );
    check(
        &invoke(
            &b,
            "review_code",
            json!({"project":"demo","units":["u3"],"project_dir":temp.path(),"paths":["file.txt"],"contract":"x"}),
        ),
        "completed",
    );
    let rewrite = invoke(
        &b,
        "gemini_preview",
        json!({"prompt":"supplied text only","stage":"rewrite"}),
    );
    check(&rewrite, "completed");
    assert!(rewrite["structuredContent"]["usage"].is_object());
    assert!(!forbidden.exists());
    let events: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let models: Vec<_> = events
        .iter()
        .filter(|e| e["method"] == "thread/start")
        .map(|e| e["params"]["model"].as_str().unwrap())
        .collect();
    let efforts: Vec<_> = events
        .iter()
        .filter(|e| e["method"] == "turn/start")
        .map(|e| e["params"]["effort"].as_str().unwrap())
        .collect();
    assert_eq!(models, ["gpt-6-sol", "gpt-6-astra", "gpt-6-luna"]);
    assert_eq!(efforts, ["high", "max", "high"]);
    let starts: Vec<_> = events
        .iter()
        .filter(|e| e["method"] == "thread/start")
        .collect();
    for event in &starts {
        let instructions = event["params"]["developerInstructions"].as_str().unwrap();
        assert!(instructions.contains("Process only the text supplied"));
        assert!(instructions
            .contains("Do not search or inspect the filesystem, use tools, or edit files"));
        assert!(instructions.contains("JSON matching it"));
        assert_eq!(event["params"]["sandbox"], "read-only");
        assert_eq!(event["params"]["config"]["features"]["multi_agent"], false);
    }
    assert!(starts[2]["params"]["developerInstructions"]
        .as_str()
        .unwrap()
        .contains("report rewriter"));
}
