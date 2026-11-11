use super::{i18n, PanelState, Scope};
use super::theme;
use crate::desktop::settings::{BoardTokens, ViewMemory};
use gpui_kit::px;

pub(super) fn cost_columns(width: f32) -> usize { theme::columns_for(px(width)) }
pub(super) fn one_column_order(columns: usize) -> bool { columns == 1 }
pub(super) fn warning_summary_text(n: usize, tr: impl Fn(&'static str) -> &'static str) -> String {
    if n == 1 { tr("1 warning").to_owned() } else { tr("{n} warnings").replace("{n}", &n.to_string()) }
}
pub(super) fn page_cap() -> gpui_kit::Pixels { theme::READ_WIDTH * 2.5 }

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BoardLine { Tokens { input: u64, output: u64, unreported: u64 }, OnlyUnreported(u64), None }
pub(super) fn board_line(b: BoardTokens) -> BoardLine {
    if b.input > 0 || b.output > 0 { BoardLine::Tokens { input: b.input, output: b.output, unreported: b.unreported } }
    else if b.unreported > 0 { BoardLine::OnlyUnreported(b.unreported) } else { BoardLine::None }
}
pub(super) fn board_text(board: BoardLine, tr: impl Fn(&'static str) -> &'static str, format_tokens: impl Fn(u64) -> String) -> String {
    let unreported_text = |n: u64| if n == 1 { tr("1 call without usage").to_owned() } else { tr("{n} calls without usage").replace("{n}", &n.to_string()) };
    match board {
        BoardLine::None => tr("No summary calls yet").to_owned(),
        BoardLine::OnlyUnreported(n) => unreported_text(n),
        BoardLine::Tokens { input, output, unreported } => {
            let token = |n| if n == 0 { "0".to_string() } else { format_tokens(n) };
            let mut text = tr("{input} in · {output} out").replace("{input}", &token(input)).replace("{output}", &token(output));
            if unreported > 0 { text.push_str(" · "); text.push_str(&unreported_text(unreported)); }
            text
        }
    }
}

pub(super) fn role_tip<S: AsRef<str>>(lang: i18n::Lang, name: &str, ids: &[S]) -> String {
    let mut tip = i18n::role_tooltip(lang, name).to_string();
    if ids.len() > 1 { tip.push('\n'); tip.push_str(&ids.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", ")); }
    tip
}
pub(super) fn scroll(v: f32) -> f32 { if v.is_finite() && v >= 0. { v } else { 0. } }
pub(super) fn encode(scope: Scope, selected: Option<&str>, offset: f32) -> ViewMemory {
    ViewMemory { filter: None, sort: Some(match scope { Scope::AllRuns => "all", Scope::ThisRun => "this" }.into()), selected: selected.filter(|s| !s.is_empty() && s.chars().count() <= 200).map(str::to_owned), scroll: scroll(offset) }
}
pub(super) fn decode(v: &ViewMemory) -> (Option<Scope>, Option<String>, f32) {
    let scope = match v.sort.as_deref() { Some("all") => Some(Scope::AllRuns), Some("this") => Some(Scope::ThisRun), _ => None };
    let selected = v.selected.clone().filter(|s| !s.is_empty() && s.chars().count() <= 200);
    (scope, selected, scroll(v.scroll))
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScreenState { Loading, Empty, Ready, Failed, Stale }
pub(super) fn screen_state(panel: &PanelState, has_runs: bool) -> ScreenState {
    match panel { PanelState::Loading => ScreenState::Loading, PanelState::Empty => ScreenState::Empty, PanelState::Ready if !has_runs => ScreenState::Empty, PanelState::Ready => ScreenState::Ready, PanelState::Failed(_) => ScreenState::Failed, PanelState::Stale(_) => ScreenState::Stale }
}
#[derive(Default)]
pub(super) struct Restore { done: bool }
impl Restore {
    pub(super) fn pending(&self) -> bool { !self.done }
    pub(super) fn take(&mut self, panel: &PanelState, has_runs: bool, saved_session_present: Option<bool>, offset: f32) -> Option<f32> {
        if !self.done && has_runs && saved_session_present != Some(false) && matches!(panel, PanelState::Ready | PanelState::Stale(_)) {
            self.done = true;
            Some(scroll(offset))
        } else { None }
    }
    pub(super) fn cancel(&mut self) { self.done = true; }
}
