use super::{AppError, MemoryEntry, MemoryPrior, MenuAction, MenuTarget, PanelState, Undoable};
use super::theme::{self, Density, Step};
use gpui_kit::Pixels;
use crate::desktop::settings::ViewMemory;
use std::collections::HashMap;

/// The longest correction the memory store accepts, in lines.
pub(super) const MAX_CORRECTION_LINES: usize = 12;

/// Why a correction cannot be saved yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    Empty,
    TooLong,
    Unchanged,
}

/// What stops `text` from replacing `current`: blank, over the store's line limit, or no change.
pub(super) fn correction_problem(text: &str, current: &str) -> Option<Problem> {
    let text = text.trim();
    if text.is_empty() {
        Some(Problem::Empty)
    } else if text.lines().count() > MAX_CORRECTION_LINES {
        Some(Problem::TooLong)
    } else if text == current.trim() {
        Some(Problem::Unchanged)
    } else {
        None
    }
}

/// Corrections the person wrote and has not seen saved. Saving only queues the request, so the
/// text is kept until the entry shows it; a failure hands it back for another try.
#[derive(Default)]
pub(super) struct Corrections {
    /// Texts set aside for entries the person moved away from, or whose save failed.
    pub(super) drafts: HashMap<i64, String>,
    /// Texts handed to the engine, with how many errors had been reported by then.
    pub(super) sent: HashMap<i64, (String, u64)>,
}

impl Corrections {
    pub(super) fn submit(&mut self, id: i64, text: String, errors_seen: u64) {
        self.drafts.remove(&id);
        self.sent.insert(id, (text, errors_seen));
    }

    /// Settles what was sent: an entry that now says it is done, a failed save becomes a draft
    /// again, and a gone entry is forgotten.
    pub(super) fn settle(&mut self, body_of: impl Fn(i64) -> Option<String>, errors: &[AppError]) {
        let ids: Vec<i64> = self.sent.keys().copied().collect();
        for id in ids {
            let Some((text, mark)) = self.sent.get(&id).cloned() else { continue };
            match body_of(id) {
                None => {
                    self.sent.remove(&id);
                }
                Some(body) if body.trim() == text.trim() => {
                    self.sent.remove(&id);
                }
                Some(_) if errors.iter().any(|e| e.action == crate::desktop::model::Action::CorrectMemory && e.id > mark) => {
                    self.sent.remove(&id);
                    self.drafts.insert(id, text);
                }
                Some(_) => {}
            }
        }
    }

    /// What the box starts with when `id` is corrected: the unsaved text if there is any.
    pub(super) fn start_text(&mut self, id: i64, body: &str) -> String {
        if let Some(draft) = self.drafts.remove(&id) {
            return draft;
        }
        match self.sent.get(&id) {
            Some((text, _)) => text.clone(),
            None => body.to_owned(),
        }
    }
}



#[derive(Debug, PartialEq)]
pub enum MemoryEffect { Copy(String), Correct(i64), Delete(i64, String) }
pub fn resolve(entries: &[MemoryEntry], target: &MenuTarget, action: MenuAction) -> Option<MemoryEffect> {
    let MenuTarget::Memory(id) = target else { return None };
    let entry = entries.iter().find(|e| e.id == *id)?;
    match action {
        MenuAction::CopyMemory => Some(MemoryEffect::Copy(format!("{}\n\n{}", entry.title, entry.body))),
        MenuAction::CorrectMemory => Some(MemoryEffect::Correct(*id)),
        MenuAction::DeleteMemory => Some(MemoryEffect::Delete(*id, entry.title.to_string())),
        _ => None,
    }
}
#[derive(Default)]
pub struct UndoOffer { pub offered: Option<(i64, MemoryPrior)>, seen_top: Option<Undoable> }
impl UndoOffer {
    pub fn observe(&mut self, top: Option<Undoable>, before: usize, now: usize) {
        let memory_top = match &top { Some(Undoable::Memory { id, prior }) => Some((*id, prior)), _ => None };
        if now > before || (now == before && top != self.seen_top && top.is_some()) {
            self.offered = memory_top.map(|(id, prior)| (id, prior.clone()));
        } else if !self.matches_top(memory_top) { self.clear(); }
        self.seen_top = top;
    }
    pub fn remember_top(&mut self, top: Option<Undoable>) { self.seen_top = top; }
    pub fn matches_top(&self, top: Option<(i64, &MemoryPrior)>) -> bool {
        match (&self.offered, top) { (Some((id, prior)), Some((top_id, top_prior))) => *id == top_id && prior == top_prior, _ => false }
    }
    pub fn take(&mut self) -> Option<(i64, MemoryPrior)> { self.offered.take() }
    pub fn clear(&mut self) { self.offered = None; }
}
pub fn scroll(value: f32) -> f32 { if value.is_finite() && value >= 0.0 { value } else { 0.0 } }
pub fn encode(filter: Option<&str>, query: &str, selected: Option<i64>, offset: f32) -> ViewMemory {
    let query: String = query.chars().take(120).collect();
    ViewMemory { filter: filter.filter(|s| !s.is_empty()).map(str::to_owned), sort: (!query.is_empty()).then_some(query), selected: selected.map(|id| id.to_string()), scroll: scroll(offset) }
}
pub fn decode(v: &ViewMemory) -> (Option<String>, String, Option<i64>, f32) {
    (v.filter.clone().filter(|s| !s.is_empty()), v.sort.as_deref().unwrap_or("").chars().take(120).collect(), v.selected.as_deref().and_then(|s| s.parse().ok()), scroll(v.scroll))
}
#[derive(Debug, PartialEq)]
pub enum ScreenState { Loading, Empty, Ready, Failed, Stale }
pub fn screen_state(panel: &PanelState) -> ScreenState { match panel { PanelState::Loading => ScreenState::Loading, PanelState::Empty => ScreenState::Empty, PanelState::Ready => ScreenState::Ready, PanelState::Failed(_) => ScreenState::Failed, PanelState::Stale(_) => ScreenState::Stale } }
#[derive(Default)]
pub struct Restore { done: bool }
impl Restore {
    pub fn take(&mut self, panel: &PanelState) -> bool { if !self.done && matches!(panel, PanelState::Ready | PanelState::Stale(_)) { self.done = true; true } else { false } }
    pub fn cancel(&mut self) { self.done = true; }
    pub fn done(&self) -> bool { self.done }
}
pub fn levels(selected: bool, hovered: bool, focused: bool, lift: f32, glow: f32, reduce: bool) -> (f32, f32, bool) {
    (if reduce { hovered as u8 as f32 } else { lift }, if reduce { selected as u8 as f32 } else { glow }, selected && focused)
}
pub fn list_width(window: f32) -> f32 { (window * 0.34).clamp(360.0, 520.0) }
pub fn detail_width(d: Density) -> Pixels { theme::READ_WIDTH + theme::space(d, Step::Xl) * 2.0 }

/// The outer option distinguishes leaving the saved value alone from clearing it.
pub fn selection(rows: &[usize], memory: &[MemoryEntry], saved: Option<i64>, previous: Option<i64>, reset: bool, restoring: bool, done: bool) -> (Option<usize>, Option<i64>, bool, Option<Option<i64>>) {
    let desired = if restoring { saved } else if reset { None } else { previous };
    let row = desired.and_then(|id| rows.iter().position(|&i| memory[i].id == id))
        .or_else(|| ((reset || restoring || !done) && !rows.is_empty()).then_some(0));
    let next = row.map(|i| memory[rows[i]].id);
    let restored = restoring && desired.is_some() && desired == next;
    let save = (done && (reset || restoring || previous != next))
        .then_some(if restoring && !restored { None } else { next });
    (row, next, restored, save)
}

pub fn can_undo(id: i64, prior: &MemoryPrior, entry_exists: bool, top: Option<(i64, &MemoryPrior)>) -> bool {
    entry_exists && top.is_some_and(|(top_id, top_prior)| id == top_id && prior == top_prior)
}
