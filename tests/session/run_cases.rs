#[path = "../common/mod.rs"]
mod common;
use axiom::{
    goal,
    session::{run, Config},
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
    assert!(record["env"]["AXIOM_PYTHON"].is_null());
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
            .join(env!("CARGO_PKG_VERSION"))
    );
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
        while !wait_path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait_path.exists(), "fake did not start");
        axiom::session::trigger_interrupt();
    });
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: temp.path().join("home"),
        bin: temp.path().join("axiom"),
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
        .split_whitespace()
        .map(|s| s.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    for id in ids {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while axiom::base::proc::pid_alive(id) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !axiom::base::proc::pid_alive(id),
            "process {id} remains alive"
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
        axiom::session::resume(&config, SESSION, Some(17), empty(), &mut resume_output).unwrap(),
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
        axiom::session::resume(&config, SESSION, None, empty(), &mut Vec::new())
            .unwrap_err()
            .kind,
        "invalid_json"
    );
}

#[test]
fn resume_rejects_missing_done_and_missing_cwd() {
    let temp = common::temp();
    let home = temp.path().join("home");
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: temp.path().join("axiom"),
    };
    let attempt =
        |id| axiom::session::resume(&config, id, None, empty(), &mut Vec::new()).unwrap_err();
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
