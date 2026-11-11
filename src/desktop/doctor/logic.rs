use super::{theme, PanelState, SettingPrior, Undoable};
use gpui_kit::{Pixels, SharedString};

pub(super) fn is_change(prior: SettingPrior, now: SettingPrior) -> bool { prior != now }
pub(super) fn record_if_changed(current: SettingPrior, choice: SettingPrior, setter: impl FnOnce() -> SettingPrior) -> Option<Undoable> {
    is_change(current, choice).then(|| Undoable::Setting(setter()))
}
pub(super) fn settings_columns(width: Pixels) -> usize { theme::columns_for(width) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Card { Environment, Versions, Preferences }
pub(super) fn split(columns: usize) -> Vec<Vec<Card>> {
    if columns < 2 { vec![vec![Card::Environment, Card::Versions, Card::Preferences]] }
    else { vec![vec![Card::Environment, Card::Versions], vec![Card::Preferences]] }
}
#[derive(Debug, PartialEq)]
pub(super) enum ChecksView { Loading, Empty, Ready, Failed(SharedString), Stale(SharedString) }
pub(super) fn checks_view(panel: &PanelState) -> ChecksView {
    match panel {
        PanelState::Loading => ChecksView::Loading,
        PanelState::Empty => ChecksView::Empty,
        PanelState::Ready => ChecksView::Ready,
        PanelState::Failed(message) => ChecksView::Failed(message.clone()),
        PanelState::Stale(message) => ChecksView::Stale(message.clone()),
    }
}
