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

fn no_shutdown(source: &str) -> bool { !source.contains("shutdown(") }

#[test]
fn shutdown_scan_negative_control() { assert!(!no_shutdown("backend.shutdown(deadline)")); }

#[test]
fn source_guards() {
    let shell = include_str!("../shell.rs").split("#[cfg(test)]").next().unwrap();
    let mini = include_str!("../mini.rs").split_once("#[cfg(test)]").unwrap().0;
    let (before, after) = shell.split_once("fn end_engine(").unwrap();
    let (engine, after) = after.split_once("fn finish_close(").unwrap();
    assert!(no_shutdown(before) && no_shutdown(after) && no_shutdown(mini));
    assert_eq!(engine.matches("shutdown(").count(), 1);
    let desktop = include_str!("../mod.rs");
    assert!(desktop.find("save_on_quit(").unwrap() < desktop.find("set_reduce_motion(").unwrap());
    assert!(desktop.contains("apply_data_scene("));
    assert!(!shell.contains("request_animation_frame"));
    let command = shell.split_once("fn run_command(").unwrap().1.split_once("fn new_wish(").unwrap().0;
    assert!(!command.contains("Command::Undo => {}"));
    let focus_arm = command.split_once("Command::Focus(_)").unwrap().1.split_once("=>").unwrap().1.trim_start();
    assert!(!focus_arm.starts_with("{}"));
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

fn shell_style_ok(source: &str) -> bool {
    let large_radius = regex::Regex::new(r"rounded\(px\(\s*(\d+(?:\.\d+)?)").unwrap();
    !source.contains("text_size(px(")
        && !source.contains("[Tween; 5]")
        && !large_radius.captures_iter(source).any(|c| c[1].parse::<f64>().unwrap() > 4.0)
}

#[test]
fn shell_style_scan_has_negative_controls() {
    for bad in ["text_size(px(12.))", "rounded(px(8.))", "[Tween; 5]"] {
        assert!(!shell_style_ok(bad), "{bad}");
    }
    assert!(shell_style_ok("rounded(px(4.))"));
}

#[test]
fn shell_chrome_source_guards() {
    let shell = include_str!("../shell.rs").split("#[cfg(test)]").next().unwrap();
    assert!(shell_style_ok(shell));
    let render = shell.split_once("impl Render for Shell {").unwrap().1;
    assert_eq!(render.matches("quota::strip(").count(), 1);
    let nav = shell.split_once("fn nav_item(").unwrap().1.split_once("fn new_wish_button(").unwrap().0;
    assert!(nav.contains("focus_ring("));
}
