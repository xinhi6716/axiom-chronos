mod ux_support;

use axiom::desktop::{demo, i18n::Lang, model::{AppState, ModelCost, RunStatus, Screen, UnitState, Usage}, Store};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, TestAppContext};

fn click(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<ElementId>) {
    cx.update_window(w, |_, w, cx| w.click(id, cx)).unwrap();
    ux_support::capture(cx, w);
}
fn click_row_button(cx: &mut TestAppContext, w: AnyWindowHandle, row: usize, name: &str) {
    cx.update_window(w, |_, w, cx| {
        let row: ElementId = ("wish-row", row).into();
        let name: ElementId = name.to_owned().into();
        let b = w.elements().iter().find(|e| e.path().contains(&row) && e.path().contains(&name)).expect("row button").bounds();
        w.pointer_down(b.center(), gpui_kit::MouseButton::Left, cx);
        w.pointer_up(b.center(), gpui_kit::MouseButton::Left, cx);
    }).unwrap();
    ux_support::capture(cx, w);
}
fn press(cx: &mut TestAppContext, w: AnyWindowHandle, key: &str) {
    cx.update_window(w, |_, w, cx| w.press(key, cx)).unwrap();
    ux_support::capture(cx, w);
}
fn label_in(cx: &mut TestAppContext, w: AnyWindowHandle, scope: &str, label: &str) -> bool {
    let id: ElementId = scope.to_owned().into();
    cx.update_window(w, |_, w, _| w.elements().iter().any(|e| e.path().contains(&id) && e.label().is_some_and(|v| v.contains(label)))).unwrap()
        || ux_support::shows(&ux_support::capture(cx, w), scope, label)
}
fn visible(cx: &mut TestAppContext, w: AnyWindowHandle, text: &str) -> bool {
    ux_support::capture(cx, w).iter().any(|l| l.text.contains(text))
}
fn seed() -> AppState {
    let mut state = demo::state();
    let mut a = demo::sample_run();
    a.session = "session-a".into(); a.wish = "Alpha wish".into();
    a.status = RunStatus::Done; a.needs.clear();
    for unit in &mut a.units { unit.state = UnitState::Verified; }
    a.started_at = Some(3);
    let mut b = a.clone(); b.session = "session-b".into(); b.wish = "Beta wish".into(); b.started_at = Some(2);
    state.runs = vec![a, b]; state.current = Some(0);
    state.ux.wish_ids.clear();
    state.ux.wish_ids.insert("session-a".into(), "wish-a".into());
    state.ux.wish_ids.insert("session-b".into(), "wish-b".into());
    state
}
fn prefs(cx: &mut TestAppContext) -> axiom::desktop::ux::prefs::UxPrefs {
    cx.update(|cx| Store::current(cx).ux.clone())
}
fn menu(cx: &mut TestAppContext, w: AnyWindowHandle, row: usize, label: &str) {
    cx.update_window(w, |_, w, cx| {
        let id: ElementId = ("wish-text", row).into();
        let bounds = gpui_kit::base::test_support::texts(w).into_iter().find(|e| e.path().contains(&id)).unwrap().bounds();
        w.pointer_down(bounds.center(), gpui_kit::MouseButton::Right, cx);
        w.pointer_up(bounds.center(), gpui_kit::MouseButton::Right, cx);
    }).unwrap();
    ux_support::capture(cx, w);
    for _ in 0..12 {
        press(cx, w, "down");
        let found = cx.update_window(w, |_, w, _| w.elements().iter().any(|e| e.path().contains(&ElementId::from("popup-menu")) && e.role() == Some(gpui_kit::Role::MenuItem) && e.label() == Some(label) && e.selected() == Some(true))).unwrap();
        if found { press(cx, w, "enter"); return; }
    }
    panic!("menu item not found: {label}");
}
fn count_unread(cx: &mut TestAppContext, w: AnyWindowHandle) -> usize {
    cx.update_window(w, |_, w, _| w.elements().iter().filter(|e| e.label() == Some("Mark as read") || e.label() == Some("標為已讀")).count()).unwrap()
}

#[gpui_kit::test]
fn read_controls_and_new_attention(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let (w, state) = ux_support::open(cx, lang, seed(), Screen::Wishes);
        let (read, unread, archive, all) = if lang == Lang::En { ("Mark as read", "Mark as unread", "Archive", "Mark all read") } else { ("標為已讀", "標為未讀", "封存", "全部標為已讀") };
        assert!(label_in(cx, w, "wishes-read-filter", all));
        assert_eq!(count_unread(cx, w), 2);
        click_row_button(cx, w, 0, "run-mark-read");
        assert_eq!(count_unread(cx, w), 1);
        assert!(prefs(cx).read.contains_key("session-a"));
        menu(cx, w, 1, read);
        assert_eq!(count_unread(cx, w), 0);
        menu(cx, w, 1, unread);
        assert_eq!(count_unread(cx, w), 1);
        click(cx, w, ("wishes-read-tab", 1_usize));
        assert!(!visible(cx, w, "Alpha wish")); assert!(visible(cx, w, "Beta wish"));
        click(cx, w, "wishes-mark-all-read");
        assert!(!visible(cx, w, "Beta wish"));
        click(cx, w, ("wishes-read-tab", 0_usize));
        menu(cx, w, 1, archive);
        assert!(!visible(cx, w, "Beta wish"));
        click(cx, w, ("wishes-read-tab", 2_usize));
        assert!(visible(cx, w, "Beta wish")); assert!(!visible(cx, w, "Alpha wish"));
        // Ordinary activity leaves both marks intact; a fresh failure invalidates them.
        state.update(cx, |s, cx| { for r in &mut s.runs { r.elapsed = Some("heartbeat".into()); } cx.notify(); });
        assert!(visible(cx, w, "Beta wish"));
        click(cx, w, ("wishes-read-tab", 1_usize));
        assert!(!visible(cx, w, "Alpha wish")); assert!(!visible(cx, w, "Beta wish"));
        click(cx, w, ("wishes-read-tab", 2_usize));
        state.update(cx, |s, cx| { for r in &mut s.runs { r.units[0].state = UnitState::Failed; } cx.notify(); });
        assert!(!visible(cx, w, "Beta wish"));
        click(cx, w, ("wishes-read-tab", 1_usize));
        assert!(visible(cx, w, "Alpha wish")); assert!(visible(cx, w, "Beta wish"));
        assert_eq!(count_unread(cx, w), 2);
        // Restore all, focus the list and exercise both keyboard actions.
        click(cx, w, ("wishes-read-tab", 0_usize));
        press(cx, w, "ctrl-f"); press(cx, w, "down"); press(cx, w, "down"); press(cx, w, "up");
        press(cx, w, "r"); assert_eq!(count_unread(cx, w), 1);
        press(cx, w, "r"); assert_eq!(count_unread(cx, w), 2);
        press(cx, w, "e"); assert_eq!(count_unread(cx, w), 1);
    }
}

fn usage(amount: Option<f64>) -> Usage {
    Usage { total_usd: amount, models: vec![ModelCost { model: "test-model".into(), usd: amount, input: Some(1), cached: Some(0), cache_write: Some(0), output: Some(1), total: Some(2) }], ..Default::default() }
}
fn usage_seed() -> AppState {
    let mut s = seed();
    s.runs[0].usage = usage(Some(1.)); s.runs[1].usage = usage(Some(5.));
    let mut resumed = s.runs[0].clone(); resumed.session = "session-a2".into(); resumed.usage = usage(Some(2.));
    s.ux.wish_ids.insert("session-a2".into(), "wish-a".into());
    s.runs.push(resumed.clone()); s.runs.push(resumed);
    s
}

#[gpui_kit::test]
fn wish_card_opens_filtered_usage(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let (w, state) = ux_support::open(cx, lang, seed(), Screen::Wishes);
        assert!(label_in(cx, w, "wish-usage", if lang == Lang::En { "Usage" } else { "用量" }));
        click_row_button(cx, w, 0, "wish-usage");
        assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Usage);
        assert_eq!(prefs(cx).usage_wish.as_deref(), Some("wish-a"));
        assert!(cx.update_window(w, |_, w, _| w.elements().iter().any(|e| e.path().contains(&ElementId::from("usage-wish-picker")))).unwrap());
        let mut fallback = seed(); fallback.ux.wish_ids.clear();
        let (w, state) = ux_support::open(cx, lang, fallback, Screen::Wishes);
        click_row_button(cx, w, 0, "wish-usage");
        assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Usage);
        assert_eq!(prefs(cx).usage_wish.as_deref(), Some("session-a"));
    }
}

fn choose(cx: &mut TestAppContext, w: AnyWindowHandle, picker: &str, query: &str) {
    cx.update_window(w, |_, w, cx| {
        let scope: ElementId = picker.to_owned().into();
        let input: ElementId = "input".into();
        let b = w.elements().iter().find(|e| e.path().contains(&scope) && e.path().contains(&input)).expect("picker input").bounds();
        w.pointer_down(b.center(), gpui_kit::MouseButton::Left, cx);
        w.pointer_up(b.center(), gpui_kit::MouseButton::Left, cx);
    }).unwrap();
    ux_support::capture(cx, w);
    cx.update_window(w, |_, w, cx| w.input(query, cx)).unwrap();
    ux_support::capture(cx, w);

    press(cx, w, "down"); press(cx, w, "enter");
}
fn reopen(cx: &mut TestAppContext, state: &Entity<AppState>) -> AnyWindowHandle {
    use axiom::desktop::Shell;
    use gpui_kit::{px, size, Bounds, Point, WindowBounds, WindowOptions};
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) })), ..Default::default() };
    let state = state.clone();
    let (w, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(state, false, window, cx)))).unwrap();
    ux_support::capture(cx, w);
    w
}
#[gpui_kit::test]
fn picker_comparison_and_reopening(cx: &mut TestAppContext) {
    for lang in [Lang::En, Lang::ZhTw] {
        let (w, state) = ux_support::open(cx, lang, usage_seed(), Screen::Usage);
        for id in ["usage-wish-picker", "usage-compare"] {
            assert!(cx.update_window(w, |_, w, _| w.elements().iter().any(|e| e.path().contains(&ElementId::from(id)))).unwrap());
        }
        choose(cx, w, "usage-wish-picker", "Alpha");
        assert_eq!(prefs(cx).usage_wish.as_deref(), Some("wish-a"));
        choose(cx, w, "usage-compare", "Beta");
        assert_eq!(prefs(cx).usage_compare.as_deref(), Some("wish-b"));
        let lines = ux_support::capture(cx, w);
        for marker in ["Alpha wish", "Beta wish", "$3.00", "$5.00", "−$2.00"] {
            assert!(ux_support::shows(&lines, "usage-compare-table", marker), "missing {marker}: {lines:?}");
        }
        let reopened = reopen(cx, &state);
        assert_eq!(prefs(cx).usage_wish.as_deref(), Some("wish-a"));
        assert_eq!(prefs(cx).usage_compare.as_deref(), Some("wish-b"));
        let lines = ux_support::capture(cx, reopened);
        assert!(ux_support::shows(&lines, "usage-wish-picker", "Alpha wish"));
        assert!(ux_support::shows(&lines, "usage-compare", "Beta wish"));
        state.update(cx, |s, cx| { s.runs[2].usage = usage(None); cx.notify(); });
        let lines = ux_support::capture(cx, reopened);
        for marker in ["Alpha wish", "Beta wish", "$5.00", "—", if lang == Lang::En { "Incomplete" } else { "資料不完整" }] {
            assert!(ux_support::shows(&lines, "usage-compare-table", marker), "missing {marker}: {lines:?}");
        }
    }
}


