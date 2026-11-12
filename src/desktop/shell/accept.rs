use super::{logic::*, *};
use crate::desktop::{parse_options, state_scene, theme::Density};
use std::prelude::v1::test;

#[test]
fn all_mode_transitions() {
    use FocusPanel::*;
    use WindowMode::*;
    let modes = [Full, Focus(Graph), Focus(Checklist), Mini];
    for applied in modes {
        for wanted in modes {
            let mini = applied == Mini;
            let expected = Plan {
                open_mini: wanted == Mini && !mini,
                close_mini: applied == Mini && wanted != Mini,
                reopen_mini: false,
                minimize_main: wanted == Mini && !mini,
                restore_main: applied == Mini && wanted != Mini,
                force_run: applied != wanted && matches!(wanted, Focus(_)),
            };
            assert_eq!(plan(applied, wanted, false, false, mini), expected, "{applied:?} -> {wanted:?}");
        }
    }
    assert_eq!(plan(Mini, Mini, false, true, true), Plan { reopen_mini: true, ..Plan::default() });
    assert_eq!(plan(Mini, Mini, true, true, true), Plan::default());
    assert_eq!(plan(Mini, Mini, true, true, false), Plan { open_mini: true, minimize_main: true, ..Plan::default() });
}

#[test]
fn commands_and_keys_choose_modes() {
    use FocusPanel::*;
    use WindowMode::*;
    for (command, current, expected) in [
        (Command::Focus(Graph), Full, Some(Focus(Graph))),
        (Command::Focus(Graph), Focus(Graph), Some(Full)),
        (Command::Focus(Checklist), Focus(Graph), Some(Focus(Checklist))),
        (Command::FullWindow, Mini, Some(Full)),
        (Command::Mini, Full, Some(Mini)),
        (Command::Mini, Mini, Some(Full)),
        (Command::ToggleDensity, Full, None),
        (Command::ToggleOnTop, Full, None),
        (Command::Undo, Full, None),
        (Command::NewWish, Full, None),
        (Command::Open(Screen::Run), Full, None),
        (Command::ToggleTheme, Full, None),
        (Command::ToggleLanguage, Full, None),
        (Command::Stop, Full, None),
        (Command::Resume, Full, None),
        (Command::Shortcuts, Full, None),
    ] { assert_eq!(mode_after(&command, current, Graph), expected); }
    assert_eq!(after_key_focus(Full, Checklist), Focus(Checklist));
    assert_eq!(after_key_focus(Mini, Graph), Focus(Graph));
    assert_eq!(after_key_focus(Focus(Graph), Checklist), Full);
    assert_eq!(after_key_mini(Mini), Full);
    assert_eq!(after_key_mini(Focus(Checklist)), Mini);
}

#[test]
fn undo_and_recording() {
    let memory = MemoryPrior { body: "body".into(), summary: "summary".into(), corrected: None, confirmed: 1.0 };
    assert_eq!(undo_step(Some(Undoable::Memory { id: 7, prior: memory.clone() })), UndoStep::Memory { id: 7, prior: memory });
    let prior = SettingPrior::Mode(WindowMode::Full);
    assert_eq!(undo_step(Some(Undoable::Setting(prior))), UndoStep::Setting(prior));
    assert_eq!(undo_step(None), UndoStep::Nothing);
    for (before, after) in [
        (SettingPrior::Mode(WindowMode::Full), SettingPrior::Mode(WindowMode::Mini)),
        (SettingPrior::Density(Density::Comfortable), SettingPrior::Density(Density::Dense)),
        (SettingPrior::MiniOnTop(false), SettingPrior::MiniOnTop(true)),
    ] {
        assert!(!should_record(&before, &before));
        assert!(should_record(&before, &after));
    }
}

#[test]
fn startup_options_and_windows() {
    for (arg, mode) in [("full", WindowMode::Full), ("focus-graph", WindowMode::Focus(FocusPanel::Graph)), ("focus-checklist", WindowMode::Focus(FocusPanel::Checklist)), ("mini", WindowMode::Mini)] {
        let options = parse_options(&["--mode".into(), arg.into(), "--scene".into(), "quota-low".into()]).unwrap();
        assert_eq!(options.mode, Some(mode));
        assert_eq!(options.scene.as_deref(), Some("quota-low"));
        for ephemeral in [false, true] {
            assert_eq!(startup_windows(ephemeral, mode), if ephemeral && mode == WindowMode::Mini { Startup::MiniOnly } else { Startup::MainOnly });
        }
    }
    for args in [vec!["--mode".into()], vec!["--mode".into(), "bad".into()]] {
        assert_eq!(parse_options(&args).unwrap_err(), "--mode takes full, focus-graph, focus-checklist or mini");
    }
    assert!(start_mini_only(true, WindowMode::Mini, false));
    assert!(!start_mini_only(true, WindowMode::Mini, true));
}

#[test]
fn startup_density_and_state_scene() {
    for (arg, density) in [("dense", Density::Dense), ("comfortable", Density::Comfortable)] {
        let options = parse_options(&["--density".into(), arg.into()]).unwrap();
        assert_eq!(options.density, Some(density));
    }
    assert_eq!(parse_options(&[]).unwrap().density, None);
    for args in [vec!["--density".into()], vec!["--density".into(), "bad".into()]] {
        assert_eq!(parse_options(&args).unwrap_err(), "--density takes comfortable or dense");
    }
    let scene = ["--scene".into(), "quota-low".into()];
    assert_eq!(state_scene(&parse_options(&scene).unwrap()), None);
    let demo_scene = ["--demo".into(), "--scene".into(), "quota-low".into()];
    assert_eq!(state_scene(&parse_options(&demo_scene).unwrap()).as_deref(), Some("quota-low"));
}

#[test]
fn chrome_for_every_mode() {
    let full = Chrome { rail: true, title_extras: true, strip: true, force_run: false };
    let focus = Chrome { rail: false, title_extras: false, strip: true, force_run: true };
    assert_eq!(chrome(WindowMode::Full), full);
    assert_eq!(chrome(WindowMode::Mini), full);
    for panel in [FocusPanel::Graph, FocusPanel::Checklist] {
        assert_eq!(chrome(WindowMode::Focus(panel)), focus);
    }
}

#[derive(Default)]
struct QuitCalls { stop: usize, quit: Vec<bool>, detach: usize }
struct QuitBackend(Rc<std::cell::RefCell<QuitCalls>>);
impl Backend for QuitBackend {
    fn start_wish(&self, _: WishRequest) -> crate::base::Res<()> { Ok(()) }
    fn interject(&self, _: &str, _: &str) -> crate::base::Res<()> { Ok(()) }
    fn stop(&self, _: &str) -> crate::base::Res<()> { self.0.borrow_mut().stop += 1; Ok(()) }
    fn resume(&self, _: &str, _: Option<&str>) -> crate::base::Res<()> { Ok(()) }
    fn delete_memory(&self, _: i64) -> crate::base::Res<()> { Ok(()) }
    fn correct_memory(&self, _: i64, _: &str) -> crate::base::Res<()> { Ok(()) }
    fn refresh(&self) -> crate::base::Res<()> { Ok(()) }
    fn recheck(&self) -> crate::base::Res<()> { Ok(()) }
    fn follow(&self, _: &str) -> crate::base::Res<()> { Ok(()) }
    fn quit_service(&self, stop: bool) -> crate::base::Res<()> { self.0.borrow_mut().quit.push(stop); Ok(()) }
    fn shutdown(&self, _: Duration) { self.0.borrow_mut().detach += 1; }
}

#[test]
fn closing_detaches_while_explicit_service_quit_stops_running() {
    let calls = Rc::new(std::cell::RefCell::new(QuitCalls::default()));
    let mut state = crate::desktop::demo::state();
    state.backend = Some(Box::new(QuitBackend(calls.clone())));
    detach(&state);
    assert_eq!(calls.borrow().detach, 1);
    assert_eq!(calls.borrow().stop, 0);
    assert!(calls.borrow().quit.is_empty());
    assert!(request_service_quit(&mut state));
    assert_eq!(calls.borrow().quit, [true]);
    assert_eq!(calls.borrow().stop, 0);
}
