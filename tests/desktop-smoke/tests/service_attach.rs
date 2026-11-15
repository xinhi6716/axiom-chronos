//! Desktop windows attach to a real in-process service, not the demo backend.
extern crate self as tempfile;

// `tests/common` also exposes a tempfile helper; this smoke crate has no tempfile dependency.
// Only common::python is used here, but its other functions still have to type-check.
pub struct TempDir(std::path::PathBuf);
impl TempDir {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub fn tempdir() -> std::io::Result<TempDir> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "axiom-desktop-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path)?;
    Ok(TempDir(path))
}

#[path = "../../common/mod.rs"]
mod common;

use axiom::{
    desktop::{
        self, demo, i18n,
        model::{Action, AppState, LogLevel, RunStatus, WishRequest},
        settings, theme, Shell, Store,
    },
    service::{
        client::Client,
        login,
        server::{Server, ServerOptions},
        tray, RunningService,
    },
    session,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    component::ThemeMode, div, px, size, AnyWindowHandle, AppContext as _, BorrowAppContext as _, Bounds, Context, Entity,
    IntoElement, ParentElement as _, Point, Render, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

static ENV: Mutex<()> = Mutex::new(());
const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

struct Fixture {
    _lock: std::sync::MutexGuard<'static, ()>,
    old_env: Vec<(&'static str, Option<std::ffi::OsString>)>,
    temp: TempDir,
    home: PathBuf,
    server: Option<RunningService>,
    login_calls: Arc<Mutex<Vec<String>>>,
}
impl Fixture {
    fn new() -> Self {
        let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let keys = [
            "FAKE_CLAUDE_SCENARIO",
            "FAKE_CLAUDE_RECORD",
            "FAKE_CLAUDE_SESSION",
            "FAKE_CLAUDE_TICK_MS",
            "AXIOM_FAKE_DISK",
            "AXIOM_HEARTBEAT_MS",
        ];
        let old_env = keys.into_iter().map(|k| (k, std::env::var_os(k))).collect();
        std::env::set_var("FAKE_CLAUDE_SCENARIO", "tick");
        std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record.json"));
        std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
        std::env::set_var("FAKE_CLAUDE_TICK_MS", "100");
        std::env::remove_var("AXIOM_FAKE_DISK");
        std::env::set_var("AXIOM_HEARTBEAT_MS", "100");
        let config_home = home.clone();
        let bin = temp.path().join("axiom");
        let fake =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fakes/fake_claude.py");
        let claude = vec![
            common::python(),
            "-I".into(),
            fake.to_string_lossy().into_owned(),
        ];
        let calls = || Arc::new(Mutex::new(Vec::new()));
        let login_calls = calls();
        let server = Server::start(ServerOptions {
            home: home.clone(),
            config: Arc::new(move || session::Config {
                claude: Ok(claude.clone()),
                home: config_home.clone(),
                bin: bin.clone(),
                clock: session::Clock::system(),
            }),
            tray: Box::new(tray::Recording::new(calls())),
            desktop: Box::new(tray::Recording::new(calls())),
            login: Box::new(login::Recording::new(login_calls.clone())),
            login_launch: false,
        })
        .unwrap();
        Self {
            _lock: lock,
            old_env,
            temp,
            home,
            server: Some(server),
            login_calls,
        }
    }
    fn client(&self) -> Client {
        Client::connect(&self.home).unwrap()
    }
    fn request(&self) -> WishRequest {
        WishRequest {
            cwd: self.temp.path().to_path_buf(),
            wish: "smoke wish".into(),
            creative: false,
            max_turns: None,
            advanced_research: false,
            council_budgets: None,
            codex_max_parallel: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.quit(true);
            let _ = server.wait();
        }
        for (key, value) in &self.old_env {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn init(cx: &mut TestAppContext) {
    // The real service wakes the GPUI inbox from the engine thread.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
}
fn open(cx: &mut TestAppContext, home: PathBuf) -> (AnyWindowHandle, Entity<AppState>) {
    open_with_request(cx, Some(home), false)
}
fn open_with_request(cx: &mut TestAppContext, home: Option<PathBuf>, requested: bool) -> (AnyWindowHandle, Entity<AppState>) {
    let mut initial = demo::state();
    initial.feeds.quit_requested = requested;
    initial.runs.clear();
    initial.current = None;
    initial.memory.clear();
    initial.checks.clear();
    initial.board.clear();
    initial.codex_slots = None;
    initial.codex_models.clear();
    initial.live_models.clear();
    initial.quota = Default::default();
    let state = cx.new(|_| initial);
    let bounds = Bounds {
        origin: Point::default(),
        size: size(px(1440.), px(900.)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Shell::new(state.clone(), false, window, cx))
            })
        })
        .unwrap();
    if let Some(home) = home {
        cx.update(|cx| desktop::connect_service(state.clone(), home, None, cx));
    }
    (window, state)
}
fn until(
    cx: &mut TestAppContext,
    description: &str,
    mut condition: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn close(cx: &mut TestAppContext, window: AnyWindowHandle) {
    assert!(
        VisualTestContext::from_window(window, cx).simulate_close(),
        "window must close without confirmation"
    );
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn window_close_keeps_fake_wish_running_and_reattaches(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let client = fixture.client();
    init(cx);
    let (window, state) = open(cx, fixture.home.clone());
    cx.update(|cx| state.update(cx, |s, cx| s.start_wish(fixture.request(), cx)));
    until(cx, "initial desktop progress", |cx| {
        cx.update(|cx| {
            state.read(cx).runs.iter().any(|r| {
                r.session.as_ref() == SESSION
                    && r.hosted
                    && r.log.iter().any(|l| l.text.starts_with("tick "))
            })
        })
    });
    let before = client.snapshot().unwrap();
    let wish = before
        .wishes
        .iter()
        .find(|w| w.session.as_deref() == Some(SESSION))
        .unwrap();
    assert_eq!(wish.state, "running");
    let handle = wish.handle.clone();
    let prior = wish
        .log
        .iter()
        .find(|l| l.text.starts_with("tick "))
        .unwrap()
        .text
        .clone();
    close(cx, window);
    drop(state);
    until(cx, "service progress after window close", |_| {
        let snapshot = client.snapshot().unwrap();
        snapshot.wishes.iter().any(|w| {
            w.handle == handle
                && w.session.as_deref() == Some(SESSION)
                && w.state == "running"
                && w.log.len() > wish.log.len()
        })
    });
    let (window, state) = open(cx, fixture.home.clone());
    until(cx, "reattached session and its prior log", |cx| {
        cx.update(|cx| {
            state.read(cx).runs.iter().any(|r| {
                r.session.as_ref() == SESSION
                    && r.hosted
                    && r.status == RunStatus::Active
                    && r.log.iter().any(|l| l.text.as_ref() == prior)
            })
        })
    });
    cx.update(|cx| {
        state.update(cx, |s, cx| {
            let index = s
                .runs
                .iter()
                .position(|r| r.session.as_ref() == SESSION)
                .unwrap();
            s.open_run(index, cx);
            s.interject("hello from second window", cx);
        })
    });
    until(cx, "interjection received by fake Claude", |_| {
        client.snapshot().unwrap().wishes.iter().any(|w| {
            w.handle == handle
                && w.log
                    .iter()
                    .any(|l| l.text.contains("heard: hello from second window"))
        })
    });
    cx.update(|cx| state.update(cx, |s, cx| s.stop_current(cx)));
    until(cx, "service stopping the reattached wish", |_| {
        client
            .snapshot()
            .unwrap()
            .wishes
            .iter()
            .any(|w| w.handle == handle && w.state == "ended")
    });
    close(cx, window);
}

#[gpui_kit::test]
fn disk_pause_reaches_report(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let storage = fixture.home.join("storage");
    std::fs::create_dir_all(&storage).unwrap();
    std::fs::write(
        storage.join("policy.json"),
        r#"{"low_disk_bytes":100,"reserve_bytes":10,"estimates":{"wish":20,"batch":20}}"#,
    )
    .unwrap();
    let disk = fixture.temp.path().join("disk.json");
    std::fs::write(&disk, r#"{"free_bytes":10000}"#).unwrap();
    std::env::set_var("AXIOM_FAKE_DISK", &disk);
    axiom::goal::open(&fixture.home, SESSION, "wish", fixture.temp.path().to_str()).unwrap();
    let client = fixture.client();
    init(cx);
    let (window, state) = open(cx, fixture.home.clone());
    cx.update(|cx| state.update(cx, |s, cx| s.start_wish(fixture.request(), cx)));
    until(cx, "running wish before disk pressure", |cx| {
        cx.update(|cx| {
            state.read(cx).runs.iter().any(|r| {
                r.session.as_ref() == SESSION && r.log.iter().any(|l| l.text.starts_with("tick "))
            })
        })
    });
    axiom::base::atomic_write(&disk, br#"{"free_bytes":1}"#).unwrap();
    let mut line = None;
    until(cx, "service disk pause", |_| {
        let snapshot = client.snapshot().unwrap();
        if let Some(w) = snapshot
            .wishes
            .iter()
            .find(|w| w.session.as_deref() == Some(SESSION) && w.state == "waiting_for_space")
        {
            line = w.disk_line.clone();
        }
        line.is_some()
    });
    let line = line.unwrap();
    until(cx, "disk warning and report", |cx| {
        cx.update(|cx| {
            let s = state.read(cx);
            s.runs.iter().any(|r| {
                r.session.as_ref() == SESSION
                    && r.log
                        .iter()
                        .any(|l| l.level == LogLevel::Warn && l.text.as_ref() == line)
            }) && s.errors.iter().any(|e| {
                e.action == Action::Session
                    && e.session.as_deref() == Some(SESSION)
                    && e.message.as_ref() == line
            })
        })
    });
    close(cx, window);
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn start_running(cx: &mut TestAppContext, fixture: &Fixture, state: &Entity<AppState>) -> String {
    cx.update(|cx| state.update(cx, |s, cx| s.start_wish(fixture.request(), cx)));
    until(cx, "wish producing ticks", |cx| {
        cx.update(|cx| {
            state.read(cx).runs.iter().any(|r| {
                r.session.as_ref() == SESSION && r.log.iter().any(|l| l.text.starts_with("tick "))
            })
        })
    });
    fixture
        .client()
        .snapshot()
        .unwrap()
        .wishes
        .iter()
        .find(|w| w.session.as_deref() == Some(SESSION))
        .unwrap()
        .handle
        .clone()
}

fn show_quit(cx: &mut TestAppContext, window: AnyWindowHandle, state: &Entity<AppState>) {
    cx.update(|cx| {
        state.update(cx, |s, cx| {
            s.feeds.quit_requested = true;
            cx.notify();
        })
    });
    assert_quit_texts(cx, window);
}

fn assert_quit_texts(cx: &mut TestAppContext, window: AnyWindowHandle) {
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        let texts = gpui_kit::base::test_support::texts(window);
        for expected in ["Quit Chronos?", "Stop and quit", "Keep working"] {
            assert!(
                texts
                    .iter()
                    .any(|text| text.visible() && text.text() == expected),
                "missing {expected}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn startup_confirm_quit_is_visible_with_mini_preference(cx: &mut TestAppContext) {
    init(cx);
    let options = desktop::parse_options(&["--demo".into(), "--mode".into(), "mini".into(), "--confirm-quit".into()]).unwrap();
    cx.update(|cx| cx.update_global::<Store, _>(|store, _| store.settings.mode = options.mode.unwrap()));
    let (window, state) = open_with_request(cx, None, options.confirm_quit);
    assert_quit_texts(cx, window);
    assert!(!cx.update(|cx| state.read(cx).feeds.quit_requested));
    close(cx, window);
}

#[gpui_kit::test]
fn quit_request_opens_dialog_and_keep_working_dismisses_it(cx: &mut TestAppContext) {
    init(cx);
    let (window, state) = open_with_request(cx, None, false);
    show_quit(cx, window, &state);
    click(cx, window, "keep-working");
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert!(!gpui_kit::base::test_support::texts(window).iter()
            .any(|text| text.visible() && text.text() == "Quit Chronos?"));
    }).unwrap();
    close(cx, window);
}

#[gpui_kit::test]
fn quit_without_backend_reports_and_keeps_dialog_open(cx: &mut TestAppContext) {
    init(cx);
    let (window, state) = open_with_request(cx, None, false);
    show_quit(cx, window, &state);
    click(cx, window, "stop-and-quit");
    assert!(cx.update(|cx| state.read(cx).errors.iter().any(|error| error.action == Action::Engine)));
    assert_quit_texts(cx, window);
    close(cx, window);
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn quit_dialog_keep_working_leaves_service_running(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let client = fixture.client();
    init(cx);
    let (window, state) = open(cx, fixture.home.clone());
    let handle = start_running(cx, &fixture, &state);
    let before = client
        .snapshot()
        .unwrap()
        .wishes
        .into_iter()
        .find(|w| w.handle == handle)
        .unwrap();
    show_quit(cx, window, &state);
    click(cx, window, "keep-working");
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert!(!gpui_kit::base::test_support::texts(window)
            .iter()
            .any(|t| t.visible() && t.text() == "Quit Chronos?"));
    })
    .unwrap();
    until(cx, "wish continues after Keep working", |_| {
        client
            .snapshot()
            .unwrap()
            .wishes
            .iter()
            .any(|w| w.handle == handle && w.state == "running" && w.log.len() > before.log.len())
    });
    close(cx, window);
}

#[gpui_kit::test]
fn quit_dialog_stop_and_quit_stops_wish_and_service(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new();
    init(cx);
    let (window, state) = open(cx, fixture.home.clone());
    let handle = start_running(cx, &fixture, &state);
    show_quit(cx, window, &state);
    click(cx, window, "stop-and-quit");
    let server = fixture.server.take().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(server.wait());
    });
    until(cx, "service exit", |_| match finished.try_recv() {
        Ok(code) => {
            assert_eq!(code, 0, "service must exit successfully");
            true
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => false,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!("service waiter failed"),
    });
    let store = axiom::service::recover::load(&fixture.home).unwrap();
    assert!(
        store
            .sessions
            .iter()
            .any(|s| s.handle == handle && s.user_stopped && s.state == "ended"),
        "quit must stop the running wish"
    );
}

#[gpui_kit::test]
fn quit_dialog_stays_open_when_service_is_gone(cx: &mut TestAppContext) {
    let mut fixture = Fixture::new();
    init(cx);
    let (window, state) = open(cx, fixture.home.clone());
    start_running(cx, &fixture, &state);
    let server = fixture.server.take().unwrap();
    server.quit(true).unwrap();
    server.wait();
    show_quit(cx, window, &state);
    let last_error = cx.update(|cx| state.read(cx).errors.last().map_or(0, |error| error.id));
    click(cx, window, "stop-and-quit");
    assert!(
        cx.update_window(window, |_, _, _| ()).is_ok(),
        "dialog window must stay open after quit fails"
    );
    until(cx, "quit failure report", |cx| {
        cx.update(|cx| state.read(cx).errors.iter().any(|error| error.id > last_error && error.action == Action::Engine))
    });
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert!(gpui_kit::base::test_support::texts(window)
            .iter()
            .any(|t| t.visible() && t.text() == "Quit Chronos?"));
    })
    .unwrap();
    close(cx, window);
}

struct LoginRow {
    state: Entity<AppState>,
    _subscription: gpui_kit::Subscription,
}
impl LoginRow {
    fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _subscription: subscription,
        }
    }
}
impl Render for LoginRow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().child(settings::login_start_row(&self.state, cx))
    }
}

#[gpui_kit::test]
fn login_start_row_reflects_and_toggles(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    init(cx);
    let state = cx.new(|_| {
        let mut state = demo::state();
        // A distinct initial value proves the unchecked row came from the service snapshot.
        state.feeds.login_start = true;
        state
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(1440.), px(900.)),
        })),
        ..Default::default()
    };
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| LoginRow::new(state.clone(), cx))
            })
        })
        .unwrap();
    cx.update(|cx| desktop::connect_service(state.clone(), fixture.home.clone(), None, cx));
    until(cx, "initial login setting snapshot", |cx| {
        cx.update(|cx| state.read(cx).backend.is_some() && !state.read(cx).feeds.login_start)
    });
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert_eq!(window.find("login-start").checked(), Some(false));
        assert_eq!(window.find("login-start").label(), Some("Start at login"));
    })
    .unwrap();
    click(cx, window, "login-start");
    until(cx, "login adapter and desktop snapshot", |cx| {
        fixture
            .login_calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.starts_with("enable("))
            && fixture.client().snapshot().unwrap().login_start
            && cx.update(|cx| state.read(cx).feeds.login_start)
    });
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert_eq!(window.find("login-start").checked(), Some(true))
    })
    .unwrap();
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
}
