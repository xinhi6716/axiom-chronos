use super::model::{EvidenceBundle, Manuscript, RenderedTable};
use crate::base::{Failure, Res};
use std::path::Path;

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

pub fn check(
    _ms: &Manuscript,
    _bundle: &EvidenceBundle,
    _tables: &[RenderedTable],
    _project_root: &Path,
) -> Res<IntegrityReport> {
    Err(Failure::new("not_implemented", "integrity check"))
}
