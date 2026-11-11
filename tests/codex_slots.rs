mod common;
use axiom::codex::{AppServer, RunRequest};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn until(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < end, "child did not make progress");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn locked(home: &Path) -> File {
    fs::create_dir_all(home.join("codex-slots")).unwrap();
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join("codex-slots/0.lock"))
        .unwrap();
    f.lock().unwrap();
    f
}
fn child(home: &Path, id: &str, cap: &str, action: &str) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "slot_child", "--nocapture"])
        .env("SLOT_TEST_HOME", home)
        .env("SLOT_TEST_ID", id)
        .env("SLOT_TEST_ACTION", action)
        .env("AXIOM_DATA_HOME", home)
        .env("AXIOM_HOME", home)
        .env("AXIOM_CODEX_MAX_PARALLEL", cap)
        .env("FAKE_CODEX_LOG", home.join(format!("{id}.log")))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}
fn result(home: &Path, id: &str, c: &mut Child) -> Value {
    until(|| c.try_wait().unwrap().is_some());
    serde_json::from_slice(&fs::read(home.join(format!("{id}.json"))).unwrap()).unwrap()
}
fn turns(home: &Path, id: &str) -> usize {
    fs::read_to_string(home.join(format!("{id}.log")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["method"] == "turn/start")
        .count()
}

#[test]
fn slot_child() {
    let Some(home) = std::env::var_os("SLOT_TEST_HOME") else {
        return;
    };
    let home = Path::new(&home);
    let id = std::env::var("SLOT_TEST_ID").unwrap();
    let action = std::env::var("SLOT_TEST_ACTION").unwrap();
    if action == "holder" {
        let _lock = locked(home);
        fs::write(home.join(format!("{id}.ready")), "").unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let server = Arc::new(AppServer::new(common::fake_codex(&[
        "--fake-interrupt-delay=0.4",
    ])));
    fs::write(home.join(format!("{id}.ready")), "").unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    if action == "cancel" || action == "close" || action == "active" || action == "stop" {
        let s = server.clone();
        let stop = stop.clone();
        let signal = home.join(format!("{id}.signal"));
        std::thread::spawn(move || {
            until(|| signal.exists());
            if action == "close" {
                s.close();
            } else if action == "stop" {
                stop.store(true, Ordering::Release);
            } else {
                s.cancel_all();
            }
        });
    }
    let active = std::env::var("SLOT_TEST_ACTION").unwrap() == "active";
    let req = RunRequest {
        brief: if active { "HANG" } else { "ok" }.into(),
        cwd: home.to_string_lossy().into_owned(),
        model: "gpt-6.1-terra".into(),
        sandbox: "read-only".into(),
        effort: None,
        output_schema: None,
        developer_instructions: None,
        timeout_s: if active { 10 } else { 1 },
        lean: false,
        keep_plugins: vec![],
        web_search: false,
    };
    let r = match server.run_cancellable(&req, None, Some(&stop)) {
        Ok(v) => v,
        Err(e) => json!({"error":e.to_json()}),
    };
    fs::write(home.join(format!("{id}.json")), r.to_string()).unwrap();
}

#[test]
fn another_process_waits_then_reports_its_wait() {
    let t = common::temp();
    let home = t.path();
    let lock = locked(home);
    let mut c = child(home, "wait", "1", "run");
    until(|| home.join("wait.ready").exists());
    std::thread::sleep(Duration::from_millis(350));
    let started = turns(home, "wait");
    drop(lock);
    let r = result(home, "wait", &mut c);
    assert_eq!(started, 0, "Codex started before a slot was free");
    assert_eq!(r["status"], "completed", "{r}");
    assert!(r["slot_wait_ms"].as_u64().unwrap_or(0) >= 200, "{r}");
    assert!(
        r["slot_wait"].as_str().unwrap_or("").contains("1 in use"),
        "{r}"
    );
}

#[test]
fn queued_calls_honour_cancel_and_close_without_starting_codex() {
    for action in ["cancel", "close", "stop"] {
        let t = common::temp();
        let home = t.path();
        let _lock = locked(home);
        let mut c = child(home, action, "1", action);
        until(|| home.join(format!("{action}.ready")).exists());
        std::thread::sleep(Duration::from_millis(200));
        fs::write(home.join(format!("{action}.signal")), "").unwrap();
        let r = result(home, action, &mut c);
        assert_eq!(turns(home, action), 0, "{r}");
        assert_eq!(r["status"], "interrupted", "{r}");
        assert!(r["slot_wait_ms"].as_u64().unwrap_or(0) > 0, "{r}");
    }
}

#[test]
fn waiting_obeys_the_call_timeout() {
    let t = common::temp();
    let home = t.path();
    let _lock = locked(home);
    let mut c = child(home, "timeout", "1", "run");
    let r = result(home, "timeout", &mut c);
    assert_eq!(turns(home, "timeout"), 0, "{r}");
    assert_eq!(r["status"], "timeout", "{r}");
}

#[test]
fn crash_releases_os_lock_and_stale_files_are_free() {
    let t = common::temp();
    let home = t.path();
    let mut holder = child(home, "holder", "1", "holder");
    until(|| home.join("holder.ready").exists());
    let mut c = child(home, "crash", "1", "run");
    until(|| home.join("crash.ready").exists());
    std::thread::sleep(Duration::from_millis(200));
    let started = turns(home, "crash");
    holder.kill().unwrap();
    holder.wait().unwrap();
    let r = result(home, "crash", &mut c);
    assert_eq!(started, 0);
    assert_eq!(r["status"], "completed", "{r}");
    let mut next = child(home, "next", "1", "run");
    assert_eq!(result(home, "next", &mut next)["status"], "completed");
}

#[test]
fn invalid_caps_name_the_variable_and_value() {
    for value in ["", "0", "-1", "1.5", "NaN", "text", "18446744073709551616"] {
        let t = common::temp();
        let mut c = child(t.path(), "invalid", value, "run");
        let r = result(t.path(), "invalid", &mut c);
        assert_eq!(turns(t.path(), "invalid"), 0, "{r}");
        assert_eq!(r["error"]["kind"], "invalid_argument", "{r}");
        let msg = r["error"]["message"].as_str().unwrap();
        assert!(
            msg.contains("AXIOM_CODEX_MAX_PARALLEL") && msg.contains(&format!("{value:?}")),
            "{msg}"
        );
    }
}

#[test]
fn concurrent_processes_share_the_pool_and_reuse_released_slots() {
    let t = common::temp();
    let home = t.path();
    let mut a = child(home, "a", "2", "active");
    let mut b = child(home, "b", "2", "active");
    until(|| turns(home, "a") == 1 && turns(home, "b") == 1);
    assert_eq!(axiom::codex::slots::in_use(home).unwrap(), 2);
    let mut c = child(home, "c", "2", "run");
    until(|| home.join("c.ready").exists());
    std::thread::sleep(Duration::from_millis(200));
    let started = turns(home, "c");
    fs::write(home.join("a.signal"), "").unwrap();
    until(|| {
        fs::read_to_string(home.join("a.log"))
            .unwrap_or_default()
            .contains("turn/interrupt")
    });
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        axiom::codex::slots::in_use(home).unwrap(),
        2,
        "interrupt acknowledgement must retain the slot until turn/completed"
    );
    let ar = result(home, "a", &mut a);
    let cr = result(home, "c", &mut c);
    fs::write(home.join("b.signal"), "").unwrap();
    let br = result(home, "b", &mut b);
    assert_eq!(started, 0);
    assert_eq!(ar["status"], "interrupted");
    assert_eq!(br["status"], "interrupted");
    assert_eq!(cr["status"], "completed");
    assert!(cr["slot_wait_ms"].as_u64().unwrap() >= 100);
    assert_eq!(axiom::codex::slots::in_use(home).unwrap(), 0);
}

#[test]
fn counting_uses_locks_and_propagates_unreadable_folders() {
    let t = common::temp();
    assert_eq!(axiom::codex::slots::in_use(t.path()).unwrap(), 0);
    let f = locked(t.path());
    fs::write(t.path().join("codex-slots/unused.lock"), "unrelated").unwrap();
    assert_eq!(axiom::codex::slots::in_use(t.path()).unwrap(), 1);
    drop(f);
    assert_eq!(axiom::codex::slots::in_use(t.path()).unwrap(), 0);
    let bad = t.path().join("bad");
    fs::create_dir(&bad).unwrap();
    fs::write(bad.join("codex-slots"), "file").unwrap();
    assert!(axiom::codex::slots::in_use(&bad).is_err());
    assert_eq!(axiom::codex::slots::parse(None).unwrap(), 48);
    assert_eq!(axiom::codex::slots::parse(Some("50".into())).unwrap(), 50);
}
