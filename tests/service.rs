mod common;
use axiom::{
    goal,
    service::{
        self,
        client::Client,
        login,
        protocol::StartRequest,
        server::{Server, ServerOptions},
        tray,
    },
    session,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
macro_rules! client_ok {
    ($home:expr, $call:expr) => {
        $call.unwrap_or_else(|e| {
            let path = $home.join("service/accept-error.txt");
            let diagnostic = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| "(no accept error recorded)".into());
            panic!(
                "{} failed: {}; {}: {}",
                stringify!($call),
                e,
                path.display(),
                diagnostic
            )
        })
    };
}
static ENV: Mutex<()> = Mutex::new(());
const ID: &str = "12345678-1234-1234-1234-123456789abc";
struct Setup {
    _guard: std::sync::MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    home: PathBuf,
    old: Vec<(&'static str, Option<std::ffi::OsString>)>,
}
impl Setup {
    fn new() -> Self {
        let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let temp = common::temp();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let keys = [
            "FAKE_CLAUDE_SCENARIO",
            "FAKE_CLAUDE_RECORD",
            "FAKE_CLAUDE_SESSION",
        ];
        let old = keys.into_iter().map(|k| (k, std::env::var_os(k))).collect();
        std::env::set_var("FAKE_CLAUDE_SCENARIO", "tick");
        std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
        std::env::remove_var("FAKE_CLAUDE_SESSION");
        Self {
            _guard: guard,
            temp,
            home,
            old,
        }
    }
    fn server(
        &self,
        launch: bool,
        tray_calls: Arc<Mutex<Vec<String>>>,
        desktop_calls: Arc<Mutex<Vec<String>>>,
        login_calls: Arc<Mutex<Vec<String>>>,
    ) -> service::RunningService {
        let home = self.home.clone();
        let bin = self.temp.path().join("axiom");
        Server::start(ServerOptions {
            home: self.home.clone(),
            config: Arc::new(move || session::Config {
                claude: Ok(common::fake("fake_claude")),
                home: home.clone(),
                bin: bin.clone(),
                clock: session::Clock::system(),
            }),
            tray: Box::new(tray::Recording::new(tray_calls)),
            desktop: Box::new(tray::Recording::new(desktop_calls)),
            login: Box::new(login::Recording::new(login_calls)),
            login_launch: launch,
        })
        .unwrap()
    }
    fn start(
        &self,
    ) -> (
        service::RunningService,
        Client,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let t = Arc::new(Mutex::new(vec![]));
        let d = Arc::new(Mutex::new(vec![]));
        let l = Arc::new(Mutex::new(vec![]));
        let s = self.server(false, t.clone(), d.clone(), l.clone());
        let c = client_ok!(self.home, Client::connect(&self.home));
        (s, c, t, d, l)
    }
    fn req(&self) -> StartRequest {
        StartRequest {
            wish: "test wish".into(),
            cwd: self.temp.path().to_string_lossy().into(),
            ..Default::default()
        }
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        for (k, v) in &self.old {
            if let Some(v) = v {
                std::env::set_var(k, v)
            } else {
                std::env::remove_var(k)
            }
        }
    }
}
fn until(mut p: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    while !p() {
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(30));
    }
}
fn running(c: &Client) -> service::Snapshot {
    let mut out = None;
    until(|| {
        let s = client_ok!(c.home(), c.snapshot());
        if s.wishes
            .first()
            .is_some_and(|w| w.state == "running" && !w.log.is_empty())
        {
            out = Some(s);
            true
        } else {
            false
        }
    });
    out.unwrap()
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn finish(s: service::RunningService, c: &Client) {
    client_ok!(c.home(), c.quit(true));
    assert_eq!(s.wait(), 0);
}
#[test]
fn service_attach_detach() {
    let x = Setup::new();
    let (s, c, _, _, _) = x.start();
    let h = client_ok!(c.home(), c.start(x.req()));
    let snap = running(&c);
    drop(c);
    let c = client_ok!(x.home, Client::connect(&x.home));
    let sub = client_ok!(c.home(), c.subscribe(snap.cursor));
    let e = loop {
        if let Some(e) = client_ok!(c.home(), sub.recv_timeout(Duration::from_secs(3))) {
            if e.kind == "text" {
                break e;
            }
        }
    };
    assert_eq!(e.handle, h);
    finish(s, &c);
}
#[test]
fn service_reattach_returns_live_state() {
    let x = Setup::new();
    let (s, c, _, _, _) = x.start();
    let h = client_ok!(c.home(), c.start(x.req()));
    let a = running(&c);
    drop(c);
    let c = client_ok!(x.home, Client::connect(&x.home));
    let b = client_ok!(c.home(), c.snapshot());
    assert_eq!(a.wishes[0].key, b.wishes[0].key);
    assert_eq!(b.wishes[0].handle, h);
    assert_eq!(b.wishes[0].session.as_deref(), Some(ID));
    assert_eq!(b.wishes[0].state, "running");
    assert!(!b.wishes[0].log.is_empty());
    let sub = client_ok!(c.home(), c.subscribe(b.cursor));
    assert!(client_ok!(c.home(), sub.recv_timeout(Duration::from_secs(4))).is_some());
    finish(s, &c);
}
#[test]
fn cli_wish_works_without_service() {
    let x = Setup::new();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["wish", "test"])
        .current_dir(x.temp.path())
        .env("AXIOM_HOME", &x.home)
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env("FAKE_CLAUDE_SCENARIO", "ok")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn service_single_instance_per_canonical_home() {
    let x = Setup::new();
    let lock = service::lock::acquire(&x.home).unwrap();
    let err = match service::lock::acquire(&x.home.join(".")) {
        Ok(_) => panic!("lock unexpectedly acquired"),
        Err(e) => e,
    };
    assert_eq!(err.kind, "already_running");
    assert!(err.message.contains(&std::process::id().to_string()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["serve", "--no-tray"])
        .env("AXIOM_HOME", &x.home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    drop(lock);
}
#[test]
fn service_quit_cancel_keeps_work_running() {
    let x = Setup::new();
    let (s, c, _, d, _) = x.start();
    client_ok!(c.home(), c.start(x.req()));
    let snap = running(&c);
    assert_eq!(c.quit(false).unwrap_err().kind, "running");
    let sub = client_ok!(c.home(), c.subscribe(snap.cursor));
    s.tray_actions().send(tray::TrayAction::Quit).unwrap();
    let mut quit = false;
    let mut text = false;
    until(|| {
        if let Some(e) = client_ok!(c.home(), sub.recv_timeout(Duration::from_millis(300))) {
            quit |= e.kind == "quit_requested";
            text |= e.kind == "text";
        }
        quit && text
    });
    until(|| d.lock().unwrap().iter().any(|v| v == "confirm_quit(1)"));
    finish(s, &c);
    assert!(service::lock::acquire(&x.home).is_ok());
}
#[test]
fn tray_status_and_actions_follow_service() {
    let x = Setup::new();
    let (s, c, t, d, _) = x.start();
    client_ok!(c.home(), c.start(x.req()));
    running(&c);
    s.tray_actions().send(tray::TrayAction::Open).unwrap();
    s.tray_actions().send(tray::TrayAction::PauseAll).unwrap();
    until(|| d.lock().unwrap().iter().any(|v| v == "open"));
    until(|| {
        t.lock()
            .unwrap()
            .iter()
            .filter(|v| v.as_str() == "Chronos: idle")
            .count()
            >= 2
    });
    assert!(t
        .lock()
        .unwrap()
        .iter()
        .any(|v| v == "Chronos: 1 wish running"));
    let store = service::recover::load(&x.home).unwrap();
    assert!(store.sessions[0].user_stopped);
    finish(s, &c);
}
#[test]
fn login_start_is_opt_in_and_resumes_interrupted_only() {
    let x = Setup::new();
    let (s, c, _, _, l) = x.start();
    assert!(!client_ok!(c.home(), c.snapshot()).login_start);
    assert!(l.lock().unwrap().is_empty());
    client_ok!(c.home(), c.set_login_start(true));
    assert!(l.lock().unwrap()[0].contains("serve"));
    finish(s, &c);
    let stopped = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    for id in [ID, stopped] {
        goal::open(&x.home, id, "wish", Some(x.temp.path().to_str().unwrap())).unwrap();
    }
    let store = service::recover::Store {
        instance: "old".into(),
        pid: 1,
        sessions: [(ID, false), (stopped, true)]
            .into_iter()
            .map(|(id, stop)| service::recover::Session {
                handle: id.into(),
                session: Some(id.into()),
                cwd: x.temp.path().to_string_lossy().into(),
                text: "wish".into(),
                state: "running".into(),
                user_stopped: stop,
                recovered: false,
            })
            .collect(),
    };
    service::recover::save(&x.home, &store).unwrap();
    let t = Arc::new(Mutex::new(vec![]));
    let s = x.server(true, t.clone(), t.clone(), t);
    let c = client_ok!(x.home, Client::connect(&x.home));
    until(|| {
        client_ok!(c.home(), c.snapshot())
            .wishes
            .iter()
            .any(|w| w.session.as_deref() == Some(ID) && w.state == "running")
    });
    let snap = client_ok!(c.home(), c.snapshot());
    assert_ne!(
        snap.wishes
            .iter()
            .find(|w| w.session.as_deref() == Some(stopped))
            .unwrap()
            .state,
        "running"
    );
    client_ok!(c.home(), c.set_login_start(false));
    finish(s, &c);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["serve", "--login"])
        .env("AXIOM_HOME", &x.home)
        .output()
        .unwrap();
    assert!(out.status.success());
}
#[test]
fn service_waits_for_space_then_resumes() {
    let x = Setup::new();
    let (s, c, _, _, _) = x.start();
    let mut req = x.req();
    req.need_bytes = Some(u64::MAX);
    let h = client_ok!(c.home(), c.start(req));
    let a = client_ok!(c.home(), c.snapshot());
    assert_eq!(a.wishes[0].state, "waiting_for_space");
    assert!(a.wishes[0].disk_line.is_some());
    assert_eq!(client_ok!(c.home(), c.resume(&h, None)), h);
    assert_eq!(
        client_ok!(c.home(), c.snapshot()).wishes[0].state,
        "waiting_for_space"
    );
    finish(s, &c);
}
#[test]
fn service_crash_releases_lock_and_preserves_resume() {
    let x = Setup::new();
    let mut child = ChildGuard(
        std::process::Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args(["serve", "--no-tray"])
            .env("AXIOM_HOME", &x.home)
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .spawn()
            .unwrap(),
    );
    until(|| Client::connect(&x.home).is_ok());
    let c = client_ok!(x.home, Client::connect(&x.home));
    client_ok!(c.home(), c.start(x.req()));
    running(&c);
    goal::open(&x.home, ID, "wish", Some(x.temp.path().to_str().unwrap())).unwrap();
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    until(|| service::lock::acquire(&x.home).is_ok());
    let t = Arc::new(Mutex::new(vec![]));
    let s = x.server(false, t.clone(), t.clone(), t);
    let store = service::recover::load(&x.home).unwrap();
    assert!(store.sessions[0].recovered);
    assert_eq!(goal::load(&x.home, ID).unwrap().unwrap().status, "blocked");
    let other = x.temp.path().join("other");
    assert!(service::lock::acquire(&other).is_ok());
    let c = client_ok!(x.home, Client::connect(&x.home));
    finish(s, &c);
}
