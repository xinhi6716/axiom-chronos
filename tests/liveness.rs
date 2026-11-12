mod common;

use axiom::{
    goal,
    orch::{self, events},
    session::{self, Clock, Config},
    setup,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Mutex,
    },
    thread,
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
                "FAKE_CLAUDE_TURNS",
                "FAKE_CLAUDE_ALIVE_MS",
                "AXIOM_HEARTBEAT_MS",
            ]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
        )
    }
}
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

struct Fixture {
    temp: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    store: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let goal = goal::open(&home, SESSION, "test goal", Some(repo.to_str().unwrap())).unwrap();
        assert!(goal.project.is_some() && goal.wish_id.is_some());
        let store = orch::store_for_goal(&home, &goal).unwrap();
        Self {
            temp,
            home,
            repo,
            store,
        }
    }
    fn execute(
        &self,
        scenario: &str,
        max_turns: Option<u32>,
        stop_on_init: bool,
        observe: &mut dyn FnMut(&serde_json::Value),
    ) -> (i32, String) {
        std::env::set_var("FAKE_CLAUDE_RECORD", self.temp.path().join("record.json"));
        std::env::set_var("FAKE_CLAUDE_SCENARIO", scenario);
        std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
        std::env::set_var("FAKE_CLAUDE_TURNS", "9");
        let config = Config {
            claude: Ok(common::fake("fake_claude")),
            home: self.home.clone(),
            bin: self.temp.path().join("axiom"),
            clock: Clock::system(),
        };
        let spawned = session::spawn(&config, "wish", "test", &self.repo, max_turns, None).unwrap();
        let stop = AtomicBool::new(false);
        let mut out = Vec::new();
        let code = session::drive(spawned, &config, &mut out, &stop, &mut |event| {
            observe(event);
            if stop_on_init && event["type"] == "system" && event["subtype"] == "init" {
                stop.store(true, Ordering::SeqCst);
            }
        })
        .unwrap();
        (code, String::from_utf8(out).unwrap())
    }
}
fn count(store: &Path) -> usize {
    events::read_events(store, 0)
        .unwrap()
        .iter()
        .filter(|e| e.kind == events::HEARTBEAT)
        .count()
}

#[test]
fn heartbeats_during_drive_stop_at_return_and_do_not_change_activity() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    std::env::set_var("AXIOM_HEARTBEAT_MS", "100");
    std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "800");
    let f = Fixture::new();
    fs::create_dir_all(&f.store).unwrap();
    events::activity_start(&f.store, "m1", "core", "brief").unwrap();
    let latest = events::latest_activity_at(&f.store).unwrap();
    let mut during = 0;
    let (code, _) = f.execute("alive", None, false, &mut |e| {
        if e["type"] == "result" {
            during = count(&f.store);
            assert_eq!(events::latest_activity_at(&f.store).unwrap(), latest);
        }
    });
    assert_eq!(code, 0);
    assert!(during >= 3, "heartbeats during drive: {during}");
    let after = count(&f.store);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(count(&f.store), after);
}

#[test]
fn missing_store_is_not_created() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    std::env::set_var("AXIOM_HEARTBEAT_MS", "100");
    std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "350");
    let f = Fixture::new();
    assert!(!f.store.exists());
    assert_eq!(f.execute("alive", None, false, &mut |_| {}).0, 0);
    assert!(!f.store.exists());
    assert!(!f.store.join("events.jsonl").exists());
}

#[test]
fn heartbeat_interval_invalid_defaults_and_small_values_clamp() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "350");
    for (value, expected) in [("abc", false), ("5", true)] {
        std::env::set_var("AXIOM_HEARTBEAT_MS", value);
        let f = Fixture::new();
        fs::create_dir_all(&f.store).unwrap();
        let mut during = 0;
        let (code, _) = f.execute("alive", None, false, &mut |e| {
            if e["type"] == "result" {
                during = count(&f.store);
            }
        });
        assert_eq!(code, 0);
        if expected {
            assert!(during >= 2 && during <= 5, "clamped heartbeats: {during}");
        } else {
            assert_eq!(during, 0);
        }
    }
}

#[test]
fn closes_open_activity_on_each_exit() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "100");
    for (scenario, stop, max, expected) in [
        ("alive", false, None, 0),
        ("error", false, None, 1),
        ("alive", true, None, 130),
        ("max-turns", false, Some(4), 3),
    ] {
        let f = Fixture::new();
        let start = events::activity_start(&f.store, "m1", "core", "brief").unwrap();
        assert_eq!(events::open_activities(&f.store).unwrap().len(), 1);
        let (code, output) = f.execute(scenario, max, stop, &mut |_| {});
        assert_eq!(code, expected, "{scenario}: {output}");
        if scenario == "error" {
            assert!(output.contains("claude reported an error"), "{output}");
        }
        if scenario == "max-turns" {
            assert!(
                output.contains(&format!(
                    "wish cut off at 4 turns; resume: axiom wish --resume {SESSION}"
                )),
                "{output}"
            );
        }
        let rows = events::read_events(&f.store, 0).unwrap();
        assert!(rows.iter().any(|e| e.kind == events::ACTIVITY_FINISH
            && e.data["activity"] == start.data["activity"]
            && e.data["outcome"] == "exit"));
        assert!(events::open_activities(&f.store).unwrap().is_empty());
    }
}

#[test]
fn limit_wait_keeps_heartbeats_and_interrupt_closes_activity() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    std::env::set_var("AXIOM_HEARTBEAT_MS", "100");
    let f = Fixture::new();
    std::env::set_var("FAKE_CLAUDE_RECORD", f.temp.path().join("record.json"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "limit-reset");
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    events::activity_start(&f.store, "m1", "core", "brief").unwrap();
    let real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let offset = i64::from(session::local_offset_secs(real));
    let start = (real + offset).div_euclid(86400) * 86400 + 2 * 3600 - offset;
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: f.home.clone(),
        bin: f.temp.path().join("axiom"),
        clock: Clock {
            now: Box::new(move || start),
            sleep: Box::new(thread::sleep),
        },
    };
    let spawned = session::spawn(&config, "wish", "test", &f.repo, None, None).unwrap();
    let stop = AtomicBool::new(false);
    let mut out = Vec::new();
    let mut before = 0;
    let mut after = 0;
    let code = session::drive(spawned, &config, &mut out, &stop, &mut |event| {
        if event["subtype"] == "limit_wait" {
            before = count(&f.store);
            thread::sleep(Duration::from_millis(350));
            after = count(&f.store);
            stop.store(true, Ordering::SeqCst);
        }
    })
    .unwrap();
    assert_eq!(code, 130);
    assert!(String::from_utf8(out).unwrap().contains(&format!(
        "interrupted; resume: axiom wish --resume {SESSION}"
    )));
    assert!(after >= before + 2, "before={before}, after={after}");
    assert!(events::open_activities(&f.store).unwrap().is_empty());
    assert!(events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == events::ACTIVITY_FINISH && e.data["outcome"] == "exit"));
}

#[test]
fn error_return_after_limit_wait_closes_activity() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let f = Fixture::new();
    std::env::set_var("FAKE_CLAUDE_RECORD", f.temp.path().join("record.json"));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "limit-reset");
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    events::activity_start(&f.store, "m1", "core", "brief").unwrap();
    let real = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let offset = i64::from(session::local_offset_secs(real));
    let start = (real + offset).div_euclid(86400) * 86400 + 2 * 3600 - offset;
    let now = std::sync::Arc::new(AtomicI64::new(start));
    let now_clock = now.clone();
    let config = Config {
        claude: Ok(common::fake("fake_claude")),
        home: f.home.clone(),
        bin: f.temp.path().join("axiom"),
        clock: Clock {
            now: Box::new(move || now_clock.load(Ordering::SeqCst)),
            sleep: Box::new(thread::sleep),
        },
    };
    let spawned = session::spawn(&config, "wish", "test", &f.repo, None, None).unwrap();
    let mut out = Vec::new();
    let result = session::drive(
        spawned,
        &config,
        &mut out,
        &AtomicBool::new(false),
        &mut |event| {
            if event["subtype"] == "limit_wait" {
                let path = f.home.join("goals").join(format!("{SESSION}.json"));
                let mut goal: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                goal["status"] = json!("done");
                fs::write(path, serde_json::to_vec(&goal).unwrap()).unwrap();
                now.store(start + 20_000, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(result.unwrap_err().kind, "invalid_argument");
    assert!(events::open_activities(&f.store).unwrap().is_empty());
    assert!(events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == events::ACTIVITY_FINISH && e.data["outcome"] == "exit"));
}

#[test]
fn cancels_running_setup_only_for_driven_store() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = EnvRestore::new();
    let f = Fixture::new();
    fs::create_dir_all(f.repo.join(".axiom")).unwrap();
    let stop_file = f.temp.path().join("stop-setup");
    struct Cleanup(PathBuf, Vec<u32>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::write(&self.0, "stop");
            for &pid in &self.1 {
                let _ = axiom::base::proc::wait_pid_exit(pid, Duration::from_secs(3));
            }
        }
    }
    let mut cleanup_guard = Cleanup(stop_file.clone(), Vec::new());
    fs::write(f.repo.join(setup::MANIFEST), serde_json::to_vec(&json!({"version":1,"steps":[
        {"id":"index","argv":["unused"],"plan_independent":false},
        {"id":"work","argv":[common::python(),"-c","import pathlib,sys,time\np=pathlib.Path(sys.argv[1])\nwhile not p.exists(): time.sleep(0.05)",stop_file],"timeout_s":120,"plan_independent":true}
    ]})).unwrap()).unwrap();
    let control = f.temp.path().join("control");
    for store in [&f.store, &control] {
        let states = setup::start(store, &f.repo).unwrap();
        let work = states.iter().find(|s| s.id == "work").unwrap();
        if let Some(pid) = work.pid {
            cleanup_guard.1.push(pid);
        }
        assert_eq!(work.state, "running");
    }
    std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "100");
    let result = f.execute("alive", None, false, &mut |_| {});
    let control_state = setup::status(&control)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "work")
        .unwrap();
    let state = setup::status(&f.store)
        .unwrap()
        .into_iter()
        .find(|s| s.id == "work")
        .unwrap();
    let cleanup = setup::cancel_all(&control, "test_cleanup");
    // Also clean up on failure, so a failing-first run does not leave a child behind.
    let _ = setup::cancel_all(&f.store, "test_cleanup");
    fs::write(&stop_file, "stop").unwrap();
    assert_eq!(result.0, 0);
    assert_eq!(control_state.state, "running");
    assert_eq!(state.state, "cancelled");
    assert_eq!(state.reason.as_deref(), Some("session_end"));
    assert!(!f.store.join("setup/work.ok").exists());
    cleanup.unwrap();
}
