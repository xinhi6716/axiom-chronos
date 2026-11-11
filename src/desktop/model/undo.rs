use crate::desktop::settings::SettingPrior;
use std::collections::VecDeque;

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryPrior { pub body: String, pub summary: String, pub corrected: Option<f64>, pub confirmed: f64 }

#[derive(Clone, Debug, PartialEq)]
pub enum Undoable { Memory { id: i64, prior: MemoryPrior }, Setting(SettingPrior) }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UndoStack { entries: VecDeque<Undoable> }
impl UndoStack {
    pub fn push(&mut self, entry: Undoable) {
        if self.entries.len() == 20 { self.entries.pop_front(); }
        self.entries.push_back(entry);
    }
    pub fn pop(&mut self) -> Option<Undoable> { self.entries.pop_back() }
    pub fn peek_label(&self) -> Option<&'static str> {
        self.entries.back().map(|entry| match entry { Undoable::Memory { .. } => "Undo memory correction", Undoable::Setting(_) => "Undo setting change" })
    }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}
