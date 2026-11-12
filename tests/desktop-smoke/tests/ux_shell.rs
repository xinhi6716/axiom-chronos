mod ux_support;

use axiom::desktop::{
    demo,
    i18n::Lang,
    model::{AppState, Backend, Screen, WishRequest},
    ux::{seams::*, Page},
    Store,
};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use std::{
    cell::Cell,
    ffi::OsString,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PALETTE: &str = if cfg!(target_os = "macos") {
    "cmd-k"
} else {
    "ctrl-k"
};
const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};
const COPY: &str = if cfg!(target_os = "macos") {
    "cmd-c"
} else {
    "ctrl-c"
};

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    ux_support::capture(cx, window);
}
fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> bool {
    let id: gpui_kit::ElementId = id.to_owned().into();
    cx.update_window(window, |_, window, _| {
        window.try_find(id.clone()).is_some()
            || window.elements().iter().any(|e| e.path().contains(&id))
            || gpui_kit::base::test_support::texts(window)
                .iter()
                .any(|t| t.path().contains(&id))
    })
    .unwrap()
}
fn shows(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str, text: &str) -> bool {
    ux_support::shows(&ux_support::capture(cx, window), id, text)
}
fn button_label(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    id: &'static str,
) -> Option<String> {
    cx.update_window(window, |_, window, _| {
        window.find(id).label().map(str::to_owned)
    })
    .unwrap()
}
fn palette(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    press(cx, window, PALETTE);
    input(cx, window, label);
    press(cx, window, "enter");
}
fn state_with(modes: Arc<dyn ModeEngine>) -> AppState {
    let mut state = demo::state();
    state.ux.seams.modes = Some(modes);
    state.ux.seams.storage = Some(Arc::new(FakeStorage::new(vec![], None)));
    state
}

#[gpui_kit::test]
fn palette_pages_and_escape(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let (window, state) = ux_support::open(
            cx,
            lang,
            state_with(Arc::new(FakeModes::new())),
            Screen::Run,
        );
        for (page, id, en, zh) in [
            (Page::Chat, "chat-page", "Chat", "對話"),
            (Page::QuickTask, "quick-page", "Quick task", "快速任務"),
            (Page::Plaza, "plaza-page", "Plaza", "廣場討論"),
            (Page::Storage, "storage-page", "Storage", "儲存空間"),
        ] {
            palette(cx, window, if lang == Lang::En { en } else { zh });
            assert_eq!(state.read_with(cx, |s, _| s.ux.page.clone()), Some(page));
            assert!(shown(cx, window, id), "{id}");
            press(cx, window, "escape");
            if state.read_with(cx, |s, _| s.ux.page.is_some()) || shown(cx, window, id) {
                failures.push(format!("{lang:?} {id}"));
                cx.update(|cx| state.update(cx, |s, cx| s.close_page(cx)));
                ux_support::capture(cx, window);
            }
        }
    }
    assert!(failures.is_empty(), "Escape did not close: {failures:?}");
}

#[gpui_kit::test]
fn chat_quick_and_handoff(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let fake = Arc::new(FakeModes::new());
        let (window, state) = ux_support::open(cx, lang, state_with(fake.clone()), Screen::Run);
        let count = state.read_with(cx, |s, _| s.runs.len());
        palette(cx, window, if lang == Lang::En { "Chat" } else { "對話" });
        click(cx, window, "chat-input");
        input(cx, window, "investigate carry");
        click(cx, window, "chat-send");
        assert!(shows(cx, window, "chat-page", "identify the next step"));
        assert_eq!(state.read_with(cx, |s, _| s.runs.len()), count);
        assert_eq!(
            button_label(cx, window, "mode-promote").as_deref(),
            Some(if lang == Lang::En {
                "Make it a quick task"
            } else {
                "轉為快速任務"
            })
        );
        click(cx, window, "mode-promote");
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::QuickTask)
        );
        click(cx, window, "quick-input");
        input(cx, window, "carry this quick task");
        click(cx, window, "quick-review-toggle");
        click(cx, window, "quick-start");
        let task = fake
            .quick("quick-2")
            .expect("quick task started after chat");
        assert!(task
            .request
            .context
            .as_deref()
            .unwrap()
            .contains("investigate carry"));
        assert!(shows(
            cx,
            window,
            "quick-review",
            if lang == Lang::En { "Review" } else { "審查" }
        ));
        for _ in 0..4 {
            fake.step_quick(&task.id);
        }
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
        assert!(shows(
            cx,
            window,
            "quick-step",
            if lang == Lang::En { "Done" } else { "完成" }
        ));
        assert!(shows(cx, window, "quick-result", "Changes verified."));
        assert_eq!(
            button_label(cx, window, "mode-promote").as_deref(),
            Some(if lang == Lang::En {
                "Make it a wish"
            } else {
                "轉為願望"
            })
        );
        click(cx, window, "mode-promote");
        assert!(state.read_with(cx, |s, _| s.composing));
        assert_eq!(state.read_with(cx, |s, _| s.ux.page.clone()), None);
        // The composer consumes composer_prefill during rendering; read its actual input.
        press(cx, window, SELECT_ALL);
        press(cx, window, COPY);
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some("carry this quick task")
        );
    }
}

// FakeModes supplies all routing; expose a speaker to exercise the active-seat rendering.
struct SpeakingModes(FakeModes);
impl ModeEngine for SpeakingModes {
    fn chat_send(&self, id: &str, text: &str) -> axiom::base::Res<()> {
        self.0.chat_send(id, text)
    }
    fn chat(&self, id: &str) -> ChatThread {
        self.0.chat(id)
    }
    fn quick_start(&self, request: &QuickRequest) -> axiom::base::Res<String> {
        self.0.quick_start(request)
    }
    fn quick(&self, id: &str) -> Option<QuickTask> {
        self.0.quick(id)
    }
    fn plaza_send(&self, id: &str, ask: &PlazaAsk) -> axiom::base::Res<RouteDecision> {
        self.0.plaza_send(id, ask)
    }
    fn plaza(&self, id: &str) -> PlazaRoom {
        let mut room = self.0.plaza(id);
        room.speaker = Some(Speaker::Astra);
        room
    }
    fn pin(&self, id: &str, message: u64) -> axiom::base::Res<()> {
        self.0.pin(id, message)
    }
    fn handoff(&self, from: &ModeRef) -> axiom::base::Res<String> {
        self.0.handoff(from)
    }
}

#[gpui_kit::test]
fn plaza_seat_glyphs(cx: &mut TestAppContext) {
    let mut missing = Vec::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let (window, _) = ux_support::open(
            cx,
            lang,
            state_with(Arc::new(SpeakingModes(FakeModes::new()))),
            Screen::Run,
        );
        palette(
            cx,
            window,
            if lang == Lang::En {
                "Plaza"
            } else {
                "廣場討論"
            },
        );
        let lines = ux_support::capture(cx, window);
        for glyph in ["⌛", "✦", "♃", "♊"] {
            if !ux_support::shows(&lines, "plaza-seat", glyph) {
                missing.push(format!("{lang:?}: {glyph}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "missing rendered seat glyphs: {missing:?}"
    );
}

#[gpui_kit::test]
fn plaza_routing_and_pins(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let fake = Arc::new(SpeakingModes(FakeModes::new()));
        let (window, _) = ux_support::open(cx, lang, state_with(fake.clone()), Screen::Run);
        palette(
            cx,
            window,
            if lang == Lang::En {
                "Plaza"
            } else {
                "廣場討論"
            },
        );
        let lines = ux_support::capture(cx, window);
        assert!(shown(cx, window, "plaza-speaker"));
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.path.iter().any(|id| id == "plaza-budget"))
                .count(),
            4
        );
        for (id, en, zh) in [
            ("plaza-round", "Round", "輪流發言"),
            ("plaza-debate", "Debate", "辯論"),
            ("plaza-vote", "Vote", "表決"),
        ] {
            assert_eq!(
                button_label(cx, window, id).as_deref(),
                Some(if lang == Lang::En { en } else { zh })
            );
        }
        for (button, text, reason) in [
            ("plaza-send", "@Astra hello", "Selected mentioned members"),
            ("plaza-round", "review", "Selected members by topic"),
            ("plaza-debate", "debate", "Selected debate participants"),
            ("plaza-vote", "vote", "Selected council members for vote"),
        ] {
            click(cx, window, "plaza-input");
            input(cx, window, text);
            click(cx, window, button);
            assert!(shows(cx, window, "plaza-route-reason", reason), "{button}");
        }
        let first = fake.plaza("plaza").messages[0].id;
        click(cx, window, ("pin", first as usize));
        assert!(shows(cx, window, "plaza-pins", "@Astra hello"));
    }
}

struct Home {
    path: PathBuf,
    prior: Option<OsString>,
}
impl Home {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("axiom-ux-shell-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let prior = std::env::var_os("AXIOM_HOME");
        std::env::set_var("AXIOM_HOME", &path);
        Self { path, prior }
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        if let Some(prior) = &self.prior {
            std::env::set_var("AXIOM_HOME", prior);
        } else {
            std::env::remove_var("AXIOM_HOME");
        }
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

#[gpui_kit::test]
fn extreme_strip_settings_and_persistence(cx: &mut TestAppContext) {
    let home = Home::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let settings = home.path.join("app-settings.json");
        if settings.exists() {
            std::fs::remove_file(settings).unwrap();
        }
        let mut initial = demo::state();
        initial.reduce_motion = true;
        let (window, state) = ux_support::open(cx, lang, initial, Screen::Doctor);
        cx.update(|cx| cx.set_global(Store::open(false)));
        assert!(!cx.update(|cx| Store::current(cx).ux.extreme));
        click(cx, window, "extreme-toggle");
        assert!(cx.update(|cx| Store::current(cx).ux.extreme));
        assert!(shown(cx, window, "extreme-current"));
        assert!(state.read_with(cx, |s, _| s.ux.extreme_flash.is_none()));
        click(cx, window, "extreme-switch");
        assert!(!cx.update(|cx| Store::current(cx).ux.extreme));
        assert!(!shown(cx, window, "extreme-current"));
        click(cx, window, "extreme-switch");
        assert!(cx.update(|cx| Store::current(cx).ux.extreme));
        assert!(shown(cx, window, "extreme-current"));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|cx| cx.set_global(Store::open(false)));
        assert!(cx.update(|cx| Store::current(cx).ux.extreme));
    }
}

#[gpui_kit::test]
fn low_disk_and_toggle_in_main_and_mini(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let mut app = cx.new_app();
        let cx = &mut app;
        let storage = Arc::new(FakeStorage::new(
            vec![],
            Some(DiskSpace {
                free_bytes: 1 << 30,
                total_bytes: 500 << 30,
            }),
        ));
        let mut initial = demo::state();
        initial.ux.seams.storage = Some(storage.clone());
        let (main, state) = ux_support::open(cx, lang, initial, Screen::Run);
        assert!(shown(cx, main, "low-disk"));
        click(cx, main, "low-disk");
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Storage)
        );
        assert!(shown(cx, main, "storage-page"));
        press(cx, main, "escape");
        storage.set_space(DiskSpace {
            free_bytes: 100_000_000_000,
            total_bytes: 500 << 30,
        });
        cx.update(|cx| state.update(cx, |_, cx| cx.notify()));
        ux_support::capture(cx, main);
        assert!(!shown(cx, main, "low-disk"));
        storage.set_space(DiskSpace {
            free_bytes: 1 << 30,
            total_bytes: 500 << 30,
        });
        cx.update(|cx| state.update(cx, |_, cx| cx.notify()));
        press(cx, main, "tab");

        // The test platform creates the mini window, then panics when minimizing main.
        let existing = cx.windows();
        let site = Arc::new(std::sync::Mutex::new(None));
        let captured = site.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(p) = info.location() {
                *captured.lock().unwrap() = Some((p.file().replace('\\', "/"), p.line()));
            }
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cx.update_window(main, |_, window, cx| {
                window.press(
                    if cfg!(target_os = "macos") {
                        "cmd-shift-m"
                    } else {
                        "ctrl-shift-m"
                    },
                    cx,
                )
            })
            .unwrap();
            cx.run_until_parked();
        }));
        std::panic::set_hook(previous);
        if let Err(error) = result {
            let allowed = site.lock().unwrap().as_ref().is_some_and(|(file, line)| {
                file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473
            }) && error.downcast_ref::<&str>() == Some(&"not implemented");
            if !allowed {
                std::panic::resume_unwind(error);
            }
        }
        let mini = cx
            .windows()
            .into_iter()
            .find(|w| !existing.contains(w))
            .expect("mini window");
        ux_support::capture(cx, mini);
        assert!(shown(cx, mini, "low-disk"));
        click(cx, mini, "extreme-toggle");
        assert!(cx.update(|cx| Store::current(cx).ux.extreme));
        assert!(shown(cx, mini, "extreme-current"));
        click(cx, mini, "extreme-toggle");
        assert!(!cx.update(|cx| Store::current(cx).ux.extreme));
        click(cx, mini, "low-disk");
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Storage)
        );
        storage.set_space(DiskSpace {
            free_bytes: 100_000_000_000,
            total_bytes: 500 << 30,
        });
        cx.update(|cx| state.update(cx, |_, cx| cx.notify()));
        ux_support::capture(cx, mini);
        assert!(!shown(cx, mini, "low-disk"));
    }
}

#[gpui_kit::test]
fn navigation_methods_persist_mode(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        cx.update(|cx| {
            gpui_kit::init(cx);
            axiom::desktop::i18n::set(lang, cx);
            cx.set_global(Store::open(true));
        });
        let state: Entity<AppState> = cx.new(|_| demo::state());
        assert_ne!(
            cx.update(|cx| Store::current(cx).ux.last_mode),
            ModeKind::Plaza
        );
        cx.update(|cx| state.update(cx, |s, cx| s.open_mode(ModeKind::Plaza, None, cx)));
        assert_eq!(
            cx.update(|cx| Store::current(cx).ux.last_mode),
            ModeKind::Plaza
        );
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Plaza)
        );
        cx.update(|cx| state.update(cx, |s, cx| s.open_page(Page::Storage, cx)));
        assert_eq!(
            state.read_with(cx, |s, _| s.ux.page.clone()),
            Some(Page::Storage)
        );
        cx.update(|cx| state.update(cx, |s, cx| s.close_page(cx)));
        assert_eq!(state.read_with(cx, |s, _| s.ux.page.clone()), None);
        cx.update(|cx| {
            state.update(cx, |s, cx| {
                s.open_mode(ModeKind::Wish, Some("carry".into()), cx)
            })
        });
        assert!(state.read_with(cx, |s, _| s.composing));
        assert_eq!(
            state
                .read_with(cx, |s, _| s.ux.composer_prefill.clone())
                .as_deref(),
            Some("carry")
        );
    }
}

#[gpui_kit::test]
fn session_page_opens_and_closes(cx: &mut TestAppContext) {
    let mut failures = Vec::new();
    for lang in [Lang::En, Lang::ZhTw] {
        let mut initial = demo::state();
        let sessions = Arc::new(FakeSessions::new(vec![WorkerSession {
            id: "session-one".into(),
            run: "run".into(),
            unit: None,
            title: "Test terminal".into(),
            kind: SessionKind::Terminal,
        }]));
        sessions.push("session-one", "terminal output");
        initial.ux.seams.sessions = Some(sessions);
        let (window, state) = ux_support::open(cx, lang, initial, Screen::Run);
        cx.update(|cx| {
            state.update(cx, |s, cx| {
                s.open_page(Page::Session("session-one".into()), cx)
            })
        });
        assert!(shows(cx, window, "session-page", "terminal output"));
        press(cx, window, "escape");
        if state.read_with(cx, |s, _| s.ux.page.is_some()) {
            failures.push(lang);
        }
    }
    assert!(
        failures.is_empty(),
        "Escape did not close session page: {failures:?}"
    );
}

struct RecordingBackend(Rc<Cell<usize>>);
impl Backend for RecordingBackend {
    fn start_wish(&self, _: WishRequest) -> axiom::base::Res<()> {
        Ok(())
    }
    fn interject(&self, _: &str, _: &str) -> axiom::base::Res<()> {
        Ok(())
    }
    fn stop(&self, _: &str) -> axiom::base::Res<()> {
        self.0.set(self.0.get() + 1);
        Ok(())
    }
    fn resume(&self, _: &str, _: Option<&str>) -> axiom::base::Res<()> {
        Ok(())
    }
    fn delete_memory(&self, _: i64) -> axiom::base::Res<()> {
        Ok(())
    }
    fn correct_memory(&self, _: i64, _: &str) -> axiom::base::Res<()> {
        Ok(())
    }
    fn refresh(&self) -> axiom::base::Res<()> {
        Ok(())
    }
    fn recheck(&self) -> axiom::base::Res<()> {
        Ok(())
    }
    fn follow(&self, _: &str) -> axiom::base::Res<()> {
        Ok(())
    }
}

#[gpui_kit::test]
fn close_with_and_without_background(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        for availability in [Some(true), Some(false), None] {
            let background = availability.map(|available| Arc::new(FakeBackground::new(available)));
            let stops = Rc::new(Cell::new(0));
            let mut initial = demo::state();
            initial.backend = Some(Box::new(RecordingBackend(stops.clone())));
            initial.ux.seams.background =
                background.clone().map(|b| b as Arc<dyn BackgroundService>);
            let sessions: Vec<_> = initial
                .runs
                .iter()
                .filter(|r| r.status == axiom::desktop::model::RunStatus::Active)
                .map(|r| r.session.to_string())
                .collect();
            let (window, _) = ux_support::open(cx, lang, initial, Screen::Run);
            assert!(!gpui_kit::VisualTestContext::from_window(window, cx).simulate_close());
            cx.run_until_parked();
            if availability == Some(true) {
                let lines = ux_support::capture(cx, window);
                assert!(ux_support::shows(
                    &lines,
                    "close-keeps-working",
                    if lang == Lang::En {
                        "keeps working in the background"
                    } else {
                        "在背景繼續"
                    }
                ));
                cx.update_window(window, |_, window, cx| window.click("close-window", cx))
                    .unwrap();
                cx.run_until_parked();
                assert_eq!(background.unwrap().detached(), sessions);
                assert_eq!(stops.get(), 0);
                assert!(!cx.windows().contains(&window));
            } else {
                ux_support::capture(cx, window);
                assert!(shown(cx, window, "keep-working"));
                assert!(shown(cx, window, "stop-and-close"));
                assert!(!shown(cx, window, "close-keeps-working"));
            }
        }
    }
}
