use super::{
    index::{self, ArtifactStatus, Index, RowRef},
    manifest::{Artifact, Capability, Role},
    read,
};
use crate::base::{Failure, Res};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataLayer {
    Extracted,
    Raw,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Value(String),
    Empty { reason: Option<String> },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conversion {
    Converted { rule: String },
    NotConvertible { reason: String },
    NotNumeric,
    Same,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetRow {
    pub record_id: String,
    pub layer: DataLayer,
    pub paper_or_source: String,
    pub condition_id: String,
    pub variable: String,
    pub entity: String,
    pub original: Cell,
    pub unit_original: String,
    pub canonical: Cell,
    pub unit_canonical: String,
    pub conversion: Conversion,
    pub quality_grade: Option<String>,
    pub evidence: Vec<String>,
    pub flags: Vec<String>,
    pub locator: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DatasetQuery {
    pub layer: DataLayer,
    pub filters: Vec<index::Filter>,
    pub sort: Option<index::Sort>,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetPage {
    pub rows: Vec<DatasetRow>,
    pub total: usize,
    pub offset: usize,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Validation {
    Valid { data_sha256: String },
    Invalid { errors: u64, data_sha256: String },
    Stale { recorded: String, current: String },
    NotRun,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingRate {
    pub column: String,
    pub missing: usize,
    pub total: usize,
}

fn artifact(ix: &Index, layer: DataLayer) -> Res<&Artifact> {
    let (capability, role) = match layer {
        DataLayer::Extracted => (Capability::Dataset, Role::Measurements),
        DataLayer::Raw => (Capability::RawMeasurements, Role::RawMeasurements),
    };
    let located = ix.located();
    located.require(capability)?;
    let a = located
        .manifest
        .artifacts
        .iter()
        .find(|a| a.role == role)
        .ok_or_else(|| Failure::new("not_capable", format!("no artifact for {layer:?}")))?;
    match ix.status(&a.id) {
        ArtifactStatus::Ready(_) => Ok(a),
        ArtifactStatus::Missing => Err(Failure::new(
            "missing_artifact",
            format!("artifact {} missing", a.id),
        )),
        ArtifactStatus::Corrupt(message) => Err(Failure::new("corrupt_artifact", message)),
        ArtifactStatus::NotTabular => Err(Failure::new(
            "unavailable",
            format!("artifact {} is not tabular", a.id),
        )),
    }
}
fn optional(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}
fn cell(s: &str, reason: &str) -> Cell {
    if s.is_empty() {
        Cell::Empty {
            reason: optional(reason),
        }
    } else {
        Cell::Value(s.to_owned())
    }
}
fn split(s: &str) -> Vec<String> {
    s.split([';', '|'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn make_row(row: RowRef<'_>, layer: DataLayer) -> DatasetRow {
    let original = row.get("value_original");
    let canonical = row.get("value_canonical");
    let rule = row.get("conversion_rule");
    let reason = row.get("missing_reason");
    let unit_original = row.get("unit_original");
    let unit_canonical = row.get("unit_canonical");
    let conversion =
        if !original.is_empty() && original.trim().parse::<f64>().is_ok() && canonical.is_empty() {
            Conversion::NotConvertible {
                reason: if !reason.is_empty() {
                    reason
                } else if !rule.is_empty() {
                    rule
                } else {
                    "no canonical value recorded"
                }
                .to_owned(),
            }
        } else if !original.is_empty()
            && original.trim().parse::<f64>().is_err()
            && (canonical.is_empty() || canonical == original)
        {
            Conversion::NotNumeric
        } else if !canonical.is_empty()
            && !rule.is_empty()
            && (canonical != original || unit_canonical != unit_original)
        {
            Conversion::Converted {
                rule: rule.to_owned(),
            }
        } else {
            Conversion::Same
        };
    let source = row.get("source_file");
    let source_row = row.get("source_row");
    let source_column = row.get("source_column");
    let locator = (layer == DataLayer::Raw
        && !source.is_empty()
        && !source_row.is_empty()
        && !source_column.is_empty())
    .then(|| format!("{source}#{source_row}/{source_column}"));
    let entity = row.get("entity_canonical");
    DatasetRow {
        record_id: row.record_id(),
        layer,
        paper_or_source: if layer == DataLayer::Raw {
            source
        } else {
            row.get("paper_id")
        }
        .to_owned(),
        condition_id: row.get("condition_id").to_owned(),
        variable: row.get("variable").to_owned(),
        entity: if entity.is_empty() {
            row.get("entity_raw")
        } else {
            entity
        }
        .to_owned(),
        original: cell(original, reason),
        unit_original: unit_original.to_owned(),
        canonical: cell(canonical, reason),
        unit_canonical: unit_canonical.to_owned(),
        conversion,
        quality_grade: optional(row.get("quality_grade")),
        evidence: split(row.get("evidence_id")),
        flags: split(row.get("flags")),
        locator,
    }
}
pub fn page(ix: &Index, q: &DatasetQuery) -> Res<DatasetPage> {
    let a = artifact(ix, q.layer)?;
    let p = ix.query(&index::Query {
        artifact: a.id.clone(),
        filters: q.filters.clone(),
        sort: q.sort.clone(),
        offset: q.offset,
        limit: q.limit,
    })?;
    let rows = p
        .rows
        .into_iter()
        .map(|i| {
            ix.row(&a.id, i)
                .map(|row| make_row(row, q.layer))
                .ok_or_else(|| Failure::new("unavailable", "indexed row missing"))
        })
        .collect::<Res<Vec<_>>>()?;
    Ok(DatasetPage {
        rows,
        total: p.total,
        offset: p.offset,
        generation: p.generation,
    })
}
pub fn validation(ix: &Index) -> Validation {
    let located = ix.located();
    let result = (|| -> Option<Validation> {
        let report = located
            .manifest
            .artifacts
            .iter()
            .find(|a| a.role == Role::Validation)?;
        let measurements = located
            .manifest
            .artifacts
            .iter()
            .find(|a| a.role == Role::Measurements)?;
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(located.artifact_path(report).ok()?).ok()?)
                .ok()?;
        let recorded = report.get("measurements_sha256")?.as_str()?.to_owned();
        let errors = report.get("n_errors")?.as_u64()?;
        let current = read::hash_file(&located.artifact_path(measurements).ok()?).ok()?;
        Some(if recorded != current {
            Validation::Stale { recorded, current }
        } else if errors == 0 {
            Validation::Valid {
                data_sha256: current,
            }
        } else {
            Validation::Invalid {
                errors,
                data_sha256: current,
            }
        })
    })();
    result.unwrap_or(Validation::NotRun)
}
pub fn missing_rates(ix: &Index, layer: DataLayer) -> Res<Vec<MissingRate>> {
    let a = artifact(ix, layer)?;
    let total = ix.len(&a.id);
    let mut rates: Vec<_> = a
        .columns
        .keys()
        .map(|column| MissingRate {
            column: column.clone(),
            missing: 0,
            total,
        })
        .collect();
    rates.sort_by(|a, b| a.column.cmp(&b.column));
    for i in 0..total {
        let row = ix
            .row(&a.id, i as u32)
            .ok_or_else(|| Failure::new("unavailable", "indexed row missing"))?;
        for rate in &mut rates {
            if row.get(&rate.column).is_empty() {
                rate.missing += 1;
            }
        }
    }
    Ok(rates)
}
pub fn evidence_filter(evidence_id: &str) -> index::Filter {
    index::Filter {
        column: "evidence_id".into(),
        op: index::Op::Contains(evidence_id.into()),
    }
}
