use super::{Change, EvidenceBundle, Manuscript};

#[derive(Clone, Debug, PartialEq)]
pub enum ViolationKind {
    ClaimChanged,
    NumberChanged,
    CitationChanged,
    NewClaim,
    StrengthenedClaim,
    LiteralNumber,
    StructureChanged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StyleViolation {
    pub location: String,
    pub kind: ViolationKind,
    pub detail: String,
}

pub fn revise_instructions() -> String {
    unimplemented!()
}
pub fn style_instructions() -> String {
    unimplemented!()
}
pub fn check_style_only(
    _before: &Manuscript,
    _after: &Manuscript,
) -> Result<(), Vec<StyleViolation>> {
    unimplemented!()
}
pub fn check_revision(
    _before: &Manuscript,
    _after: &Manuscript,
    _bundle: &EvidenceBundle,
    _change_log: &[Change],
) -> Result<(), Vec<StyleViolation>> {
    unimplemented!()
}
