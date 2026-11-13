mod common;
use axiom::{
    base, goal, orch,
    session::{self, Clock, Config},
    storage::{preflight, retention, Class, Ledger, Owner},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Cursor, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const ID: &str = "12345678-1234-1234-1234-123456789abc";
struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Env {
    fn new() -> Self {
        Self(
            [
                "AXIOM_FAKE_DISK",
                "AXIOM_HEARTBEAT_MS",
                "AXIOM_DISK_POLL_MS",
                "FAKE_CLAUDE_RECORD",
                "FAKE_CLAUDE_SCENARIO",
                "FAKE_CLAUDE_SESSION",
                "FAKE_CLAUDE_ALIVE_MS",
            ]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect(),
        )
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            if let Some(v) = v {
                std::env::set_var(k, v)
            } else {
                std::env::remove_var(k)
            }
        }
    }
}
struct Fixture {
    temp: tempfile::TempDir,
    config: Config,
    repo: PathBuf,
    disk: PathBuf,
    store: PathBuf,
}
impl Fixture {
    fn observe_poll(&mut self) -> Arc<AtomicUsize> {
        let observed = Arc::new(AtomicUsize::new(0));
        let flag = Arc::clone(&observed);
        self.config.clock.sleep = Box::new(move |duration| {
            flag.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(duration);
        });
        observed
    }
    fn new() -> Self {
        let temp = common::temp();
        let home = temp.path().join("home");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(home.join("storage")).unwrap();
        fs::write(
            home.join("storage/policy.json"),
            json!({"low_disk_bytes":100,"reserve_bytes":10,"estimates":{"wish":20,"batch":20}})
                .to_string(),
        )
        .unwrap();
        let disk = temp.path().join("disk.json");
        fs::write(&disk, "{\"free_bytes\":10000}").unwrap();
        std::env::set_var("AXIOM_FAKE_DISK", &disk);
        std::env::set_var("AXIOM_HEARTBEAT_MS", "100");
        std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
        std::env::set_var("FAKE_CLAUDE_SCENARIO", "alive");
        std::env::set_var("FAKE_CLAUDE_SESSION", ID);
        std::env::set_var("FAKE_CLAUDE_ALIVE_MS", "1500");
        let g = goal::open(&home, ID, "wish", repo.to_str()).unwrap();
        let store = orch::store_for_goal(&home, &g).unwrap();
        fs::create_dir_all(&store).unwrap();
        let config = Config {
            home,
            claude: Ok(common::fake("fake_claude")),
            bin: temp.path().join("axiom"),
            clock: Clock::system(),
        };
        Self {
            temp,
            config,
            repo,
            disk,
            store,
        }
    }
    fn space(&self, n: u64) {
        base::atomic_write(&self.disk, json!({"free_bytes":n}).to_string().as_bytes()).unwrap();
    }
    fn goal(&self) -> goal::Goal {
        goal::load(&self.config.home, ID).unwrap().unwrap()
    }
    fn has(&self, kind: &str) -> bool {
        orch::events::read_events(&self.store, 0)
            .unwrap()
            .iter()
            .any(|e| e.kind == kind)
    }
    fn spawn(&self, resume: Option<&str>) -> axiom::base::Res<session::Spawned> {
        session::spawn_with(
            &self.config,
            "wish",
            "wish",
            &self.repo,
            None,
            resume,
            &session::Launch::default(),
        )
    }
    fn drive(&self, spawned: session::Spawned) -> (i32, String, Vec<Value>) {
        let mut out = Vec::new();
        let mut events = Vec::new();
        let code = session::drive(
            spawned,
            &self.config,
            &mut out,
            &AtomicBool::new(false),
            &mut |v| events.push(v.clone()),
        )
        .unwrap();
        (code, String::from_utf8(out).unwrap(), events)
    }
    fn registered(&self, name: &str, class: Class, meta: Value, age: i64) -> PathBuf {
        let path = self.config.home.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("data"), b"reclaim me").unwrap();
        let l = Ledger::open(&self.config.home).unwrap();
        l.register(
            &path,
            class,
            &Owner {
                project: "test".into(),
                wish: None,
            },
            meta,
        )
        .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        l.touch(&path, now - age).unwrap();
        path
    }
    fn batch(&self, pid: u32) -> goal::Batch {
        goal::Batch {
            pid,
            progress_file: self
                .temp
                .path()
                .join("progress")
                .to_string_lossy()
                .into_owned(),
            marker_file: self
                .temp
                .path()
                .join("marker")
                .to_string_lossy()
                .into_owned(),
        }
    }
    fn waiting(&self, pid: u32) {
        goal::set_status_with_batch(
            &self.config.home,
            ID,
            "waiting",
            Some("batch"),
            Some(self.batch(pid)),
        )
        .unwrap();
    }
    fn launch_batch(&self) -> u32 {
        let spec = session::launch::LaunchSpec {
            kind: preflight::Kind::Batch,
            argv: vec![
                common::python(),
                "-I".into(),
                "-c".into(),
                "import time; time.sleep(60)".into(),
            ],
            cwd: self.repo.clone(),
            need_bytes: None,
            session: Some(ID.into()),
            progress_file: None,
            marker_file: None,
            register: vec![],
        };
        match session::launch::launch(&self.config.home, &*preflight::os_probe(), &spec).unwrap() {
            session::launch::Launched::Started { pid, .. } => pid,
            _ => panic!("not started"),
        }
    }
}
fn wait_for_poll(flag: &AtomicUsize, count: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while flag.load(Ordering::SeqCst) < count && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        flag.load(Ordering::SeqCst) >= count,
        "disk wait did not reach poll {count}"
    );
}
fn locked() -> (std::sync::MutexGuard<'static, ()>, Env) {
    (
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
        Env::new(),
    )
}
fn error(f: &Fixture, resume: Option<&str>) -> axiom::base::Failure {
    match f.spawn(resume) {
        Err(e) => e,
        Ok(_) => panic!("spawn succeeded"),
    }
}
fn paused(f: &Fixture) {
    let g = f.goal();
    assert_eq!(g.status, "blocked");
    assert_eq!(g.reason.as_deref(), Some("disk_space"));
}

#[test]
fn wish_start_preflight_blocks_spawn() {
    let _guard = locked();
    let f = Fixture::new();
    f.space(1);
    let e = error(&f, None);
    assert_eq!(e.kind, "disk_space");
    assert!(e.message.contains("paused"));
    assert!(!f.config.home.join("logs").exists());
    assert!(!f.temp.path().join("record").exists());
    f.space(10000);
    let s = f.spawn(None).unwrap();
    assert!(s.pid() > 0);
    let (code, _, _) = f.drive(s);
    assert_eq!(code, 0);
    assert_ne!(f.goal().status, "blocked");
}

#[test]
fn start_waits_for_space_then_runs_once() {
    let _guard = locked();
    let mut f = Fixture::new();
    fs::write(
        f.config.home.join("storage/policy.json"),
        r#"{"low_disk_bytes":1,"reserve_bytes":0,"estimates":{"wish":20}}"#,
    )
    .unwrap();
    f.space(1);
    std::env::set_var("AXIOM_DISK_POLL_MS", "10");
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "ok");
    std::env::set_var("FAKE_CLAUDE_SESSION", "different-session");
    let poll = f.observe_poll();
    let disk = f.disk.clone();
    let record = f.temp.path().join("record");
    let helper = std::thread::spawn(move || {
        wait_for_poll(&poll, 2);
        assert!(!record.exists(), "started while space was still short");
        base::atomic_write(&disk, b"{\"free_bytes\":10000}").unwrap();
    });
    let mut out = Vec::new();
    let code = session::run_with(
        &f.config,
        "wish",
        "wish",
        &f.repo,
        None,
        Box::new(Cursor::new(Vec::new())),
        &mut out,
        &session::Launch::default(),
    )
    .unwrap();
    helper.join().unwrap();
    assert_eq!(code, 0);
    assert!(f.temp.path().join("record").exists());
    let output = String::from_utf8(out).unwrap();
    assert_eq!(
        output
            .matches("wish paused, free space then resume:")
            .count(),
        1,
        "{output}"
    );
}

#[test]
fn interrupted_wait_does_not_start_and_records_pause() {
    let _guard = locked();
    let mut f = Fixture::new();
    fs::write(
        f.config.home.join("storage/policy.json"),
        r#"{"low_disk_bytes":1,"reserve_bytes":0,"estimates":{"wish":20}}"#,
    )
    .unwrap();
    f.space(1);
    std::env::set_var("AXIOM_DISK_POLL_MS", "10");
    let poll = f.observe_poll();
    let record = f.temp.path().join("record");
    let helper = std::thread::spawn(move || {
        wait_for_poll(&poll, 1);
        assert!(!record.exists(), "started without space");
        session::trigger_interrupt();
    });
    let mut out = Vec::new();
    let code = session::resume(
        &f.config,
        ID,
        None,
        None,
        Box::new(Cursor::new(Vec::new())),
        &mut out,
    )
    .unwrap();
    helper.join().unwrap();
    assert_eq!(code, 75);
    assert!(!f.temp.path().join("record").exists());
    let line = String::from_utf8(out).unwrap().trim().to_owned();
    assert_eq!(f.goal().note.as_deref(), Some(line.as_str()));
    assert_eq!(f.goal().need_bytes, Some(20));
    let events = orch::events::read_events(&f.store, 0).unwrap();
    let event = events
        .iter()
        .find(|e| e.kind == preflight::EVENT_PREFLIGHT_PAUSED)
        .unwrap();
    assert_eq!(event.data["line"], line);
}

#[test]
fn interrupted_new_wish_never_starts() {
    let _guard = locked();
    let mut f = Fixture::new();
    fs::write(
        f.config.home.join("storage/policy.json"),
        r#"{"low_disk_bytes":1,"reserve_bytes":0,"estimates":{"wish":20}}"#,
    )
    .unwrap();
    f.space(1);
    std::env::set_var("AXIOM_DISK_POLL_MS", "10");
    let poll = f.observe_poll();
    let record = f.temp.path().join("record");
    let helper = std::thread::spawn(move || {
        wait_for_poll(&poll, 1);
        assert!(!record.exists());
        session::trigger_interrupt();
    });
    let mut out = Vec::new();
    let code = session::run_with(
        &f.config,
        "wish",
        "wish",
        &f.repo,
        None,
        Box::new(Cursor::new(Vec::new())),
        &mut out,
        &session::Launch::default(),
    )
    .unwrap();
    helper.join().unwrap();
    assert_eq!(code, 75);
    assert!(!f.temp.path().join("record").exists());
    let output = String::from_utf8(out).unwrap();
    assert_eq!(
        output
            .matches("wish paused, free space then resume:")
            .count(),
        1
    );
}

#[test]
fn override_is_recorded_on_first_pause_and_reused_on_resume() {
    let _guard = locked();
    let f = Fixture::new();
    fs::write(
        f.config.home.join("storage/policy.json"),
        r#"{"low_disk_bytes":1,"reserve_bytes":0,"estimates":{"wish":20}}"#,
    )
    .unwrap();
    f.space(50);
    let launch = session::Launch {
        need_bytes: Some(100),
        ..Default::default()
    };
    let err = session::spawn_with(&f.config, "wish", "wish", &f.repo, None, Some(ID), &launch)
        .err()
        .unwrap();
    assert_eq!(err.kind, "disk_space");
    assert_eq!(f.goal().need_bytes, Some(100));
    assert_eq!(
        session::resume_target(&f.config, ID, None)
            .unwrap_err()
            .kind,
        "disk_space"
    );
    f.space(100);
    session::resume_target(&f.config, ID, None).unwrap();
    assert!(f.goal().reason.is_none());
}

#[test]
fn resume_waits_using_recorded_need_then_unblocks() {
    let _guard = locked();
    let mut f = Fixture::new();
    fs::write(
        f.config.home.join("storage/policy.json"),
        r#"{"low_disk_bytes":1,"reserve_bytes":0,"estimates":{"wish":20}}"#,
    )
    .unwrap();
    goal::record_need(&f.config.home, ID, 100).unwrap();
    goal::disk_pause(&f.config.home, ID, "previous pause").unwrap();
    f.space(50);
    std::env::set_var("AXIOM_DISK_POLL_MS", "10");
    let poll = f.observe_poll();
    let disk = f.disk.clone();
    let record = f.temp.path().join("record");
    let helper = std::thread::spawn(move || {
        wait_for_poll(&poll, 2);
        assert!(!record.exists());
        base::atomic_write(&disk, b"{\"free_bytes\":10000}").unwrap();
    });
    let mut out = Vec::new();
    let code = session::resume(
        &f.config,
        ID,
        None,
        None,
        Box::new(Cursor::new(Vec::new())),
        &mut out,
    )
    .unwrap();
    helper.join().unwrap();
    assert_eq!(code, 0);
    assert!(f.temp.path().join("record").exists());
    assert_eq!(f.goal().need_bytes, Some(100));
    assert!(f.goal().reason.is_none());
    assert!(f.has(preflight::EVENT_DISK_RESUMED));
    let output = String::from_utf8(out).unwrap();
    assert_eq!(
        output
            .matches("wish paused, free space then resume:")
            .count(),
        1
    );
    let events = orch::events::read_events(&f.store, 0).unwrap();
    let pause = events
        .iter()
        .find(|e| e.kind == preflight::EVENT_PREFLIGHT_PAUSED)
        .unwrap();
    assert_eq!(output.lines().next(), pause.data["line"].as_str());
}
#[test]
fn preflight_preserves_unstarted_wish_goal() {
    let _guard = locked();
    let f = Fixture::new();
    f.space(1);
    assert_eq!(error(&f, Some(ID)).kind, "disk_space");
    paused(&f);
    assert!(f.has(preflight::EVENT_PREFLIGHT_PAUSED));
    assert!(fs::read_to_string(f.store.join("reports/disk.txt"))
        .unwrap()
        .contains("paused"));
    assert_eq!(
        goal::accept_hold(&f.config.home, f.goal().project.as_deref().unwrap())
            .unwrap()
            .as_deref(),
        Some("disk_space")
    );
}
#[test]
fn low_disk_cleans_then_pauses_with_distinct_reason() {
    let _guard = locked();
    let f = Fixture::new();
    let scratch = f.registered("scratch", Class::Scratch, json!({}), 1_000_000_000);
    let cache = f.registered(
        "cache",
        Class::BuildCache,
        json!({"worktree_state":"merged","state_since_ms":0}),
        0,
    );
    let keep = f.registered("evidence", Class::Evidence, json!({}), 0);
    let project = f.registered("project", Class::Worktree, json!({}), 0);
    let other = f.config.home.join("unregistered");
    fs::create_dir_all(&other).unwrap();
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "tick");
    let s = f.spawn(None).unwrap();
    f.space(1);
    let (code, out, events) = f.drive(s);
    assert_eq!(code, 75);
    paused(&f);
    assert!(!scratch.exists() && !cache.exists());
    assert!(
        keep.exists() && project.exists() && other.exists() && f.config.home.join("logs").exists()
    );
    assert!(fs::read_to_string(retention::audit_log(&f.config.home))
        .unwrap()
        .contains("delete"));
    assert!(f.has(preflight::EVENT_DISK_PAUSE));
    assert!(events.iter().any(|e| e["subtype"] == "disk_pause"));
    assert!(out.contains("paused, free space then resume"));
}
#[test]
fn disk_pause_stops_managed_batch_writes() {
    let _guard = locked();
    let f = Fixture::new();
    let pid = f.launch_batch();
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "tick");
    let s = f.spawn(None).unwrap();
    f.space(1);
    assert_eq!(f.drive(s).0, 75);
    for _ in 0..50 {
        if !base::proc::pid_alive(pid) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!base::proc::pid_alive(pid));
    assert!(!f
        .config
        .home
        .join("storage/managed")
        .join(format!("{ID}-{pid}.json"))
        .exists());
}
#[test]
fn resume_rechecks_space_and_clears_disk_reason() {
    let _guard = locked();
    let f = Fixture::new();
    f.space(1);
    let _ = error(&f, Some(ID));
    let before = fs::read(f.config.home.join("goals").join(format!("{ID}.json"))).unwrap();
    assert_eq!(
        session::resume_target(&f.config, ID, Some("continue"))
            .unwrap_err()
            .kind,
        "disk_space"
    );
    assert_eq!(
        before,
        fs::read(f.config.home.join("goals").join(format!("{ID}.json"))).unwrap()
    );
    f.space(10000);
    session::resume_target(&f.config, ID, Some("continue")).unwrap();
    assert_eq!(f.goal().reason, None);
    assert!(f.has(preflight::EVENT_DISK_RESUMED));
}
#[test]
fn pressure_cleanup_frees_only_safe_classes() {
    let _guard = locked();
    let f = Fixture::new();
    let scratch = f.registered("scratch", Class::Scratch, json!({}), 1_000_000_000);
    let cache = f.registered(
        "cache",
        Class::BuildCache,
        json!({"worktree_state":"gone","state_since_ms":0}),
        0,
    );
    let old = f.registered("engine/old", Class::EngineCopy, json!({}), 1_000_000);
    let recent = f.registered("engine/recent", Class::EngineCopy, json!({}), 1000);
    let evidence = f.registered("evidence", Class::Evidence, json!({}), 0);
    let logs = f.registered("otherlogs", Class::Logs, json!({}), 0);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "tick");
    let s = f.spawn(None).unwrap();
    f.space(1);
    assert_eq!(f.drive(s).0, 75);
    assert!(!scratch.exists() && !cache.exists() && !old.exists());
    assert!(recent.exists() && evidence.exists() && logs.exists());
    let audit = fs::read_to_string(retention::audit_log(&f.config.home)).unwrap();
    for line in audit.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert!(matches!(
            v["class"].as_str(),
            Some("scratch" | "build_cache" | "engine_copy")
        ));
    }
}
#[test]
fn headless_batch_wait_pauses_on_low_disk() {
    let _guard = locked();
    let f = Fixture::new();
    let pid = f.launch_batch();
    f.waiting(pid);
    f.space(1);
    let mut out = Vec::new();
    assert_eq!(
        session::wait_and_resume(
            &f.config,
            ID,
            Box::new(Cursor::new(Vec::<u8>::new())),
            &mut out
        )
        .unwrap(),
        75
    );
    paused(&f);
    assert!(f.has(preflight::EVENT_DISK_PAUSE));
    assert!(String::from_utf8(out).unwrap().contains("paused"));
    for _ in 0..50 {
        if !base::proc::pid_alive(pid) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!base::proc::pid_alive(pid));
}

#[test]
fn headless_wait_negative_control_and_unregistered_batch() {
    let _guard = locked();
    let f = Fixture::new();
    let mut child = std::process::Command::new(common::python())
        .args(["-I", "-c", "import time; time.sleep(60)"])
        .spawn()
        .unwrap();
    f.waiting(child.id());
    f.space(1);
    let mut out = Vec::new();
    assert_eq!(
        session::wait_and_resume(
            &f.config,
            ID,
            Box::new(Cursor::new(Vec::<u8>::new())),
            &mut out
        )
        .unwrap(),
        75
    );
    paused(&f);
    assert!(base::proc::pid_alive(child.id()));
    child.kill().unwrap();
    child.wait().unwrap();
    f.space(10000);
    session::resume_target(&f.config, ID, Some("continue")).unwrap();
    let mut child = std::process::Command::new(common::python())
        .args(["-I", "-c", "import time; time.sleep(60)"])
        .spawn()
        .unwrap();
    let batch = f.batch(child.id());
    fs::write(&batch.marker_file, "done").unwrap();
    f.waiting(child.id());
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "resume");
    assert_eq!(
        session::wait_and_resume(
            &f.config,
            ID,
            Box::new(Cursor::new(Vec::<u8>::new())),
            &mut Vec::new()
        )
        .unwrap(),
        0
    );
    assert_ne!(f.goal().status, "blocked");
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn headless_wait_checks_again_after_poll() {
    let _guard = locked();
    let f = Fixture::new();
    let mut child = std::process::Command::new(common::python())
        .args(["-I", "-c", "import time; time.sleep(60)"])
        .spawn()
        .unwrap();
    f.waiting(child.id());
    fs::write(f.temp.path().join("progress"), "poll started").unwrap();
    struct LowerOnProgress {
        disk: PathBuf,
        output: Vec<u8>,
        lowered: bool,
    }
    impl Write for LowerOnProgress {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if !self.lowered
                && String::from_utf8_lossy(&self.output).contains("batch: poll started")
            {
                base::atomic_write(&self.disk, b"{\"free_bytes\":1}").unwrap();
                self.lowered = true;
            }
            Ok(())
        }
    }
    let mut out = LowerOnProgress {
        disk: f.disk.clone(),
        output: Vec::new(),
        lowered: false,
    };
    let result = session::wait_and_resume(
        &f.config,
        ID,
        Box::new(Cursor::new(Vec::<u8>::new())),
        &mut out,
    );
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(result.unwrap(), 75);
    assert!(out.lowered);
    paused(&f);
}
