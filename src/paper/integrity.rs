use super::{
    evidence,
    model::{tokens, EvidenceBundle, Manuscript, RenderedTable, Token},
    quantities, statistics,
};
use crate::base::Res;
use std::{collections::HashSet, path::Path, sync::OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityKind {
    UnsourcedNumber,
    WrongUnit,
    WrongClaim,
    StatSemantics,
    MissingN,
    MissingTest,
    Inconsistent,
    Dangling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityIssue {
    pub location: String,
    pub kind: IntegrityKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    pub issues: Vec<IntegrityIssue>,
    pub passed: bool,
    pub manuscript_hash: String,
    pub evidence_hash: String,
}

fn issue(
    issues: &mut Vec<IntegrityIssue>,
    location: &str,
    kind: IntegrityKind,
    detail: impl Into<String>,
) {
    issues.push(IntegrityIssue {
        location: location.into(),
        kind,
        detail: detail.into(),
    });
}

// Mask token bytes before looking for literals, preserving byte offsets for UTF-8 text.
fn scan_numbers(
    text: &str,
    bundle: &EvidenceBundle,
    heading: bool,
    location: &str,
    issues: &mut Vec<IntegrityIssue>,
) {
    static NUMBER: OnceLock<regex::Regex> = OnceLock::new();
    static HEADING: OnceLock<regex::Regex> = OnceLock::new();
    let mut mask = vec![false; text.len()];
    for (range, token) in tokens(text) {
        mask[range.clone()].fill(true);
        if let Token::Cite(keys) = token {
            let tail = &text[range.end..];
            if keys
                .iter()
                .all(|key| bundle.references.iter().any(|r| r.key == *key))
                && tail
                    .as_bytes()
                    .get(..4)
                    .map_or(false, |s| s.iter().all(u8::is_ascii_digit))
                && tail.as_bytes().get(4).map_or(true, |b| !b.is_ascii_digit())
            {
                mask[range.end..range.end + 4].fill(true);
            }
        }
    }
    if heading {
        let re = HEADING.get_or_init(|| regex::Regex::new(r"^\s*\d+(?:\.\d+)*(?:\.|\s)").unwrap());
        if let Some(m) = re.find(text) {
            mask[m.start()..m.end()].fill(true);
        }
    }
    let number = NUMBER.get_or_init(|| regex::Regex::new(r"\d+(?:\.\d+)?").unwrap());
    for m in number.find_iter(text) {
        if !mask[m.start()..m.end()].iter().all(|v| *v) {
            issue(
                issues,
                location,
                IntegrityKind::UnsourcedNumber,
                format!("unsourced number {}", m.as_str()),
            );
        }
    }
}

fn sentence_at(text: &str, start: usize, end: usize) -> &str {
    let before = &text[..start];
    let after = &text[end..];
    fn boundary(s: &str) -> impl Iterator<Item = usize> + '_ {
        s.char_indices().filter_map(move |(i, c)| {
            if matches!(c, '.' | '!' | '?' | '\n')
                && (c == '\n'
                    || s[i + c.len_utf8()..]
                        .chars()
                        .next()
                        .map_or(true, char::is_whitespace))
            {
                Some(i + c.len_utf8())
            } else {
                None
            }
        })
    }
    let left = boundary(before).last().unwrap_or(0);
    let right = boundary(after)
        .next()
        .map(|i| end + i)
        .unwrap_or(text.len());
    &text[left..right]
}

pub fn check(
    ms: &Manuscript,
    bundle: &EvidenceBundle,
    tables: &[RenderedTable],
    project_root: &Path,
) -> Res<IntegrityReport> {
    let mut issues = Vec::new();
    let mut used = HashSet::new();
    // Headings are not paragraphs; their stable issue location is sec:<id>/heading.
    for section in &ms.sections {
        scan_numbers(
            &section.heading,
            bundle,
            true,
            &format!("sec:{}/heading", section.id),
            &mut issues,
        );
    }
    for (location, paragraph) in ms.paragraphs() {
        scan_numbers(&paragraph.text, bundle, false, &location, &mut issues);
        for (range, token) in tokens(&paragraph.text) {
            match token {
                Token::Quantity(id) => {
                    let Some(q) = bundle.quantities.iter().find(|q| q.id == id) else {
                        issue(
                            &mut issues,
                            &location,
                            IntegrityKind::Dangling,
                            format!("unknown quantity {id}"),
                        );
                        continue;
                    };
                    used.insert(id);
                    if !paragraph
                        .claims
                        .iter()
                        .any(|claim| q.claim_ids.contains(&claim.claim))
                    {
                        issue(
                            &mut issues,
                            &location,
                            IntegrityKind::WrongClaim,
                            format!("quantity {} has no matching paragraph claim", q.id),
                        );
                    }
                    let checked = quantities::verify(bundle, project_root, q)?;
                    if !checked.ok {
                        let table =
                            evidence::load_table(bundle, project_root, &q.locator.artifact)?;
                        let wrong_unit = q
                            .locator
                            .column
                            .as_ref()
                            .and_then(|c| table.columns.iter().position(|name| name == c))
                            .map_or(false, |i| table.units.get(i) != Some(&q.unit));
                        issue(
                            &mut issues,
                            &location,
                            if wrong_unit {
                                IntegrityKind::WrongUnit
                            } else {
                                IntegrityKind::Inconsistent
                            },
                            checked.detail,
                        );
                    }
                    for mut stat in statistics::semantics(
                        q,
                        sentence_at(&paragraph.text, range.start, range.end),
                    ) {
                        stat.location = location.clone();
                        issues.push(stat);
                    }
                }
                Token::Cite(keys) => {
                    for key in keys {
                        if !bundle.references.iter().any(|r| r.key == key) {
                            issue(
                                &mut issues,
                                &location,
                                IntegrityKind::Dangling,
                                format!("unknown citation {key}"),
                            );
                        }
                    }
                }
                Token::Fig(id) => {
                    if !ms.figures.iter().any(|f| f.id == id) {
                        issue(
                            &mut issues,
                            &location,
                            IntegrityKind::Dangling,
                            format!("unknown figure {id}"),
                        );
                    }
                }
                Token::Tab(id) => {
                    if !ms.tables.iter().any(|t| t.id == id) {
                        issue(
                            &mut issues,
                            &location,
                            IntegrityKind::Dangling,
                            format!("unknown table {id}"),
                        );
                    }
                }
                Token::Sec(id) => {
                    if !ms.sections.iter().any(|s| s.id == id) {
                        issue(
                            &mut issues,
                            &location,
                            IntegrityKind::Dangling,
                            format!("unknown section {id}"),
                        );
                    }
                }
            }
        }
    }
    for table in tables {
        let spec = ms.tables.iter().find(|spec| spec.id == table.id);
        let data = spec
            .map(|spec| evidence::load_table(bundle, project_root, &spec.artifact))
            .transpose()?;
        for (row, cells) in table.cells.iter().enumerate() {
            for (column, cell) in cells.iter().enumerate() {
                let location = format!("tab:{}/r{}c{}", table.id, row, column);
                let value = cell.trim().parse::<f64>().ok().filter(|v| v.is_finite());
                let sourced = if let (Some(spec), Some(data), Some(value)) = (spec, &data, value) {
                    if let Some(col) = spec.columns.get(column) {
                        let tolerance = 0.5 * 10_f64.powi(-(col.precision as i32));
                        let matches = |other: f64| {
                            other.is_finite()
                                && (value - other).abs()
                                    <= tolerance
                                        + f64::EPSILON * value.abs().max(other.abs()).max(1.0) * 4.0
                        };
                        let data_match = data
                            .columns
                            .iter()
                            .position(|name| name == &col.column)
                            .and_then(|i| data.rows.get(row).and_then(|r| r.get(i)))
                            .and_then(|s| s.trim().parse::<f64>().ok())
                            .map_or(false, matches);
                        let mut quantity_match = false;
                        for q in bundle.quantities.iter().filter(|q| {
                            used.contains(&q.id)
                                && q.locator.artifact == spec.artifact
                                && q.locator.column.as_deref() == Some(col.column.as_str())
                                && (q.locator.row == Some(row as u64)
                                    || (q.locator.aggregate.is_some() && table.cells.len() == 1))
                        }) {
                            if matches(q.value) && quantities::verify(bundle, project_root, q)?.ok {
                                quantity_match = true;
                                break;
                            }
                        }
                        data_match || quantity_match
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !sourced {
                    scan_numbers(cell, bundle, false, &location, &mut issues);
                }
                for (range, token) in tokens(cell) {
                    if let Token::Quantity(id) = token {
                        if let Some(q) = bundle.quantities.iter().find(|q| q.id == id) {
                            let checked = quantities::verify(bundle, project_root, q)?;
                            if !checked.ok {
                                let data = evidence::load_table(
                                    bundle,
                                    project_root,
                                    &q.locator.artifact,
                                )?;
                                let wrong_unit = q
                                    .locator
                                    .column
                                    .as_ref()
                                    .and_then(|c| data.columns.iter().position(|name| name == c))
                                    .map_or(false, |i| data.units.get(i) != Some(&q.unit));
                                issue(
                                    &mut issues,
                                    &location,
                                    if wrong_unit {
                                        IntegrityKind::WrongUnit
                                    } else {
                                        IntegrityKind::Inconsistent
                                    },
                                    checked.detail,
                                );
                            }
                            for mut stat in
                                statistics::semantics(q, sentence_at(cell, range.start, range.end))
                            {
                                stat.location = location.clone();
                                issues.push(stat);
                            }
                        } else {
                            issue(
                                &mut issues,
                                &location,
                                IntegrityKind::Dangling,
                                format!("unknown quantity {id}"),
                            );
                        }
                    }
                }
                if let (Some(spec), Ok(value)) = (spec, cell.trim().parse::<f64>()) {
                    if let Some(col) = spec.columns.get(column) {
                        for q in bundle.quantities.iter().filter(|q| {
                            used.contains(&q.id)
                                && q.locator.artifact == spec.artifact
                                && q.locator.column.as_deref() == Some(col.column.as_str())
                                && (q.locator.row == Some(row as u64)
                                    || (q.locator.aggregate.is_some() && table.cells.len() == 1))
                        }) {
                            let tol_q = 0.5 * 10_f64.powi(-(q.precision as i32));
                            let tol_table = 0.5 * 10_f64.powi(-(col.precision as i32));
                            if !value.is_finite()
                                || (value - q.value).abs()
                                    > tol_q
                                        + tol_table
                                        + f64::EPSILON
                                            * value.abs().max(q.value.abs()).max(1.0)
                                            * 4.0
                            {
                                issue(
                                    &mut issues,
                                    &location,
                                    IntegrityKind::Inconsistent,
                                    format!(
                                        "table value {value} disagrees with quantity {} ({})",
                                        q.id, q.value
                                    ),
                                );
                            }
                            if table.units.get(column) != Some(&q.unit) {
                                issue(
                                    &mut issues,
                                    &location,
                                    IntegrityKind::WrongUnit,
                                    format!("table unit disagrees with quantity {}", q.id),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(IntegrityReport {
        passed: issues.is_empty(),
        issues,
        manuscript_hash: ms.hash(),
        evidence_hash: bundle.hash.clone(),
    })
}
