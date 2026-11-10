//! Drives the real shell from the keyboard alone, in a headless window: the command palette, the
//! shortcuts overlay and the jumps they make. Each step goes through the real key bindings.
use axiom::desktop::model::{AppState, Screen};
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};

const PALETTE: &str = if cfg!(target_os = "macos") { "cmd-k" } else { "ctrl-k" };
const NEW_WISH: &str = if cfg!(target_os = "macos") { "cmd-n" } else { "ctrl-n" };

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<AppState>) {
    open_with(cx, |_| {})
}

fn open_with(cx: &mut TestAppContext, change: impl FnOnce(&mut AppState)) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let mut initial = demo::state();
    change(&mut initial);
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx
        .update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx))))
        .expect("open the shell");
    frame(cx, window);
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render a frame");
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, keys: &str) {
    cx.update_window(window, |_, window, cx| window.press(keys, cx)).expect("press");
    frame(cx, window);
}

fn type_text(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx)).expect("type");
    frame(cx, window);
}

fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(window, |_, window, _| window.try_find(id).is_some()).expect("query")
}

#[gpui_kit::test]
fn the_palette_finds_a_memory_and_goes_to_it(cx: &mut TestAppContext) {
    let (window, state) = open(cx);
    assert!(!shown(cx, window, "palette-scrim"));
    press(cx, window, PALETTE);
    assert!(shown(cx, window, "palette-scrim"), "the palette opens on its shortcut");
    type_text(cx, window, "tiled sheets");
    press(cx, window, "enter");
    assert!(!shown(cx, window, "palette-scrim"), "choosing a result closes the palette");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Memory);
}

#[gpui_kit::test]
fn arrows_choose_a_command_and_escape_closes(cx: &mut TestAppContext) {
    let (window, state) = open(cx);
    press(cx, window, PALETTE);
    // Before anything is typed: Make a new wish, Stop the current wish, Go to Run, Go to Wishes, …
    for _ in 0..3 {
        press(cx, window, "down");
    }
    press(cx, window, "up");
    press(cx, window, "down");
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
    press(cx, window, PALETTE);
    assert!(shown(cx, window, "palette-scrim"));
    press(cx, window, "escape");
    assert!(!shown(cx, window, "palette-scrim"));
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes, "escape goes nowhere");
}

#[gpui_kit::test]
fn the_shortcut_key_toggles_the_palette(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    press(cx, window, PALETTE);
    assert!(shown(cx, window, "palette-scrim"));
    press(cx, window, PALETTE);
    assert!(!shown(cx, window, "palette-scrim"));
}

#[gpui_kit::test]
fn a_question_mark_shows_the_shortcuts_outside_text_boxes_only(cx: &mut TestAppContext) {
    let (window, _) = open(cx);
    press(cx, window, "?");
    assert!(shown(cx, window, "shortcuts-scrim"), "? opens the overlay");
    press(cx, window, "escape");
    assert!(!shown(cx, window, "shortcuts-scrim"), "escape closes it");
    // In the palette's text box, ? is a character to search for.
    press(cx, window, PALETTE);
    press(cx, window, "?");
    assert!(!shown(cx, window, "shortcuts-scrim"));
    assert!(shown(cx, window, "palette-scrim"));
}



#[gpui_kit::test]
fn an_at_sign_in_a_wish_offers_the_project_files(cx: &mut TestAppContext) {
    // The open run's folder is this repository, so git has real files to list.
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (window, state) = open_with(cx, |s| s.runs[0].cwd = Some(repo));
    press(cx, window, NEW_WISH);
    assert!(state.read_with(cx, |s, _| s.composing), "Ctrl+N opens the dialog");
    type_text(cx, window, "fix @desk");
    assert!(shown(cx, window, "composer-mention"), "@ opens the file list");
    press(cx, window, "escape");
    assert!(!shown(cx, window, "composer-mention"), "escape closes the list");
    assert!(state.read_with(cx, |s, _| s.composing), "and leaves the dialog open");
    type_text(cx, window, " @palet");
    assert!(shown(cx, window, "composer-mention"));
    press(cx, window, "enter");
    assert!(!shown(cx, window, "composer-mention"), "enter inserts the path and closes the list");
    assert!(state.read_with(cx, |s, _| s.composing));
}
