use crate::{
    base::{Failure, Res},
    paper::model::JournalRecord,
};
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
    #[derive(Deserialize)]
    struct Registry {
        templates: Vec<TemplateEntry>,
    }
    serde_json::from_str::<Registry>(include_str!("../../engine/paper/templates/registry.json"))
        .map(|r| r.templates)
        .map_err(|e| Failure::new("template_registry", e.to_string()))
}
pub fn for_journal(j: &JournalRecord) -> Res<TemplateEntry> {
    let publisher = j
        .publisher
        .value()
        .map(|p| p.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let id = match publisher.as_str() {
        "elsevier" => "elsarticle",
        "ieee" => "IEEEtran",
        "springer nature" | "springer" => "sn-jnl",
        "american chemical society" | "acs" => "achemso",
        _ => {
            return Err(Failure::new(
                "template_unconfirmed",
                format!("No confirmed template for publisher {publisher}"),
            ))
        }
    };
    let entry = registry()?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| Failure::new("template_unconfirmed", format!("Missing template {id}")))?;
    if let TemplateStatus::Unconfirmed { reason } = &entry.status {
        return Err(Failure::new("template_unconfirmed", reason));
    }
    Ok(entry)
}
