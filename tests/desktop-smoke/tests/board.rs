//! Board rendering in the real Shell, with isolated demo data.
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use axiom::desktop::model::{AppState, RunStatus};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext, WindowBounds, WindowOptions};

fn open(cx: &mut TestAppContext, width: f32, height: f32) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let state = cx.new(|_| demo::state());
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds { origin: Point::default(), size: size(px(width), px(height)) })), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))).unwrap();
    for _ in 0..4 { state.update(cx, |_, cx| cx.notify()); frame(cx, window); }
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| { window.refresh(); window.render_frame(cx); }).unwrap();
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx)).unwrap();
    frame(cx, window);
}

fn board_texts(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> Vec<String> {
    cx.update_window(window, |_, window, _| {
        let id: ElementId = id.into();
        gpui_kit::base::test_support::texts(window).into_iter().filter(|t| t.path().contains(&id) && t.visible()).map(|t| t.text().to_string()).collect()
    }).unwrap()
}

#[gpui_kit::test]
fn strip_lists_attention_in_both_languages(cx: &mut TestAppContext) {
    let (window, state) = open(cx, 1100., 700.);
    let en = board_texts(cx, window, "board-strip");
    cx.update(|cx| i18n::set(i18n::Lang::ZhTw, cx));
    frame(cx, window);
    let zh = board_texts(cx, window, "board-strip");
    let mut cases = Vec::new();
    for status in [RunStatus::Active, RunStatus::Waiting, RunStatus::Interrupted, RunStatus::Done, RunStatus::Blocked] {
        state.update(cx, |state, cx| {
            let need = state.runs[2].needs[0].clone();
            let mut quiet = state.runs[0].clone();
            quiet.status = RunStatus::Active;
            quiet.needs.clear();
            let mut other = state.runs[1].clone();
            other.status = status;
            other.needs = if status == RunStatus::Active { vec![need] } else { vec![] };
            state.runs = vec![quiet, other, state.runs[2].clone()];
            state.runs[2].status = RunStatus::Done;
            cx.notify();
        });
        for lang in [i18n::Lang::En, i18n::Lang::ZhTw] {
            cx.update(|cx| i18n::set(lang, cx));
            frame(cx, window);
            cases.push(board_texts(cx, window, "board-strip"));
        }
    }
    let one_en = vec!["1 wish needs you: rram-batch".to_string()];
    let one_zh = vec!["1 個願望需要你：rram-batch".to_string()];
    assert_eq!((en, zh, cases), (vec!["2 wishes need you: rram-batch, billing".to_string()], vec!["2 個願望需要你：rram-batch、billing".to_string()], vec![one_en.clone(), one_zh.clone(), one_en.clone(), one_zh.clone(), one_en.clone(), one_zh.clone(), vec![], vec![], one_en, one_zh]));
}

#[gpui_kit::test]
fn strip_does_not_reserve_a_row_when_no_run_needs_attention(cx: &mut TestAppContext) {
    let (window, state) = open(cx, 1100., 700.);
    let before = cx.update_window(window, |_, window, _| window.try_find("board-strip").map(|e| e.bounds().size.height)).unwrap();
    state.update(cx, |state, cx| { for run in &mut state.runs { if run.status != RunStatus::Done { run.status = RunStatus::Active; run.needs.clear(); } } cx.notify(); });
    frame(cx, window);
    let after = cx.update_window(window, |_, window, _| window.try_find("board-strip").map(|e| e.bounds().size.height)).unwrap();
    assert_eq!((before, after), (Some(px(24.)), None));
}

#[gpui_kit::test]
fn settings_language_changes_demo_board_sentence(cx: &mut TestAppContext) {
    let (window, _) = open(cx, 1440., 900.);
    let en = board_texts(cx, window, "overview-board").join(" ");
    click(cx, window, "nav-doctor");
    click(cx, window, "lang-zh");
    click(cx, window, "nav-run");
    let zh = board_texts(cx, window, "overview-board").join(" ");
    assert_eq!((en.contains("Work progress has been updated"), zh.contains("工作進度已更新")), (true, true));
}
