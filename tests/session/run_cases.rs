#[path = "../common/mod.rs"]
mod common;
use axiom::{
    goal,
    session::{run, Clock, Config},
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Cursor, Read},
    path::Path,
    sync::Mutex,
    time::Duration,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SESSION: &str = "12345678-1234-1234-1234-123456789abc";
struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvRestore {
    fn new() -> Self {
        Self(
            [
                "FAKE_CLAUDE_RECORD",
                "FAKE_CLAUDE_SCENARIO",
                "FAKE_CLAUDE_SESSION",
                "FAKE_CLAUDE_PIDS",
                "AXIOM_PYTHON",
                "AXIOM_HEADLESS",
                "CLAUDE_CODE_PROMPT_CACHE_TTL",
                "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL",
            ]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect(),
        )
    }
}
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            if let Some(v) = v {
                std::env::set_var(k, v);
            } else {
                std::env::remove_var(k);
            }
        }
    }
}

struct Delayed(bool);
impl Read for Delayed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(300));
        self.0 = true;
        let bytes = "補充中文\r\n".as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
fn execute(
    scenario: &str,
    input: Box<dyn BufRead + Send>,
    active: bool,
) -> (i32, String, Value, Vec<String>) {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let home = temp.path().join("home");
    let record = temp.path().join("record.json");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", scenario);
    std::env::remove_var("FAKE_CLAUDE_SESSION");
    std::env::set_var("AXIOM_PYTHON", "/x");
    std::env::set_var("AXIOM_HEADLESS", "0");
    std::env::set_var("CLAUDE_CODE_PROMPT_CACHE_TTL", "5m");
    std::env::set_var("CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL", "5m");
    if active {
        goal::open(&home, "12345678-1234-1234-1234-123456789abc", "目標", None).unwrap();
    }
    if scenario == "blocked" {
        goal::open(&home, SESSION, "目標", None).unwrap();
        goal::set_status(&home, SESSION, "blocked", Some("need input")).unwrap();
    }
    if scenario == "bad-goal" {
        std::fs::create_dir_all(home.join("goals")).unwrap();
        std::fs::write(home.join("goals").join(format!("{SESSION}.json")), "{").unwrap();
    }
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    let mut output = Vec::new();
    let code = run(
        &config,
        "wish",
        "幫 todo 加統計功能",
        temp.path(),
        None,
        input,
        &mut output,
    )
    .unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    let paths: Vec<_> = std::fs::read_dir(home.join("logs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    let lines = std::fs::read_to_string(&paths[0])
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        std::fs::canonicalize(record["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(temp.path()).unwrap()
    );
    assert_eq!(record["env"]["AXIOM_DATA_HOME"], home.to_str().unwrap());
    assert_eq!(record["env"]["AXIOM_HEADLESS"], "1");
    assert_eq!(record["env"]["AXIOM_BIN"], config.bin.to_str().unwrap());
    assert_eq!(record["env"]["CLAUDE_CODE_DISABLE_BACKGROUND_TASKS"], "1");
    assert!(record["env"]["AXIOM_PYTHON"].is_null());
    assert_eq!(record["env"]["CLAUDE_CODE_PROMPT_CACHE_TTL"], "1h");
    assert_eq!(record["env"]["CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL"], "1h");
    assert_eq!(
        record["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish 幫 todo 加統計功能"
    );
    if scenario == "continue" {
        let prompt = record["messages"][1]["message"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(prompt.starts_with(axiom::session::CONTINUE_TAG));
        let before = goal::load(&home, SESSION).unwrap().unwrap().amendments;
        axiom::hook::handle(
            &serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":SESSION, "prompt":prompt}),
            Some(&home),
            true,
        );
        assert_eq!(
            goal::load(&home, SESSION).unwrap().unwrap().amendments,
            before
        );
    }
    let mut record = record;
    record["log_path"] = paths[0].to_string_lossy().to_string().into();
    record["home"] = home.to_string_lossy().to_string().into();
    (code, String::from_utf8(output).unwrap(), record, lines)
}
fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}
#[test]
fn launch_progress_and_log() {
    let (code, out, record, lines) = execute("ok", empty(), false);
    assert_eq!(code, 0);
    for expected in [
        "> chronos:dispatcher: Dispatch wish",
        "- orch unit add u1",
        "- codex terra: todo.py defines stats(items).",
        "< chronos:dispatcher finished",
        "統計功能已完成。",
        "log:",
    ] {
        assert!(out.contains(expected), "{out}");
    }
    assert!(lines
        .iter()
        .any(|l| serde_json::from_str::<Value>(l).unwrap()["type"] == "result"));
    let name = Path::new(record["log_path"].as_str().unwrap())
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(regex::Regex::new(r"^\d{8}-\d{6}-wish\.jsonl$")
        .unwrap()
        .is_match(name));
    let argv = record["argv"].as_array().unwrap();
    let flag = |name: &str| argv.iter().position(|v| v == name).unwrap();
    assert_eq!(argv[flag("--model") + 1], "opus");
    assert_eq!(argv[flag("--input-format") + 1], "stream-json");
    assert!(!argv.iter().any(|v| v == "--max-turns"));
    assert_eq!(
        Path::new(argv[flag("--plugin-dir") + 1].as_str().unwrap()),
        Path::new(record["home"].as_str().unwrap())
            .join("engine")
            .join(format!(
                "{}-{}",
                env!("CARGO_PKG_VERSION"),
                axiom::engine::build_id()
            ))
    );
}
#[test]
fn provenance_leads_the_log_and_closes_the_output() {
    let (code, out, record, lines) = execute("ok", empty(), false);
    assert_eq!(code, 0);
    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(first["type"], "provenance");
    let block = &first["provenance"];
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(block["binary_version"], version);
    assert_eq!(block["binary_build"], "unknown");
    assert_eq!(block["engine_build"], axiom::engine::build_id());
    assert_eq!(block["plugin_version"], version);
    assert_eq!(block["plugin_rules"], "match");
    assert_eq!(block["claude_version"], "2.1.281");
    assert_eq!(block["git_commit"], "unknown");
    assert_eq!(block["git_tree"], "unknown");
    for line in &lines {
        serde_json::from_str::<Value>(line).unwrap();
    }
    assert_eq!(
        lines
            .iter()
            .filter(|l| serde_json::from_str::<Value>(l).unwrap()["type"] == "provenance")
            .count(),
        1
    );
    assert_eq!(
        serde_json::from_str::<Value>(&lines[1]).unwrap()["subtype"],
        "init"
    );
    let handed: Value = serde_json::from_str(record["provenance_env"].as_str().unwrap()).unwrap();
    assert_eq!(&handed, block);
    let shown = out
        .lines()
        .find_map(|l| l.strip_prefix("provenance: "))
        .unwrap();
    for part in [
        format!(
            "axiom {version} (build unknown, engine {})",
            block["engine_build"].as_str().unwrap()
        ),
        format!("plugin {version} (rules match)"),
        "claude 2.1.281".to_owned(),
        "git unknown (unknown)".to_owned(),
    ] {
        assert!(shown.contains(&part), "{shown}");
    }
}
#[test]
fn crashed_run_still_reports_what_produced_it() {
    let (code, out, _, lines) = execute("crash", empty(), false);
    assert_eq!(code, 3);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"type\":\"provenance\""));
    assert!(out.lines().any(|l| l.starts_with("provenance: axiom ")));
}
#[test]
fn error_exit() {
    assert_eq!(execute("error", empty(), false).0, 1);
}
#[test]
fn crash_reports_real_path() {
    let (code, out, _, _) = execute("crash", empty(), false);
    assert_eq!(code, 3);
    let log = out.lines().find_map(|l| l.strip_prefix("log: ")).unwrap();
    assert!(out.contains(&format!(
        "claude exited 3 without a result; raw events are in {log}"
    )));
}
#[test]
fn no_bridge_warning() {
    let (code, out, _, _) = execute("no-bridge", empty(), false);
    assert_eq!(code, 0);
    assert!(out
        .lines()
        .any(|l| l == "! the plugin:chronos:bridge MCP server is failed; run `axiom doctor`"));
}
#[test]
fn interjection() {
    let (code, out, record, _) =
        execute("interject", Box::new(BufReader::new(Delayed(false))), false);
    assert_eq!(code, 0);
    assert!(out.contains("補充中文"));
    assert_eq!(
        record["messages"][1]["message"]["content"][0]["text"],
        "補充中文"
    );
}
#[test]
fn continuation_and_last_summary() {
    let (code, out, record, _) = execute("continue", empty(), true);
    assert_eq!(code, 0);
    assert!(out.contains("done"));
    assert_eq!(out.matches("$0.50").count(), 1);
    assert!(!out.contains("$0.80"));
    assert!(record["messages"][1]["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("still active"));
}
#[test]
fn input_eof_before_result() {
    assert_eq!(execute("ok", empty(), false).0, 0);
}
#[test]
fn forwarding_skips_blank_lines() {
    let (code, _, record, _) = execute(
        "forward",
        Box::new(Cursor::new(b"\r\n\n  \nA\r\n\nB\n".to_vec())),
        false,
    );
    assert_eq!(code, 0);
    let messages = record["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["message"]["content"][0]["text"], "A");
    assert_eq!(messages[2]["message"]["content"][0]["text"], "B");
}
#[test]
fn blocked_goal() {
    let (code, out, _, _) = execute("blocked", empty(), false);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l == "goal blocked: need input"));
}
#[test]
fn invalid_goal_is_reported() {
    let (code, out, _, _) = execute("bad-goal", empty(), false);
    assert_eq!(code, 1);
    assert!(out.contains("goal error: invalid_json:"));
}

#[test]
fn existing_log_is_not_overwritten() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let home = temp.path().join("home");
    std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "ok");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    std::fs::create_dir_all(home.join("logs")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for second in (now - 60)..=(now + 60) {
        let stamp = axiom::session::log_stamp(second, axiom::session::local_offset_secs(second));
        std::fs::write(
            home.join("logs").join(format!("{stamp}-wish.jsonl")),
            "sentinel",
        )
        .unwrap();
    }
    let mut out = Vec::new();
    assert_eq!(
        run(
            &config,
            "wish",
            "test",
            temp.path(),
            None,
            empty(),
            &mut out
        )
        .unwrap(),
        0
    );
    let output = String::from_utf8(out).unwrap();
    let actual = Path::new(
        output
            .lines()
            .find_map(|l| l.strip_prefix("log: "))
            .unwrap(),
    );
    let name = actual.file_name().unwrap().to_str().unwrap();
    let stamp = name
        .strip_suffix("-wish-1.jsonl")
        .expect("collision must use suffix 1");
    assert_eq!(
        std::fs::read_to_string(home.join("logs").join(format!("{stamp}-wish.jsonl"))).unwrap(),
        "sentinel"
    );
}

#[test]
fn interrupt_kills_process_tree() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let pids = temp.path().join("pids");
    std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "interrupt");
    std::env::set_var("FAKE_CLAUDE_PIDS", &pids);
    let wait_path = pids.clone();
    let trigger = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !std::fs::read_to_string(&wait_path)
            .map(|s| s.lines().last() == Some("ready"))
            .unwrap_or(false)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            std::fs::read_to_string(&wait_path)
                .map(|s| s.lines().last() == Some("ready"))
                .unwrap_or(false),
            "fake did not signal readiness"
        );
        axiom::session::trigger_interrupt();
    });
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: temp.path().join("home"),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    goal::open(&config.home, SESSION, "test", None).unwrap();
    let mut out = Vec::new();
    let code = run(
        &config,
        "wish",
        "test",
        temp.path(),
        None,
        empty(),
        &mut out,
    )
    .unwrap();
    trigger.join().unwrap();
    assert_eq!(code, 130);
    let note = goal::load(&config.home, SESSION)
        .unwrap()
        .unwrap()
        .note
        .unwrap();
    assert_eq!(
        note,
        format!("interrupted by the user. Resume with: axiom wish --resume {SESSION}")
    );
    let output = String::from_utf8(out).unwrap();
    assert!(output
        .lines()
        .any(|line| line == format!("goal blocked: {note}")));
    let path = output
        .lines()
        .find_map(|l| l.strip_prefix("log: "))
        .unwrap();
    assert!(Path::new(path).is_file());
    let ids = std::fs::read_to_string(pids)
        .unwrap()
        .lines()
        .filter(|s| *s != "ready")
        .map(|s| s.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while ids.iter().any(|&id| axiom::base::proc::pid_alive(id))
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let states: Vec<_> = ids
        .iter()
        .map(|&id| (id, axiom::base::proc::pid_alive(id)))
        .collect();
    assert!(
        states.iter().all(|&(_, alive)| !alive),
        "process states (pid, alive): {states:?}"
    );
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
fn interrupted_and_resume() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let home = temp.path().join("home");
    let record = temp.path().join("record");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::remove_var("FAKE_CLAUDE_SESSION");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    for scenario in ["limit", "crash"] {
        goal::open(&home, SESSION, "original", temp.path().to_str()).unwrap();
        goal::interject(&home, SESSION, "amendment").unwrap();
        std::env::set_var("FAKE_CLAUDE_SCENARIO", scenario);
        let mut output = Vec::new();
        let code = run(
            &config,
            "wish",
            "original",
            temp.path(),
            None,
            empty(),
            &mut output,
        )
        .unwrap();
        assert_eq!(code, if scenario == "limit" { 1 } else { 3 });
        let g = goal::load(&home, SESSION).unwrap().unwrap();
        assert_eq!(g.status, "blocked");
        assert_eq!(g.reason.as_deref(), Some("interrupted"));
        let cause = if scenario == "limit" {
            "claude reported an error: You've hit your usage limit"
        } else {
            "claude exited 3 without a result"
        };
        let note = format!("{cause}. Resume with: axiom wish --resume {SESSION}");
        assert_eq!(g.note.as_deref(), Some(note.as_str()));
        let output = String::from_utf8(output).unwrap();
        assert!(output
            .lines()
            .any(|line| line == format!("goal blocked: {note}")));
        assert!(output
            .lines()
            .any(|line| line == format!("resume: axiom wish --resume {SESSION}")));
    }
    std::fs::remove_dir_all(home.join("logs")).unwrap();
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "resume");
    let mut resume_output = Vec::new();
    assert_eq!(
        axiom::session::resume(
            &config,
            SESSION,
            Some(17),
            None,
            empty(),
            &mut resume_output
        )
        .unwrap(),
        0
    );
    let output = String::from_utf8(resume_output).unwrap();
    let log = output
        .lines()
        .find_map(|line| line.strip_prefix("log: "))
        .unwrap();
    assert!(Path::new(log)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("-wish.jsonl"));
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    let i = argv.iter().position(|v| v == "--max-turns").unwrap();
    assert_eq!(argv[i + 1], "17");
    assert_eq!(
        std::fs::canonicalize(record["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(temp.path()).unwrap()
    );
    let prompt = record["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.starts_with(axiom::session::CONTINUE_TAG));
    axiom::hook::handle(
        &serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":SESSION, "prompt":prompt}),
        Some(&home),
        true,
    );
    let g = goal::load(&home, SESSION).unwrap().unwrap();
    assert_eq!(g.status, "done");
    assert_eq!(g.wish, "original");
    assert_eq!(g.amendments, ["amendment"]);
    std::fs::write(home.join("goals").join(format!("{SESSION}.json")), "{").unwrap();
    assert_eq!(
        axiom::session::resume(&config, SESSION, None, None, empty(), &mut Vec::new())
            .unwrap_err()
            .kind,
        "invalid_json"
    );
}

#[test]
fn resume_message_is_the_first_turn() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let home = temp.path().join("home");
    let record = temp.path().join("record");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "resume");
    std::env::remove_var("FAKE_CLAUDE_SESSION");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    goal::open(&home, SESSION, "original", temp.path().to_str()).unwrap();
    goal::interrupt(&home, SESSION, "claude exited 1 without a result").unwrap();
    let mut out = Vec::new();
    let message = "the binary is fixed; the denied action is now allowed";
    assert_eq!(
        axiom::session::resume(&config, SESSION, None, Some(message), empty(), &mut out).unwrap(),
        0
    );
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    let messages = record["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    let prompt = messages[0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(prompt, message);
    let g = goal::load(&home, SESSION).unwrap().unwrap();
    assert_eq!(g.status, "done");
    assert_eq!(g.reason, None);
    goal::open(&home, "other", "original", None).unwrap();
    axiom::hook::handle(
        &serde_json::json!({"hook_event_name":"UserPromptSubmit", "session_id":"other", "prompt":prompt}),
        Some(&home),
        true,
    )
    .unwrap();
    let g = goal::load(&home, "other").unwrap().unwrap();
    assert_eq!(g.amendments, [message]);
    assert_eq!(g.interjection.unwrap().prompt, message);
}

fn fake_clock(
    start: i64,
    on_sleep: impl Fn() + Send + Sync + 'static,
) -> (Clock, std::sync::Arc<Mutex<(i64, u32)>>) {
    let state = std::sync::Arc::new(Mutex::new((start, 0u32)));
    let (now_state, sleep_state) = (state.clone(), state.clone());
    let clock = Clock {
        now: Box::new(move || now_state.lock().unwrap().0),
        sleep: Box::new(move |d| {
            let mut s = sleep_state.lock().unwrap();
            s.0 += d.as_secs() as i64;
            s.1 += 1;
            on_sleep();
        }),
    };
    (clock, state)
}
fn local_two_am() -> (i64, i32) {
    let real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let offset = axiom::session::local_offset_secs(real);
    let day = (real + i64::from(offset)).div_euclid(86400) * 86400;
    (day + 2 * 3600 - i64::from(offset), offset)
}
fn limit_run(clock: Clock) -> (i32, String, Value, tempfile::TempDir) {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let temp = common::temp();
    let home = temp.path().join("home");
    let record = temp.path().join("record");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "limit-reset");
    std::env::remove_var("FAKE_CLAUDE_SESSION");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock,
    };
    goal::open(&home, SESSION, "original", temp.path().to_str()).unwrap();
    let mut out = Vec::new();
    let code = run(
        &config,
        "wish",
        "original",
        temp.path(),
        None,
        empty(),
        &mut out,
    )
    .unwrap();
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    (code, String::from_utf8(out).unwrap(), record, temp)
}
#[test]
fn usage_limit_waits_for_the_reset_and_resumes() {
    let (start, offset) = local_two_am();
    let (clock, state) = fake_clock(start, || ());
    let (code, out, record, temp) = limit_run(clock);
    let home = temp.path().join("home");
    assert_eq!(code, 0, "{out}");
    let expected = 2 * 3600 + 40 * 60 + axiom::session::LIMIT_MARGIN_SECS;
    let (now, sleeps) = *state.lock().unwrap();
    assert_eq!(now - start, expected);
    assert_eq!(sleeps, expected as u32);
    assert!(
        out.lines()
            .any(|l| l == "usage limit: continuing at 04:42 (in 2h42m); Ctrl-C to stop"),
        "{out}"
    );
    assert!(out.lines().any(|l| l.starts_with(
        "goal blocked: claude reported an error: You've hit your session limit · resets 4:40am"
    )));
    assert!(!out.contains("resume: axiom wish --resume"), "{out}");
    assert_eq!(out.matches("\nlog: ").count(), 2);
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    let prompt = record["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.starts_with(axiom::session::CONTINUE_TAG));
    assert_eq!(goal::load(&home, SESSION).unwrap().unwrap().status, "done");
    let stamp = axiom::session::log_stamp(start + expected, offset);
    assert!(home
        .join("logs")
        .join(format!("{stamp}-wish.jsonl"))
        .is_file());
}
#[test]
fn usage_limit_already_past_waits_only_the_margin() {
    let (two_am, _) = local_two_am();
    let start = two_am + 2 * 3600 + 50 * 60;
    let (clock, state) = fake_clock(start, || ());
    let (code, out, record, _temp) = limit_run(clock);
    assert_eq!(code, 0, "{out}");
    let (now, sleeps) = *state.lock().unwrap();
    assert_eq!(now - start, axiom::session::LIMIT_MARGIN_SECS);
    assert_eq!(i64::from(sleeps), axiom::session::LIMIT_MARGIN_SECS);
    assert!(
        out.contains("usage limit: continuing at 04:52 (in 0h02m)"),
        "{out}"
    );
    assert!(record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "--resume"));
}
#[test]
fn usage_limit_wait_stops_on_interrupt() {
    let (start, _) = local_two_am();
    let (clock, state) = fake_clock(start, axiom::session::trigger_interrupt);
    let (code, out, record, temp) = limit_run(clock);
    let home = temp.path().join("home");
    assert_eq!(code, 130, "{out}");
    assert_eq!(state.lock().unwrap().1, 1);
    assert!(out
        .lines()
        .any(|l| l == format!("interrupted; resume: axiom wish --resume {SESSION}")));
    assert!(!record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "--resume"));
    let g = goal::load(&home, SESSION).unwrap().unwrap();
    assert_eq!(g.status, "blocked");
    assert!(g
        .note
        .unwrap()
        .starts_with("claude reported an error: You've hit your session limit"));
    assert_eq!(std::fs::read_dir(home.join("logs")).unwrap().count(), 1);
}
#[test]
fn limit_reset_parsing() {
    use axiom::session::limit_reset;
    let offset: i32 = 8 * 3600;
    let day = 1_793_232_000; // 2026-10-29 00:00 UTC
    let local = |h: i64, m: i64| day + h * 3600 + m * 60 - i64::from(offset);
    let taipei = "You've hit your session limit · resets 4:40am (Asia/Taipei)";
    assert_eq!(limit_reset(taipei, local(2, 0), offset), Some(local(4, 40)));
    assert_eq!(
        limit_reset(taipei, local(4, 50), offset),
        Some(local(4, 40))
    );
    assert_eq!(
        limit_reset(taipei, local(5, 0), offset),
        Some(local(28, 40))
    );
    assert_eq!(
        limit_reset("limit reached, resets 12:30pm", local(1, 0), offset),
        Some(local(12, 30))
    );
    assert_eq!(
        limit_reset("limit · resets 12am", local(1, 0), offset),
        Some(local(24, 0))
    );
    assert_eq!(
        limit_reset("limit · resets 3pm", local(1, 0), offset),
        Some(local(15, 0))
    );
    assert_eq!(
        limit_reset("limit · resets at 16:05", local(1, 0), offset),
        Some(local(16, 5))
    );
    assert_eq!(
        limit_reset("limit · resets 23:59", day, 0),
        Some(day + 23 * 3600 + 59 * 60)
    );
    for message in [
        "You've hit your usage limit",
        "limit · resets 25:00",
        "limit · resets 13pm",
        "limit · resets 0am",
        "limit · resets 4:60am",
        "weekly limit · resets Nov 12, 4am",
        "the timer resets 4:40am",
    ] {
        assert_eq!(limit_reset(message, local(1, 0), offset), None, "{message}");
    }
}

#[test]
fn resume_rejects_missing_done_and_missing_cwd() {
    let temp = common::temp();
    let home = temp.path().join("home");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
        clock: Clock::system(),
    };
    let attempt =
        |id| axiom::session::resume(&config, id, None, None, empty(), &mut Vec::new()).unwrap_err();
    assert_eq!(attempt("missing").kind, "not_found");
    goal::open(&home, "done", "wish", None).unwrap();
    goal::set_status(&home, "done", "done", None).unwrap();
    let err = attempt("done");
    assert_eq!(err.kind, "invalid_argument");
    assert!(err.message.contains("is already done"));
    for cwd in [
        None,
        Some(temp.path().join("nonexistent").to_str().unwrap().to_owned()),
    ] {
        goal::open(&home, "no-cwd", "wish", cwd.as_deref()).unwrap();
        let err = attempt("no-cwd");
        assert_eq!(err.kind, "invalid_argument");
        assert!(err.message.contains("has no project directory"));
    }
}
