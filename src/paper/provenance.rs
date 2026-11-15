use crate::{
    base::{Failure, Res},
    paper::{
        cache::{read_body, Fetched},
        csv,
        model::{SourceKind, SourceTag},
    },
};
use std::path::{Component, Path};
pub fn tag(
    kind: SourceKind,
    fetched: &Fetched,
    url: &str,
    pointer: &str,
    data_year: Option<i32>,
) -> SourceTag {
    SourceTag {
        kind,
        url: Some(url.into()),
        fetched_at: fetched.fetched_at.clone(),
        data_year,
        raw: format!("{}#{pointer}", fetched.locator),
    }
}
pub fn resolve(cache_root: &Path, tag: &SourceTag) -> Res<serde_json::Value> {
    let (locator, pointer) = tag
        .raw
        .split_once('#')
        .ok_or_else(|| Failure::new("provenance", "missing pointer"))?;
    if locator.is_empty()
        || Path::new(locator)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Failure::new("provenance", "invalid locator"));
    }
    let root = std::fs::canonicalize(cache_root).map_err(|e| Failure::new("io", e.to_string()))?;
    let path =
        std::fs::canonicalize(root.join(locator)).map_err(|e| Failure::new("io", e.to_string()))?;
    if !path.starts_with(&root) {
        return Err(Failure::new("provenance", "locator outside cache"));
    }
    let body = read_body(&path)?;
    if let Some(cell) = pointer.strip_prefix("/row/") {
        let (row, col) = cell
            .split_once('/')
            .ok_or_else(|| Failure::new("provenance", "invalid cell pointer"))?;
        let row: usize = row
            .parse()
            .map_err(|_| Failure::new("provenance", "invalid row"))?;
        let col = col.replace("~1", "/").replace("~0", "~");
        let text = std::str::from_utf8(&body).map_err(|e| Failure::new("csv", e.to_string()))?;
        let table = csv::parse(text, b';')?;
        let index = table
            .headers
            .iter()
            .position(|h| h == &col)
            .ok_or_else(|| Failure::new("provenance", "unknown column"))?;
        return table
            .rows
            .get(row)
            .and_then(|r| r.get(index))
            .cloned()
            .map(serde_json::Value::String)
            .ok_or_else(|| Failure::new("provenance", "unknown row"));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| Failure::new("json", e.to_string()))?;
    json.pointer(pointer)
        .cloned()
        .ok_or_else(|| Failure::new("provenance", "unknown pointer"))
}
