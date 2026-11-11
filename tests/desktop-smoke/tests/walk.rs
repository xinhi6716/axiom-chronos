//! Walk the real Shell with platform key events, without a live backend or translation runner.
//! Persistent stores are confined to .axiom-tmp/audit-walk-1; restart means a new GPUI app.
use axiom::desktop::model::{AppState, Screen, Undoable};
use axiom::desktop::theme::{Density, Step};
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// settings is private to axiom. Read the REAL Store's public fields by inference;
// do not include/recompile settings.rs or replace the Shell's Store with a test copy.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ColumnWidths {
    checklist: f32,
    details: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum FocusPanel {
    Graph,
    Checklist,
}
#[derive(Debug)]
struct Preferences {
    mode: String,
    density: Density,
    mini_on_top: bool,
    reduce_motion: Option<bool>,
    columns: Option<ColumnWidths>,
}
macro_rules! snapshot {
    ($settings:expr) => {{
        let settings = &$settings;
        Preferences {
            mode: format!("{:?}", settings.mode),
            density: settings.density,
            mini_on_top: settings.mini_on_top,
            reduce_motion: settings.reduce_motion,
            columns: settings.columns.map(|w| ColumnWidths {
                checklist: w.checklist,
                details: w.details,
            }),
        }
    }};
}

fn focus_mode(panel: FocusPanel) -> &'static str {
    match panel {
        FocusPanel::Graph => "Focus(Graph)",
        FocusPanel::Checklist => "Focus(Checklist)",
    }
}

// Expected undo values use the real public Settings mutators (the private return
// type can be inferred). Actual commands still go through platform input events.
fn mode_prior(mode: &str) -> Undoable {
    let word = match mode {
        "Full" => "full",
        "Mini" => "mini",
        "Focus(Graph)" => "focus-graph",
        "Focus(Checklist)" => "focus-checklist",
        _ => panic!("unknown expected mode {mode}"),
    };
    let mode = axiom::desktop::parse_options(&["--mode".into(), word.into()])
        .unwrap()
        .mode
        .unwrap();
    let mut settings = Store::open(true).settings;
    settings.mode = mode;
    Undoable::Setting(settings.set_mode(mode))
}
fn density_prior(density: Density) -> Undoable {
    let mut settings = Store::open(true).settings;
    settings.density = density;
    Undoable::Setting(settings.set_density(density))
}
fn on_top_prior(on: bool) -> Undoable {
    let mut settings = Store::open(true).settings;
    settings.mini_on_top = on;
    Undoable::Setting(settings.set_mini_on_top(on))
}
fn motion_none_prior() -> Undoable {
    Undoable::Setting(Store::open(true).settings.set_reduce_motion(Some(true)))
}

const PALETTE: &str = if cfg!(target_os = "macos") { "cmd-k" } else { "ctrl-k" };
const FOCUS: &str = if cfg!(target_os = "macos") { "cmd-shift-f" } else { "ctrl-shift-f" };
const MINI: &str = if cfg!(target_os = "macos") { "cmd-shift-m" } else { "ctrl-shift-m" };
const DENSITY: &str = if cfg!(target_os = "macos") { "cmd-shift-d" } else { "ctrl-shift-d" };
const UNDO: &str = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
const DEFAULT_COLUMNS: ColumnWidths = ColumnWidths {
    checklist: 300.,
    details: 380.,
};

// AXIOM_HOME is process-wide. Hold this for the entire test, including every restarted app.
static HOME_LOCK: Mutex<()> = Mutex::new(());
struct TestHome {
    _lock: MutexGuard<'static, ()>,
    prior: Option<OsString>,
    path: PathBuf,
}
impl TestHome {
    fn new(name: &str) -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = root
            .join(".axiom-tmp/audit-walk-1")
            .join(format!("{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create isolated settings folder");
        let prior = std::env::var_os("AXIOM_HOME");
        std::env::set_var("AXIOM_HOME", &path);
        Self { _lock: lock, prior, path }
    }

    fn reopen(&self) -> Store {
        assert_eq!(axiom::base::axiom_home(), self.path, "never open the user's ~/.axiom");
        Store::open(false)
    }

    fn saved(&self) -> Preferences {
        assert!(self.path.join("app-settings.json").is_file(), "settings were written");
        let store = self.reopen();
        snapshot!(store.settings)
    }
}
impl Drop for TestHome {
    fn drop(&mut self) {
        match &self.prior {
            Some(prior) => std::env::set_var("AXIOM_HOME", prior),
            None => std::env::remove_var("AXIOM_HOME"),
        }
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<AppState>) {
    open_with(cx, Store::open(true), |_| {})
}

fn open_with(cx: &mut TestAppContext, store: Store, change: impl FnOnce(&mut AppState)) -> (AnyWindowHandle, Entity<AppState>) {
    let prefs = store.settings.clone();
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(store);
        // Leave the default unset so the fallback test really exercises a missing global.
        if prefs.density != Density::Comfortable {
            theme::set_density(prefs.density, cx);
        }
        // Same ordering as desktop::run: capture the system value BEFORE applying the override.
        Store::save_on_quit(cx);
        if let Some(on) = prefs.reduce_motion {
            cx.set_reduce_motion(on);
        }
    });
    let mut initial = demo::state();
    initial.reduce_motion = cx.update(|cx| cx.reduce_motion());
    initial.view = prefs.last_screen;
    change(&mut initial);
    assert!(initial.backend.is_none(), "walks must never connect to an engine");
    let state = cx.new(|_| initial);
    let bounds = Bounds {
        origin: Point::default(),
        size: size(px(1440.), px(900.)),
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let shell_state = state.clone();
    let (window, _) = cx
        .update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx))))
        .expect("open the real Shell");
    frame(cx, window);
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .expect("render a frame");
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, keys: &str) {
    cx.update_window(window, |_, window, cx| window.press(keys, cx))
        .expect("press real keys");
    frame(cx, window);
}

fn type_text(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .expect("type into the focused input");
    frame(cx, window);
}

fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(window, |_, window, _| {
        // Bare application Divs are identity scopes, not registered snapshots.
        // Their real rendered descendants/text retain the ancestor's element ID.
        let id: ElementId = id.into();
        window.try_find(id.clone()).is_some()
            || window.elements().iter().any(|element| element.path().contains(&id))
            || gpui_kit::base::test_support::texts(window)
                .iter()
                .any(|text| text.path().contains(&id))
    })
    .expect("find rendered element ID")
}

fn wish_row_height(cx: &mut TestAppContext, window: AnyWindowHandle) -> gpui_kit::Pixels {
    cx.update_window(window, |_, window, _| {
        let descendant = window.find(("wish-progress-ring", 0_usize));
        window
            .elements()
            .into_iter()
            .filter(|e| e.path().len() < descendant.path().len() && descendant.path().starts_with(e.path()))
            .max_by_key(|e| e.path().len())
            .expect("registered ListItem enclosing wish 0")
            .bounds()
            .size
            .height
    })
    .expect("measure the real Wishes row")
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx))
        .expect("click through the test platform");
    frame(cx, window);
}

// The LOCKED gpui-pre 0.3.11 TestWindow::minimize is unimplemented at this exact
// site. A panic interrupts GPUI's update stack, so the affected main window must
// not be reused. Never catch a product panic or silently call it a platform limit.
fn mini_platform_step(step: impl FnOnce()) -> bool {
    let site = std::sync::Arc::new(Mutex::new(None));
    let captured = site.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            *captured.lock().unwrap() = Some((location.file().replace('\\', "/"), location.line()));
        }
    }));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(step));
    std::panic::set_hook(previous);
    match result {
        Ok(()) => true,
        Err(error) => {
            let location = site.lock().unwrap().clone();
            let message = error
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| error.downcast_ref::<String>().map(String::as_str));
            let unsupported = location
                .as_ref()
                .is_some_and(|(file, line)| file.ends_with("/gpui-pre-0.3.11/src/platform/test/window.rs") && *line == 473)
                && message == Some("not implemented");
            if !unsupported {
                eprintln!("unexpected mini panic at {location:?}: {message:?}");
                std::panic::resume_unwind(error);
            }
            eprintln!("HANDOFF OPEN: TestWindow::minimize is unimplemented; native main-window minimize/restore, mini cleanup and palette Mini Undo cannot be verified after its interrupted update.");
            false
        }
    }
}

fn mini_window(cx: &mut TestAppContext, main: AnyWindowHandle) -> AnyWindowHandle {
    let windows = cx.windows();
    assert_eq!(windows.len(), 2, "the Shell reacts by opening a real mini window");
    let mini = windows.into_iter().find(|handle| *handle != main).expect("mini handle");
    frame(cx, mini);
    cx.update_window(mini, |_, window, _| {
        assert_eq!(
            window.viewport_size(),
            size(px(380.), px(520.)),
            "the real mini window has its compact bounds"
        );
    })
    .unwrap();
    assert!(shown(cx, mini, "title-bar"));
    mini
}

fn back_from_mini(cx: &mut TestAppContext, mini: AnyWindowHandle) {
    cx.update_window(mini, |_, window, cx| {
        // mini-back is an unobserved Div (no queryable snapshot or observed text).
        // Click its center above the bottom quota row, with REAL pointer events.
        let viewport = window.viewport_size();
        let row = theme::row_height(theme::density(cx));
        let inset = theme::space(theme::density(cx), Step::Sm);
        let position = gpui_kit::point(viewport.width / 2., viewport.height - row - inset - row / 2.);
        window.pointer_down(position, gpui_kit::MouseButton::Left, cx);
        window.pointer_up(position, gpui_kit::MouseButton::Left, cx);
    })
    .expect("click Back in the real mini window");
    cx.run_until_parked(); // On a complete platform, Back may already have removed this window.
}

fn command(cx: &mut TestAppContext, window: AnyWindowHandle, words: &str) {
    assert!(!shown(cx, window, "palette-scrim"));
    press(cx, window, PALETTE);
    assert!(shown(cx, window, "palette-scrim"), "Ctrl+K opens the palette for {words}");
    type_text(cx, window, words);
    press(cx, window, "enter");
    assert!(!shown(cx, window, "palette-scrim"), "Enter chooses {words} and closes the palette");
}

fn prefs(cx: &mut TestAppContext) -> Preferences {
    cx.update(|cx| snapshot!(Store::current(cx)))
}

fn assert_undo(state: &Entity<AppState>, cx: &TestAppContext, prior: Undoable, len: usize) {
    state.read_with(cx, |s, _| {
        assert_eq!(s.undo.len(), len, "one setting change records exactly one undo entry");
        assert_eq!(s.undo.clone().pop(), Some(prior));
    });
}

fn current_columns(cx: &mut TestAppContext, window: AnyWindowHandle) -> ColumnWidths {
    frame(cx, window);
    // RunView records ACTUAL ResizableState sizes after handling a splitter key.
    // Unchanged defaults have no stored override; no unobserved Div bounds are invented.
    prefs(cx).columns.unwrap_or(DEFAULT_COLUMNS)
}

fn focus_columns(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| {
        // Click the canvas above its registered bottom-right zoom controls.
        // The controls stop propagation, whereas the real canvas focuses Mesh.
        let controls = window.find("mesh-zoom-out").bounds();
        let position = gpui_kit::point(controls.left(), controls.top() - px(60.));
        window.pointer_down(position, gpui_kit::MouseButton::Left, cx);
        window.pointer_up(position, gpui_kit::MouseButton::Left, cx);
    })
    .expect("focus the real graph with a pointer click");
    frame(cx, window);
}

fn flush(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| Store::last_error(cx)), None, "debounced settings save succeeded");
}

fn assert_full(cx: &mut TestAppContext, window: AnyWindowHandle) {
    assert_eq!(prefs(cx).mode, "Full");
    for id in ["rail", "title-search", "mesh-graph", "todo-list", "run-columns"] {
        assert!(shown(cx, window, id), "Full renders {id}");
    }
    assert!(!shown(cx, window, "exit-focus"));
}

fn assert_focus(cx: &mut TestAppContext, window: AnyWindowHandle, state: &Entity<AppState>, panel: FocusPanel) {
    assert_eq!(prefs(cx).mode, focus_mode(panel));
    assert_eq!(
        state.read_with(cx, |s, _| s.view),
        Screen::Run,
        "focus forces the actual Run screen"
    );
    assert!(!shown(cx, window, "rail"), "focus hides the rail");
    assert!(!shown(cx, window, "title-search"), "focus hides title extras");
    assert!(shown(cx, window, "exit-focus"), "focus has an Exit focus control");
    assert_eq!(
        shown(cx, window, "mesh-graph"),
        panel == FocusPanel::Graph,
        "only Graph focus renders Graph"
    );
    assert_eq!(
        shown(cx, window, "todo-list"),
        panel == FocusPanel::Checklist,
        "only Checklist focus renders Checklist"
    );
    assert!(
        !shown(cx, window, "run-columns"),
        "focus does not render the full three-column layout"
    );
}

#[gpui_kit::test]
fn density_global_falls_back_sets_and_returns(cx: &mut TestAppContext) {
    let _home = TestHome::new("density-global");
    let (window, _) = open_with(cx, Store::open(true), |s| s.view = Screen::Wishes);
    cx.update(|cx| {
        assert!(
            cx.try_global::<Density>().is_none(),
            "the fallback is not a preinstalled Comfortable global"
        );
        assert_eq!(theme::density(cx), Density::Comfortable);
        assert_eq!(theme::row_height(theme::density(cx)), px(36.));
        assert_eq!(theme::space(theme::density(cx), Step::Xxl), px(32.));
    });
    // The painted ListItem changes with the density: 82 -> 58 -> 82.
    assert_eq!(wish_row_height(cx, window), px(82.));
    cx.update(|cx| theme::set_density(Density::Dense, cx));
    frame(cx, window);
    cx.update(|cx| {
        assert_eq!(theme::density(cx), Density::Dense);
        assert_eq!(theme::row_height(theme::density(cx)), px(28.));
        assert_eq!(theme::space(theme::density(cx), Step::Xxl), px(24.));
    });
    assert_eq!(wish_row_height(cx, window), px(58.), "an existing Shell redraws at Dense size");
    cx.update(|cx| theme::set_density(Density::Comfortable, cx));
    frame(cx, window);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Comfortable);
    assert_eq!(wish_row_height(cx, window), px(82.));
}

#[gpui_kit::test]
fn save_on_quit_captures_reduce_motion_and_restore_none_clears_it(cx: &mut TestAppContext) {
    let home = TestHome::new("quit-motion");
    cx.update(|cx| cx.set_reduce_motion(false));
    let (window, state) = open_with(cx, home.reopen(), |s| s.view = Screen::Doctor);
    assert_eq!(prefs(cx).reduce_motion, None);
    assert!(!state.read_with(cx, |s, _| s.reduce_motion));
    click(cx, window, "reduce-motion");
    assert!(state.read_with(cx, |s, _| s.reduce_motion));
    assert!(cx.update(|cx| cx.reduce_motion()));
    assert_eq!(prefs(cx).reduce_motion, Some(true));
    assert_undo(&state, cx, motion_none_prior(), 1);
    // No advance_clock/flush: TestAppContext::quit calls App::shutdown, awaiting real quit hooks.
    cx.quit();
    assert_eq!(home.saved().reduce_motion, Some(true), "save_on_quit flushes the pending override");
    let store = home.reopen();
    assert_eq!(store.settings.reduce_motion, Some(true), "a new Store reads the saved override");

    let mut restarted = cx.new_app();
    restarted.update(|cx| cx.set_reduce_motion(true));
    let (window, state) = open_with(&mut restarted, store, |_| {});
    // Change the LIVE driver after hook registration, not its captured system preference.
    restarted.update(|cx| cx.set_reduce_motion(false));
    assert!(!restarted.update(|cx| cx.reduce_motion()));
    // Inject precisely ReduceMotion(None); Ctrl+Z invokes the REAL private
    // settings::restore via Shell::undo, rather than restoring a copied Settings.
    state.update(&mut restarted, |s, _| s.record_undo(motion_none_prior()));
    press(&mut restarted, window, UNDO);
    assert_eq!(prefs(&mut restarted).reduce_motion, None);
    assert!(
        restarted.update(|cx| cx.reduce_motion()),
        "None restores the true system value captured by save_on_quit, not the live false override"
    );
    assert!(
        state.read_with(&restarted, |s, _| s.reduce_motion),
        "AppState follows the restored motion driver"
    );
    restarted.quit();
    assert_eq!(home.saved().reduce_motion, None, "quit persists the cleared override");
    assert_eq!(home.reopen().settings.reduce_motion, None);
}

#[gpui_kit::test]
fn palette_mode_commands(cx: &mut TestAppContext) {
    let _home = TestHome::new("palette-modes");
    let (window, state) = open(cx);
    assert_full(cx, window);
    for (words, mode) in [
        ("Focus on the graph", "Focus(Graph)"),
        ("Focus on the checklist", "Focus(Checklist)"),
    ] {
        assert_eq!(prefs(cx).mode, "Full");
        command(cx, window, words);
        assert_eq!(prefs(cx).mode, mode, "palette command: {words}");
        assert_undo(&state, cx, mode_prior("Full"), 1);
        command(cx, window, "Undo the last change");
        assert_full(cx, window);
        assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
    }
    command(cx, window, "Focus on the graph");
    command(cx, window, "Back to the full window");
    assert_full(cx, window);
    assert_undo(&state, cx, mode_prior("Focus(Graph)"), 2);
    command(cx, window, "Undo the last change");
    assert_focus(cx, window, &state, FocusPanel::Graph);
    command(cx, window, "Undo the last change");
    assert_full(cx, window);

    assert_eq!(prefs(cx).density, Density::Comfortable);
    command(cx, window, "Switch between comfortable and dense spacing");
    assert_eq!(prefs(cx).density, Density::Dense);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Dense);
    assert_undo(&state, cx, density_prior(Density::Comfortable), 1);
    command(cx, window, "Undo the last change");
    assert_eq!(prefs(cx).density, Density::Comfortable);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Comfortable);

    assert!(!prefs(cx).mini_on_top);
    command(cx, window, "Keep the mini window on top");
    assert!(prefs(cx).mini_on_top);
    assert_undo(&state, cx, on_top_prior(false), 1);
    command(cx, window, "Undo the last change");
    assert!(!prefs(cx).mini_on_top);
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);

    // Last: an unsupported native minimize invalidates this main window's update
    // stack, but must not prevent the other palette/Undo commands being checked.
    assert_eq!(prefs(cx).mode, "Full");
    let native = mini_platform_step(|| command(cx, window, "Mini window"));
    assert_eq!(prefs(cx).mode, "Mini");
    assert_undo(&state, cx, mode_prior("Full"), 1);
    let mini = mini_window(cx, window);
    if native {
        command(cx, window, "Undo the last change");
        assert_full(cx, window);
        assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
    } else {
        back_from_mini(cx, mini);
        assert_eq!(prefs(cx).mode, "Full", "Back still calls the real settings setter");
        assert_undo(&state, cx, mode_prior("Mini"), 2);
    }
}

#[gpui_kit::test]
fn ctrl_shift_f_enters_and_leaves_focus_for_graph_and_checklist(cx: &mut TestAppContext) {
    let _home = TestHome::new("focus-key");
    let (window, state) = open(cx);
    assert_full(cx, window);
    for (words, panel) in [
        ("Focus on the graph", FocusPanel::Graph),
        ("Focus on the checklist", FocusPanel::Checklist),
    ] {
        // The palette chooses the remembered panel; the shortcut toggles that same panel.
        command(cx, window, words);
        assert_focus(cx, window, &state, panel);
        press(cx, window, FOCUS);
        assert_full(cx, window);
        press(cx, window, FOCUS);
        assert_focus(cx, window, &state, panel);
        click(cx, window, "exit-focus");
        assert_full(cx, window);
        press(cx, window, FOCUS);
        assert_focus(cx, window, &state, panel);
        press(cx, window, FOCUS);
        assert_full(cx, window);
    }
    command(cx, window, "Go to Wishes");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
    assert!(!shown(cx, window, "exit-focus"));
    press(cx, window, FOCUS);
    assert_focus(cx, window, &state, FocusPanel::Checklist);
    press(cx, window, FOCUS);
    assert_full(cx, window);
}

#[gpui_kit::test]
fn ctrl_shift_d_toggles_density_and_ctrl_z_undoes_it(cx: &mut TestAppContext) {
    let _home = TestHome::new("density-key");
    let (window, state) = open(cx);
    assert_eq!(prefs(cx).density, Density::Comfortable);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Comfortable);
    press(cx, window, DENSITY);
    assert_eq!(prefs(cx).density, Density::Dense);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Dense);
    assert_undo(&state, cx, density_prior(Density::Comfortable), 1);
    press(cx, window, DENSITY);
    assert_eq!(prefs(cx).density, Density::Comfortable);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Comfortable);
    assert_undo(&state, cx, density_prior(Density::Dense), 2);
    press(cx, window, UNDO);
    assert_eq!(
        prefs(cx).density,
        Density::Dense,
        "Ctrl+Z restores the previous density outside text inputs"
    );
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Dense);
    assert_undo(&state, cx, density_prior(Density::Comfortable), 1);
    press(cx, window, UNDO);
    assert_eq!(prefs(cx).density, Density::Comfortable);
    assert_eq!(cx.update(|cx| theme::density(cx)), Density::Comfortable);
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
}

#[gpui_kit::test]
fn ctrl_shift_m_mini_and_back(cx: &mut TestAppContext) {
    let _home = TestHome::new("mini-key");
    let (window, state) = open(cx);
    assert_full(cx, window);
    assert_eq!(cx.windows().len(), 1);
    let native = mini_platform_step(|| press(cx, window, MINI));
    assert_eq!(prefs(cx).mode, "Mini");
    assert_undo(&state, cx, mode_prior("Full"), 1);
    let mini = mini_window(cx, window);
    back_from_mini(cx, mini);
    assert_eq!(prefs(cx).mode, "Full", "Back keeps Full in the real Store");
    assert_undo(&state, cx, mode_prior("Mini"), 2);
    if !native {
        return;
    } // Only the exact dependency limitation above permits this branch.
    frame(cx, window);
    assert_full(cx, window);
    assert_eq!(cx.windows().len(), 1, "Back closes the mini window");
    assert_undo(&state, cx, mode_prior("Mini"), 2);
    press(cx, window, MINI);
    assert_eq!(prefs(cx).mode, "Mini");
    assert_eq!(cx.windows().len(), 2);
    press(cx, window, MINI);
    assert_full(cx, window);
    assert_eq!(cx.windows().len(), 1, "the shortcut also closes the mini window");
}

#[gpui_kit::test]
fn alt_arrows_on_a_fresh_run_screen_resize_columns_without_a_click(cx: &mut TestAppContext) {
    let _home = TestHome::new("fresh-columns");
    let (window, state) = open(cx);
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Run);
    assert_eq!(current_columns(cx, window), DEFAULT_COLUMNS);
    // NO click, Tab, direct focus, or seeded column widths before these real key events.
    let steps = [
        (
            "alt-right",
            ColumnWidths {
                checklist: 316.,
                details: 380.,
            },
        ),
        ("alt-left", DEFAULT_COLUMNS),
        (
            "alt-shift-right",
            ColumnWidths {
                checklist: 300.,
                details: 364.,
            },
        ),
        ("alt-shift-left", DEFAULT_COLUMNS),
        (
            "alt-right",
            ColumnWidths {
                checklist: 316.,
                details: 380.,
            },
        ),
        (
            "alt-shift-left",
            ColumnWidths {
                checklist: 316.,
                details: 396.,
            },
        ),
        (
            "alt-up",
            ColumnWidths {
                checklist: 316.,
                details: 396.,
            },
        ),
        ("alt-0", DEFAULT_COLUMNS),
    ];
    let mut observed = Vec::new();
    let mut expected = Vec::new();
    for (keys, widths) in steps {
        press(cx, window, keys);
        flush(cx);
        observed.push((keys, current_columns(cx, window)));
        expected.push((keys, widths));
    }
    assert_eq!(
        observed, expected,
        "fresh Run splitter keys must work without a click, step by 16 px, ignore Alt+Up, and reset with Alt+0"
    );
    assert_eq!(prefs(cx).columns, Some(DEFAULT_COLUMNS));
}

#[gpui_kit::test]
fn column_widths_survive_a_restart(cx: &mut TestAppContext) {
    let home = TestHome::new("columns-restart");
    let (window, _) = open_with(cx, home.reopen(), |_| {});
    assert_eq!(current_columns(cx, window), DEFAULT_COLUMNS);
    // Focus is allowed here: the fresh/no-click contract has its own test above.
    focus_columns(cx, window);
    press(cx, window, "alt-right");
    press(cx, window, "alt-shift-left");
    let expected = ColumnWidths {
        checklist: 316.,
        details: 396.,
    };
    assert_eq!(current_columns(cx, window), expected, "real keys resize before saving");
    assert_eq!(prefs(cx).columns, Some(expected));
    // Drive past the lower bounds as well; clamping must be real, not just
    // in-range example values which would pass if clamping were removed.
    for _ in 0..6 {
        press(cx, window, "alt-left");
    }
    for _ in 0..7 {
        press(cx, window, "alt-shift-right");
    }
    let expected = ColumnWidths {
        checklist: 240.,
        details: 300.,
    };
    assert_eq!(prefs(cx).columns, Some(expected), "both splitters clamp at their lower limits");
    flush(cx);
    assert_eq!(home.saved().columns, Some(expected), "debounce wrote the changed widths");
    cx.quit();

    let mut restarted = cx.new_app();
    let store = home.reopen();
    assert_eq!(snapshot!(store.settings).columns, Some(expected));
    let (window, _) = open_with(&mut restarted, store, |_| {});
    assert_eq!(prefs(&mut restarted).columns, Some(expected));
    assert_eq!(
        current_columns(&mut restarted, window),
        expected,
        "a new Shell uses saved widths, not defaults"
    );
    let widths = current_columns(&mut restarted, window);
    assert!((240.0..480.0).contains(&widths.checklist));
    assert!((300.0..560.0).contains(&widths.details));
    // A key reads the NEW ResizableState, proving the Shell applied the file to
    // its layout rather than merely retaining the values in the Store.
    focus_columns(&mut restarted, window);
    press(&mut restarted, window, "alt-right");
    assert_eq!(
        prefs(&mut restarted).columns,
        Some(ColumnWidths {
            checklist: 256.,
            details: 300.
        })
    );
    restarted.quit();
}

#[gpui_kit::test]
fn reduce_motion_focus_full_and_density_survive_a_restart(cx: &mut TestAppContext) {
    let home = TestHome::new("preferences-restart");
    let (window, state) = open_with(cx, home.reopen(), |_| {});
    assert_eq!(prefs(cx).reduce_motion, None);
    assert_eq!(prefs(cx).mode, "Full");
    assert_eq!(prefs(cx).density, Density::Comfortable);
    command(cx, window, "Go to Settings");
    click(cx, window, "reduce-motion");
    command(cx, window, "Focus on the checklist");
    press(cx, window, DENSITY);
    assert_focus(cx, window, &state, FocusPanel::Checklist);
    assert_eq!(prefs(cx).reduce_motion, Some(true));
    assert_eq!(prefs(cx).density, Density::Dense);
    flush(cx);
    cx.quit();

    let mut restarted = cx.new_app();
    let store = home.reopen();
    assert_eq!(store.settings.reduce_motion, Some(true));
    assert_eq!(snapshot!(store.settings).mode, "Focus(Checklist)");
    assert_eq!(store.settings.density, Density::Dense);
    let (window, state) = open_with(&mut restarted, store, |_| {});
    restarted.executor().advance_clock(Duration::from_millis(60));
    frame(&mut restarted, window);
    assert_focus(&mut restarted, window, &state, FocusPanel::Checklist);
    assert!(state.read_with(&restarted, |s, _| s.reduce_motion));
    assert!(restarted.update(|cx| cx.reduce_motion()));
    assert_eq!(restarted.update(|cx| theme::density(cx)), Density::Dense);
    command(&mut restarted, window, "Back to the full window");
    assert_full(&mut restarted, window);
    restarted.quit();

    let mut full_restart = restarted.new_app();
    let store = home.reopen();
    assert_eq!(snapshot!(store.settings).mode, "Full", "leaving focus persists Full too");
    let (window, state) = open_with(&mut full_restart, store, |_| {});
    assert_full(&mut full_restart, window);
    assert_eq!(prefs(&mut full_restart).density, Density::Dense);
    assert!(state.read_with(&full_restart, |s, _| s.reduce_motion));
    full_restart.quit();
}

// audit-walk-2: keep the original walks above byte-for-byte; all new persistent
// fixtures live in their own tree and still share the process-wide HOME_LOCK.
impl TestHome {
    fn audit2(name: &str) -> Self {
        let lock = HOME_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = root.join(".axiom-tmp/audit-walk-2").join(format!("{name}-{}-{nonce}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let prior = std::env::var_os("AXIOM_HOME");
        std::env::set_var("AXIOM_HOME", &path);
        Self { _lock: lock, prior, path }
    }
}

const FIND: &str = if cfg!(target_os = "macos") { "cmd-f" } else { "ctrl-f" };
const SELECT_ALL: &str = if cfg!(target_os = "macos") { "cmd-a" } else { "ctrl-a" };
const NEW_WISH: &str = if cfg!(target_os = "macos") { "cmd-n" } else { "ctrl-n" };

fn focused(cx: &mut TestAppContext, window: AnyWindowHandle) -> gpui_kit::FocusHandle {
    cx.update_window(window, |_, window, cx| window.focused(cx).expect("a real control owns focus")).unwrap()
}

fn tab_to(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    for _ in 0..60 {
        press(cx, window, "tab");
        if cx.update_window(window, |_, window, _| window.find(id).focused() == Some(true)).unwrap() {
            return;
        }
    }
    let details = cx.update_window(window, |_, window, _| window.elements().into_iter().filter(|e| e.focused() == Some(true) || e.path().iter().any(|id| format!("{id:?}").contains("memory-save"))).collect::<Vec<_>>()).unwrap();
    panic!("Tab never reached {id}: {details:?}");
}

fn selected_in_store(cx: &mut TestAppContext, screen: &str) -> Option<String> {
    cx.update(|cx| Store::current(cx).view_memory(screen).selected)
}

fn focus_list_by_keys(cx: &mut TestAppContext, window: AnyWindowHandle) {
    press(cx, window, FIND);
    press(cx, window, "down"); // search's real MoveDown handler transfers focus to the list
    press(cx, window, "down");
    press(cx, window, "up"); // persist the first row's selection, even with no saved selection
    for _ in 0..3 { frame(cx, window); }
}

fn open_menu_keys(cx: &mut TestAppContext, window: AnyWindowHandle) {
    press(cx, window, "shift-f10");
    // Offscreen rows deliberately take two next-frame callbacks to obtain hitboxes.
    for _ in 0..3 { frame(cx, window); }
}

fn menu_items(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<(String, bool)> {
    assert!(shown(cx, window, "popup-menu"), "Shift+F10 must render a real popup menu");
    cx.update_window(window, |_, window, _| {
        let scope: ElementId = "popup-menu".into();
        window.elements().into_iter()
            .filter(|e| e.path().contains(&scope) && e.role() == Some(gpui_kit::Role::MenuItem))
            .map(|e| (e.label().expect("menu item label").to_owned(), e.selected().expect("menu exposes native selection")))
            .collect()
    }).unwrap()
}

fn assert_menu(cx: &mut TestAppContext, window: AnyWindowHandle, expected: &[(&str, bool)]) {
    let actual: Vec<_> = menu_items(cx, window).into_iter().map(|(label, _)| label).collect();
    assert_eq!(actual, expected.iter().map(|(label, _)| label.to_string()).collect::<Vec<_>>(), "entries come from the invoking target");
    // The locked kit omits aria-disabled on MenuItem. Prove enabled/disabled
    // behaviour by traversing a complete keyboard cycle, not by inventing a flag.
    let enabled: Vec<_> = expected.iter().filter(|(_, disabled)| !disabled).map(|(label, _)| label.to_string()).collect();
    let mut visited = Vec::new();
    for _ in 0..enabled.len() {
        press(cx, window, "down");
        let selected: Vec<_> = menu_items(cx, window).into_iter().filter(|(_, selected)| *selected).map(|(label, _)| label).collect();
        assert_eq!(selected.len(), 1, "exactly one menu item is selected");
        visited.push(selected[0].clone());
    }
    // Starting from an existing pointer highlight may rotate the cycle.
    assert!((0..enabled.len()).any(|offset| visited.iter().enumerate().all(|(i, label)| *label == enabled[(i + offset) % enabled.len()])), "Down visits enabled entries only: {visited:?}");
    // Leave the last entry selected, so the next Down always selects the first.
    for _ in 0..enabled.len() {
        if menu_items(cx, window).iter().any(|(label, selected)| Some(label) == enabled.last() && *selected) { return; }
        press(cx, window, "down");
    }
    panic!("could not reach the last enabled entry");
}

fn choose_menu(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    // Menus begin with no keyboard selection. Always use real arrows and inspect
    // their rendered selected state, never invoke the menu callback directly.
    for _ in 0..8 {
        press(cx, window, "down");
        if menu_items(cx, window).iter().any(|(text, selected)| text == label && *selected) {
            press(cx, window, "enter");
            assert!(!shown(cx, window, "popup-menu"), "Enter dismisses the activated menu");
            return;
        }
    }
    panic!("keyboard could not select menu entry {label}");
}

fn select_menu(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    for _ in 0..8 {
        press(cx, window, "down");
        if menu_items(cx, window).iter().any(|(text, selected)| text == label && *selected) { return; }
    }
    panic!("keyboard could not highlight {label}");
}

fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

fn seed_pair(s: &mut AppState) {
    use axiom::desktop::model::{RunStatus, Selection};
    let mut run = demo::sample_run();
    run.session = "walk-X".into();
    run.wish = "walk fixture X".into();
    run.status = RunStatus::Blocked;
    run.cwd = None; // no folder opener or project-file process can run
    run.goals.clear();
    run.units.truncate(2);
    run.modules.truncate(1);
    run.modules[0].units = vec![0, 1];
    let mut other = run.clone();
    other.session = "walk-Y".into();
    other.wish = "walk fixture Y".into();
    other.status = RunStatus::Active;
    s.runs = vec![run, other];
    s.current = Some(0);
    s.selection = Selection::Unit(0);
    s.memory.truncate(2);
    assert_eq!(s.memory.len(), 2);
    for (i, entry) in s.memory.iter_mut().enumerate() {
        entry.id = 200 - i as i64;
        entry.project = "walk-project".into();
        entry.title = format!("walk memory {}", if i == 0 { "X" } else { "Y" }).into();
        entry.body = format!("original body {i}").into();
        entry.corrected = false;
        entry.confirmed = "Nov 3, 05:15".into();
    }
    while s.take_undo().is_some() {}
}

const WISH_MENU: &[(&str, bool)] = &[("Open", false), ("Resume", false), ("Open folder", true), ("Copy session id", false)];
const UNIT_MENU: &[(&str, bool)] = &[("Open details", false), ("Copy", false), ("Comment on this item", false)];
const MEMORY_MENU: &[(&str, bool)] = &[("Copy", false), ("Correct…", false), ("Delete…", false)];

// Real defects: row bounds were cleared after prepaint and popup Up/Escape were
// captured by the list. Fixture correction: needs-you X precedes running Y.
#[gpui_kit::test]
fn shift_f10_menu_on_a_wish(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("wish-menu");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Wishes; });
    focus_list_by_keys(cx, window);
    assert_eq!(selected_in_store(cx, "wishes").as_deref(), Some("walk-X"));
    let row = focused(cx, window);
    open_menu_keys(cx, window);
    assert_menu(cx, window, WISH_MENU);
    press(cx, window, "down");
    press(cx, window, "down");
    press(cx, window, "up");
    assert!(menu_items(cx, window).iter().any(|(label, selected)| label == "Open" && *selected), "Up reverses Down");
    press(cx, window, "escape");
    assert!(!shown(cx, window, "popup-menu"));
    assert_eq!(focused(cx, window), row, "Escape returns focus to the invoking wish list");
    open_menu_keys(cx, window);
    choose_menu(cx, window, "Copy session id");
    assert_eq!(clipboard(cx).as_deref(), Some("walk-X"));
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
    // An active run still shows Resume, but it must not be clickable.
    press(cx, window, "down"); // running Y follows needs-you X
    assert_eq!(selected_in_store(cx, "wishes").as_deref(), Some("walk-Y"));
    open_menu_keys(cx, window);
    assert_menu(cx, window, &[("Open", false), ("Resume", true), ("Open folder", true), ("Copy session id", false)]);
}

// Find the actual checklist/graph focus scope by tabbing, not by injecting a
// FocusHandle or calling ContextMenu. The popup's ancestry identifies its owner.
fn unit_menu_by_keys(cx: &mut TestAppContext, window: AnyWindowHandle, panel: &'static str) -> gpui_kit::FocusHandle {
    for _ in 0..60 {
        press(cx, window, "tab");
        let invoker = focused(cx, window);
        open_menu_keys(cx, window);
        if shown(cx, window, "popup-menu") {
            let owner = cx.update_window(window, |_, window, _| window.find("popup-menu").path().contains(&ElementId::from(panel))).unwrap();
            if owner { assert_menu(cx, window, UNIT_MENU); return invoker; }
            press(cx, window, "escape");
        }
    }
    panic!("Tab + Shift+F10 never opened a unit menu in {panel}");
}

#[gpui_kit::test]
fn shift_f10_menu_on_a_unit(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("unit-menu");
    let (window, state) = open_with(cx, Store::open(true), seed_pair);
    for panel in ["todo-list", "mesh-graph"] {
        let invoker = unit_menu_by_keys(cx, window, panel);
        press(cx, window, "down");
        press(cx, window, "down");
        press(cx, window, "up");
        assert!(menu_items(cx, window).iter().any(|(label, selected)| label == "Open details" && *selected));
        press(cx, window, "escape");
        assert!(!shown(cx, window, "popup-menu"));
        assert_eq!(focused(cx, window), invoker, "{panel} gets focus back");
        open_menu_keys(cx, window);
        choose_menu(cx, window, "Copy");
        let expected = state.read_with(cx, |s, _| format!("{} — {}", s.runs[0].units[0].id, s.runs[0].units[0].title));
        assert_eq!(clipboard(cx), Some(expected), "{panel} menu acts on the selected unit");
    }
}

// Real defect: the parent prepaint cleared the selected row's keyboard-menu bounds.
#[gpui_kit::test]
fn shift_f10_menu_on_a_memory_entry(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("memory-menu");
    let (window, _) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    focus_list_by_keys(cx, window);
    assert_eq!(selected_in_store(cx, "memory").as_deref(), Some("200"));
    let row = focused(cx, window);
    open_menu_keys(cx, window);
    assert_menu(cx, window, MEMORY_MENU);
    press(cx, window, "down");
    press(cx, window, "down");
    press(cx, window, "up");
    assert!(menu_items(cx, window).iter().any(|(label, selected)| label == "Copy" && *selected));
    press(cx, window, "escape");
    assert!(!shown(cx, window, "popup-menu"));
    assert_eq!(focused(cx, window), row);
    open_menu_keys(cx, window);
    choose_menu(cx, window, "Copy");
    assert_eq!(clipboard(cx).as_deref(), Some("walk memory X\n\noriginal body 0"));
}

// Independent paths use a native right-click to open the SAME context menu,
// then real arrows/Enter. This lets action-time guards and correction/undo fail
// independently of a broken Shift+F10 anchor. No application action is dispatched.
fn pointer_menu(cx: &mut TestAppContext, window: AnyWindowHandle, row: impl Into<ElementId>) {
    let row = row.into();
    cx.update_window(window, |_, window, cx| {
        let leaf = window.elements().into_iter().find(|e| e.path().contains(&row))
            .map(|e| e.bounds())
            .or_else(|| gpui_kit::base::test_support::texts(window).into_iter().find(|e| e.path().contains(&row)).map(|e| e.bounds()))
            .or_else(|| match &row {
                ElementId::NamedInteger(name, 200) if name.as_ref() == "memory-row" => Some(window.find_by_label("walk memory X").bounds()),
                ElementId::NamedInteger(name, 199) if name.as_ref() == "memory-row" => Some(window.find_by_label("walk memory Y").bounds()),
                _ => None,
            })
            .expect("rendered row descendant supplies a real hit-test position");
        let position = leaf.center();
        window.pointer_down(position, gpui_kit::MouseButton::Right, cx);
        window.pointer_up(position, gpui_kit::MouseButton::Right, cx);
    }).unwrap();
    frame(cx, window);
    assert!(shown(cx, window, "popup-menu"), "native right-click opens the target's context menu");
}

fn assert_delete_dialog(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, _| {
        assert!(window.elements().iter().any(|e| e.role() == Some(gpui_kit::Role::AlertDialog) && e.focused() == Some(true)), "Delete opens an actual modal confirmation");
        let ok = window.find("ok");
        assert_eq!(ok.label(), Some("Delete"));
        assert!(ok.path().contains(&ElementId::from("dialog")));
        assert_eq!(window.find("cancel").label(), Some("Cancel"));
    }).unwrap();
    // HANDOFF OPEN NOT VERIFIED: the title/name are unobserved plain Div text;
    // snapshots expose neither. Target identity is checked by the deletion below.
}

fn pointer_menu_focus_walk(cx: &mut TestAppContext, screen: Screen) {
    let _home = TestHome::audit2("pointer-menu-focus");
    let mut app = cx.new_app();
    let (window, _) = open_with(&mut app, Store::open(true), |s| { seed_pair(s); s.view = screen; });
    focus_list_by_keys(&mut app, window);
    let (row, entries, action, copied): (ElementId, _, _, _) = match screen {
        Screen::Wishes => {
            press(&mut app, window, "down");
            (("wish-row", 0_usize).into(), WISH_MENU, "Copy session id", "walk-X")
        }
        Screen::Memory => (("memory-row", 200_u64).into(), MEMORY_MENU, "Copy", "walk memory X\n\noriginal body 0"),
        _ => unreachable!(),
    };
    let invoker = focused(&mut app, window);
    pointer_menu(&mut app, window, row.clone());
    assert_ne!(focused(&mut app, window), invoker);
    assert_menu(&mut app, window, entries);
    press(&mut app, window, "escape");
    assert!(!shown(&mut app, window, "popup-menu"), "{screen:?} context menu Escape must close it");
    assert_eq!(focused(&mut app, window), invoker, "{screen:?} menu Escape restores its invoking list");
    pointer_menu(&mut app, window, row);
    choose_menu(&mut app, window, action);
    assert_eq!(clipboard(&mut app).as_deref(), Some(copied));
}

// Real defect: Wishes captured Cancel before its descendant popup could dismiss.
#[gpui_kit::test]
fn wish_context_menu_escape_returns_focus(cx: &mut TestAppContext) { pointer_menu_focus_walk(cx, Screen::Wishes); }
#[gpui_kit::test]
fn memory_context_menu_escape_returns_focus(cx: &mut TestAppContext) { pointer_menu_focus_walk(cx, Screen::Memory); }

// Real defect: Wishes captured SelectUp before its descendant popup could navigate.
#[gpui_kit::test]
fn wish_context_menu_up_moves_the_selection(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("wish-menu-up");
    let (window, _) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Wishes; });
    focus_list_by_keys(cx, window);
    press(cx, window, "down");
    pointer_menu(cx, window, ("wish-row", 0_usize));
    select_menu(cx, window, "Resume");
    press(cx, window, "up");
    assert!(menu_items(cx, window).iter().any(|(label, selected)| label == "Open" && *selected), "Up moves from Resume to Open in the wish context menu");
}

// Real defect: the popup's identity followed the array index, not its wish session.
// Test correction: backend=None silently skips commands; use a local rejecting
// backend so the Resume request and its target are observable, without an engine.
#[gpui_kit::test]
fn menu_target_is_resolved_by_id_after_a_reorder_wish(cx: &mut TestAppContext) {
    use axiom::base::{Failure, Res};
    use axiom::desktop::model::{Action, Backend, WishRequest};
    struct RejectResume;
    impl Backend for RejectResume {
        fn start_wish(&self, _: WishRequest) -> Res<()> { panic!("unexpected start_wish") }
        fn interject(&self, _: &str, _: &str) -> Res<()> { panic!("unexpected interject") }
        fn stop(&self, _: &str) -> Res<()> { panic!("unexpected stop") }
        fn resume(&self, session: &str, note: Option<&str>) -> Res<()> {
            assert_eq!(session, "walk-X", "the real Resume call must target X");
            assert_eq!(note, None);
            Err(Failure::new("walk", "walk Resume probe"))
        }
        fn delete_memory(&self, _: i64) -> Res<()> { panic!("unexpected delete_memory") }
        fn correct_memory(&self, _: i64, _: &str) -> Res<()> { panic!("unexpected correct_memory") }
        fn refresh(&self) -> Res<()> { panic!("unexpected refresh") }
        fn recheck(&self) -> Res<()> { panic!("unexpected recheck") }
        fn follow(&self, _: &str) -> Res<()> { Ok(()) }
    }
    let _home = TestHome::audit2("wish-reorder");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Wishes; });
    state.update(cx, |s, _| s.backend = Some(Box::new(RejectResume)));
    focus_list_by_keys(cx, window);
    press(cx, window, "down");
    pointer_menu(cx, window, ("wish-row", 0_usize));
    assert_menu(cx, window, WISH_MENU);
    select_menu(cx, window, "Resume");
    state.update(cx, |s, cx| { s.runs.swap(0, 1); cx.notify(); });
    frame(cx, window);
    assert!(menu_items(cx, window).iter().any(|(label, selected)| label == "Resume" && *selected), "reordering preserves the open menu and its highlighted action");
    press(cx, window, "enter");
    state.read_with(cx, |s, _| {
        assert_eq!(s.run().unwrap().session.as_ref(), "walk-X", "Resume resolves X's new index");
        assert_eq!(s.view, Screen::Run);
        assert_eq!(s.errors.len(), 1, "exactly one Resume request reaches the local rejecting backend");
        assert_eq!(s.errors.last().map(|e| (e.action, e.session.as_deref())), Some((Action::Resume, Some("walk-X"))), "Resume reaches the backend for X, never Y");
    });
}

#[gpui_kit::test]
fn wish_resume_rechecks_allowed_at_enter(cx: &mut TestAppContext) {
    use axiom::desktop::model::RunStatus;
    let _home = TestHome::audit2("wish-allowed");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Wishes; });
    focus_list_by_keys(cx, window);
    press(cx, window, "down");
    pointer_menu(cx, window, ("wish-row", 0_usize));
    select_menu(cx, window, "Resume");
    let before = state.read_with(cx, |s, _| s.errors.clone());
    // Change permission between menu paint and activation, without refreshing
    // its enabled snapshot. The stale Resume entry must still re-check state.
    state.update(cx, |s, _| { s.runs.iter_mut().find(|r| r.session == "walk-X").unwrap().status = RunStatus::Done; });
    assert!(shown(cx, window, "popup-menu"));
    press(cx, window, "enter"); // enabled snapshot, disallowed NOW
    assert_eq!(state.read_with(cx, |s, _| s.errors.clone()), before, "Resume re-checks current status at Enter");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
}

#[gpui_kit::test]
fn wish_menu_removed_target_is_a_noop(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("wish-removed");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Wishes; });
    focus_list_by_keys(cx, window);
    press(cx, window, "down");
    pointer_menu(cx, window, ("wish-row", 0_usize));
    select_menu(cx, window, "Resume");
    let before = state.read_with(cx, |s, _| s.errors.clone());
    state.update(cx, |s, _| { s.runs.retain(|r| r.session != "walk-X"); s.current = Some(0); });
    assert!(shown(cx, window, "popup-menu"), "activate the stale menu, not an already-dismissed one");
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.errors.clone()), before, "a removed run cannot receive Resume");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
}

#[gpui_kit::test]
fn menu_target_is_resolved_by_id_after_a_reorder_unit(cx: &mut TestAppContext) {
    use axiom::desktop::model::Selection;
    let _home = TestHome::audit2("unit-reorder");
    let (window, state) = open_with(cx, Store::open(true), seed_pair);
    unit_menu_by_keys(cx, window, "todo-list");
    select_menu(cx, window, "Open details");
    state.update(cx, |s, cx| { s.runs[0].units.swap(0, 1); s.selection = Selection::None; cx.notify(); });
    frame(cx, window);
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.selection.clone()), Selection::Unit(1), "Open details follows unit X to index 1");
    state.update(cx, |s, cx| { s.selection = Selection::Unit(1); cx.notify(); });
    frame(cx, window);
    // A fresh graph owns its own cursor; select X explicitly in AppState, then
    // tab into the graph without arrow movement before opening the next menu.
    unit_menu_by_keys(cx, window, "mesh-graph");
    select_menu(cx, window, "Open details");
    state.update(cx, |s, cx| {
        s.runs[0].units.retain(|u| u.id != "core");
        s.runs[0].modules[0].units = vec![0];
        s.selection = Selection::None;
        cx.notify();
    });
    frame(cx, window);
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.selection.clone()), Selection::None, "removed unit X never resolves to its old index");
}

#[gpui_kit::test]
fn menu_target_is_resolved_by_id_after_a_reorder_memory(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("memory-reorder");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    focus_list_by_keys(cx, window);
    pointer_menu(cx, window, ("memory-row", 200_u64));
    select_menu(cx, window, "Copy");
    state.update(cx, |s, cx| { s.memory.swap(0, 1); cx.notify(); });
    frame(cx, window);
    press(cx, window, "enter");
    assert_eq!(clipboard(cx).as_deref(), Some("walk memory X\n\noriginal body 0"), "Copy follows memory id 200 after reorder");
    pointer_menu(cx, window, ("memory-row", 200_u64));
    select_menu(cx, window, "Copy");
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into())));
    let remaining = state.read_with(cx, |s, _| s.memory.iter().find(|m| m.id == 199).unwrap().clone());
    state.update(cx, |s, _| { s.memory.retain(|m| m.id != 200); });
    assert!(shown(cx, window, "popup-menu"), "the stale entry is still available for Enter");
    press(cx, window, "enter");
    assert_eq!(clipboard(cx).as_deref(), Some("sentinel"), "removed memory cannot copy Y at X's old index");
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), vec![remaining]);
}

#[gpui_kit::test]
fn delete_reaches_the_confirmation(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("delete-key");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    focus_list_by_keys(cx, window);
    let before = state.read_with(cx, |s, _| s.memory.clone());
    press(cx, window, "delete");
    assert_delete_dialog(cx, window);
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before, "Delete alone never deletes");
    press(cx, window, "escape");
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before, "Escape cancels deletion");
    assert!(!shown(cx, window, "dialog"));
    press(cx, window, "delete");
    assert_delete_dialog(cx, window);
    tab_to(cx, window, "ok");
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), vec![before[1].clone()], "confirmation removes only the named entry locally");
}

#[gpui_kit::test]
fn menu_delete_reaches_the_same_confirmation(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("delete-menu");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    focus_list_by_keys(cx, window);
    let before = state.read_with(cx, |s, _| s.memory.clone());
    pointer_menu(cx, window, ("memory-row", 200_u64));
    choose_menu(cx, window, "Delete…");
    assert_delete_dialog(cx, window);
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before, "menu Delete also requires confirmation");
    press(cx, window, "escape");
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before);
}

fn correct_by_keys(cx: &mut TestAppContext, window: AnyWindowHandle) {
    focus_list_by_keys(cx, window);
    pointer_menu(cx, window, ("memory-row", 200_u64));
    choose_menu(cx, window, "Correct…");
    assert!(shown(cx, window, "memory-save"), "Correct opens the real inline correction editor");
    press(cx, window, SELECT_ALL);
    type_text(cx, window, "corrected through real keys");
    press(cx, window, FIND); // Tab inside a textarea is indentation, not navigation.
    let mut reached = false;
    for _ in 0..60 {
        press(cx, window, "shift-tab");
        if cx.update_window(window, |_, window, _| window.find("memory-save").focused() == Some(true)).unwrap() { reached = true; break; }
    }
    assert!(reached, "Shift+Tab reaches Save correction from outside the textarea");
    press(cx, window, "enter");
    assert!(!shown(cx, window, "memory-save"), "saving closes the editor");
}

#[gpui_kit::test]
fn ctrl_z_undoes_a_correction(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("correction-undo");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    let before = state.read_with(cx, |s, _| s.memory.clone());
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
    correct_by_keys(cx, window);
    state.read_with(cx, |s, _| {
        let entry = s.memory.iter().find(|m| m.id == 200).unwrap();
        assert_eq!(entry.body.as_ref(), "corrected through real keys");
        assert!(entry.corrected);
        assert_eq!(s.undo.len(), 1);
        match s.undo.clone().pop().expect("correction records undo") {
            Undoable::Memory { id, prior } => {
                assert_eq!(id, 200);
                assert_eq!(prior.body, before[0].body.as_ref());
                assert_eq!(prior.summary, before[0].title.as_ref());
                assert_eq!(prior.corrected, None);
            }
            other => panic!("expected memory undo, got {other:?}"),
        }
    });
    assert!(shown(cx, window, "memory-undo"), "Memory offers Undo after a successful correction");
    press(cx, window, UNDO);
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before, "undo restores body, summary/title, corrected AND confirmed fields");
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
    assert!(!shown(cx, window, "memory-undo"));
    press(cx, window, UNDO);
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), before, "empty-stack undo is a no-op");
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0);
}

#[gpui_kit::test]
fn ctrl_z_inside_a_text_box_is_not_intercepted(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("text-undo");
    let (window, state) = open_with(cx, Store::open(true), |s| { seed_pair(s); s.view = Screen::Memory; });
    // Produce a genuine memory undo before focusing the text box.
    state.update(cx, |s, cx| s.correct_memory(200, "pending correction", cx));
    frame(cx, window);
    let corrected = state.read_with(cx, |s, _| s.memory.clone());
    press(cx, window, FIND);
    type_text(cx, window, "original");
    let search_value = |cx: &mut TestAppContext| cx.update_window(window, |_, window, _| {
        window.elements().into_iter().find(|e| e.placeholder() == Some("Search memories"))
            .expect("real memory search input").value().map(str::to_owned)
    }).unwrap();
    assert_eq!(search_value(cx).as_deref(), Some("original"));
    press(cx, window, UNDO);
    assert_ne!(search_value(cx).as_deref(), Some("original"), "the input itself receives and handles Ctrl+Z");
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 1, "text-box undo must not pop AppState.undo");
    assert_eq!(state.read_with(cx, |s, _| s.memory.clone()), corrected, "no memory undo ran in the text box");
    tab_to(cx, window, "nav-memory");
    press(cx, window, UNDO);
    assert_eq!(state.read_with(cx, |s, _| s.undo.len()), 0, "outside text inputs the same key pops the stack");
    assert_eq!(state.read_with(cx, |s, _| s.memory.iter().find(|m| m.id == 200).unwrap().body.to_string()), "original body 0");
}

#[gpui_kit::test]
fn tab_enter_space_escape_return_focus(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("focus-return");
    let mut store = Store::open(true);
    store.settings.reduce_motion = Some(true); // no outgoing screen's temporary tab stops
    let (window, state) = open_with(cx, store, seed_pair);
    tab_to(cx, window, "nav-run");
    let rail_run = focused(cx, window);
    press(cx, window, "tab");
    cx.update_window(window, |_, window, _| assert_eq!(window.find("nav-wishes").focused(), Some(true), "Tab order: Run -> Wishes")).unwrap();
    let rail_wishes = focused(cx, window);
    press(cx, window, "shift-tab");
    assert_eq!(focused(cx, window), rail_run, "reverse Tab returns along the same order");
    press(cx, window, "tab");
    assert_eq!(focused(cx, window), rail_wishes, "the forward order is stable");
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes, "Enter activates the focused rail control");
    tab_to(cx, window, "nav-memory");
    press(cx, window, "space");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Memory, "Space activates the focused rail control");
    let invoker = focused(cx, window);
    assert_ne!(invoker, rail_run);
    for (keys, overlay) in [(PALETTE, "palette-scrim"), ("?", "shortcuts-scrim")] {
        press(cx, window, keys);
        assert!(shown(cx, window, overlay));
        assert_ne!(focused(cx, window), invoker);
        press(cx, window, "escape");
        assert!(!shown(cx, window, overlay));
        assert_eq!(focused(cx, window), invoker, "{overlay} returns focus to its invoking control");
    }
    // No folder: Composer never runs git status or enumerates project files.
    // Members::detect only resolves executables; no backend or model is invoked.
    tab_to(cx, window, "new-wish");
    let new_wish = focused(cx, window);
    press(cx, window, "enter");
    assert!(state.read_with(cx, |s, _| s.composing));
    assert_ne!(focused(cx, window), new_wish);
    press(cx, window, "escape");
    assert!(!state.read_with(cx, |s, _| s.composing));
    assert_eq!(focused(cx, window), new_wish, "New wish returns focus to its invoking button");
    press(cx, window, NEW_WISH);
    assert!(state.read_with(cx, |s, _| s.composing), "the real New wish shortcut also opens it");
    press(cx, window, "escape");
    assert_eq!(focused(cx, window), new_wish);
    state.update(cx, |s, cx| s.set_view(Screen::Run, cx));
    frame(cx, window);
    let unit = unit_menu_by_keys(cx, window, "todo-list");
    press(cx, window, "escape");
    assert!(!shown(cx, window, "popup-menu"));
    assert_eq!(focused(cx, window), unit, "the context menu returns focus to its invoking checklist");
}

fn saved_view_store(home: &TestHome, screen: Screen, missing: bool) -> Store {
    let mut store = home.reopen();
    store.settings.last_screen = screen;
    store.settings.reduce_motion = Some(true);
    let key = match screen { Screen::Wishes => "wishes", Screen::Memory => "memory", Screen::Usage => "usage", _ => unreachable!() };
    store.settings.with_view(key, |v| {
        v.selected = Some(if missing { if screen == Screen::Memory { "999" } else { "absent-selection" } } else { match screen { Screen::Wishes => "walk-Y", Screen::Memory => "199", Screen::Usage => "walk-Y", _ => unreachable!() } }.into());
        match screen {
            Screen::Wishes => v.filter = Some("all|walk fixture".into()),
            Screen::Memory => { v.filter = Some("walk-project".into()); v.sort = Some("original".into()); }
            Screen::Usage => v.sort = Some("all".into()),
            _ => unreachable!(),
        }
        v.scroll = 12.;
    });
    std::fs::write(home.path.join("app-settings.json"), store.settings.to_json()).expect("write real Store fixture under audit-walk-2");
    let loaded = home.reopen();
    assert_eq!(loaded.settings.view_memory(key), store.settings.view_memory(key), "persisted selection exists BEFORE constructing Shell");
    loaded
}

fn settle_restore(cx: &mut TestAppContext, window: AnyWindowHandle) {
    // Drive both nested on_next_frame callbacks and subsequent list paint. A
    // product panic is intentionally NOT caught (decision 73 regression).
    for _ in 0..5 { frame(cx, window); }
}

fn saved_wishes(cx: &mut TestAppContext, missing: bool) {
    let home = TestHome::audit2(if missing { "saved-wishes-missing" } else { "saved-wishes" });
    let store = saved_view_store(&home, Screen::Wishes, missing);
    let (window, state) = open_with(cx, store, seed_pair);
    settle_restore(cx, window);
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Wishes);
    cx.update_window(window, |_, window, _| {
        let search = window.elements().into_iter().find(|e| e.placeholder() == Some("Search wishes and projects")).unwrap();
        assert_eq!(search.value(), Some("walk fixture"), "saved search is applied, not just retained in Store");
    }).unwrap();
    press(cx, window, FIND);
    press(cx, window, "down"); // transfer focus ONLY; do not change the restored selection
    press(cx, window, "enter");
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Run);
    assert_eq!(state.read_with(cx, |s, _| s.run().unwrap().session.to_string()), if missing { "walk-X" } else { "walk-Y" }, "Enter opens the restored wish (or the first visible fallback)");
    flush(cx);
    assert_ne!(home.reopen().settings.view_memory("wishes").selected.as_deref(), Some("absent-selection"));
}

#[gpui_kit::test]
fn saved_selection_restores_without_a_panic_wishes(cx: &mut TestAppContext) { saved_wishes(cx, false); }
#[gpui_kit::test]
fn saved_selection_missing_falls_back_wishes(cx: &mut TestAppContext) { saved_wishes(cx, true); }

fn saved_memory(cx: &mut TestAppContext, missing: bool) {
    let home = TestHome::audit2(if missing { "saved-memory-missing" } else { "saved-memory" });
    let store = saved_view_store(&home, Screen::Memory, missing);
    let (window, state) = open_with(cx, store, seed_pair);
    settle_restore(cx, window);
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Memory);
    cx.update_window(window, |_, window, _| {
        assert!(window.elements().iter().any(|e| e.placeholder() == Some("Search memories") && e.value() == Some("original")), "Memory restores its saved search");
        assert!(window.elements().iter().any(|e| e.role() == Some(gpui_kit::Role::ComboBox) && e.value() == Some("walk-project")), "Memory restores its saved project filter");
    }).unwrap();
    // The detail menu uses the selected entry. Do not move the list cursor:
    // copying from it proves the saved ID reached the actual view, not just Store.
    tab_to(cx, window, "memory-menu");
    press(cx, window, "enter");
    choose_menu(cx, window, "Copy");
    assert_eq!(clipboard(cx).as_deref(), Some(if missing { "walk memory X\n\noriginal body 0" } else { "walk memory Y\n\noriginal body 1" }));
    flush(cx);
    assert_ne!(home.reopen().settings.view_memory("memory").selected.as_deref(), Some("999"));
}

#[gpui_kit::test]
fn saved_selection_restores_without_a_panic_memory(cx: &mut TestAppContext) { saved_memory(cx, false); }
#[gpui_kit::test]
fn saved_selection_missing_falls_back_memory(cx: &mut TestAppContext) { saved_memory(cx, true); }

fn saved_usage(cx: &mut TestAppContext, missing: bool) {
    let home = TestHome::audit2(if missing { "saved-usage-missing" } else { "saved-usage" });
    let store = saved_view_store(&home, Screen::Usage, missing);
    let (window, state) = open_with(cx, store, |s| { seed_pair(s); for run in &mut s.runs { run.usage = Default::default(); } });
    settle_restore(cx, window);
    assert_eq!(state.read_with(cx, |s, _| s.view), Screen::Usage);
    cx.update_window(window, |_, window, cx| {
        let rows: Vec<_> = window.elements().into_iter().filter(|e| e.role() == Some(gpui_kit::Role::Row) && e.path().contains(&ElementId::from("usage-runs"))).collect();
        assert_eq!(rows.len(), 3, "usage renders a header and both run rows");
        let expected: ElementId = ("tr", if missing { 0_usize } else { 1_usize }).into();
        let row = rows.iter().find(|e| e.path().contains(&expected)).expect("saved run's actual table row");
        let bounds = row.bounds();
        let scale = window.scale_factor();
        assert!(window.painted_quads().iter().any(|q| {
            (q.bounds.origin.x.0 - f32::from(bounds.origin.x) * scale).abs() < 1.
                && (q.bounds.origin.y.0 - f32::from(bounds.origin.y) * scale).abs() < 1.
                && (q.bounds.size.width.0 - f32::from(bounds.size.width) * scale).abs() < 1.
                && q.border_color == cx.global::<theme::Tokens>().gold
                && q.border_widths.left.0 > 0.
        }), "the restored usage run has the painted gold selection border, not merely a saved ID: {bounds:?}");
    }).unwrap();
    assert_eq!(selected_in_store(cx, "usage").as_deref(), if missing { None } else { Some("walk-Y") });
    flush(cx);
    assert_ne!(home.reopen().settings.view_memory("usage").selected.as_deref(), Some("absent-selection"));
}

#[gpui_kit::test]
fn saved_selection_restores_without_a_panic_usage(cx: &mut TestAppContext) { saved_usage(cx, false); }
#[gpui_kit::test]
fn saved_selection_missing_falls_back_usage(cx: &mut TestAppContext) { saved_usage(cx, true); }

#[gpui_kit::test]
fn mini_back_and_close_return_to_full(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("mini-back-close");
    // Each native minimize interruption leaves that main window unusable. Use
    // independent apps for Back and Close; never catch a product panic.
    let mut close_app = cx.new_app();
    for (app, close) in [(cx, false), (&mut close_app, true)] {
        let (main, state) = open_with(app, Store::open(true), seed_pair);
        let before = state.read_with(app, |s, _| (s.view, s.current, s.selection.clone(), s.runs.clone(), s.memory.clone()));
        let native = mini_platform_step(|| press(app, main, MINI));
        assert_eq!(prefs(app).mode, "Mini");
        let mini = mini_window(app, main);
        if close {
            // Windows title-bar buttons are native WindowControlArea, not
            // on_click handlers. Drive the real should-close hook; the locked
            // kit keeps TestWindow's final close notification private.
            let mut visual = gpui_kit::VisualTestContext::from_window(mini, app);
            assert!(visual.simulate_close(), "the mini permits native close");
            app.run_until_parked();
        } else {
            back_from_mini(app, mini);
        }
        assert_eq!(prefs(app).mode, "Full", "both Back and Close restore Full");
        assert!(app.windows().contains(&main), "neither path closes/quits the main window");
        state.read_with(app, |s, _| {
            assert!(s.backend.is_none(), "no Backend exists to shut down");
            assert_eq!((s.view, s.current, s.selection.clone(), s.runs.clone(), s.memory.clone()), before, "main window data survives the mini transition");
        });
        if native {
            frame(app, main);
            assert_full(app, main);
            assert_eq!(app.windows(), vec![main]);
        }
        // HANDOFF OPEN NOT VERIFIED: native main minimize/restore and Back's
        // mini cleanup require TestWindow::minimize, unimplemented in the lock;
        // final native close notification is private in this dependency.
    }
}

#[gpui_kit::test]
fn wishes_hover_lift_changes_the_painted_card(cx: &mut TestAppContext) {
    let _home = TestHome::audit2("wish-hover");
    let mut store = Store::open(true);
    store.settings.reduce_motion = Some(true); // deterministic immediate hover level
    let (window, _) = open_with(cx, store, |s| { seed_pair(s); s.view = Screen::Wishes; });
    let card_y = |cx: &mut TestAppContext| cx.update_window(window, |_, window, cx| {
        let leaf = window.find(("wish-progress-ring", 0_usize));
        let scale = window.scale_factor();
        let point = leaf.bounds().center();
        window.painted_quads().into_iter().find(|q| q.background.as_solid() == Some(cx.global::<theme::Tokens>().raised)
            && q.bounds.size.width.0 > 700. * scale
            && q.bounds.origin.y.0 < f32::from(point.y) * scale
            && q.bounds.origin.y.0 + q.bounds.size.height.0 > f32::from(point.y) * scale)
            .expect("painted raised wish card").bounds.origin.y.0 / scale
    }).unwrap();
    let before = card_y(cx);
    cx.update_window(window, |_, window, cx| {
        let position = window.find(("wish-progress-ring", 0_usize)).bounds().center();
        window.pointer_move(position, None, cx);
    }).unwrap();
    frame(cx, window);
    assert!(card_y(cx) < before, "hover must lift the real painted card");
    cx.update_window(window, |_, window, cx| window.pointer_move(gpui_kit::point(px(1400.), px(880.)), None, cx)).unwrap();
    frame(cx, window);
    assert_eq!(card_y(cx), before, "leaving removes the lift");
    // HANDOFF OPEN NOT VERIFIED: selection glow and instantaneous row/rail focus
    // rings are BoxShadows; the locked platform exposes quads, not shadow data,
    // and has no HeadlessRenderer for raster comparison.
}
