use super::{Command, MemoryPrior, Undoable};
use crate::desktop::settings::{FocusPanel, SettingPrior, WindowMode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    pub open_mini: bool,
    pub close_mini: bool,
    pub reopen_mini: bool,
    pub minimize_main: bool,
    pub restore_main: bool,
    pub force_run: bool,
}

pub(super) fn plan(applied: WindowMode, wanted: WindowMode, applied_on_top: bool, wanted_on_top: bool, mini_open: bool) -> Plan {
    let entering = wanted == WindowMode::Mini && !mini_open;
    let leaving = applied == WindowMode::Mini && wanted != WindowMode::Mini;
    Plan {
        open_mini: entering,
        close_mini: leaving,
        reopen_mini: wanted == WindowMode::Mini && mini_open && applied_on_top != wanted_on_top,
        minimize_main: entering,
        restore_main: leaving,
        force_run: applied != wanted && matches!(wanted, WindowMode::Focus(_)),
    }
}

pub(super) fn mode_after(command: &Command, current: WindowMode, _last_panel: FocusPanel) -> Option<WindowMode> {
    match command {
        Command::Focus(panel) => Some(current.toggle_focus(*panel)),
        Command::FullWindow => Some(WindowMode::Full),
        Command::Mini => Some(current.toggle_mini()),
        _ => None,
    }
}

pub(super) fn after_key_focus(current: WindowMode, last_panel: FocusPanel) -> WindowMode {
    if matches!(current, WindowMode::Focus(_)) { WindowMode::Full } else { WindowMode::Focus(last_panel) }
}
pub(super) fn after_key_mini(current: WindowMode) -> WindowMode { current.toggle_mini() }

#[derive(Debug, PartialEq)]
pub(super) enum UndoStep { Memory { id: i64, prior: MemoryPrior }, Setting(SettingPrior), Nothing }
pub(super) fn undo_step(u: Option<Undoable>) -> UndoStep {
    match u {
        Some(Undoable::Memory { id, prior }) => UndoStep::Memory { id, prior },
        Some(Undoable::Setting(prior)) => UndoStep::Setting(prior),
        None => UndoStep::Nothing,
    }
}
pub(super) fn should_record(prior: &SettingPrior, now: &SettingPrior) -> bool { prior != now }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Startup { MainOnly, MiniOnly }
pub(super) fn startup_windows(ephemeral: bool, mode: WindowMode) -> Startup {
    if ephemeral && mode == WindowMode::Mini { Startup::MiniOnly } else { Startup::MainOnly }
}
