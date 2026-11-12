use super::model::{EvidenceBundle, Quantity};
use crate::base::{Failure, Res};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct QuantityCheck {
    pub id: String,
    pub recomputed: Option<f64>,
    pub ok: bool,
    pub detail: String,
}

pub fn verify(_bundle: &EvidenceBundle, _project_root: &Path, _q: &Quantity) -> Res<QuantityCheck> {
    Err(Failure::new("not_implemented", "quantity verification"))
}

pub fn render(_q: &Quantity) -> String {
    String::new()
}

pub fn fill(_text: &str, _bundle: &EvidenceBundle) -> Res<String> {
    Err(Failure::new("not_implemented", "quantity filling"))
}
