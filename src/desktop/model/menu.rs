use super::{AppState, RunStatus};
use gpui_kit::SharedString;

#[derive(Clone, Debug, PartialEq)]
pub enum MenuTarget { Run(SharedString), Module { session: SharedString, module: SharedString }, Unit { session: SharedString, unit: SharedString }, Memory(i64) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction { OpenRun, Resume, OpenFolder, CopySessionId, OpenDetail, CopyText, CommentOn, CopyMemory, CorrectMemory, DeleteMemory }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEffect { Now, Confirm, Dialog }
impl MenuAction {
    pub fn effect(self) -> MenuEffect {
        match self { Self::DeleteMemory => MenuEffect::Confirm, Self::CorrectMemory => MenuEffect::Dialog, _ => MenuEffect::Now }
    }
}
impl AppState {
    pub fn menu_entries(&self, target: &MenuTarget) -> Vec<(MenuAction, bool)> {
        use MenuAction::*;
        match target {
            MenuTarget::Run(session) => {
                let Some(run) = self.runs.iter().find(|r| &r.session == session) else { return vec![] };
                vec![(OpenRun, true), (Resume, matches!(run.status, RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted)), (OpenFolder, run.cwd.is_some()), (CopySessionId, true)]
            }
            MenuTarget::Module { session, module } => {
                let Some(run) = self.runs.iter().find(|r| &r.session == session) else { return vec![] };
                if !run.modules.iter().any(|m| &m.id == module) { return vec![]; }
                vec![(OpenDetail, true), (CopyText, true), (CommentOn, self.run().is_some_and(|r| &r.session == session))]
            }
            MenuTarget::Unit { session, unit } => {
                let Some(run) = self.runs.iter().find(|r| &r.session == session) else { return vec![] };
                if !run.units.iter().any(|u| &u.id == unit) { return vec![]; }
                vec![(OpenDetail, true), (CopyText, true), (CommentOn, self.run().is_some_and(|r| &r.session == session))]
            }
            MenuTarget::Memory(id) => if self.memory.iter().any(|m| m.id == *id) { vec![(CopyMemory, true), (CorrectMemory, true), (DeleteMemory, true)] } else { vec![] },
        }
    }
    pub fn menu_allowed(&self, target: &MenuTarget, action: MenuAction) -> bool {
        self.menu_entries(target).into_iter().any(|(a, enabled)| a == action && enabled)
    }
}
