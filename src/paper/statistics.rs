use super::{
    integrity::{IntegrityIssue, IntegrityKind},
    model::{Quantity, Uncertainty},
};
use std::sync::OnceLock;

pub fn semantics(q: &Quantity, sentence: &str) -> Vec<IntegrityIssue> {
    static SD: OnceLock<regex::Regex> = OnceLock::new();
    static CI: OnceLock<regex::Regex> = OnceLock::new();
    let sd = SD
        .get_or_init(|| regex::Regex::new(r"(?i)\b(?:sd|standard deviations?)\b").unwrap())
        .is_match(sentence);
    let ci = CI
        .get_or_init(|| regex::Regex::new(r"(?i)\b(?:ci|confidence intervals?)\b").unwrap())
        .is_match(sentence);
    let mut issues = Vec::new();
    let mut add = |kind, detail: &str| {
        issues.push(IntegrityIssue {
            location: String::new(),
            kind,
            detail: detail.into(),
        })
    };
    if sd && !matches!(q.uncertainty, Some(Uncertainty::Sd(_))) {
        add(
            IntegrityKind::StatSemantics,
            "standard deviation does not match quantity uncertainty",
        );
    }
    if ci && !matches!(q.uncertainty, Some(Uncertainty::Ci { .. })) {
        add(
            IntegrityKind::StatSemantics,
            "confidence interval does not match quantity uncertainty",
        );
    }
    if let Some(Uncertainty::Ci { level, .. }) = q.uncertainty {
        if !(level > 0.0 && level < 1.0) {
            add(
                IntegrityKind::StatSemantics,
                "confidence level must be between zero and one",
            );
        }
    }
    if q.p_value.is_some() {
        if q.n.is_none() {
            add(IntegrityKind::MissingN, "p-value requires sample size");
        }
        if q.test.as_ref().map_or(true, |s| s.trim().is_empty()) {
            add(IntegrityKind::MissingTest, "p-value requires a test");
        }
    }
    issues
}
