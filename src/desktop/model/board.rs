use super::{AppState, Run, RunStatus, UnitState};
use gpui_kit::SharedString;

#[derive(Clone, Debug, PartialEq)]
pub enum BoardStatus { Idle, Writing, Failed(SharedString), Off }
#[derive(Clone, Debug, PartialEq)]
pub struct BoardText { pub recent: SharedString, pub next: Option<SharedString>, pub written_unix: i64 }
#[derive(Clone, Debug, PartialEq)]
pub struct BoardEntry { pub text: Option<BoardText>, pub status: BoardStatus }
#[derive(Clone, Debug, PartialEq)]
pub struct BoardFacts { pub status: RunStatus, pub verified: usize, pub total: usize, pub running: usize, pub failed: usize, pub needs_you: bool }
pub fn needs_you(run: &Run) -> bool {
    match run.status {
        RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted => true,
        RunStatus::Active | RunStatus::LimitWait => !run.needs.is_empty(),
        RunStatus::Done => false,
    }
}
pub fn board_facts(run: &Run) -> BoardFacts {
    BoardFacts { status: run.status, verified: run.count(UnitState::Verified), total: run.units.len(), running: run.count(UnitState::Running), failed: run.count(UnitState::Failed), needs_you: needs_you(run) }
}
pub fn board_runs(state: &AppState) -> Vec<usize> {
    let mut runs: Vec<_> = state.runs.iter().enumerate().filter(|(_, run)| run.status != RunStatus::Done).map(|(i, _)| i).collect();
    runs.sort_by_key(|&i| !needs_you(&state.runs[i]));
    runs
}
