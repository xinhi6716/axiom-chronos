//! Mini interaction walks against the real Shell and demo model.
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use axiom::desktop::model::{self, AppState, Undoable};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext, WindowBounds, WindowOptions};

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| { window.refresh(); window.render_frame(cx); }).unwrap();
}
fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx)).unwrap();
    frame(cx, window);
}
fn open(cx: &mut TestAppContext, chinese: bool) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| { gpui_kit::init(cx); theme::apply(ThemeMode::Dark, None, cx); i18n::set(i18n::Lang::En, cx); cx.set_global(Store::open(true)); });
    let state = cx.new(|_| demo::state());
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) })), ..Default::default() };
    let shell_state = state.clone();
    let (main, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))).unwrap();
    frame(cx, main);
    if chinese {
        for id in ["nav-doctor", "lang-zh"] {
            cx.update_window(main, |_, window, cx| window.click(id, cx)).unwrap();
            frame(cx, main);
        }
    }
    // The locked test platform cannot minimize. Catch only that documented platform panic.
    let site = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = site.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| { if let Some(p) = info.location() { *captured.lock().unwrap() = Some((p.file().replace('\\', "/"), p.line())); } }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| press(cx, main, if cfg!(target_os = "macos") { "cmd-shift-m" } else { "ctrl-shift-m" })));
    std::panic::set_hook(previous);
    if let Err(error) = result {
        let allowed = site.lock().unwrap().as_ref().is_some_and(|(file, line)| file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473)
            && error.downcast_ref::<&str>() == Some(&"not implemented");
        if !allowed { std::panic::resume_unwind(error); }
    }
    let mini = cx.windows().into_iter().find(|w| *w != main).expect("mini window");
    frame(cx, mini);
    (mini, state)
}
fn texts(cx: &mut TestAppContext, window: AnyWindowHandle, id: ElementId) -> Vec<String> {
    cx.update_window(window, |_, window, _| gpui_kit::base::test_support::texts(window).into_iter().filter(|t| t.path().contains(&id)).map(|t| t.text().to_string()).collect()).unwrap()
}
fn focus_border(cx: &mut TestAppContext, window: AnyWindowHandle, id: ElementId) -> bool {
    cx.update_window(window, |_, window, cx| {
        let Some(element) = window.try_find(id) else { return false };
        let bounds = element.bounds();
        let scale = window.scale_factor();
        element.focused() == Some(true) && window.painted_quads().iter().any(|q| {
            (q.bounds.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 1.
                && (q.bounds.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 1.
                && (q.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.
                && q.border_color == cx.global::<theme::Tokens>().gold
                && (q.border_widths.left.0 - 2. * scale).abs() < 1.
        })
    }).unwrap()
}

#[gpui_kit::test]
fn cards_follow_board_and_language(cx: &mut TestAppContext) {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut other = cx.new_app();
    let mut results = Vec::new();
    for (app, zh) in [(cx, false), (&mut other, true)] {
        let (mini, state) = open(app, zh);
        let expected = state.read_with(app, |s, _| model::board_runs(s).into_iter().map(|i| (s.runs[i].project.to_string(), s.runs[i].status_label().to_string())).collect::<Vec<_>>());
        let mut all = String::new();
        for (position, (project, status)) in expected.iter().enumerate() {
            let card = texts(app, mini, ("mini-card", position).into()).join(" ");
            let translated = if zh { i18n::COMMON.iter().find(|(en, _)| *en == status).map(|(_, zh)| *zh).unwrap_or(status) } else { status };
            results.push(card.contains(project) && card.contains(translated)
                && card.contains(if zh { "工作進度已更新" } else { "Work progress has been updated" })
                && texts(app, mini, ("mini-row", position).into()).is_empty());
            all.push_str(&card);
        }
        results.push(texts(app, mini, ("mini-card", expected.len()).into()).is_empty());
        results.push(texts(app, mini, "mini-row".into()).is_empty());
        results.push(all.contains(if zh { "工作進度已更新" } else { "Work progress has been updated" }));
    }
    assert_eq!(results, vec![true; 12], "every board run has exactly one localized Mini card");
}

#[gpui_kit::test]
fn tab_focus_and_enter_controls(cx: &mut TestAppContext) {
    let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut back_app = cx.new_app();
    let mut card_app = cx.new_app();
    let mut click_app = cx.new_app();
    let mut result = Vec::new();
    let (mini, state) = open(cx, false);
    let count = state.read_with(cx, |s, _| model::board_runs(s).len());
    press(cx, mini, "tab");
    result.push(focus_border(cx, mini, "mini-pin".into()));
    press(cx, mini, "enter");
    let pin_prior = Undoable::Setting(Store::open(true).settings.set_mini_on_top(true));
    result.push(cx.update(|cx| Store::current(cx).mini_on_top) && state.read_with(cx, |s, _| s.undo.clone().pop() == Some(pin_prior)));
    let (mini, state) = open(&mut back_app, false);
    press(&mut back_app, mini, "tab");
    for i in 0..count { press(&mut back_app, mini, "tab"); result.push(focus_border(&mut back_app, mini, ("mini-card", i).into())); }
    press(&mut back_app, mini, "tab");
    result.push(focus_border(&mut back_app, mini, "mini-back".into()));
    let undo = state.read_with(&back_app, |s, _| s.undo.len());
    press(&mut back_app, mini, "enter");
    let mini_mode = axiom::desktop::parse_options(&["--mode".into(), "mini".into()]).unwrap().mode.unwrap();
    let mut preferences = Store::open(true).settings;
    preferences.mode = mini_mode;
    let back_prior = Undoable::Setting(preferences.set_mode(mini_mode));
    result.push(back_app.update(|cx| format!("{:?}", Store::current(cx).mode)) == "Full" && state.read_with(&back_app, |s, _| s.undo.len() == undo + 1 && s.undo.clone().pop() == Some(back_prior)));
    let (mini, state) = open(&mut card_app, false);
    let expected = state.read_with(&card_app, |s, _| model::board_runs(s)[0]);
    press(&mut card_app, mini, "tab");
    press(&mut card_app, mini, "tab");
    press(&mut card_app, mini, "enter");
    result.push(card_app.update(|cx| format!("{:?}", Store::current(cx).mode)) == "Full" && state.read_with(&card_app, |s, _| s.current) == Some(expected));
    let (mini, state) = open(&mut click_app, false);
    let expected = state.read_with(&click_app, |s, _| model::board_runs(s)[1]);
    let present = click_app.update_window(mini, |_, window, _| window.try_find(("mini-card", 1_usize)).is_some()).unwrap();
    if present {
        click_app.update_window(mini, |_, window, cx| window.click(("mini-card", 1_usize), cx)).unwrap();
        click_app.run_until_parked();
    }
    result.push(present && click_app.update(|cx| format!("{:?}", Store::current(cx).mode)) == "Full" && state.read_with(&click_app, |s, _| s.current) == Some(expected));
    assert_eq!(result, vec![true; count + 6], "Tab order, visible gold focus and activation effects");
}
