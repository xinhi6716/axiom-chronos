use super::{Command, MemoryPrior, Undoable};
use crate::desktop::settings::{FocusPanel, SettingPrior, WindowMode};
use crate::desktop::{components::panel::PanelState, data::faults::Fault, model::FeedStatus};

pub(super) fn panel_state(feed: &FeedStatus, fault: Option<&Fault>, slow_elapsed: bool, now_ms: i64) -> PanelState {
    match fault {
        Some(Fault::Slow { .. }) if !slow_elapsed => return PanelState::Loading,
        Some(Fault::Corrupt) => return PanelState::Corrupt { reason: "injected corrupt".into() },
        Some(Fault::Missing) => return PanelState::Empty,
        Some(Fault::Stale { age_ms }) => return PanelState::Stale { since_ms: now_ms.saturating_sub(*age_ms) },
        Some(Fault::Offline) => return PanelState::Offline { reason: "injected offline".into() },
        _ => {}
    }
    if let Some((message, _)) = &feed.error {
        PanelState::Failed { reason: message.to_string() }
    } else if feed.loaded {
        PanelState::Ready
    } else {
        PanelState::Loading
    }
}

#[cfg(test)]
mod panel_tests {
    use super::*;
    use crate::desktop::components::panel::copy;
    use crate::desktop::i18n::Lang;
    use std::prelude::v1::test;
    use std::time::SystemTime;

    #[test]
    fn faults_and_feed_state_map_to_panel_states() {
        let ready = FeedStatus { loaded: true, error: None };
        for (fault, expected) in [
            (Fault::Slow { ms: 100 }, PanelState::Loading),
            (Fault::Corrupt, PanelState::Corrupt { reason: "injected corrupt".into() }),
            (Fault::Missing, PanelState::Empty),
            (Fault::Stale { age_ms: 40 }, PanelState::Stale { since_ms: 960 }),
            (Fault::Offline, PanelState::Offline { reason: "injected offline".into() }),
            (Fault::Panic, PanelState::Ready),
        ] {
            assert_eq!(panel_state(&ready, Some(&fault), false, 1000), expected);
        }
        assert_eq!(panel_state(&ready, Some(&Fault::Slow { ms: 100 }), true, 1000), PanelState::Ready);
        assert_eq!(panel_state(&ready, None, false, 1000), PanelState::Ready);
        assert_eq!(copy(Lang::En, &panel_state(&ready, None, false, 1000)), None);
        assert_eq!(panel_state(&FeedStatus::default(), None, false, 1000), PanelState::Loading);
        let failed = FeedStatus { loaded: true, error: Some(("failed".into(), SystemTime::now())) };
        assert_eq!(panel_state(&failed, None, false, 1000), PanelState::Failed { reason: "failed".into() });
    }
}

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
