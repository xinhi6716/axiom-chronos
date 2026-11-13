//! Local JCR imports.
use crate::{
    base::{atomic_write, Failure, Res},
    paper::{
        csv,
        journals::xlsx,
        model::{
            data_dir, Quartile, QuartileScheme, QuartileValue, SourceKind, SourceTag, Sourced,
        },
    },
    seams::sha256_hex,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, path::Path};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JcrRecord {
    pub title: String,
    pub issn: Option<String>,
    pub eissn: Option<String>,
    pub year: i32,
    pub category: String,
    pub quartile: Quartile,
    pub jif: Option<f64>,
    pub row: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JcrImport {
    pub file_name: String,
    pub file_sha256: String,
    pub imported_at: String,
    pub records: Vec<JcrRecord>,
    pub warnings: Vec<String>,
}
fn normalized(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}
fn optional(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}
fn header(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

pub fn import(path: &Path) -> Res<JcrImport> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "csv" && ext != "xlsx" {
        return Err(Failure::new("jcr_format", "expected CSV or XLSX"));
    }
    let bytes = fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
    let (headers, rows) = if ext == "csv" {
        let text = std::str::from_utf8(&bytes).map_err(|e| Failure::new("csv", e.to_string()))?;
        // Count delimiters in the first logical row, excluding quoted fields.
        let mut counts = [0usize; 3];
        let mut quoted = false;
        let mut it = text.bytes().peekable();
        while let Some(b) = it.next() {
            if b == b'"' {
                if quoted && it.peek() == Some(&b'"') {
                    it.next();
                } else {
                    quoted = !quoted;
                }
            } else if !quoted {
                if b == b'\n' || b == b'\r' {
                    break;
                }
                for (i, d) in [b',', b';', b'\t'].iter().enumerate() {
                    if b == *d {
                        counts[i] += 1;
                    }
                }
            }
        }
        let index = (0..3).max_by_key(|&i| counts[i]).unwrap_or(0);
        let table = csv::parse(text, [b',', b';', b'\t'][index])?;
        (table.headers, table.rows)
    } else {
        let mut rows = xlsx::read_first_sheet(&bytes)?.into_iter();
        (rows.next().unwrap_or_default(), rows.collect())
    };
    let find = |names: &[&str]| {
        headers
            .iter()
            .position(|h| names.contains(&header(h).as_str()))
    };
    let required = |names: &[&str], name: &str| {
        find(names).ok_or_else(|| Failure::new("jcr_columns", format!("missing {name} column")))
    };
    let title = required(&["journal name"], "journal name")?;
    let issn = required(&["issn"], "ISSN")?;
    let eissn = required(&["eissn", "e-issn"], "eISSN")?;
    let category = required(&["category"], "category")?;
    let year = required(&["year"], "year")?;
    let quartile = required(&["jif quartile", "quartile"], "quartile")?;
    let jif = find(&["journal impact factor", "jif"]);
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    for (i, row) in rows.iter().enumerate() {
        let field = |col: usize| row.get(col).map(String::as_str).unwrap_or("").trim();
        if row.iter().all(|s| s.trim().is_empty()) {
            continue;
        }
        let year_value = field(year).parse::<i32>().ok().or_else(|| {
            field(year)
                .parse::<f64>()
                .ok()
                .filter(|v| {
                    v.is_finite()
                        && v.fract() == 0.0
                        && *v >= i32::MIN as f64
                        && *v <= i32::MAX as f64
                })
                .map(|v| v as i32)
        });
        let q = match field(quartile).to_ascii_uppercase().as_str() {
            "Q1" => Some(Quartile::Q1),
            "Q2" => Some(Quartile::Q2),
            "Q3" => Some(Quartile::Q3),
            "Q4" => Some(Quartile::Q4),
            _ => None,
        };
        let (year_value, q) = match (year_value, q) {
            (Some(y), Some(q)) => (y, q),
            _ => {
                warnings.push(format!("row {}: invalid year or quartile", i + 1));
                continue;
            }
        };
        let issn_value = optional(field(issn));
        let eissn_value = optional(field(eissn));
        let key = (
            normalized(
                issn_value
                    .as_deref()
                    .or(eissn_value.as_deref())
                    .unwrap_or(""),
            ),
            year_value,
            field(category).to_ascii_lowercase(),
        );
        if !key.0.is_empty() && !seen.insert(key) {
            warnings.push(format!("row {}: duplicate ISSN, year and category", i + 1));
            continue;
        }
        records.push(JcrRecord {
            title: field(title).into(),
            issn: issn_value,
            eissn: eissn_value,
            year: year_value,
            category: field(category).into(),
            quartile: q,
            jif: jif
                .and_then(|j| field(j).parse::<f64>().ok())
                .filter(|v| v.is_finite()),
            row: i + 1,
        });
    }
    Ok(JcrImport {
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        file_sha256: sha256_hex(&bytes),
        imported_at: crate::base::now_iso(),
        records,
        warnings,
    })
}
pub fn save(data_home: &Path, import: &JcrImport) -> Res<()> {
    if import.file_sha256.len() != 64 || !import.file_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Failure::new("jcr", "invalid file hash"));
    }
    let dir = data_dir(data_home).join("jcr");
    fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes =
        serde_json::to_vec_pretty(import).map_err(|e| Failure::new("json", e.to_string()))?;
    atomic_write(&dir.join(format!("{}.json", import.file_sha256)), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}
pub fn load_latest(data_home: &Path) -> Res<Option<JcrImport>> {
    let dir = data_dir(data_home).join("jcr");
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let mut latest: Option<JcrImport> = None;
    for entry in entries {
        let path = entry.map_err(|e| Failure::new("io", e.to_string()))?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
        let value: JcrImport =
            serde_json::from_slice(&bytes).map_err(|e| Failure::new("json", e.to_string()))?;
        if latest
            .as_ref()
            .map_or(true, |old| value.imported_at > old.imported_at)
        {
            latest = Some(value);
        }
    }
    Ok(latest)
}
impl JcrImport {
    pub fn quartiles_for(
        &self,
        issns: &[String],
        year: Option<i32>,
    ) -> Vec<Sourced<QuartileValue>> {
        let wanted: HashSet<_> = issns
            .iter()
            .map(|s| normalized(s))
            .filter(|s| !s.is_empty())
            .collect();
        let matches = |r: &&JcrRecord| {
            [r.issn.as_deref(), r.eissn.as_deref()]
                .into_iter()
                .flatten()
                .any(|s| wanted.contains(&normalized(s)))
        };
        let selected = year.or_else(|| self.records.iter().filter(matches).map(|r| r.year).max());
        let mut categories = HashSet::new();
        self.records
            .iter()
            .filter(matches)
            .filter(|r| Some(r.year) == selected && categories.insert(r.category.clone()))
            .map(|r| Sourced {
                value: QuartileValue {
                    quartile: r.quartile.clone(),
                    scheme: QuartileScheme::Jcr,
                    category: r.category.clone(),
                    year: r.year,
                },
                source: SourceTag {
                    kind: SourceKind::JcrImport,
                    url: None,
                    fetched_at: self.imported_at.clone(),
                    data_year: Some(r.year),
                    raw: format!("{}:{}:quartile", self.file_sha256, r.row),
                },
            })
            .collect()
    }
}
