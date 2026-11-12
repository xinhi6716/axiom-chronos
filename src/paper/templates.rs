use crate::{base::Res, paper::model::JournalRecord};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TemplateStatus {
    Verified,
    Unconfirmed { reason: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TemplateEntry {
    pub id: String,
    pub publisher: String,
    pub author_page: String,
    pub download_source: String,
    pub version: String,
    pub sha256: String,
    pub class: String,
    pub class_options: Vec<String>,
    pub bib_style: String,
    pub preamble: Vec<String>,
    #[serde(flatten)]
    pub status: TemplateStatus,
}

pub fn registry() -> Res<Vec<TemplateEntry>> {
    todo!()
}
pub fn for_journal(_j: &JournalRecord) -> Res<TemplateEntry> {
    todo!()
}
