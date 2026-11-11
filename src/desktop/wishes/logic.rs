use super::{Filter, MenuAction, MenuTarget, PanelState, Run, RunGroup, RunStatus};
use gpui_kit::SharedString;
use std::path::PathBuf;

impl Filter {
    pub fn to_key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Running => "active",
            Self::NeedsYou => "review",
            Self::Done => "failed",
        }
    }
    pub fn from_key(key: &str) -> Self {
        match key {
            "active" => Self::Running,
            "review" => Self::NeedsYou,
            "failed" => Self::Done,
            _ => Self::All,
        }
    }
}
pub fn encode(filter: Filter, query: &str) -> String {
    format!(
        "{}|{}",
        filter.to_key(),
        query.chars().take(120).collect::<String>()
    )
}
pub fn decode(value: Option<&str>) -> (Filter, String) {
    let (key, query) = value
        .unwrap_or("")
        .split_once('|')
        .unwrap_or((value.unwrap_or(""), ""));
    (Filter::from_key(key), query.chars().take(120).collect())
}
pub fn scroll(value: f32) -> f32 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

pub fn selected_index(groups: &[RunGroup], runs: &[Run], session: Option<&SharedString>) -> Option<usize> {
    let indices = groups.iter().flat_map(|group| group.runs.iter().copied());
    session.and_then(|session| indices.clone().find(|&i| runs[i].session == *session))
        .or_else(|| indices.into_iter().next())
}

pub fn cancels_restore(old_filter: Filter, new_filter: Filter, old_query: &str, new_query: &str) -> bool {
    old_filter != new_filter || old_query != new_query
}

pub fn row_meta(run: &Run) -> String {
    run.cwd.as_ref().map_or_else(|| "—".to_string(), |cwd| cwd.display().to_string())
}

#[derive(Debug, PartialEq)]
pub enum RowEffect {
    Open(usize),
    Resume(usize),
    Folder(PathBuf),
    Copy(String),
}
pub fn resolve(runs: &[Run], target: &MenuTarget, action: MenuAction) -> Option<RowEffect> {
    let MenuTarget::Run(session) = target else {
        return None;
    };
    let index = runs.iter().position(|run| &run.session == session)?;
    let run = &runs[index];
    match action {
        MenuAction::OpenRun => Some(RowEffect::Open(index)),
        MenuAction::Resume
            if matches!(
                run.status,
                RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted
            ) =>
        {
            Some(RowEffect::Resume(index))
        }
        MenuAction::OpenFolder => run.cwd.clone().map(RowEffect::Folder),
        MenuAction::CopySessionId => Some(RowEffect::Copy(session.to_string())),
        _ => None,
    }
}
#[derive(Debug, PartialEq)]
pub enum ScreenState {
    Loading,
    Empty,
    Ready,
    Failed,
    Stale,
}
pub fn screen_state(panel: &PanelState) -> ScreenState {
    match panel {
        PanelState::Loading => ScreenState::Loading,
        PanelState::Empty => ScreenState::Empty,
        PanelState::Ready => ScreenState::Ready,
        PanelState::Failed(_) => ScreenState::Failed,
        PanelState::Stale(_) => ScreenState::Stale,
    }
}
#[derive(Default)]
pub struct Restore {
    done: bool,
}
impl Restore {
    pub fn take(&mut self, panel: &PanelState) -> bool {
        if !self.done && matches!(panel, PanelState::Ready | PanelState::Stale(_)) {
            self.done = true;
            true
        } else {
            false
        }
    }
    pub fn cancel(&mut self) {
        self.done = true;
    }
    pub fn done(&self) -> bool {
        self.done
    }
}
pub fn levels(
    selected: bool,
    hovered: bool,
    focused: bool,
    lift: f32,
    glow: f32,
    reduce: bool,
) -> (f32, f32, bool) {
    (
        if reduce { hovered as u8 as f32 } else { lift },
        if reduce { selected as u8 as f32 } else { glow },
        selected && focused,
    )
}
