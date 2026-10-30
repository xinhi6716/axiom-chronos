#[path = "../common/mod.rs"]
mod common;
use axiom::{
    goal,
    session::{resume, run, Config},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

struct Harness {
    temp: tempfile::TempDir,
    config: Config,
    record: PathBuf,
}
impl Harness {
    fn new(scenario: &str) -> Self {
        let temp = common::temp();
        let home = temp.path().join("home");
        let record = temp.path().join("record.json");
        std::fs::write(temp.path().join("input.txt"), "readable source").unwrap();
        let mut cmd = common::fake("fake_codex_session");
        for (k, v) in [
            ("--scenario", scenario.to_owned()),
            ("--record", record.to_string_lossy().into_owned()),
            ("--home", home.to_string_lossy().into_owned()),
            ("--project", temp.path().to_string_lossy().into_owned()),
        ] {
            cmd.extend([k.into(), v]);
        }
        Self {
            config: Config {
                codex: Ok(cmd),
                home,
                bin: temp.path().join("axiom"),
            },
            record,
            temp,
        }
    }
    fn arg(&mut self, key: &str, value: &Path) {
        self.config
            .codex
            .as_mut()
            .unwrap()
            .extend([key.into(), value.to_string_lossy().into_owned()]);
    }
    fn execute(&self, input: Box<dyn BufRead + Send>, limit: Option<u32>) -> (i32, String) {
        let mut output = Vec::new();
        let code = run(
            &self.config,
            "wish",
            "幫 todo 加統計功能",
            self.temp.path(),
            limit,
            input,
            &mut output,
        )
        .unwrap();
        (code, String::from_utf8(output).unwrap())
    }
    fn records(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.record).unwrap()).unwrap()
    }
    fn requests(&self, method: &str) -> Vec<Value> {
        self.records()["requests"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == method)
            .map(|r| r["params"].clone())
            .collect()
    }
    fn goal(&self) -> goal::Goal {
        goal::load(&self.config.home, "thread-1").unwrap().unwrap()
    }
    fn logs(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.config.home.join("logs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }
}
fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}
struct Delayed(bool);
impl Read for Delayed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(350));
        self.0 = true;
        let bytes = "補充中文\r\n".as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
#[test]
fn launch_progress_log_eof_and_readonly_authority() {
    let h = Harness::new("ok");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("完成中文"));
    assert!(out.contains("codex tokens: gpt-6-astra 100 in (20 cached) / 10 out"));
    assert!(!out.contains('$'));
    assert_eq!(h.goal().status, "done");
    assert_eq!(h.goal().wish, "幫 todo 加統計功能");
    let start = &h.requests("thread/start")[0];
    assert_eq!(start["model"], "gpt-6-astra");
    assert_eq!(start["sandbox"], "read-only");
    assert_eq!(start["ephemeral"], false);
    assert_eq!(start["approvalPolicy"], "never");
    assert_eq!(start["config"]["plugins"]["fixture@test"]["enabled"], false);
    assert_eq!(start["config"]["mcp_servers"]["fixture"]["enabled"], false);
    let tools = start["dynamicTools"].as_array().unwrap();
    assert!(tools
        .iter()
        .all(|t| t["type"] == "function" && t["inputSchema"]["type"] == "object"));
    assert!(tools
        .iter()
        .all(|t| !t["name"].as_str().unwrap().starts_with("mcp__")));
    assert!(tools.iter().any(|t| t["name"] == "chronos_goal"));
    assert!(!tools
        .iter()
        .any(|t| t["name"].as_str().unwrap().ends_with("codex_run")));
    assert!(!start["config"].get("tools").is_some()); // No invented native-tool removal keys.
    let turn = &h.requests("turn/start")[0];
    assert_eq!(turn["threadId"], "thread-1");
    assert_eq!(turn["effort"], "max");
    assert_eq!(turn["disabledPluginIds"], json!(["fixture@test"]));
    assert_eq!(turn["input"][0]["text"], "/chronos:wish 幫 todo 加統計功能");
    let logs = h.logs();
    assert_eq!(logs.len(), 1);
    assert!(regex::Regex::new(r"^\d{8}-\d{6}-wish\.jsonl$")
        .unwrap()
        .is_match(logs[0].file_name().unwrap().to_str().unwrap()));
    let events: Vec<Value> = std::fs::read_to_string(&logs[0])
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(events.iter().any(|e| e["method"] == "turn/completed"));
    assert!(events
        .iter()
        .any(|e| e["method"] == "thread/tokenUsage/updated"));
    assert_eq!(h.requests("thread/start").len(), 1);
    // A forged session in the callback must not close a different root.
    assert!(goal::load(&h.config.home, "forged-root").unwrap().is_none());
}

#[test]
fn server_must_confirm_actual_role_model_and_readonly_sandbox() {
    for (scenario, kind) in [
        ("unsafe-sandbox", "permission_denied"),
        ("model-mismatch", "model_mismatch"),
    ] {
        let h = Harness::new(scenario);
        let error = run(
            &h.config,
            "wish",
            "test",
            h.temp.path(),
            None,
            empty(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(h.requests("turn/start").is_empty());
    }
}
#[test]
fn continuation_does_not_change_wish_or_amendments() {
    let h = Harness::new("continue");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["threadId"], turns[1]["threadId"]);
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    let prompt = turns[1]["input"][0]["text"].as_str().unwrap();
    assert!(prompt.starts_with(axiom::session::CONTINUE_TAG));
    assert!(prompt.contains("still active"));
    assert!(h.goal().amendments.is_empty());
    assert_eq!(out.matches("codex tokens:").count(), 1);
}
#[test]
fn interjection_updates_goal_and_steers_expected_live_turn() {
    let h = Harness::new("interject");
    let (code, out) = h.execute(Box::new(BufReader::new(Delayed(false))), None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("補充中文"));
    assert_eq!(h.goal().amendments, ["補充中文"]);
    assert!(h.goal().interjection.is_none());
    let steer = &h.requests("turn/steer")[0];
    assert_eq!(steer["threadId"], "thread-1");
    assert_eq!(steer["expectedTurnId"], "turn-1");
    assert_eq!(steer["input"][0]["text"], "補充中文");
    assert!(h.records()["responses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["result"]["success"] == false
            && r["result"]["contentItems"][0]["text"]
                .as_str()
                .unwrap_or("")
                .contains("pending_interjection")));
}

#[test]
fn completion_racing_steer_defers_without_losing_the_message() {
    let h = Harness::new("stale");
    let (code, out) = h.execute(Box::new(BufReader::new(Delayed(false))), Some(8));
    assert_eq!(code, 0, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert!(turns[1]["input"][0]["text"]
        .as_str()
        .unwrap()
        .contains("補充中文"));
    assert_eq!(h.goal().amendments, ["補充中文"]);
    assert_eq!(h.goal().status, "done");
}
#[test]
fn forwarding_skips_blank_lines() {
    let h = Harness::new("forward");
    let (code, out) = h.execute(
        Box::new(Cursor::new(b"\r\n\n  \nA\r\n\nB\n".to_vec())),
        None,
    );
    assert_eq!(code, 0, "{out}");
    let steers = h.requests("turn/steer");
    assert_eq!(steers.len(), 2);
    assert_eq!(steers[0]["input"][0]["text"], "A");
    assert_eq!(steers[1]["input"][0]["text"], "B");
}
#[test]
fn blocked_is_not_process_success() {
    let h = Harness::new("blocked");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1);
    assert!(out.contains("goal blocked: need input"));
    assert_eq!(h.goal().reason, None);
}
#[test]
fn capacity_crash_and_error_mark_interrupted_and_offer_resume() {
    for scenario in ["limit", "crash", "error"] {
        let h = Harness::new(scenario);
        let (code, out) = h.execute(empty(), None);
        assert_eq!(code, 1, "{scenario}: {out}");
        assert_eq!(h.goal().status, "blocked");
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("resume: axiom wish --resume thread-1"));
        assert!(h.logs()[0].is_file());
        if scenario == "limit" {
            assert!(out.contains("usage capacity limit"));
        }
        if scenario == "crash" {
            assert!(out.contains("codex_exited"));
        }
    }
}
#[test]
fn bounded_turns_including_zero() {
    for limit in [0, 2] {
        let h = Harness::new("forever");
        let (code, out) = h.execute(empty(), Some(limit));
        assert_eq!(code, 1);
        assert_eq!(h.requests("turn/start").len(), limit as usize);
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("maximum turns reached"));
    }
}
#[test]
fn corrupt_goal_is_reported_not_silently_successful() {
    let h = Harness::new("bad-goal");
    let (code, out) = h.execute(empty(), None);
    // Corruption must remain visible, even when an interrupted marker cannot be persisted.
    assert_eq!(code, 1);
    assert!(out.contains("invalid_json"));
}
#[test]
fn existing_log_is_not_overwritten() {
    let h = Harness::new("ok");
    std::fs::create_dir_all(h.config.home.join("logs")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for second in now - 60..=now + 60 {
        let stamp = axiom::session::log_stamp(second, axiom::session::local_offset_secs(second));
        std::fs::write(
            h.config
                .home
                .join("logs")
                .join(format!("{stamp}-wish.jsonl")),
            "sentinel",
        )
        .unwrap();
    }
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let log = Path::new(out.lines().find_map(|l| l.strip_prefix("log: ")).unwrap());
    let stamp = log
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_suffix("-wish-1.jsonl")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(
            h.config
                .home
                .join("logs")
                .join(format!("{stamp}-wish.jsonl"))
        )
        .unwrap(),
        "sentinel"
    );
}
#[test]
fn interrupt_kills_process_tree_and_preserves_resume() {
    let mut h = Harness::new("interrupt");
    let pids = h.temp.path().join("pids");
    h.arg("--pids", &pids);
    let wait = pids.clone();
    let trigger = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !wait.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait.exists());
        axiom::session::trigger_interrupt();
    });
    let (code, out) = h.execute(empty(), None);
    trigger.join().unwrap();
    assert_eq!(code, 130, "{out}");
    assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
    assert!(out.contains("interrupted by the user"));
    assert!(out.contains("resume:"));
    for id in std::fs::read_to_string(pids)
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse::<u32>().unwrap())
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while axiom::base::proc::pid_alive(id) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!axiom::base::proc::pid_alive(id), "process {id} survived");
    }
}
#[test]
fn resume_uses_actual_persistent_thread_and_original_goal() {
    let mut h = Harness::new("limit");
    assert_eq!(h.execute(empty(), None).0, 1);
    goal::interject(&h.config.home, "thread-1", "amendment").unwrap();
    goal::interrupt(&h.config.home, "thread-1", "test interruption").unwrap();
    let cmd = h.config.codex.as_mut().unwrap();
    let idx = cmd.iter().position(|s| s == "--scenario").unwrap();
    cmd[idx + 1] = "ok".into();
    let mut out = Vec::new();
    assert_eq!(
        resume(&h.config, "thread-1", Some(17), empty(), &mut out).unwrap(),
        0
    );
    assert_eq!(h.requests("thread/start").len(), 0);
    assert_eq!(h.requests("thread/resume")[0]["threadId"], "thread-1");
    assert_eq!(
        h.requests("thread/resume")[0]["config"]["features"]["multi_agent_v2"],
        false
    );
    assert!(h
        .requests("turn/start")
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert!(h.requests("turn/start")[0]["input"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with(axiom::session::CONTINUE_TAG));
    assert_eq!(h.goal().status, "done");
    assert_eq!(h.goal().wish, "幫 todo 加統計功能");
    assert_eq!(h.goal().amendments, ["amendment"]);
}
#[test]
fn resume_rejects_missing_done_corrupt_and_missing_cwd() {
    let h = Harness::new("ok");
    let attempt = |id| resume(&h.config, id, None, empty(), &mut Vec::new()).unwrap_err();
    assert_eq!(attempt("missing").kind, "not_found");
    goal::open(&h.config.home, "done", "wish", None).unwrap();
    goal::set_status(&h.config.home, "done", "done", None).unwrap();
    assert!(attempt("done").message.contains("already done"));
    for cwd in [
        None,
        Some(
            h.temp
                .path()
                .join("nonexistent")
                .to_string_lossy()
                .into_owned(),
        ),
    ] {
        goal::open(&h.config.home, "no-cwd", "wish", cwd.as_deref()).unwrap();
        assert!(attempt("no-cwd").message.contains("no project directory"));
    }
    std::fs::write(h.config.home.join("goals/no-cwd.json"), "{").unwrap();
    assert_eq!(attempt("no-cwd").kind, "invalid_json");
}
#[test]
fn core_dispatcher_and_actual_reviewer_models_have_distinct_threads() {
    let h = Harness::new("agents");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}");
    let threads = h.requests("thread/start");
    assert_eq!(threads.len(), 4, "{threads:?}");
    assert_eq!(
        threads
            .iter()
            .map(|p| p["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gpt-6-astra", "gpt-6-terra", "gpt-6-sol", "gpt-6-luna"]
    );
    for child in &threads[1..] {
        assert_eq!(child["ephemeral"], true);
        assert_eq!(child["sandbox"], "read-only");
    }
    let turns = h.requests("turn/start");
    for thread in &threads {
        assert_eq!(thread["config"]["features"]["multi_agent"], false);
        assert_eq!(thread["config"]["features"]["multi_agent_v2"], false);
        assert_eq!(thread["config"]["features"]["shell_tool"], false);
        assert_eq!(thread["config"]["features"]["unified_exec"], false);
        assert_eq!(thread["config"]["features"]["view_image"], true);
        let instructions = thread["developerInstructions"].as_str().unwrap();
        assert!(!instructions.starts_with("---"));
        assert!(!instructions.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(!instructions.contains("mcp__"));
    }
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert_eq!(turns[1]["effort"], "high");
    assert_eq!(turns[2]["effort"], "high");
    assert_eq!(turns[3]["effort"], "low");
    assert_eq!(turns[1]["input"][0]["text"], "dispatch directly");
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
}

#[test]
fn sol_writer_fallback_uses_astra_max_and_returns_actual_model_while_critic_stays_sol_high() {
    let h = Harness::new("fallback");
    let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
    std::fs::create_dir_all(store.join("reports")).unwrap();
    let brief = store.join("u.brief");
    std::fs::write(&brief, "GOAL x\nSCOPE input.txt\nVERIFY echo ok\n").unwrap();
    std::fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tbuilt\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    // Sol is historical, not latest; it still contributed to the reviewed unit.
    std::fs::write(store.join("reports/u.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6.1-sol\t0\told\t1\nrun\tgpt-6.1-terra\t0\tnew\t2\nverify\ttest\t0\tnew\t3\n").unwrap();
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}");
    let threads = h.requests("thread/start");
    assert_eq!(
        threads
            .iter()
            .map(|t| t["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gpt-6-astra", "gpt-6-terra", "gpt-6-astra", "gpt-6-sol"]
    );
    let turns = h.requests("turn/start");
    assert_eq!(turns[2]["effort"], "max");
    assert_eq!(turns[3]["effort"], "high");
    let records = h.records();
    let reports = records["responses"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| {
            r["result"]["contentItems"][0]["text"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
        })
        .collect::<Vec<_>>();
    let reviewer = reports
        .iter()
        .find(|r| r["thread_id"] == "thread-3")
        .unwrap();
    assert_eq!(reviewer["model"], "gpt-6-astra");
    assert_eq!(reviewer["status"], "completed");
    let states = axiom::orch::record_review(
        &store,
        &["u".into()],
        reviewer["model"].as_str().unwrap(),
        true,
        h.temp.path(),
    )
    .unwrap();
    assert_eq!(states[0].1, "verified");
    assert_eq!(
        axiom::orch::evidence(&store, "u")
            .unwrap()
            .last()
            .unwrap()
            .command,
        "gpt-6-astra"
    );
}
#[test]
fn independent_planner_thread_output_schema_saved_and_compressed() {
    let mut h = Harness::new("planner");
    let schema = h.temp.path().join("schema.json");
    std::fs::write(&schema,json!({"type":"object","properties":{"plan":{"type":"string"},"coverage":{"type":"object"}},"required":["plan","coverage"]}).to_string()).unwrap();
    let output = axiom::orch::store_for(&h.config.home, "demo")
        .unwrap()
        .join("research/plan.json");
    let second_cwd = h.temp.path().join("second-project");
    std::fs::create_dir(&second_cwd).unwrap();
    std::fs::write(second_cwd.join("input.txt"), "separate planner evidence").unwrap();
    h.arg("--planner-cwd", &second_cwd);
    h.arg("--schema", &schema);
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let threads = h.requests("thread/start");
    assert_eq!(threads.len(), 3);
    assert_eq!(Path::new(threads[2]["cwd"].as_str().unwrap()), second_cwd);
    assert!(!h.config.home.join("sessions/thread-1.planner").exists());
    assert_eq!(threads[0]["model"], threads[1]["model"]);
    assert_eq!(threads[1]["ephemeral"], true);
    assert_eq!(threads[2]["ephemeral"], true);
    assert!(h.requests("thread/resume").is_empty());
    assert_eq!(threads[1]["config"]["web_search"], "disabled");
    assert!(threads[2]["config"].get("web_search").is_none());
    assert!(!threads[1]["dynamicTools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "Agent" || t["name"].as_str().unwrap().ends_with("codex_run")));
    let turns = h.requests("turn/start");
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
    assert_ne!(turns[1]["threadId"], turns[2]["threadId"]);
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert!(turns[1]["outputSchema"].is_object());
    assert_eq!(turns[1]["effort"], "max");
    let saved: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(saved["plan"], "independent");
    let responses = h.records()["responses"].as_array().unwrap().clone();
    let reports: Vec<Value> = responses
        .iter()
        .filter_map(|r| r["result"]["contentItems"][0]["text"].as_str())
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|r| r.get("saved_to").is_some())
        .collect();
    assert_eq!(reports.len(), 2);
    for r in reports {
        assert!(r.get("final_message").is_none());
        assert!(r.get("structured_output").is_none());
        assert_eq!(r["model"], "gpt-6-astra");
        assert!(r["coverage"].is_object());
        assert!(r["usage"].is_object());
    }
}
#[test]
fn planner_rejects_invalid_schema_and_outside_store_destination() {
    for outside in [false, true] {
        let mut h = Harness::new("planner-invalid");
        let schema = h.temp.path().join("schema.json");
        std::fs::write(
            &schema,
            json!({"type":"object","required":["missing"]}).to_string(),
        )
        .unwrap();
        let output = if outside {
            h.temp.path().join("escape.json")
        } else {
            axiom::orch::store_for(&h.config.home, "demo")
                .unwrap()
                .join("bad.json")
        };
        h.arg("--schema", &schema);
        h.arg("--output", &output);
        assert_eq!(h.execute(empty(), None).0, 0);
        assert!(!output.exists());
        assert!(h.records()["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["result"]["success"] == false));
    }
}

#[test]
fn resume_keeps_research_bound_with_fresh_independent_planners() {
    let mut h = Harness::new("planner-limit");
    let schema = h.temp.path().join("schema.json");
    std::fs::write(
        &schema,
        json!({"type":"object","required":["plan"]}).to_string(),
    )
    .unwrap();
    let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
    let output = store.join("research/plan.json");
    h.arg("--schema", &schema);
    h.arg("--output", &output);
    let mut out = Vec::new();
    assert_eq!(
        run(
            &h.config,
            "research",
            "research question",
            h.temp.path(),
            Some(16),
            empty(),
            &mut out
        )
        .unwrap(),
        1
    );
    assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
    let ledger = std::fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap();
    assert!(ledger.contains("astra\tgpt-6-astra\t0\t"));
    let cmd = h.config.codex.as_mut().unwrap();
    let i = cmd.iter().position(|s| s == "--scenario").unwrap();
    cmd[i + 1] = "planner".into();
    assert_eq!(
        resume(&h.config, "thread-1", None, empty(), &mut Vec::new()).unwrap(),
        0
    );
    assert_eq!(h.requests("thread/start").len(), 2);
    let resumed = h.requests("thread/resume");
    assert_eq!(
        resumed
            .iter()
            .map(|p| p["threadId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["thread-1"]
    );
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(h.config.home.join("sessions/thread-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["skill"], "research");
    assert_eq!(metadata["max_turns"], 16);
    assert_eq!(h.goal().wish, "research question");
    let turns = h.requests("turn/start");
    assert!(turns[0]["input"][1]["text"]
        .as_str()
        .unwrap()
        .contains("# Research"));
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
}
#[test]
fn luna_content_has_reads_exact_outputs_and_no_arbitrary_nesting() {
    let mut h = Harness::new("authority");
    let output = h.temp.path().join("authorized.artifact");
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "explicit document artifact"
    );
    assert!(!PathBuf::from(format!("{}.other", output.display())).exists());
    assert!(!PathBuf::from(format!("{}.core", output.display())).exists());
    let threads = h.requests("thread/start");
    assert_eq!(threads[1]["model"], "gpt-6-luna");
    assert_eq!(threads[1]["sandbox"], "read-only");
    let tools = threads[1]["dynamicTools"].as_array().unwrap();
    for name in ["Read", "Glob", "Grep", "Write"] {
        assert!(tools.iter().any(|t| t["name"] == name));
    }
    assert!(!tools
        .iter()
        .any(|t| t["name"] == "Agent" || t["name"].as_str().unwrap().ends_with("codex_run")));
    let failures = h.records()["responses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["result"]["success"] == false)
        .count();
    assert!(
        failures >= 4,
        "expected core-write, core-codex, escaped content write and nesting denials"
    );
}
#[test]
fn classified_stop_cancels_root_and_outstanding_child_calls() {
    let h = Harness::new("cancel");
    let (code, out) = h.execute(Box::new(BufReader::new(Delayed(false))), None);
    assert_eq!(code, 1, "{out}");
    assert_eq!(h.goal().reason.as_deref(), Some("user_stop"));
    let interrupts = h.requests("turn/interrupt");
    assert!(interrupts.iter().any(|r| r["threadId"] == "thread-1"));
    assert!(interrupts.iter().any(|r| r["threadId"] == "thread-2"));
}

struct StopWhenVerifierRuns(PathBuf, bool);
impl Read for StopWhenVerifierRuns {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1 {
            return Ok(0);
        }
        wait_for_verifier(&self.0);
        self.1 = true;
        let bytes = b"stop\n";
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
fn wait_for_verifier(path: &Path) -> Vec<u32> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(ids) = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids| ids.len() == 2)
        {
            return ids;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "verifier never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn stop_and_ctrl_c_kill_verifier_tree_after_worker_model_completes() {
    for scenario in ["verify-stop", "verify-interrupt"] {
        let h = Harness::new(scenario);
        let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir(store.join("inbox")).unwrap();
        let pids = h.temp.path().join("verifier-pids");
        let script = h.temp.path().join("verifier.py");
        std::fs::write(&script, "import os,sys,time,subprocess\nfrom pathlib import Path\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])\nPath(sys.argv[1]).write_text(f'{os.getpid()} {p.pid}')\ntime.sleep(60)\n").unwrap();
        let brief = store.join("u.brief");
        std::fs::write(
            &brief,
            format!(
                "GOAL build\nSCOPE input.txt\nVERIFY \"{}\" \"{}\" \"{}\"\n",
                common::python(),
                script.display(),
                pids.display()
            ),
        )
        .unwrap();
        std::fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let trigger = if scenario == "verify-interrupt" {
            let pids = pids.clone();
            Some(std::thread::spawn(move || {
                wait_for_verifier(&pids);
                axiom::session::trigger_interrupt();
            }))
        } else {
            None
        };
        let input = if scenario == "verify-stop" {
            Box::new(BufReader::new(StopWhenVerifierRuns(pids.clone(), false)))
                as Box<dyn BufRead + Send>
        } else {
            empty()
        };
        let (code, out) = h.execute(input, None);
        if let Some(trigger) = trigger {
            trigger.join().unwrap();
        }
        assert_eq!(
            code,
            if scenario == "verify-stop" { 1 } else { 130 },
            "{out}"
        );
        assert_eq!(h.goal().status, "blocked");
        for pid in wait_for_verifier(&pids) {
            assert!(
                !axiom::base::proc::pid_alive(pid),
                "{scenario}: verifier {pid} survived"
            );
        }
        let report: Value =
            serde_json::from_slice(&std::fs::read(store.join("reports/u.json")).unwrap()).unwrap();
        assert_eq!(
            report["status"], "completed",
            "worker had finished before cancellation"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let evidence = axiom::orch::evidence(&store, "u").unwrap();
            if evidence
                .iter()
                .any(|r| r.kind == "verify" && r.exit_code != 0)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing cancelled VERIFY evidence"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            axiom::orch::read_units(&store).unwrap()[0].state,
            "verified"
        );
    }
}
#[test]
fn local_log_stamp() {
    assert_eq!(
        axiom::session::log_stamp(1793312048, 28800),
        "20261030-061408"
    );
    assert_eq!(axiom::session::log_stamp(1793312048, 0), "20261029-221408");
}

#[test]
fn tool_iterations_inside_one_turn_are_bounded_and_resumable() {
    for scenario in ["tool-loop", "native-loop"] {
        let h = Harness::new(scenario);
        let (code, out) = h.execute(empty(), Some(3));
        assert_eq!(code, 1, "{scenario}: {out}");
        assert_eq!(h.requests("turn/start").len(), 1);
        assert!(out.contains("maximum turns reached"), "{out}");
        assert!(h
            .requests("turn/interrupt")
            .iter()
            .any(|p| p["threadId"] == "thread-1"));
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("resume: axiom wish --resume thread-1"));
        if scenario == "tool-loop" {
            assert_eq!(
                h.records()["responses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["result"]["success"] == true)
                    .count(),
                2
            );
        }
    }
}
#[test]
fn workflow_templates_expand_at_load_without_rewriting_user_text() {
    let h = Harness::new("ok");
    let text = "verbatim $ARGUMENTS ${CLAUDE_PLUGIN_ROOT} mcp__keep_this";
    assert_eq!(
        run(
            &h.config,
            "wish",
            text,
            h.temp.path(),
            None,
            empty(),
            &mut Vec::new()
        )
        .unwrap(),
        0
    );
    let turns = h.requests("turn/start");
    let contract = turns[0]["input"][1]["text"].as_str().unwrap();
    assert!(contract.contains(&format!("Wish: {text}")));
    assert!(contract.contains(&format!(
        "{}/research/audit.schema.json",
        h.config
            .home
            .join("engine")
            .join(env!("CARGO_PKG_VERSION"))
            .display()
    )));
    assert!(!contract.contains("disable-model-invocation:"));
}
#[test]
fn dispatcher_can_delegate_only_root_authorized_content_outputs() {
    let mut h = Harness::new("delegated-authority");
    let output = h.temp.path().join("image-description.md");
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}"); // Explicit blocked end, not a forged engineering completion.
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "explicit document artifact"
    );
    assert!(!PathBuf::from(format!("{}.other", output.display())).exists());
    let threads = h.requests("thread/start");
    assert_eq!(threads.len(), 3);
    assert_eq!(threads[1]["model"], "gpt-6-terra");
    assert_eq!(threads[2]["model"], "gpt-6-luna");
    assert_eq!(threads[2]["config"]["features"]["view_image"], true);
    let log = std::fs::read_to_string(&h.logs()[0]).unwrap();
    assert!(log.contains("imageView"));
    assert!(log.contains("content output requires exact root authorization"));
}

#[test]
fn host_allows_planning_before_mode_but_not_project_writes_or_dispatch() {
    let mut h = Harness::new("planning-policy");
    h.config.codex.as_mut().unwrap()[2] = common::repo_root()
        .join("tests/session/planning_fake.py")
        .to_string_lossy()
        .into_owned();
    let output = h.temp.path().join("artifact.md");
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let read_results = |suffix: &str| -> Value {
        serde_json::from_slice(
            &std::fs::read(format!("{}.{}", h.record.display(), suffix)).unwrap(),
        )
        .unwrap()
    };
    let core = read_results("core");
    for key in [
        "inside",
        "architect",
        "content_before",
        "mode",
        "content_after",
    ] {
        assert_eq!(core[key]["success"], true, "{key}: {core}");
    }
    for key in ["outside", "dispatch"] {
        assert_eq!(core[key]["success"], false, "{key}: {core}");
    }
    let architect = read_results("thread-2");
    assert_eq!(architect["inside"]["success"], true);
    assert_eq!(architect["outside"]["success"], false);
    let before = read_results("thread-3");
    assert_eq!(before["output"]["success"], false);
    assert_eq!(before["other"]["success"], false);
    let after = read_results("thread-4");
    assert_eq!(after["output"]["success"], true);
    assert_eq!(after["other"]["success"], false);
    assert_eq!(
        std::fs::read_to_string(h.config.home.join("core-plan.md")).unwrap(),
        "core plan"
    );
    assert_eq!(
        std::fs::read_to_string(h.config.home.join("architect-plan.md")).unwrap(),
        "architect plan"
    );
    assert_eq!(std::fs::read_to_string(output).unwrap(), "content artifact");
    assert!(!h.temp.path().join("artifact.md.core").exists());
    assert!(!h.temp.path().join("artifact.md.architect").exists());
    assert!(!h.temp.path().join("artifact.md.other").exists());
    assert_eq!(h.goal().mode.as_deref(), Some("answer"));
}

#[test]
fn research_completion_is_plan_only_without_weakening_wish_gates() {
    for skill in ["research", "wish"] {
        let h = Harness::new("ok");
        let project = axiom::base::slug_for_dir(h.temp.path());
        // Pre-existing engineering work in this project is unrelated to a research plan.
        let store = axiom::orch::store_for(&h.config.home, &project).unwrap();
        assert_eq!(
            axiom::orch::run(&store, &["init".into()], false, false).exit_code,
            0
        );
        let brief = store.join("brief.md");
        std::fs::write(&brief, "GOAL pending\nSCOPE src/\nVERIFY cargo test\n").unwrap();
        assert_eq!(
            axiom::orch::run(
                &store,
                &[
                    "unit".into(),
                    "add".into(),
                    "pending".into(),
                    "--track".into(),
                    "build".into(),
                    "--brief".into(),
                    brief.to_string_lossy().into_owned()
                ],
                false,
                false
            )
            .exit_code,
            0
        );
        let mut output = Vec::new();
        let code = run(
            &h.config,
            skill,
            "question",
            h.temp.path(),
            Some(8),
            empty(),
            &mut output,
        )
        .unwrap();
        if skill == "research" {
            assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
            assert_eq!(h.goal().status, "done");
            assert_eq!(h.requests("turn/start").len(), 1);
            let turns = h.requests("turn/start");
            let workflow = turns[0]["input"][1]["text"].as_str().unwrap();
            assert!(workflow.contains("Declare goal mode answer"));
            assert!(workflow.contains("plan-only"));
        } else {
            assert_eq!(code, 1);
            assert_ne!(h.goal().status, "done");
            assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        }
    }
}
