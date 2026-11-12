use super::{integrity::IntegrityIssue, model::Quantity};

pub fn semantics(_q: &Quantity, _sentence: &str) -> Vec<IntegrityIssue> {
    Vec::new()
}
