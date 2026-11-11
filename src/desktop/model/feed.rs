use gpui_kit::SharedString;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedKind { Runs, Memory, Checks, Usage, Quota }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedStatus {
    pub loaded: bool,
    pub error: Option<(SharedString, SystemTime)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feeds {
    pub runs: FeedStatus,
    pub memory: FeedStatus,
    pub checks: FeedStatus,
    pub usage: FeedStatus,
    pub quota: FeedStatus,
}

impl Feeds {
    pub fn get(&self, kind: FeedKind) -> &FeedStatus {
        match kind { FeedKind::Runs => &self.runs, FeedKind::Memory => &self.memory, FeedKind::Checks => &self.checks, FeedKind::Usage => &self.usage, FeedKind::Quota => &self.quota }
    }
    pub fn get_mut(&mut self, kind: FeedKind) -> &mut FeedStatus {
        match kind { FeedKind::Runs => &mut self.runs, FeedKind::Memory => &mut self.memory, FeedKind::Checks => &mut self.checks, FeedKind::Usage => &mut self.usage, FeedKind::Quota => &mut self.quota }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelState { Loading, Empty, Ready, Failed(SharedString), Stale(SharedString) }

pub fn panel_state(s: &FeedStatus, empty: bool) -> PanelState {
    match (s.loaded, &s.error, empty) {
        (false, None, _) => PanelState::Loading,
        (false, Some((message, _)), _) => PanelState::Failed(message.clone()),
        (true, Some((message, _)), _) => PanelState::Stale(message.clone()),
        (true, None, true) => PanelState::Empty,
        (true, None, false) => PanelState::Ready,
    }
}
