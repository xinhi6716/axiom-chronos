use crate::{
    base::Res,
    paper::{
        bibliography::{BibEntry, Bibliography},
        journals::openalex::WorkMeta,
        model::{Manuscript, SourceTag},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FieldCheck {
    Verified,
    Mismatch { expected: String, found: String },
    Unknown { reason: String },
    NotApplicable,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CitationCheck {
    pub key: String,
    pub id: Option<String>,
    pub fields: BTreeMap<String, FieldCheck>,
    pub verified: bool,
    pub sources: Vec<SourceTag>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CitationReport {
    pub checks: Vec<CitationCheck>,
    pub unknown_keys: Vec<(String, String)>,
    pub blocking: Vec<String>,
    pub manuscript_hash: String,
}

pub fn normalize_doi(_value: &str) -> Option<String> {
    todo!()
}
pub fn normalize_openalex(_value: &str) -> Option<String> {
    todo!()
}
pub fn verify_entry(
    _e: &BibEntry,
    _crossref: &dyn Fn(&str) -> Res<Option<WorkMeta>>,
    _openalex: &dyn Fn(&str) -> Res<Option<WorkMeta>>,
) -> CitationCheck {
    todo!()
}
pub fn verify_manuscript(
    _ms: &Manuscript,
    _bib: &Bibliography,
    _crossref: &dyn Fn(&str) -> Res<Option<WorkMeta>>,
    _openalex: &dyn Fn(&str) -> Res<Option<WorkMeta>>,
) -> CitationReport {
    todo!()
}
