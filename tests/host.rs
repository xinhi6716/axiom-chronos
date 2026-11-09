mod common;
use axiom::{
    goal,
    host::{Event, Options, Wish},
    session::{Clock, Config, CONTINUE_TAG, LIMIT_MARGIN_SECS},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{mpsc::RecvTimeoutError, Arc, Mutex},
    time::{Duration, Instant},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SESSION: &str = "12345678-1234-1234-1234-123456789abc";
const KEYS: [&str; 4] = [
    "FAKE_CLAUDE_RECORD",
    "FAKE_CLAUDE_SCENARIO",
    "FAKE_CLAUDE_SESSION",
    "FAKE_CLAUDE_PIDS",
];

struct Setup {
    _lock: std::sync::MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    temp: tempfile::TempDir,
    home: PathBuf,
    record: PathBuf,
}
impl Setup {
    fn new(scenario: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = KEYS.into_iter().map(|k| (k, std::env::var_os(k))).collect();
        let temp = common::temp();
        let home = temp.path().join("home");
        let record = temp.path().join("record.json");
        std::env::set_var("FAKE_CLAUDE_RECORD", &record);
        std::env::set_var("FAKE_CLAUDE_SCENARIO", scenario);
        std::env::remove_var("FAKE_CLAUDE_SESSION");
        std::env::remove_var("FAKE_CLAUDE_PIDS");
        Self {
            _lock: lock,
            saved,
            temp,
            home,
            record,
        }
    }
    fn config(&self) -> Config {
        self.config_with(Clock::system())
    }
    fn config_with(&self, clock: Clock) -> Config {
        Config {
            claude: Ok(common::fake("fake_claude")),
            home: self.home.clone(),
            bin: self.temp.path().join("axiom"),
            clock,
        }
    }
    fn project(&self) -> &Path {
        self.temp.path()
    }
    fn record(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.record).unwrap()).unwrap()
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

fn next(wish: &Wish) -> Event {
    match wish.events().recv_timeout(Duration::from_secs(20)) {
        Ok(event) => event,
        Err(RecvTimeoutError::Timeout) => panic!("no event within 20s"),
        Err(RecvTimeoutError::Disconnected) => panic!("event stream closed without an exit"),
    }
}
fn collect(wish: &Wish) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let event = next(wish);
        let last = matches!(event, Event::Exited { .. } | Event::Failed(_));
        events.push(event);
        if last {
            return events;
        }
    }
}
fn wait_until(deadline: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + deadline;
    while !ready() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    ready()
}

#[test]
fn start_streams_structured_events() {
    let setup = Setup::new("ok");
    let wish = Wish::start(
        setup.config(),
        setup.project(),
        "幫 todo 加統計功能",
        &Options::default(),
    )
    .unwrap();
    let events = collect(&wish);
    assert_eq!(
        events,
        [
            Event::Started {
                session: SESSION.into()
            },
            Event::Text("開始處理".into()),
            Event::ToolUse {
                name: "Agent".into(),
                label: Some("> chronos:dispatcher: Dispatch wish".into()),
            },
            Event::ToolUse {
                name: "mcp__plugin_chronos_bridge__orch".into(),
                label: Some("- orch unit add u1".into()),
            },
            Event::ToolUse {
                name: "mcp__plugin_chronos_bridge__codex_run".into(),
                label: Some("- codex terra: todo.py defines stats(items).".into()),
            },
            Event::ToolUse {
                name: "Read".into(),
                label: None,
            },
            Event::ToolResult {
                name: "Agent".into(),
                summary: "done".into(),
            },
            Event::Result {
                text: "統計功能已完成。".into(),
                is_error: false,
            },
            Event::Exited { code: 0 },
        ]
    );
    assert_eq!(wish.wait().unwrap(), 0);
    let record = setup.record();
    assert_eq!(
        record["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish 幫 todo 加統計功能"
    );
    assert_eq!(
        std::fs::canonicalize(record["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(setup.project()).unwrap()
    );
    assert_eq!(record["env"]["AXIOM_HEADLESS"], "1");
    assert_eq!(
        record["env"]["AXIOM_DATA_HOME"],
        setup.home.to_str().unwrap()
    );
    assert!(!record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "--max-turns"));
    assert_eq!(
        std::fs::read_dir(setup.home.join("logs")).unwrap().count(),
        1
    );
}

#[test]
fn options_and_argument_checks() {
    let setup = Setup::new("ok");
    let options = Options {
        creative: true,
        max_turns: Some(7),
    };
    let wish = Wish::start(setup.config(), setup.project(), "  做點什麼 ", &options).unwrap();
    assert_eq!(wish.wait().unwrap(), 0);
    let record = setup.record();
    assert_eq!(
        record["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish [creative] 做點什麼"
    );
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--max-turns").unwrap();
    assert_eq!(argv[i + 1], "7");

    let wish = Wish::start(setup.config(), setup.project(), "", &options).unwrap();
    assert_eq!(wish.wait().unwrap(), 0);
    assert_eq!(
        setup.record()["messages"][0]["message"]["content"][0]["text"],
        "/chronos:wish [creative]"
    );

    let empty = Wish::start(setup.config(), setup.project(), " \n", &Options::default());
    assert_eq!(empty.unwrap_err().kind, "invalid_argument");
    let missing = Wish::start(
        setup.config(),
        &setup.project().join("nowhere"),
        "x",
        &Options::default(),
    );
    let err = missing.unwrap_err();
    assert_eq!(err.kind, "invalid_argument");
    assert!(err.message.contains("nowhere"), "{}", err.message);
    assert_eq!(
        Wish::start(setup.config(), setup.project(), "x", &Options::default())
            .unwrap()
            .events()
            .recv_timeout(Duration::from_secs(20))
            .unwrap(),
        Event::Started {
            session: SESSION.into()
        }
    );
}

#[test]
fn interjection_is_delivered_while_running() {
    let setup = Setup::new("interject");
    let wish = Wish::start(
        setup.config(),
        setup.project(),
        "等一下",
        &Options::default(),
    )
    .unwrap();
    assert_eq!(
        wish.interject(" \r\n").unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        next(&wish),
        Event::Started {
            session: SESSION.into()
        }
    );
    assert!(matches!(next(&wish), Event::ToolUse { ref name, .. } if name.ends_with("codex_run")));
    assert!(wish.is_running());
    wish.interject("補充中文\r\n").unwrap();
    assert_eq!(
        collect(&wish),
        [
            Event::ToolResult {
                name: "mcp__plugin_chronos_bridge__codex_run".into(),
                summary: "補充中文".into(),
            },
            Event::Result {
                text: "補充中文".into(),
                is_error: false,
            },
            Event::Exited { code: 0 },
        ]
    );
    assert_eq!(wish.interject("too late").unwrap_err().kind, "finished");
    assert_eq!(wish.wait().unwrap(), 0);
    let record = setup.record();
    assert_eq!(record["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        record["messages"][1]["message"]["content"][0]["text"],
        "補充中文"
    );
}

fn pids_ready(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.lines().last() == Some("ready"))
        .unwrap_or(false)
}
fn pids(path: &Path) -> Vec<u32> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|s| *s != "ready")
        .map(|s| s.parse().unwrap())
        .collect()
}
fn assert_all_dead(ids: &[u32]) {
    wait_until(Duration::from_secs(5), || {
        ids.iter().all(|&id| !axiom::base::proc::pid_alive(id))
    });
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
fn stop_kills_the_process_tree_and_blocks_the_goal() {
    let setup = Setup::new("interrupt");
    let pidfile = setup.temp.path().join("pids");
    std::env::set_var("FAKE_CLAUDE_PIDS", &pidfile);
    goal::open(&setup.home, SESSION, "test", None).unwrap();
    let wish = Wish::start(setup.config(), setup.project(), "test", &Options::default()).unwrap();
    assert_eq!(
        next(&wish),
        Event::Started {
            session: SESSION.into()
        }
    );
    assert!(
        wait_until(Duration::from_secs(15), || pids_ready(&pidfile)),
        "fake did not signal readiness"
    );
    let ids = pids(&pidfile);
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|&id| axiom::base::proc::pid_alive(id)));
    wish.stop();
    assert_eq!(collect(&wish), [Event::Exited { code: 130 }]);
    assert_eq!(wish.wait().unwrap(), 130);
    assert_all_dead(&ids);
    let g = goal::load(&setup.home, SESSION).unwrap().unwrap();
    assert_eq!(g.status, "blocked");
    assert_eq!(
        g.note.as_deref(),
        Some(
            format!("interrupted by the user. Resume with: axiom wish --resume {SESSION}").as_str()
        )
    );
}

#[test]
fn drop_stops_a_running_wish() {
    let setup = Setup::new("interrupt");
    let pidfile = setup.temp.path().join("pids");
    std::env::set_var("FAKE_CLAUDE_PIDS", &pidfile);
    let wish = Wish::start(setup.config(), setup.project(), "test", &Options::default()).unwrap();
    assert!(
        wait_until(Duration::from_secs(15), || pids_ready(&pidfile)),
        "fake did not signal readiness"
    );
    let ids = pids(&pidfile);
    drop(wish);
    assert_all_dead(&ids);
}

#[test]
fn resume_continues_a_blocked_goal() {
    let setup = Setup::new("resume");
    goal::open(&setup.home, SESSION, "original", setup.project().to_str()).unwrap();
    goal::set_status(&setup.home, SESSION, "blocked", Some("stopped")).unwrap();
    let wish = Wish::resume(setup.config(), SESSION, Some(17)).unwrap();
    assert_eq!(
        collect(&wish),
        [
            Event::Started {
                session: SESSION.into()
            },
            Event::Result {
                text: "done".into(),
                is_error: false,
            },
            Event::Exited { code: 0 },
        ]
    );
    assert_eq!(wish.wait().unwrap(), 0);
    let record = setup.record();
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    let i = argv.iter().position(|v| v == "--max-turns").unwrap();
    assert_eq!(argv[i + 1], "17");
    assert_eq!(
        std::fs::canonicalize(record["cwd"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(setup.project()).unwrap()
    );
    let prompt = record["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.starts_with(CONTINUE_TAG));
    assert!(prompt.contains("still active"));
    assert_eq!(
        goal::load(&setup.home, SESSION).unwrap().unwrap().status,
        "done"
    );

    let err = Wish::resume(setup.config(), SESSION, None).unwrap_err();
    assert_eq!(err.kind, "invalid_argument");
    assert!(err.message.contains("already done"));
    assert_eq!(
        Wish::resume(setup.config(), "missing", None)
            .unwrap_err()
            .kind,
        "not_found"
    );
}

const LIMIT_MESSAGE: &str = "You've hit your session limit · resets 4:40am (Asia/Taipei)";
fn local_two_am() -> i64 {
    let real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let offset = i64::from(axiom::session::local_offset_secs(real));
    (real + offset).div_euclid(86400) * 86400 + 2 * 3600 - offset
}

#[test]
fn usage_limit_waits_then_resumes_into_the_same_wish() {
    let setup = Setup::new("limit-reset");
    goal::open(&setup.home, SESSION, "original", setup.project().to_str()).unwrap();
    let start = local_two_am();
    let now = Arc::new(Mutex::new(start));
    let (reads, sleeps) = (Arc::clone(&now), Arc::clone(&now));
    let clock = Clock {
        now: Box::new(move || *reads.lock().unwrap()),
        sleep: Box::new(move |d| *sleeps.lock().unwrap() += d.as_secs() as i64),
    };
    let wish = Wish::start(
        setup.config_with(clock),
        setup.project(),
        "original",
        &Options::default(),
    )
    .unwrap();
    let until = start + 2 * 3600 + 40 * 60 + LIMIT_MARGIN_SECS;
    assert_eq!(
        collect(&wish),
        [
            Event::Started {
                session: SESSION.into()
            },
            Event::Result {
                text: LIMIT_MESSAGE.into(),
                is_error: true,
            },
            Event::Waiting { until },
            Event::Started {
                session: SESSION.into()
            },
            Event::Result {
                text: "done".into(),
                is_error: false,
            },
            Event::Exited { code: 0 },
        ]
    );
    assert_eq!(wish.wait().unwrap(), 0);
    assert_eq!(*now.lock().unwrap(), until);
    let record = setup.record();
    let argv = record["argv"].as_array().unwrap();
    let i = argv.iter().position(|v| v == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    assert_eq!(record["env"]["CLAUDE_CODE_PROMPT_CACHE_TTL"], "1h");
    assert!(record["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with(CONTINUE_TAG));
    assert_eq!(
        goal::load(&setup.home, SESSION).unwrap().unwrap().status,
        "done"
    );
}

#[test]
fn stop_ends_the_usage_limit_wait() {
    let setup = Setup::new("limit-reset");
    goal::open(&setup.home, SESSION, "original", setup.project().to_str()).unwrap();
    let start = local_two_am();
    let clock = Clock {
        now: Box::new(move || start),
        sleep: Box::new(|_| std::thread::sleep(Duration::from_millis(20))),
    };
    let wish = Wish::start(
        setup.config_with(clock),
        setup.project(),
        "original",
        &Options::default(),
    )
    .unwrap();
    assert_eq!(
        next(&wish),
        Event::Started {
            session: SESSION.into()
        }
    );
    assert_eq!(
        next(&wish),
        Event::Result {
            text: LIMIT_MESSAGE.into(),
            is_error: true,
        }
    );
    assert!(matches!(next(&wish), Event::Waiting { .. }));
    assert!(wish.is_running());
    assert_eq!(wish.interject("still there?").unwrap_err().kind, "waiting");
    wish.stop();
    assert_eq!(collect(&wish), [Event::Exited { code: 130 }]);
    assert_eq!(wish.wait().unwrap(), 130);
    let g = goal::load(&setup.home, SESSION).unwrap().unwrap();
    assert_eq!(g.status, "blocked");
    assert!(g.note.unwrap().contains("session limit"));
    assert!(!setup.record()["argv"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "--resume"));
}
