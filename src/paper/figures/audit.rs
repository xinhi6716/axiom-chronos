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
pub fn audit_figure(l: &FigureLayout, p: &Preset, m: &dyn TextMeasure) -> Vec<AuditIssue> {
    let target = format!("fig:{}", l.id);
    let mut issues = Vec::new();
    let mut add = |kind, detail: String| {
        issues.push(AuditIssue {
            target: target.clone(),
            kind,
            detail,
        })
    };
    let texts: Vec<_> = l
        .texts
        .iter()
        .filter(|t| !t.text.starts_with("axiom:plot:"))
        .collect();
    for (i, t) in texts.iter().enumerate() {
        for other in &texts[..i] {
            if t.w_mm > 0.0
                && t.h_mm > 0.0
                && other.w_mm > 0.0
                && other.h_mm > 0.0
                && t.x_mm < other.x_mm + other.w_mm
                && other.x_mm < t.x_mm + t.w_mm
                && t.y_mm < other.y_mm + other.h_mm
                && other.y_mm < t.y_mm + t.h_mm
            {
                add(
                    IssueKind::Overlap,
                    format!("Text boxes {:?} and {:?} overlap", t.text, other.text),
                );
            }
        }
        if t.x_mm < 0.0
            || t.y_mm < 0.0
            || t.x_mm + t.w_mm > l.width_mm
            || t.y_mm + t.h_mm > l.height_mm
        {
            add(
                IssueKind::OutOfCanvas,
                format!("Text {:?} extends outside the canvas", t.text),
            );
        }
        if t.font_pt < p.min_font_pt {
            add(
                IssueKind::FontTooSmall,
                format!(
                    "Text {:?} is {} pt; minimum is {} pt",
                    t.text, t.font_pt, p.min_font_pt
                ),
            );
        }
        match m.covers(&t.text, &t.family) {
            Ok(missing) if !missing.is_empty() => add(
                IssueKind::MissingGlyph,
                format!(
                    "Font {} lacks glyphs {:?} in {:?}",
                    t.family, missing, t.text
                ),
            ),
            Err(e) if e.kind == "font_missing" => add(
                IssueKind::FontMissing,
                format!("{}: {}", t.family, e.message),
            ),
            Err(e) => add(
                IssueKind::MissingGlyph,
                format!("Cannot check glyphs for {}: {}", t.family, e),
            ),
            _ => {}
        }
    }
    for axis in &l.axes {
        if let Some(unit) = &axis.data_unit {
            if !contains_unit(&axis.label, unit) {
                add(
                    IssueKind::MissingUnit,
                    format!("Axis {:?} lacks unit {:?}", axis.label, unit),
                );
            }
        }
    }
    issues
}
pub fn audit_table(t: &RenderedTable, text_width_mm: f64) -> Vec<AuditIssue> {
    let target = format!("tab:{}", t.id);
    let mut issues = Vec::new();
    if t.width_mm > text_width_mm {
        issues.push(AuditIssue {
            target: target.clone(),
            kind: IssueKind::TableTooWide,
            detail: format!("Table width {} mm exceeds {} mm", t.width_mm, text_width_mm),
        });
    }
    for (i, unit) in t.units.iter().enumerate() {
        if let Some(unit) = unit {
            if !t
                .headers
                .get(i)
                .is_some_and(|header| contains_unit(header, unit))
            {
                issues.push(AuditIssue {
                    target: target.clone(),
                    kind: IssueKind::MissingUnit,
                    detail: format!("Column {i} header lacks unit {unit:?}"),
                });
            }
        }
    }
    issues
}

fn contains_unit(label: &str, unit: &str) -> bool {
    label.match_indices(unit).any(|(start, _)| {
        let end = start + unit.len();
        let left = label[..start].chars().next_back();
        let right = label[end..].chars().next();
        !(unit.chars().next().is_some_and(char::is_alphanumeric)
            && left.is_some_and(char::is_alphanumeric))
            && !(unit.chars().next_back().is_some_and(char::is_alphanumeric)
                && right.is_some_and(char::is_alphanumeric))
    })
}
