use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SourceRef {
    pub source: String,
    pub author: String,
    pub confidence: Confidence,
    pub evidence: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DossierItem {
    pub id: String,
    pub claim: String,
    pub sources: Vec<SourceRef>,
    pub confidence: Confidence,
    pub independent_support: usize,
    pub contradictions: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rejected {
    pub author: String,
    pub reason: String,
    pub raw: Value,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Dossier {
    pub question: String,
    pub items: Vec<DossierItem>,
    pub rejected: Vec<Rejected>,
}

pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_end_matches(|c: char| {
            c.is_ascii_punctuation()
                || matches!(
                    c,
                    '。' | '，' | '！' | '？' | '；' | '：' | '…' | '、' | '”' | '’'
                )
        })
        .trim_end()
        .to_string()
}

pub fn build(question: &str, findings: &[(String, Value)]) -> Dossier {
    let mut dossier = Dossier {
        question: question.into(),
        items: Vec::new(),
        rejected: Vec::new(),
    };
    let mut keys = HashMap::new();
    let mut pending = Vec::new();
    for (author, finding) in findings {
        let Some(items) = finding.get("items").and_then(Value::as_array) else {
            continue;
        };
        for raw in items {
            let parsed = (|| -> Result<(&str, &str, Confidence), &str> {
                let claim = raw
                    .get("claim")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("missing or empty claim")?;
                let source = raw
                    .get("source")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("missing or empty source")?;
                let confidence = match raw.get("confidence").and_then(Value::as_str) {
                    Some("low") => Confidence::Low,
                    Some("medium") => Confidence::Medium,
                    Some("high") => Confidence::High,
                    _ => return Err("missing or invalid confidence"),
                };
                Ok((claim, source, confidence))
            })();
            let (claim, source, confidence) = match parsed {
                Ok(value) => value,
                Err(reason) => {
                    dossier.rejected.push(Rejected {
                        author: author.clone(),
                        reason: reason.into(),
                        raw: raw.clone(),
                    });
                    continue;
                }
            };
            let key = (normalize(source), normalize(claim));
            let index = *keys.entry(key).or_insert_with(|| {
                let index = dossier.items.len();
                dossier.items.push(DossierItem {
                    id: format!("E{}", index + 1),
                    claim: claim.into(),
                    sources: Vec::new(),
                    confidence,
                    independent_support: 0,
                    contradictions: Vec::new(),
                });
                index
            });
            let item = &mut dossier.items[index];
            if !item.sources.iter().any(|s| s.author == *author) {
                item.sources.push(SourceRef {
                    source: source.into(),
                    author: author.clone(),
                    confidence,
                    evidence: raw
                        .get("evidence")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                });
                item.independent_support = item.sources.len();
                item.confidence = item.confidence.min(confidence);
            }
            if let Some(contradiction) = raw
                .get("contradicts")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                pending.push((index, contradiction.to_owned()));
            }
        }
    }
    for (index, text) in pending {
        let target = normalize(&text);
        let matches: Vec<usize> = dossier
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (i != index && normalize(&item.claim) == target).then_some(i))
            .collect();
        if matches.is_empty() {
            if !dossier.items[index].contradictions.contains(&text) {
                dossier.items[index].contradictions.push(text);
            }
        } else {
            for other in matches {
                let other_id = dossier.items[other].id.clone();
                let this_id = dossier.items[index].id.clone();
                if !dossier.items[index].contradictions.contains(&other_id) {
                    dossier.items[index].contradictions.push(other_id);
                }
                if !dossier.items[other].contradictions.contains(&this_id) {
                    dossier.items[other].contradictions.push(this_id);
                }
            }
        }
    }
    dossier
}

pub fn write(path: &Path, dossier: &Dossier) -> Res<()> {
    let data =
        serde_json::to_vec_pretty(dossier).map_err(|e| Failure::new("parse", e.to_string()))?;
    fs::write(path, data).map_err(|e| Failure::new("io", e.to_string()))
}
pub fn read(path: &Path) -> Res<Dossier> {
    let data = fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
    serde_json::from_slice(&data).map_err(|e| Failure::new("parse", e.to_string()))
}
