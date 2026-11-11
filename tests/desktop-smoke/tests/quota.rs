//! Quota meters in the real Shell and Mini, with isolated demo state.
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use axiom::usage::quota::{ProviderQuota, Window, WindowKind};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
use std::time::{SystemTime, UNIX_EPOCH};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<axiom::desktop::model::AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let sample = |kind, used| Some(Window { kind, used, resets_at: if used == 1.0 { Some(now + 1200) } else { None }, observed_at: now, status: None });
    let mut initial = demo::state();
    initial.quota.claude = ProviderQuota { five_hour: sample(WindowKind::FiveHour, 0.25), weekly: sample(WindowKind::Weekly, 0.5) };
    initial.quota.codex = ProviderQuota { five_hour: sample(WindowKind::FiveHour, 1.0), weekly: sample(WindowKind::Weekly, 0.0) };
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(state.clone(), false, window, cx)))).unwrap();
    frame(cx, window);
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
}

fn check(cx: &mut TestAppContext, handle: AnyWindowHandle, mini: bool, zh: bool) {
    cx.update_window(handle, |_, window, cx| {
        let viewport = window.viewport_size();
        if mini { assert_eq!(viewport.width, px(380.)); }
        let texts = gpui_kit::base::test_support::texts(window);
        let row = theme::row_height(theme::density(cx));
        let strip = window.find("quota-strip").bounds();
        assert_eq!(strip.size.height, row);
        assert_eq!(strip.origin.y + strip.size.height, viewport.height);
        if mini {
            let scale = window.scale_factor();
            let back_y = f32::from(strip.origin.y - theme::space(theme::density(cx), theme::Step::Sm) - row) * scale;
            let back_height = f32::from(row) * scale;
            assert!(window.painted_quads().iter().any(|q| q.background.as_solid() == Some(cx.global::<theme::Tokens>().raised)
                && (q.bounds.origin.y.0 - back_y).abs() < 1.0
                && (q.bounds.size.height.0 - back_height).abs() < 1.0), "Back is immediately above the quota row");
        }
        let strip_id: ElementId = "quota-strip".into();
        let strip_texts: Vec<_> = texts.iter().filter(|t| t.path().contains(&strip_id) && t.visible()).collect();
        assert!(!strip_texts.is_empty());
        let top = strip_texts[0].bounds().origin.y;
        assert!(strip_texts.iter().all(|t| !t.text().contains(['\n', '\r']) && t.bounds().origin.y == top), "strip text must occupy one line without line breaks");
        let exhausted = match (mini, zh) {
            (true, true) => "0% ↻20分", (true, false) => "0% ↻20m",
            (false, true) => "0% · 20 分後重置", (false, false) => "0% · resets in 20 min",
        };
        let cases = if zh {
            [("quota-claude-five-hour", "75%", "5 小時"), ("quota-claude-weekly", "50%", "每週"),
             ("quota-codex-five-hour", exhausted, "5 小時"), ("quota-codex-weekly", "100%", "每週")]
        } else {
            [("quota-claude-five-hour", "75%", "5 hours"), ("quota-claude-weekly", "50%", "weekly"),
             ("quota-codex-five-hour", exhausted, "5 hours"), ("quota-codex-weekly", "100%", "weekly")]
        };
        for (id, expected, name) in cases {
            let bounds = window.find(id).bounds();
            assert!(bounds.origin.x >= px(0.) && bounds.origin.x + bounds.size.width <= viewport.width, "{id} meter exceeds window: {bounds:?}");
            assert!(bounds.origin.y >= strip.origin.y && bounds.origin.y + bounds.size.height <= viewport.height, "{id} meter exceeds row: {bounds:?}");
            let element_id: ElementId = id.into();
            let matching: Vec<_> = texts.iter().filter(|t| t.path().contains(&element_id) && t.visible()).collect();
            assert!(matching.iter().any(|t| t.text() == expected), "{id} must paint {expected}");
            assert_eq!(matching.iter().any(|t| t.text() == name), !mini, "{id} window label");
            assert!(matching.iter().all(|t| !t.text().contains(['\n', '\r'])), "{id} text must not contain line breaks");
            let top = matching[0].bounds().origin.y;
            assert!(matching.iter().all(|t| t.bounds().origin.y == top), "{id} text must occupy one line");
            for text in matching {
                let b = text.bounds();
                assert!(b.origin.x >= px(0.) && b.origin.x + b.size.width <= viewport.width, "{id} exceeds window: {b:?}");
                assert!(b.origin.y >= viewport.height - row && b.origin.y + b.size.height <= viewport.height, "{id} exceeds quota row: {b:?}");
            }
        }
    }).unwrap();
}

#[gpui_kit::test]
fn main_quota_meters(cx: &mut TestAppContext) {
    let (main, _) = open(cx);
    check(cx, main, false, false);
    cx.update(|cx| { i18n::set(i18n::Lang::ZhTw, cx); theme::set_density(theme::Density::Dense, cx); });
    frame(cx, main);
    check(cx, main, false, true);
}

#[gpui_kit::test]
fn mini_quota_meters(cx: &mut TestAppContext) {
    let (main, state) = open(cx);
    // Match walk.rs: the locked test platform cannot minimize the main window.
    let site = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = site.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(loc) = info.location() { *captured.lock().unwrap() = Some((loc.file().replace('\\', "/"), loc.line())); }
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update_window(main, |_, window, cx| window.press(if cfg!(target_os = "macos") { "cmd-shift-m" } else { "ctrl-shift-m" }, cx)).unwrap();
    }));
    std::panic::set_hook(previous);
    if let Err(error) = result {
        let location = site.lock().unwrap().clone();
        let message = error.downcast_ref::<&str>().copied().or_else(|| error.downcast_ref::<String>().map(String::as_str));
        if !location.as_ref().is_some_and(|(file, line)| file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473) || message != Some("not implemented") {
            std::panic::resume_unwind(error);
        }
    }
    let mini = cx.windows().into_iter().find(|window| *window != main).expect("real Mini window");
    frame(cx, mini);
    check(cx, mini, true, false);
    cx.update(|cx| { i18n::set(i18n::Lang::ZhTw, cx); theme::set_density(theme::Density::Dense, cx); });
    frame(cx, mini);
    check(cx, mini, true, true);
    cx.update(|cx| state.update(cx, |state, _| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let exhausted = |kind| Some(Window { kind, used: 1.0, resets_at: Some(now + 1200), observed_at: now, status: None });
        for provider in [&mut state.quota.claude, &mut state.quota.codex] {
            provider.five_hour = exhausted(WindowKind::FiveHour);
            provider.weekly = exhausted(WindowKind::Weekly);
        }
    }));
    for (lang, density) in [(i18n::Lang::ZhTw, theme::Density::Dense), (i18n::Lang::En, theme::Density::Dense),
                            (i18n::Lang::ZhTw, theme::Density::Comfortable), (i18n::Lang::En, theme::Density::Comfortable)] {
        cx.update(|cx| { i18n::set(lang, cx); theme::set_density(density, cx); });
        frame(cx, mini);
        cx.update_window(mini, |_, window, _| {
            for id in ["quota-claude-five-hour", "quota-claude-weekly", "quota-codex-five-hour", "quota-codex-weekly"] {
                let b = window.find(id).bounds();
                assert!(b.origin.x >= px(0.) && b.origin.x + b.size.width <= window.viewport_size().width, "{id} exhausted countdown exceeds Mini: {b:?}");
            }
        }).unwrap();
    }
}
