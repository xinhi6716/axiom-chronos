mod common;
use axiom::{
    base::{Failure, Res},
    routines::{
        self,
        store::{Outcome, Store},
        LaunchRequest, Routine, Schedule, WishLauncher,
    },
    seams::{ApprovalRequest, Decision, FakeApproval, FakeClock, PersonApproval},
    service::{
        self,
        client::Client,
        login,
        scheduler::{EnabledRoutineApproval, SchedulerConfig},
        server::{Server, ServerOptions},
        tray, StartRequest,
    },
    session,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

fn routine(enabled: bool) -> Routine {
    Routine {
        id: "test".into(),
        project_dir: ".".into(),
        wish: "test wish".into(),
        enabled,
        schedule: Some(Schedule {
            every_minutes: 1,
            start_ms: 1_000,
        }),
        triggers: vec![],
    }
}
fn options(home: &Path, login_launch: bool) -> ServerOptions {
    let cfg_home = home.to_path_buf();
    let calls = Arc::new(Mutex::new(vec![]));
    ServerOptions {
        home: home.to_path_buf(),
        config: Arc::new(move || {
            let mut c = session::Config::from_env();
            c.home = cfg_home.clone();
            c
        }),
        tray: Box::new(tray::Recording::new(calls.clone())),
        desktop: Box::new(tray::Recording::new(calls.clone())),
        login: Box::new(login::Recording::new(calls)),
        login_launch,
    }
}
fn config(
    clock: Arc<FakeClock>,
    launcher: Arc<dyn WishLauncher>,
    approval: Arc<dyn PersonApproval>,
) -> SchedulerConfig {
    SchedulerConfig {
        clock,
        launcher: Some(launcher),
        approval,
        interval: Duration::from_millis(25),
    }
}
fn until(mut test: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !test() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn finish(s: service::RunningService) {
    s.quit(true).unwrap();
    assert_eq!(s.wait(), 0);
}

#[test]
fn schedule_and_restart_launch_once() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(true)).unwrap();
    let clock = Arc::new(FakeClock::new(999));
    let launcher = Arc::new(routines::FakeLauncher::new());
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            clock.clone(),
            launcher.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert!(launcher.launched().is_empty());
    clock.set(1_000);
    until(|| launcher.launched().len() == 1);
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(launcher.launched().len(), 1);
    finish(s);
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(clock, launcher.clone(), Arc::new(FakeApproval::approving())),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(120));
    finish(s);
    assert_eq!(launcher.launched().len(), 1);
}
#[test]
fn disabled_and_denied_do_not_launch() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(false)).unwrap();
    let clock = Arc::new(FakeClock::new(1_000));
    let launcher = Arc::new(routines::FakeLauncher::new());
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(clock, launcher.clone(), Arc::new(FakeApproval::denying())),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert!(store.state().unwrap().log.is_empty());
    assert!(launcher.launched().is_empty());
    store.put(&routine(true)).unwrap();
    until(|| !store.state().unwrap().log.is_empty());
    finish(s);
    assert_eq!(store.state().unwrap().log[0].outcome, Outcome::Denied);
    assert!(launcher.launched().is_empty());
}
#[test]
fn enabled_approval_checks_current_store_and_request() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(true)).unwrap();
    let approval = EnabledRoutineApproval::new(temp.path());
    let mut req = ApprovalRequest {
        subject: "routine:test".into(),
        action: "launch_wish".into(),
        summary: String::new(),
        digest: String::new(),
    };
    assert_eq!(approval.decide(&req), Decision::Approved);
    store.put(&routine(false)).unwrap();
    assert_eq!(approval.decide(&req), Decision::Denied);
    req.subject = "routine:unknown".into();
    assert_eq!(approval.decide(&req), Decision::Denied);
    req.subject = "routine:test".into();
    req.action = "other".into();
    assert_eq!(approval.decide(&req), Decision::Denied);
    std::fs::write(temp.path().join("routines/routines.json"), b"invalid").unwrap();
    req.action = "launch_wish".into();
    assert_eq!(approval.decide(&req), Decision::Denied);
}
struct Fail;
impl WishLauncher for Fail {
    fn launch(&self, _: &LaunchRequest) -> Res<String> {
        Err(Failure::new("test", "launch failed"))
    }
}
#[test]
fn failed_launch_is_logged() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(true)).unwrap();
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            Arc::new(FakeClock::new(1_000)),
            Arc::new(Fail),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    until(|| !store.state().unwrap().log.is_empty());
    finish(s);
    assert!(
        matches!(&store.state().unwrap().log[0].outcome, Outcome::Failed { message } if message.contains("launch failed"))
    );
}
struct Block {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    calls: AtomicUsize,
}
impl WishLauncher for Block {
    fn launch(&self, _: &LaunchRequest) -> Res<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok("ok".into())
    }
}
#[test]
fn ipc_remains_responsive_during_launch_and_shutdown_joins() {
    let temp = common::temp();
    Store::open(temp.path())
        .unwrap()
        .put(&routine(true))
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let launcher = Arc::new(Block {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let clock = Arc::new(FakeClock::new(1_000));
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            clock.clone(),
            launcher.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let client = Client::connect(temp.path()).unwrap();
    assert!(client.status().is_ok());
    let (quit_tx, quit_rx) = mpsc::channel();
    let ipc_quit = std::thread::spawn(move || quit_tx.send(client.quit(true)).unwrap());
    let quit_result = quit_rx.recv_timeout(Duration::from_secs(2));
    if quit_result.is_err() {
        release_tx.send(()).unwrap();
        let _ = ipc_quit.join();
        let _ = s.wait();
        panic!("IPC quit blocked on launcher");
    }
    quit_result.unwrap().unwrap();
    ipc_quit.join().unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let join = std::thread::spawn(move || done_tx.send(s.wait()).unwrap());
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    join.join().unwrap();
    clock.advance(60_000);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn disabled_login_start_does_not_start_scheduler() {
    let temp = common::temp();
    Store::open(temp.path())
        .unwrap()
        .put(&routine(true))
        .unwrap();
    let launcher = Arc::new(routines::FakeLauncher::new());
    let s = Server::start_with_scheduler(
        options(temp.path(), true),
        config(
            Arc::new(FakeClock::new(1_000)),
            launcher.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    finish(s);
    assert!(launcher.launched().is_empty());
}
#[test]
fn tick_errors_are_reported_and_later_ticks_continue() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    std::fs::write(temp.path().join("routines/routines.json"), b"invalid").unwrap();
    let launcher = Arc::new(routines::FakeLauncher::new());
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            Arc::new(FakeClock::new(1_000)),
            launcher.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    until(|| temp.path().join("service/scheduler-error.txt").exists());
    std::fs::remove_file(temp.path().join("routines/routines.json")).unwrap();
    store.put(&routine(true)).unwrap();
    until(|| launcher.launched().len() == 1);
    finish(s);
}
#[test]
fn quit_wakes_long_scheduler_interval_and_ipc_quit_joins() {
    let temp = common::temp();
    Store::open(temp.path())
        .unwrap()
        .put(&routine(true))
        .unwrap();
    let clock = Arc::new(FakeClock::new(1_000));
    let launcher = Arc::new(routines::FakeLauncher::new());
    let probe = Arc::downgrade(&launcher);
    let mut cfg = config(clock, launcher, Arc::new(FakeApproval::approving()));
    cfg.interval = Duration::from_secs(60);
    let s = Server::start_with_scheduler(options(temp.path(), false), cfg).unwrap();
    until(|| {
        probe
            .upgrade()
            .is_some_and(|launcher| !launcher.launched().is_empty())
    });
    let client = Client::connect(temp.path()).unwrap();
    client.quit(true).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || tx.send(s.wait()).unwrap());
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    assert!(probe.upgrade().is_none());
}
#[test]
fn quit_joins_an_inflight_launcher() {
    let temp = common::temp();
    Store::open(temp.path())
        .unwrap()
        .put(&routine(true))
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let launcher = Arc::new(Block {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let clock = Arc::new(FakeClock::new(1_000));
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            clock.clone(),
            launcher.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (tx, rx) = mpsc::channel();
    let (quit_tx, quit_rx) = mpsc::channel();
    let join = std::thread::spawn(move || {
        s.quit(true).unwrap();
        quit_tx.send(()).unwrap();
        tx.send(s.wait()).unwrap();
    });
    let quit_prompt = quit_rx.recv_timeout(Duration::from_secs(1));
    if quit_prompt.is_err() {
        release_tx.send(()).unwrap();
        join.join().unwrap();
        panic!("quit blocked on launcher");
    }
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    release_tx.send(()).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
    join.join().unwrap();
    clock.advance(60_000);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(launcher.calls.load(Ordering::SeqCst), 1);
}
#[test]
fn direct_quit_wakes_long_interval() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(true)).unwrap();
    let launcher = Arc::new(routines::FakeLauncher::new());
    let mut cfg = config(
        Arc::new(FakeClock::new(1_000)),
        launcher.clone(),
        Arc::new(FakeApproval::approving()),
    );
    cfg.interval = Duration::from_secs(60);
    let s = Server::start_with_scheduler(options(temp.path(), false), cfg).unwrap();
    until(|| !store.state().unwrap().log.is_empty());
    let (tx, rx) = mpsc::channel();
    let join = std::thread::spawn(move || {
        finish(s);
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    join.join().unwrap();
}

fn fake_host_options(home: &Path) -> ServerOptions {
    let mut opts = options(home, false);
    let home = home.to_path_buf();
    opts.config = Arc::new(move || session::Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.clone(),
        bin: home.join("axiom"),
        clock: session::Clock::system(),
    });
    opts
}

#[test]
fn default_launcher_starts_through_own_service() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    let mut r = routine(true);
    r.project_dir = temp.path().into();
    store.put(&r).unwrap();
    let cfg = SchedulerConfig {
        clock: Arc::new(FakeClock::new(1_000)),
        launcher: None,
        approval: Arc::new(EnabledRoutineApproval::new(temp.path())),
        interval: Duration::from_millis(25),
    };
    let s = Server::start_with_scheduler(fake_host_options(temp.path()), cfg).unwrap();
    until(|| !store.state().unwrap().log.is_empty());
    let state = store.state().unwrap();
    assert!(
        matches!(&state.log[0].outcome, Outcome::Launched { .. }),
        "{:?}",
        state.log
    );
    assert_eq!(
        Client::connect(temp.path())
            .unwrap()
            .snapshot()
            .unwrap()
            .wishes
            .len(),
        1
    );
    finish(s);
}

#[test]
fn production_config_and_server_start_run_scheduler() {
    let temp = common::temp();
    let cfg = SchedulerConfig::production(temp.path());
    assert_eq!(cfg.interval, Duration::from_secs(60));
    assert!(cfg.launcher.is_none());
    let store = Store::open(temp.path()).unwrap();
    let mut r = routine(true);
    r.schedule.as_mut().unwrap().start_ms = 0;
    r.project_dir = temp.path().into();
    store.put(&r).unwrap();
    let s = Server::start(fake_host_options(temp.path())).unwrap();
    until(|| !store.state().unwrap().log.is_empty());
    assert!(matches!(
        store.state().unwrap().log[0].outcome,
        Outcome::Launched { .. }
    ));
    finish(s);
}

#[test]
fn stop_preserves_unclaimed_slots_for_restart() {
    let temp = common::temp();
    let store = Store::open(temp.path()).unwrap();
    store.put(&routine(true)).unwrap();
    let mut second = routine(true);
    second.id = "second".into();
    store.put(&second).unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let launcher = Arc::new(Block {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let cfg = config(
        Arc::new(FakeClock::new(1_000)),
        launcher,
        Arc::new(FakeApproval::approving()),
    );
    let s = Server::start_with_scheduler(fake_host_options(temp.path()), cfg).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    s.quit(true).unwrap();
    release_tx.send(()).unwrap();
    let _ = release_tx.send(());
    assert_eq!(s.wait(), 0);
    let state = store.state().unwrap();
    assert!(!state.last_due_ms.contains_key("second"), "{state:?}");
    let replacement = Arc::new(routines::FakeLauncher::new());
    let s = Server::start_with_scheduler(
        options(temp.path(), false),
        config(
            Arc::new(FakeClock::new(1_000)),
            replacement.clone(),
            Arc::new(FakeApproval::approving()),
        ),
    )
    .unwrap();
    until(|| replacement.launched().len() == 1);
    finish(s);
    assert_eq!(replacement.launched()[0].routine, "second");
}

#[test]
fn start_is_accepted_during_quit() {
    let temp = common::temp();
    Store::open(temp.path())
        .unwrap()
        .put(&routine(true))
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let launcher = Arc::new(Block {
        entered: entered_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let cfg = config(
        Arc::new(FakeClock::new(1_000)),
        launcher,
        Arc::new(FakeApproval::approving()),
    );
    let s = Server::start_with_scheduler(fake_host_options(temp.path()), cfg).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let client = match Client::connect(temp.path()) {
        Ok(client) => client,
        Err(error) => {
            release_tx.send(()).unwrap();
            finish(s);
            panic!("connect: {error}");
        }
    };
    s.quit(true).unwrap();
    let start = client.start(StartRequest {
        wish: "during quit".into(),
        cwd: temp.path().to_string_lossy().into(),
        need_bytes: Some(u64::MAX),
        ..Default::default()
    });
    release_tx.send(()).unwrap();
    assert_eq!(s.wait(), 0);
    assert!(start.is_ok(), "{start:?}");
}
