use crate::paper::{
    figures::{fonts::TextMeasure, layout::FigureLayout, presets::Preset},
    model::RenderedTable,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueKind {
    Overlap,
    OutOfCanvas,
    FontTooSmall,
    MissingUnit,
    MissingGlyph,
    FontMissing,
    TableTooWide,
    PageRaster,
    PageCount,
    MissingCharacter,
    Unreadable,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditIssue {
    pub target: String,
    pub kind: IssueKind,
    pub detail: String,
}
pub fn audit_figure(_l: &FigureLayout, _p: &Preset, _m: &dyn TextMeasure) -> Vec<AuditIssue> {
    todo!()
}
pub fn audit_table(_t: &RenderedTable, _text_width_mm: f64) -> Vec<AuditIssue> {
    todo!()
}
