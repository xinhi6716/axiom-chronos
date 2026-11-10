//! Exercises auto-evolve through the real Usage screen and a recording backend.
use axiom::base::Res;
use axiom::desktop::model::*;
use axiom::desktop::{demo, i18n, theme, Shell, Store};
use gpui_kit::component::ThemeMode;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{px, size, AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions};
use std::{cell::{Cell, RefCell}, path::{Path, PathBuf}, rc::Rc};

#[derive(Default)]
struct Recording {
    checks: Cell<usize>,
    starts: RefCell<Vec<PathBuf>>,
    refreshes: Cell<usize>,
}

struct Fake(Rc<Recording>);
impl Backend for Fake {
    fn evolve_check(&self) -> Res<()> { self.0.checks.set(self.0.checks.get() + 1); Ok(()) }
    fn evolve_start(&self, repo: &Path) -> Res<()> { self.0.starts.borrow_mut().push(repo.to_owned()); Ok(()) }
    fn start_wish(&self, _: WishRequest) -> Res<()> { Ok(()) }
    fn interject(&self, _: &str, _: &str) -> Res<()> { Ok(()) }
    fn stop(&self, _: &str) -> Res<()> { Ok(()) }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> { Ok(()) }
    fn delete_memory(&self, _: i64) -> Res<()> { Ok(()) }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> { Ok(()) }
    fn refresh(&self) -> Res<()> { self.0.refreshes.set(self.0.refreshes.get() + 1); Ok(()) }
    fn recheck(&self) -> Res<()> { Ok(()) }
    fn follow(&self, _: &str) -> Res<()> { Ok(()) }
}

fn open(cx: &mut TestAppContext, initial: AppState) -> (AnyWindowHandle, Entity<AppState>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, None, cx);
        i18n::set(i18n::Lang::En, cx);
        cx.set_global(Store::open(true));
    });
    let state = cx.new(|_| initial);
    let bounds = Bounds { origin: Point::default(), size: size(px(1440.), px(900.)) };
    let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
    let shell_state = state.clone();
    let (window, _) = cx.update(|cx| gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Shell::new(shell_state, false, window, cx)))).expect("open shell");
    frame(cx, window);
    (window, state)
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render");
    cx.run_until_parked();
}
fn view(cx: &mut TestAppContext, window: AnyWindowHandle, state: &Entity<AppState>, screen: Screen) {
    state.update(cx, |state, cx| state.set_view(screen, cx));
    frame(cx, window);
}
fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) {
    cx.update_window(window, |_, window, cx| window.click(id, cx)).expect("click");
    frame(cx, window);
}
fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(window, |_, window, _| window.try_find(id).is_some()).expect("query")
}
fn finish_check(cx: &mut TestAppContext, window: AnyWindowHandle, state: &Entity<AppState>, repo: Option<PathBuf>) {
    state.update(cx, |state, cx| {
        let mut preview = demo::evolve_preview();
        preview.repo = repo;
        state.evolve.preview = Some(preview);
        state.evolve.checking = false;
        cx.notify();
    });
    frame(cx, window);
}

#[gpui_kit::test]
fn navigation_refresh_open_and_cancel_never_start(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    assert!(recording.starts.borrow().is_empty());
    for screen in [Screen::Usage, Screen::Run, Screen::Wishes, Screen::Usage] { view(cx, window, &state, screen); }
    state.update(cx, |state, cx| {
        state.backend.as_ref().unwrap().refresh().unwrap();
        state.replace_runs(state.runs.clone(), &[], None);
        cx.notify();
    });
    frame(cx, window);
    assert_eq!(recording.refreshes.get(), 1);
    assert!(shown(cx, window, "evolve-open"));
    click(cx, window, "evolve-open");
    assert_eq!(recording.checks.get(), 1);
    assert!(shown(cx, window, "evolve-cancel"));
    assert!(!shown(cx, window, "evolve-pick"));
    click(cx, window, "evolve-cancel");
    assert!(!shown(cx, window, "evolve-confirm"));
    assert!(recording.starts.borrow().is_empty());
}

#[gpui_kit::test]
fn confirm_starts_once_with_the_preview_repo_even_on_double_click(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    let repo = demo::evolve_preview().repo.unwrap();
    finish_check(cx, window, &state, Some(repo.clone()));
    assert!(shown(cx, window, "evolve-pick"));
    cx.update_window(window, |_, window, _| {
        assert_ne!(window.find("evolve-confirm").disabled(), Some(true));
    }).unwrap();
    click(cx, window, "evolve-confirm");
    assert_eq!(&*recording.starts.borrow(), &[repo.clone()]);
    click(cx, window, "evolve-confirm");
    assert_eq!(&*recording.starts.borrow(), &[repo]);
}

#[gpui_kit::test]
fn dialog_displays_the_preview_repo(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording)));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    let repo = demo::evolve_preview().repo.unwrap();
    finish_check(cx, window, &state, Some(repo.clone()));
    cx.update_window(window, |_, window, _| {
        assert!(gpui_kit::base::test_support::texts(window).iter().any(|text| text.visible() && text.text() == repo.to_string_lossy()), "preview repository was not observed as painted text");
    }).unwrap();
}

#[gpui_kit::test]
fn picked_folder_is_displayed_and_passed_to_start(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    let repo = demo::evolve_preview().repo.unwrap();
    finish_check(cx, window, &state, Some(repo.clone()));
    let picked = repo.with_file_name("picked-repository");
    click(cx, window, "evolve-pick");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![picked.clone()])
    });
    frame(cx, window);
    cx.update_window(window, |_, window, _| {
        assert!(gpui_kit::base::test_support::texts(window).iter().any(|text| text.visible() && text.text() == picked.to_string_lossy()));
    }).unwrap();
    click(cx, window, "evolve-confirm");
    assert_eq!(&*recording.starts.borrow(), &[picked]);
}

#[gpui_kit::test]
fn bad_repository_error_keeps_folder_picker_available(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    let repo = demo::evolve_preview().repo.unwrap();
    finish_check(cx, window, &state, Some(repo.clone()));
    click(cx, window, "evolve-confirm");
    assert_eq!(&*recording.starts.borrow(), &[repo]);
    state.update(cx, |state, cx| {
        state.evolve.starting = false;
        state.evolve.error = Some(EvolveFailure { kind: "not_git_repo".into(), message: "bad path".into() });
        cx.notify();
    });
    frame(cx, window);
    assert!(shown(cx, window, "evolve-pick"));
}

#[gpui_kit::test]
fn outcome_disables_confirmation_until_dialog_is_reopened(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    let repo = demo::evolve_preview().repo.unwrap();
    finish_check(cx, window, &state, Some(repo.clone()));
    click(cx, window, "evolve-confirm");
    state.update(cx, |state, cx| {
        state.evolve.starting = false;
        state.evolve.outcome = Some(EvolveOutcome::Ran { exit_code: 0 });
        cx.notify();
    });
    frame(cx, window);
    cx.update_window(window, |_, window, _| assert_eq!(window.find("evolve-confirm").disabled(), Some(true))).unwrap();
    click(cx, window, "evolve-confirm");
    assert_eq!(&*recording.starts.borrow(), &[repo.clone()]);
    click(cx, window, "evolve-cancel");
    click(cx, window, "evolve-open");
    finish_check(cx, window, &state, Some(repo));
    cx.update_window(window, |_, window, _| assert_ne!(window.find("evolve-confirm").disabled(), Some(true))).unwrap();
}

#[gpui_kit::test]
fn missing_repo_requires_a_pick_and_disables_confirmation(cx: &mut TestAppContext) {
    let recording = Rc::new(Recording::default());
    let mut initial = demo::state();
    initial.backend = Some(Box::new(Fake(recording.clone())));
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    click(cx, window, "evolve-open");
    finish_check(cx, window, &state, None);
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("evolve-pick").is_some());
        assert_eq!(window.find("evolve-confirm").disabled(), Some(true));
    }).unwrap();
    assert!(recording.starts.borrow().is_empty());
}

#[gpui_kit::test]
fn entry_is_present_on_empty_and_populated_usage(cx: &mut TestAppContext) {
    let mut initial = demo::state();
    let runs = std::mem::take(&mut initial.runs);
    initial.current = None;
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    assert!(shown(cx, window, "evolve-open"));
    assert!(shown(cx, window, "usage-make-wish"));
    state.update(cx, |state, cx| { state.replace_runs(runs, &[], None); cx.notify(); });
    frame(cx, window);
    assert!(shown(cx, window, "evolve-open"));
    click(cx, window, "evolve-open");
    assert!(shown(cx, window, "evolve-confirm"));
}

#[gpui_kit::test]
fn evolve_scene_opens_demo_dialog_with_past_runs(cx: &mut TestAppContext) {
    let mut initial = demo::state();
    initial.scene = Some("evolve".into());
    let (window, state) = open(cx, initial);
    view(cx, window, &state, Screen::Usage);
    assert!(shown(cx, window, "evolve-confirm"));
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find(("evolve-run", 0_usize)).is_some());
        assert!(window.try_find(("evolve-run", 1_usize)).is_some());
    }).unwrap();
}
