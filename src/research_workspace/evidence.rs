use super::{
    index::{ArtifactStatus, Index, RowRef},
    manifest::{Artifact, Format, Role},
    read::{self, Limits},
};
use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Literature,
    Raw,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    TopLeft,
    BottomLeft,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    Pt,
    Px,
    Normalized,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CoordinateSystem {
    pub origin: Origin,
    pub units: Units,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PageGeometry {
    pub page_width: Option<f64>,
    pub page_height: Option<f64>,
    pub coordinate_system: Option<CoordinateSystem>,
    pub rotation: Option<u16>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Locator {
    PdfPage {
        paper_id: String,
        sha256: String,
        physical_page: u32,
        printed_page: Option<String>,
        bbox: Option<BBox>,
        geometry: PageGeometry,
    },
    RawField {
        root: Option<String>,
        file: String,
        row: Option<u64>,
        key: Option<String>,
        column: String,
    },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EvidenceItem {
    pub evidence_id: String,
    pub record_id: String,
    pub layer: Layer,
    pub locator: Locator,
    pub quote: String,
    pub quote_verified: bool,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Highlight {
    pub bbox: BBox,
    pub geometry: PageGeometry,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    PdfPage {
        document_path: String,
        physical_page: u32,
        printed_page: Option<String>,
        n_pages: Option<u32>,
        highlight: Option<Highlight>,
    },
    RawField {
        file: String,
        row: u64,
        column: String,
        value: String,
    },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Resolved {
    pub item: EvidenceItem,
    pub target: Target,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ValueRow {
    pub record_id: String,
    pub layer: Layer,
    pub condition_id: String,
    pub variable: String,
    pub entity_raw: String,
    pub value_original: String,
    pub unit_original: String,
    pub value_canonical: String,
    pub unit_canonical: String,
    pub evidence: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConditionGroup {
    pub condition_id: String,
    pub rows: Vec<ValueRow>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contradiction {
    pub condition_id: String,
    pub variable: String,
    pub a: ValueRow,
    pub b: ValueRow,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EntityGroup {
    pub entity: String,
    pub aliases: Vec<String>,
    pub conditions: Vec<ConditionGroup>,
    pub contradictions: Vec<Contradiction>,
}

fn artifact(ix: &Index, role: Role) -> Res<&Artifact> {
    let a = ix
        .located()
        .manifest
        .artifacts
        .iter()
        .find(|a| a.role == role)
        .ok_or_else(|| Failure::new("missing_artifact", format!("{role:?} missing")))?;
    match ix.status(&a.id) {
        ArtifactStatus::Ready(_) => Ok(a),
        ArtifactStatus::Missing => Err(Failure::new("missing_artifact", &a.id)),
        ArtifactStatus::Corrupt(e) => Err(Failure::new("corrupt_artifact", e)),
        ArtifactStatus::NotTabular => Err(Failure::new("corrupt_artifact", &a.id)),
    }
}
fn opt(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}
fn number<T: std::str::FromStr>(s: &str, field: &str) -> Res<Option<T>> {
    if s.is_empty() {
        Ok(None)
    } else {
        s.parse()
            .map(Some)
            .map_err(|_| Failure::new("corrupt_artifact", format!("invalid {field}: {s}")))
    }
}
fn bbox(s: &str) -> Option<BBox> {
    let nums: Vec<f64> = s
        .split(',')
        .map(|s| s.trim().parse())
        .collect::<Result<_, _>>()
        .ok()?;
    match nums.as_slice() {
        [x0, y0, x1, y1] if nums.iter().all(|n| n.is_finite()) => Some(BBox {
            x0: *x0,
            y0: *y0,
            x1: *x1,
            y1: *y1,
        }),
        _ => None,
    }
}
pub fn item(ix: &Index, evidence_id: &str) -> Res<EvidenceItem> {
    let a = artifact(ix, Role::Evidence)?;
    let row = ix
        .find(&a.id, evidence_id)
        .and_then(|i| ix.row(&a.id, i))
        .ok_or_else(|| Failure::new("not_found", evidence_id))?;
    let (layer, locator) = match row.get("locator_type") {
        "raw_field" => (
            Layer::Raw,
            Locator::RawField {
                root: opt(row.get("raw_root")),
                file: row.get("raw_file").into(),
                row: number(row.get("raw_row"), "raw_row")?,
                key: opt(row.get("raw_key")),
                column: row.get("raw_column").into(),
            },
        ),
        "pdf_page" | "" => {
            let origin = match row.get("coordinate_origin") {
                "top_left" => Some(Origin::TopLeft),
                "bottom_left" => Some(Origin::BottomLeft),
                _ => None,
            };
            let units = match row.get("coordinate_units") {
                "pt" => Some(Units::Pt),
                "px" => Some(Units::Px),
                "normalized" => Some(Units::Normalized),
                _ => None,
            };
            let geometry = PageGeometry {
                page_width: row.get("page_width").parse().ok(),
                page_height: row.get("page_height").parse().ok(),
                coordinate_system: origin
                    .zip(units)
                    .map(|(origin, units)| CoordinateSystem { origin, units }),
                rotation: row.get("rotation").parse().ok(),
            };
            let physical_page = number(row.get("physical_page"), "physical_page")?
                .ok_or_else(|| Failure::new("corrupt_artifact", "missing physical_page"))?;
            (
                Layer::Literature,
                Locator::PdfPage {
                    paper_id: row.get("paper_id").into(),
                    sha256: row.get("sha256").into(),
                    physical_page,
                    printed_page: opt(row.get("printed_page")),
                    bbox: bbox(row.get("bbox")),
                    geometry,
                },
            )
        }
        other => {
            return Err(Failure::new(
                "corrupt_artifact",
                format!("unknown locator type {other}"),
            ))
        }
    };
    Ok(EvidenceItem {
        evidence_id: evidence_id.into(),
        record_id: row.record_id(),
        layer,
        locator,
        quote: row.get("quote").into(),
        quote_verified: matches!(
            row.get("quote_verified").to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        ),
    })
}
pub fn resolve(ix: &Index, evidence_id: &str, lim: &Limits) -> Res<Resolved> {
    let item = item(ix, evidence_id)?;
    let target = match &item.locator {
        Locator::PdfPage {
            sha256,
            physical_page,
            printed_page,
            bbox,
            geometry,
            ..
        } => {
            let a = artifact(ix, Role::Inventory)?;
            let inventory = (0..ix.len(&a.id))
                .filter_map(|n| ix.row(&a.id, n as u32))
                .find(|r| !sha256.is_empty() && r.get("sha256").eq_ignore_ascii_case(sha256))
                .ok_or_else(|| Failure::new("unresolved_locator", "inventory hash not found"))?;
            let n_pages: Option<u32> = number(inventory.get("n_pages"), "n_pages")?;
            if *physical_page == 0 || n_pages.is_some_and(|n| *physical_page > n) {
                return Err(Failure::new(
                    "corrupt_artifact",
                    "physical page out of range",
                ));
            }
            let highlight = if geometry.page_width.is_some()
                && geometry.page_height.is_some()
                && geometry.coordinate_system.is_some()
            {
                bbox.clone().map(|bbox| Highlight {
                    bbox,
                    geometry: geometry.clone(),
                })
            } else {
                None
            };
            Target::PdfPage {
                document_path: inventory.get("path").into(),
                physical_page: *physical_page,
                printed_page: printed_page.clone(),
                n_pages,
                highlight,
            }
        }
        Locator::RawField {
            root,
            file,
            row,
            key,
            column,
        } => {
            let l = ix.located();
            read::guard(&l.root_path(root.as_deref())?, file)?;
            let format = if file.to_ascii_lowercase().ends_with(".csv") {
                Format::Csv
            } else if file.to_ascii_lowercase().ends_with(".jsonl") {
                Format::Jsonl
            } else {
                return Err(Failure::new(
                    "unresolved_locator",
                    "unsupported raw file format",
                ));
            };
            let mut temp = (**l).clone();
            let id = "__evidence_raw_reader";
            temp.manifest.artifacts.retain(|a| a.id != id);
            temp.manifest.artifacts.push(Artifact {
                id: id.into(),
                role: Role::Other,
                root: root.clone(),
                path: file.clone(),
                format,
                sha256: None,
                columns: BTreeMap::new(),
                documents_root: None,
                produced: None,
            });
            if lim.max_page_rows == 0 {
                return Err(Failure::new(
                    "invalid_argument",
                    "max_page_rows must be positive",
                ));
            }
            let mut cursor = None;
            let found = loop {
                let page = read::read_rows(&temp, id, cursor.as_ref(), lim.max_page_rows, lim)?;
                if let Some(r) = page.rows.into_iter().find(|r| match row {
                    Some(n) => r.number == *n,
                    None => key.as_ref().is_some_and(|k| r.fields.get("key") == Some(k)),
                }) {
                    break Some(r);
                }
                cursor = page.next;
                if cursor.is_none() {
                    break None;
                }
            }
            .ok_or_else(|| Failure::new("unresolved_locator", "raw row not found"))?;
            let value = found
                .fields
                .get(column)
                .ok_or_else(|| Failure::new("unresolved_locator", "raw column not found"))?
                .clone();
            Target::RawField {
                file: file.clone(),
                row: found.number,
                column: column.clone(),
                value,
            }
        }
    };
    Ok(Resolved { item, target })
}
// Unicode 14 case-fold expansions and substitutions that differ from lowercase.
fn casefold(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\u{b5}' => out.push_str("\u{3bc}"),
            '\u{df}' => out.push_str("\u{73}\u{73}"),
            '\u{149}' => out.push_str("\u{2bc}\u{6e}"),
            '\u{17f}' => out.push_str("\u{73}"),
            '\u{1f0}' => out.push_str("\u{6a}\u{30c}"),
            '\u{345}' => out.push_str("\u{3b9}"),
            '\u{390}' => out.push_str("\u{3b9}\u{308}\u{301}"),
            '\u{3b0}' => out.push_str("\u{3c5}\u{308}\u{301}"),
            '\u{3c2}' => out.push_str("\u{3c3}"),
            '\u{3d0}' => out.push_str("\u{3b2}"),
            '\u{3d1}' => out.push_str("\u{3b8}"),
            '\u{3d5}' => out.push_str("\u{3c6}"),
            '\u{3d6}' => out.push_str("\u{3c0}"),
            '\u{3f0}' => out.push_str("\u{3ba}"),
            '\u{3f1}' => out.push_str("\u{3c1}"),
            '\u{3f5}' => out.push_str("\u{3b5}"),
            '\u{587}' => out.push_str("\u{565}\u{582}"),
            '\u{13a0}' => out.push_str("\u{13a0}"),
            '\u{13a1}' => out.push_str("\u{13a1}"),
            '\u{13a2}' => out.push_str("\u{13a2}"),
            '\u{13a3}' => out.push_str("\u{13a3}"),
            '\u{13a4}' => out.push_str("\u{13a4}"),
            '\u{13a5}' => out.push_str("\u{13a5}"),
            '\u{13a6}' => out.push_str("\u{13a6}"),
            '\u{13a7}' => out.push_str("\u{13a7}"),
            '\u{13a8}' => out.push_str("\u{13a8}"),
            '\u{13a9}' => out.push_str("\u{13a9}"),
            '\u{13aa}' => out.push_str("\u{13aa}"),
            '\u{13ab}' => out.push_str("\u{13ab}"),
            '\u{13ac}' => out.push_str("\u{13ac}"),
            '\u{13ad}' => out.push_str("\u{13ad}"),
            '\u{13ae}' => out.push_str("\u{13ae}"),
            '\u{13af}' => out.push_str("\u{13af}"),
            '\u{13b0}' => out.push_str("\u{13b0}"),
            '\u{13b1}' => out.push_str("\u{13b1}"),
            '\u{13b2}' => out.push_str("\u{13b2}"),
            '\u{13b3}' => out.push_str("\u{13b3}"),
            '\u{13b4}' => out.push_str("\u{13b4}"),
            '\u{13b5}' => out.push_str("\u{13b5}"),
            '\u{13b6}' => out.push_str("\u{13b6}"),
            '\u{13b7}' => out.push_str("\u{13b7}"),
            '\u{13b8}' => out.push_str("\u{13b8}"),
            '\u{13b9}' => out.push_str("\u{13b9}"),
            '\u{13ba}' => out.push_str("\u{13ba}"),
            '\u{13bb}' => out.push_str("\u{13bb}"),
            '\u{13bc}' => out.push_str("\u{13bc}"),
            '\u{13bd}' => out.push_str("\u{13bd}"),
            '\u{13be}' => out.push_str("\u{13be}"),
            '\u{13bf}' => out.push_str("\u{13bf}"),
            '\u{13c0}' => out.push_str("\u{13c0}"),
            '\u{13c1}' => out.push_str("\u{13c1}"),
            '\u{13c2}' => out.push_str("\u{13c2}"),
            '\u{13c3}' => out.push_str("\u{13c3}"),
            '\u{13c4}' => out.push_str("\u{13c4}"),
            '\u{13c5}' => out.push_str("\u{13c5}"),
            '\u{13c6}' => out.push_str("\u{13c6}"),
            '\u{13c7}' => out.push_str("\u{13c7}"),
            '\u{13c8}' => out.push_str("\u{13c8}"),
            '\u{13c9}' => out.push_str("\u{13c9}"),
            '\u{13ca}' => out.push_str("\u{13ca}"),
            '\u{13cb}' => out.push_str("\u{13cb}"),
            '\u{13cc}' => out.push_str("\u{13cc}"),
            '\u{13cd}' => out.push_str("\u{13cd}"),
            '\u{13ce}' => out.push_str("\u{13ce}"),
            '\u{13cf}' => out.push_str("\u{13cf}"),
            '\u{13d0}' => out.push_str("\u{13d0}"),
            '\u{13d1}' => out.push_str("\u{13d1}"),
            '\u{13d2}' => out.push_str("\u{13d2}"),
            '\u{13d3}' => out.push_str("\u{13d3}"),
            '\u{13d4}' => out.push_str("\u{13d4}"),
            '\u{13d5}' => out.push_str("\u{13d5}"),
            '\u{13d6}' => out.push_str("\u{13d6}"),
            '\u{13d7}' => out.push_str("\u{13d7}"),
            '\u{13d8}' => out.push_str("\u{13d8}"),
            '\u{13d9}' => out.push_str("\u{13d9}"),
            '\u{13da}' => out.push_str("\u{13da}"),
            '\u{13db}' => out.push_str("\u{13db}"),
            '\u{13dc}' => out.push_str("\u{13dc}"),
            '\u{13dd}' => out.push_str("\u{13dd}"),
            '\u{13de}' => out.push_str("\u{13de}"),
            '\u{13df}' => out.push_str("\u{13df}"),
            '\u{13e0}' => out.push_str("\u{13e0}"),
            '\u{13e1}' => out.push_str("\u{13e1}"),
            '\u{13e2}' => out.push_str("\u{13e2}"),
            '\u{13e3}' => out.push_str("\u{13e3}"),
            '\u{13e4}' => out.push_str("\u{13e4}"),
            '\u{13e5}' => out.push_str("\u{13e5}"),
            '\u{13e6}' => out.push_str("\u{13e6}"),
            '\u{13e7}' => out.push_str("\u{13e7}"),
            '\u{13e8}' => out.push_str("\u{13e8}"),
            '\u{13e9}' => out.push_str("\u{13e9}"),
            '\u{13ea}' => out.push_str("\u{13ea}"),
            '\u{13eb}' => out.push_str("\u{13eb}"),
            '\u{13ec}' => out.push_str("\u{13ec}"),
            '\u{13ed}' => out.push_str("\u{13ed}"),
            '\u{13ee}' => out.push_str("\u{13ee}"),
            '\u{13ef}' => out.push_str("\u{13ef}"),
            '\u{13f0}' => out.push_str("\u{13f0}"),
            '\u{13f1}' => out.push_str("\u{13f1}"),
            '\u{13f2}' => out.push_str("\u{13f2}"),
            '\u{13f3}' => out.push_str("\u{13f3}"),
            '\u{13f4}' => out.push_str("\u{13f4}"),
            '\u{13f5}' => out.push_str("\u{13f5}"),
            '\u{13f8}' => out.push_str("\u{13f0}"),
            '\u{13f9}' => out.push_str("\u{13f1}"),
            '\u{13fa}' => out.push_str("\u{13f2}"),
            '\u{13fb}' => out.push_str("\u{13f3}"),
            '\u{13fc}' => out.push_str("\u{13f4}"),
            '\u{13fd}' => out.push_str("\u{13f5}"),
            '\u{1c80}' => out.push_str("\u{432}"),
            '\u{1c81}' => out.push_str("\u{434}"),
            '\u{1c82}' => out.push_str("\u{43e}"),
            '\u{1c83}' => out.push_str("\u{441}"),
            '\u{1c84}' => out.push_str("\u{442}"),
            '\u{1c85}' => out.push_str("\u{442}"),
            '\u{1c86}' => out.push_str("\u{44a}"),
            '\u{1c87}' => out.push_str("\u{463}"),
            '\u{1c88}' => out.push_str("\u{a64b}"),
            '\u{1e96}' => out.push_str("\u{68}\u{331}"),
            '\u{1e97}' => out.push_str("\u{74}\u{308}"),
            '\u{1e98}' => out.push_str("\u{77}\u{30a}"),
            '\u{1e99}' => out.push_str("\u{79}\u{30a}"),
            '\u{1e9a}' => out.push_str("\u{61}\u{2be}"),
            '\u{1e9b}' => out.push_str("\u{1e61}"),
            '\u{1e9e}' => out.push_str("\u{73}\u{73}"),
            '\u{1f50}' => out.push_str("\u{3c5}\u{313}"),
            '\u{1f52}' => out.push_str("\u{3c5}\u{313}\u{300}"),
            '\u{1f54}' => out.push_str("\u{3c5}\u{313}\u{301}"),
            '\u{1f56}' => out.push_str("\u{3c5}\u{313}\u{342}"),
            '\u{1f80}' => out.push_str("\u{1f00}\u{3b9}"),
            '\u{1f81}' => out.push_str("\u{1f01}\u{3b9}"),
            '\u{1f82}' => out.push_str("\u{1f02}\u{3b9}"),
            '\u{1f83}' => out.push_str("\u{1f03}\u{3b9}"),
            '\u{1f84}' => out.push_str("\u{1f04}\u{3b9}"),
            '\u{1f85}' => out.push_str("\u{1f05}\u{3b9}"),
            '\u{1f86}' => out.push_str("\u{1f06}\u{3b9}"),
            '\u{1f87}' => out.push_str("\u{1f07}\u{3b9}"),
            '\u{1f88}' => out.push_str("\u{1f00}\u{3b9}"),
            '\u{1f89}' => out.push_str("\u{1f01}\u{3b9}"),
            '\u{1f8a}' => out.push_str("\u{1f02}\u{3b9}"),
            '\u{1f8b}' => out.push_str("\u{1f03}\u{3b9}"),
            '\u{1f8c}' => out.push_str("\u{1f04}\u{3b9}"),
            '\u{1f8d}' => out.push_str("\u{1f05}\u{3b9}"),
            '\u{1f8e}' => out.push_str("\u{1f06}\u{3b9}"),
            '\u{1f8f}' => out.push_str("\u{1f07}\u{3b9}"),
            '\u{1f90}' => out.push_str("\u{1f20}\u{3b9}"),
            '\u{1f91}' => out.push_str("\u{1f21}\u{3b9}"),
            '\u{1f92}' => out.push_str("\u{1f22}\u{3b9}"),
            '\u{1f93}' => out.push_str("\u{1f23}\u{3b9}"),
            '\u{1f94}' => out.push_str("\u{1f24}\u{3b9}"),
            '\u{1f95}' => out.push_str("\u{1f25}\u{3b9}"),
            '\u{1f96}' => out.push_str("\u{1f26}\u{3b9}"),
            '\u{1f97}' => out.push_str("\u{1f27}\u{3b9}"),
            '\u{1f98}' => out.push_str("\u{1f20}\u{3b9}"),
            '\u{1f99}' => out.push_str("\u{1f21}\u{3b9}"),
            '\u{1f9a}' => out.push_str("\u{1f22}\u{3b9}"),
            '\u{1f9b}' => out.push_str("\u{1f23}\u{3b9}"),
            '\u{1f9c}' => out.push_str("\u{1f24}\u{3b9}"),
            '\u{1f9d}' => out.push_str("\u{1f25}\u{3b9}"),
            '\u{1f9e}' => out.push_str("\u{1f26}\u{3b9}"),
            '\u{1f9f}' => out.push_str("\u{1f27}\u{3b9}"),
            '\u{1fa0}' => out.push_str("\u{1f60}\u{3b9}"),
            '\u{1fa1}' => out.push_str("\u{1f61}\u{3b9}"),
            '\u{1fa2}' => out.push_str("\u{1f62}\u{3b9}"),
            '\u{1fa3}' => out.push_str("\u{1f63}\u{3b9}"),
            '\u{1fa4}' => out.push_str("\u{1f64}\u{3b9}"),
            '\u{1fa5}' => out.push_str("\u{1f65}\u{3b9}"),
            '\u{1fa6}' => out.push_str("\u{1f66}\u{3b9}"),
            '\u{1fa7}' => out.push_str("\u{1f67}\u{3b9}"),
            '\u{1fa8}' => out.push_str("\u{1f60}\u{3b9}"),
            '\u{1fa9}' => out.push_str("\u{1f61}\u{3b9}"),
            '\u{1faa}' => out.push_str("\u{1f62}\u{3b9}"),
            '\u{1fab}' => out.push_str("\u{1f63}\u{3b9}"),
            '\u{1fac}' => out.push_str("\u{1f64}\u{3b9}"),
            '\u{1fad}' => out.push_str("\u{1f65}\u{3b9}"),
            '\u{1fae}' => out.push_str("\u{1f66}\u{3b9}"),
            '\u{1faf}' => out.push_str("\u{1f67}\u{3b9}"),
            '\u{1fb2}' => out.push_str("\u{1f70}\u{3b9}"),
            '\u{1fb3}' => out.push_str("\u{3b1}\u{3b9}"),
            '\u{1fb4}' => out.push_str("\u{3ac}\u{3b9}"),
            '\u{1fb6}' => out.push_str("\u{3b1}\u{342}"),
            '\u{1fb7}' => out.push_str("\u{3b1}\u{342}\u{3b9}"),
            '\u{1fbc}' => out.push_str("\u{3b1}\u{3b9}"),
            '\u{1fbe}' => out.push_str("\u{3b9}"),
            '\u{1fc2}' => out.push_str("\u{1f74}\u{3b9}"),
            '\u{1fc3}' => out.push_str("\u{3b7}\u{3b9}"),
            '\u{1fc4}' => out.push_str("\u{3ae}\u{3b9}"),
            '\u{1fc6}' => out.push_str("\u{3b7}\u{342}"),
            '\u{1fc7}' => out.push_str("\u{3b7}\u{342}\u{3b9}"),
            '\u{1fcc}' => out.push_str("\u{3b7}\u{3b9}"),
            '\u{1fd2}' => out.push_str("\u{3b9}\u{308}\u{300}"),
            '\u{1fd3}' => out.push_str("\u{3b9}\u{308}\u{301}"),
            '\u{1fd6}' => out.push_str("\u{3b9}\u{342}"),
            '\u{1fd7}' => out.push_str("\u{3b9}\u{308}\u{342}"),
            '\u{1fe2}' => out.push_str("\u{3c5}\u{308}\u{300}"),
            '\u{1fe3}' => out.push_str("\u{3c5}\u{308}\u{301}"),
            '\u{1fe4}' => out.push_str("\u{3c1}\u{313}"),
            '\u{1fe6}' => out.push_str("\u{3c5}\u{342}"),
            '\u{1fe7}' => out.push_str("\u{3c5}\u{308}\u{342}"),
            '\u{1ff2}' => out.push_str("\u{1f7c}\u{3b9}"),
            '\u{1ff3}' => out.push_str("\u{3c9}\u{3b9}"),
            '\u{1ff4}' => out.push_str("\u{3ce}\u{3b9}"),
            '\u{1ff6}' => out.push_str("\u{3c9}\u{342}"),
            '\u{1ff7}' => out.push_str("\u{3c9}\u{342}\u{3b9}"),
            '\u{1ffc}' => out.push_str("\u{3c9}\u{3b9}"),
            '\u{ab70}' => out.push_str("\u{13a0}"),
            '\u{ab71}' => out.push_str("\u{13a1}"),
            '\u{ab72}' => out.push_str("\u{13a2}"),
            '\u{ab73}' => out.push_str("\u{13a3}"),
            '\u{ab74}' => out.push_str("\u{13a4}"),
            '\u{ab75}' => out.push_str("\u{13a5}"),
            '\u{ab76}' => out.push_str("\u{13a6}"),
            '\u{ab77}' => out.push_str("\u{13a7}"),
            '\u{ab78}' => out.push_str("\u{13a8}"),
            '\u{ab79}' => out.push_str("\u{13a9}"),
            '\u{ab7a}' => out.push_str("\u{13aa}"),
            '\u{ab7b}' => out.push_str("\u{13ab}"),
            '\u{ab7c}' => out.push_str("\u{13ac}"),
            '\u{ab7d}' => out.push_str("\u{13ad}"),
            '\u{ab7e}' => out.push_str("\u{13ae}"),
            '\u{ab7f}' => out.push_str("\u{13af}"),
            '\u{ab80}' => out.push_str("\u{13b0}"),
            '\u{ab81}' => out.push_str("\u{13b1}"),
            '\u{ab82}' => out.push_str("\u{13b2}"),
            '\u{ab83}' => out.push_str("\u{13b3}"),
            '\u{ab84}' => out.push_str("\u{13b4}"),
            '\u{ab85}' => out.push_str("\u{13b5}"),
            '\u{ab86}' => out.push_str("\u{13b6}"),
            '\u{ab87}' => out.push_str("\u{13b7}"),
            '\u{ab88}' => out.push_str("\u{13b8}"),
            '\u{ab89}' => out.push_str("\u{13b9}"),
            '\u{ab8a}' => out.push_str("\u{13ba}"),
            '\u{ab8b}' => out.push_str("\u{13bb}"),
            '\u{ab8c}' => out.push_str("\u{13bc}"),
            '\u{ab8d}' => out.push_str("\u{13bd}"),
            '\u{ab8e}' => out.push_str("\u{13be}"),
            '\u{ab8f}' => out.push_str("\u{13bf}"),
            '\u{ab90}' => out.push_str("\u{13c0}"),
            '\u{ab91}' => out.push_str("\u{13c1}"),
            '\u{ab92}' => out.push_str("\u{13c2}"),
            '\u{ab93}' => out.push_str("\u{13c3}"),
            '\u{ab94}' => out.push_str("\u{13c4}"),
            '\u{ab95}' => out.push_str("\u{13c5}"),
            '\u{ab96}' => out.push_str("\u{13c6}"),
            '\u{ab97}' => out.push_str("\u{13c7}"),
            '\u{ab98}' => out.push_str("\u{13c8}"),
            '\u{ab99}' => out.push_str("\u{13c9}"),
            '\u{ab9a}' => out.push_str("\u{13ca}"),
            '\u{ab9b}' => out.push_str("\u{13cb}"),
            '\u{ab9c}' => out.push_str("\u{13cc}"),
            '\u{ab9d}' => out.push_str("\u{13cd}"),
            '\u{ab9e}' => out.push_str("\u{13ce}"),
            '\u{ab9f}' => out.push_str("\u{13cf}"),
            '\u{aba0}' => out.push_str("\u{13d0}"),
            '\u{aba1}' => out.push_str("\u{13d1}"),
            '\u{aba2}' => out.push_str("\u{13d2}"),
            '\u{aba3}' => out.push_str("\u{13d3}"),
            '\u{aba4}' => out.push_str("\u{13d4}"),
            '\u{aba5}' => out.push_str("\u{13d5}"),
            '\u{aba6}' => out.push_str("\u{13d6}"),
            '\u{aba7}' => out.push_str("\u{13d7}"),
            '\u{aba8}' => out.push_str("\u{13d8}"),
            '\u{aba9}' => out.push_str("\u{13d9}"),
            '\u{abaa}' => out.push_str("\u{13da}"),
            '\u{abab}' => out.push_str("\u{13db}"),
            '\u{abac}' => out.push_str("\u{13dc}"),
            '\u{abad}' => out.push_str("\u{13dd}"),
            '\u{abae}' => out.push_str("\u{13de}"),
            '\u{abaf}' => out.push_str("\u{13df}"),
            '\u{abb0}' => out.push_str("\u{13e0}"),
            '\u{abb1}' => out.push_str("\u{13e1}"),
            '\u{abb2}' => out.push_str("\u{13e2}"),
            '\u{abb3}' => out.push_str("\u{13e3}"),
            '\u{abb4}' => out.push_str("\u{13e4}"),
            '\u{abb5}' => out.push_str("\u{13e5}"),
            '\u{abb6}' => out.push_str("\u{13e6}"),
            '\u{abb7}' => out.push_str("\u{13e7}"),
            '\u{abb8}' => out.push_str("\u{13e8}"),
            '\u{abb9}' => out.push_str("\u{13e9}"),
            '\u{abba}' => out.push_str("\u{13ea}"),
            '\u{abbb}' => out.push_str("\u{13eb}"),
            '\u{abbc}' => out.push_str("\u{13ec}"),
            '\u{abbd}' => out.push_str("\u{13ed}"),
            '\u{abbe}' => out.push_str("\u{13ee}"),
            '\u{abbf}' => out.push_str("\u{13ef}"),
            '\u{fb00}' => out.push_str("\u{66}\u{66}"),
            '\u{fb01}' => out.push_str("\u{66}\u{69}"),
            '\u{fb02}' => out.push_str("\u{66}\u{6c}"),
            '\u{fb03}' => out.push_str("\u{66}\u{66}\u{69}"),
            '\u{fb04}' => out.push_str("\u{66}\u{66}\u{6c}"),
            '\u{fb05}' => out.push_str("\u{73}\u{74}"),
            '\u{fb06}' => out.push_str("\u{73}\u{74}"),
            '\u{fb13}' => out.push_str("\u{574}\u{576}"),
            '\u{fb14}' => out.push_str("\u{574}\u{565}"),
            '\u{fb15}' => out.push_str("\u{574}\u{56b}"),
            '\u{fb16}' => out.push_str("\u{57e}\u{576}"),
            '\u{fb17}' => out.push_str("\u{574}\u{56d}"),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}
fn aliases(ix: &Index) -> Res<Vec<(String, String)>> {
    if !ix
        .located()
        .manifest
        .artifacts
        .iter()
        .any(|a| a.role == Role::Aliases)
    {
        return Ok(Vec::new());
    }
    let a = artifact(ix, Role::Aliases)?;
    Ok((0..ix.len(&a.id))
        .filter_map(|i| ix.row(&a.id, i as u32))
        .map(|r| (casefold(r.get("alias")), r.get("canonical").to_owned()))
        .collect())
}
fn canonical(s: &str, aliases: &[(String, String)]) -> String {
    let folded = casefold(s);
    aliases
        .iter()
        .find(|(a, _)| *a == folded)
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| s.to_owned())
}
fn row_entity(row: &RowRef<'_>, aliases: &[(String, String)]) -> String {
    if row.get("entity_canonical").is_empty() {
        canonical(row.get("entity_raw"), aliases)
    } else {
        row.get("entity_canonical").into()
    }
}
fn layer_role(layer: &Layer) -> Role {
    match layer {
        Layer::Literature => Role::Measurements,
        Layer::Raw => Role::RawMeasurements,
    }
}
pub fn entities(
    ix: &Index,
    layer: Layer,
    offset: usize,
    limit: usize,
) -> Res<(Vec<String>, usize)> {
    let a = artifact(ix, layer_role(&layer))?;
    let aliases = aliases(ix)?;
    let names: BTreeSet<_> = (0..ix.len(&a.id))
        .filter_map(|i| ix.row(&a.id, i as u32))
        .map(|r| row_entity(&r, &aliases))
        .collect();
    let total = names.len();
    Ok((names.into_iter().skip(offset).take(limit).collect(), total))
}
pub fn group(ix: &Index, entity: &str, layer: Layer) -> Res<EntityGroup> {
    let a = artifact(ix, layer_role(&layer))?;
    let aliases = aliases(ix)?;
    let entity = canonical(entity, &aliases);
    let mut result = EntityGroup {
        entity: entity.clone(),
        aliases: Vec::new(),
        conditions: Vec::new(),
        contradictions: Vec::new(),
    };
    for r in (0..ix.len(&a.id)).filter_map(|i| ix.row(&a.id, i as u32)) {
        if row_entity(&r, &aliases) != entity {
            continue;
        }
        let raw = r.get("entity_raw").to_owned();
        if !result.aliases.contains(&raw) {
            result.aliases.push(raw.clone());
        }
        let value = ValueRow {
            record_id: r.record_id(),
            layer: layer.clone(),
            condition_id: r.get("condition_id").into(),
            variable: r.get("variable").into(),
            entity_raw: raw,
            value_original: r.get("value_original").into(),
            unit_original: r.get("unit_original").into(),
            value_canonical: r.get("value_canonical").into(),
            unit_canonical: r.get("unit_canonical").into(),
            evidence: r
                .get("evidence_id")
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
        };
        let pos = result
            .conditions
            .iter()
            .position(|c| c.condition_id == value.condition_id)
            .unwrap_or_else(|| {
                result.conditions.push(ConditionGroup {
                    condition_id: value.condition_id.clone(),
                    rows: Vec::new(),
                });
                result.conditions.len() - 1
            });
        for previous in &result.conditions[pos].rows {
            let differs = if previous.value_canonical.is_empty() && value.value_canonical.is_empty()
            {
                previous.value_original != value.value_original
            } else {
                previous.value_canonical != value.value_canonical
            };
            if previous.variable == value.variable && differs {
                result.contradictions.push(Contradiction {
                    condition_id: value.condition_id.clone(),
                    variable: value.variable.clone(),
                    a: previous.clone(),
                    b: value.clone(),
                });
            }
        }
        result.conditions[pos].rows.push(value);
    }
    Ok(result)
}
