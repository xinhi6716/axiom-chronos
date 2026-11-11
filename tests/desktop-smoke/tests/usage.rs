use axiom::desktop::model::Screen;
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Point, TestAppContext, WindowBounds, WindowOptions};

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
}

fn open_with(cx: &mut TestAppContext, width: f32, height: f32, clear_warnings: bool) -> AnyWindowHandle {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    let current = initial.current.expect("demo current run");
    if clear_warnings {
        initial.runs[current].warnings.clear();
        initial.runs[current].notes.clear();
        initial.runs[current].usage.contexts.clear();
    }
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(width), px(height)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))).unwrap();
    state.update(cx, |state, cx| state.set_view(Screen::Usage, cx));
    for _ in 0..4 { frame(cx, window); }
    window
}

fn open(cx: &mut TestAppContext, width: f32, height: f32) -> AnyWindowHandle {
    open_with(cx, width, height, false)
}

#[gpui_kit::test]
fn compact_models_are_visible_before_context(cx: &mut TestAppContext) {
    let window = open(cx, 1100., 700.);
    cx.update_window(window, |_, window, _| {
        let elements = window.elements();
        let model: ElementId = "usage-models".into();
        let viewport = window.find("usage-scroll").bounds();
        let row = |id: ElementId| elements.iter().find(|e| e.role() == Some(gpui_kit::Role::Row) && e.path().contains(&model) && e.path().contains(&id)).expect("model table row");
        let header = row("row".into());
        let first = row(("tr", 0_usize).into());
        for row in [header, first] {
            let b = row.bounds();
            assert!(b.top() >= viewport.top() && b.bottom() <= viewport.bottom(), "row outside viewport: {b:?}, viewport: {viewport:?}");
        }
        let chart = window.find(("usage-context", 0_usize)).bounds();
        assert!(first.bounds().bottom() <= chart.top(), "costs precede context");
    }).unwrap();
}

#[gpui_kit::test]
fn compact_warning_summary_toggles(cx: &mut TestAppContext) {
    let window = open(cx, 1100., 700.);
    let expanded = |cx: &mut TestAppContext| cx.update_window(window, |_, window, _| window.try_find(("usage-warning-tip", 0_usize)).is_some()).unwrap();
    cx.update_window(window, |_, window, _| assert!(window.try_find("usage-warnings-summary").is_some())).unwrap();
    assert!(!expanded(cx));
    for expected in [true, false] {
        cx.update_window(window, |_, window, cx| window.click("usage-warnings-summary", cx)).unwrap();
        frame(cx, window);
        assert_eq!(expanded(cx), expected, "click toggle");
    }
    for _ in 0..60 {
        if cx.update_window(window, |_, window, _| window.find("usage-warnings-summary").focused() == Some(true)).unwrap() { break; }
        cx.update_window(window, |_, window, cx| window.press("tab", cx)).unwrap();
        frame(cx, window);
    }
    assert!(cx.update_window(window, |_, window, _| window.find("usage-warnings-summary").focused() == Some(true)).unwrap());
    for expected in [true, false] {
        cx.update_window(window, |_, window, cx| window.press("enter", cx)).unwrap();
        frame(cx, window);
        assert_eq!(expanded(cx), expected, "keyboard toggle");
    }
    for expected in [true, false] {
        cx.update_window(window, |_, window, cx| window.press("space", cx)).unwrap();
        frame(cx, window);
        assert_eq!(expanded(cx), expected, "space toggle");
    }
}

#[gpui_kit::test]
fn compact_without_warnings_draws_no_warning_panel(cx: &mut TestAppContext) {
    let window = open_with(cx, 1100., 700., true);
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("usage-warnings-summary").is_none());
        assert!(window.try_find("usage-warning-clear").is_none());
    }).unwrap();
}

#[gpui_kit::test]
fn wide_cost_tables_are_side_by_side(cx: &mut TestAppContext) {
    let window = open(cx, 1920., 1080.);
    cx.update_window(window, |_, window, _| {
        let models = window.find("usage-models").bounds();
        let roles = window.find("usage-roles").bounds();
        assert_eq!(models.top(), roles.top());
        assert!(models.left() < roles.left());
    }).unwrap();
}
