use crate::{
    base::{Failure, Res},
    paper::{
        cache::{Cache, FetchMode},
        citations::{normalize_doi, page_dash},
        http::Transport,
        journals::openalex::{encode_component, WorkMeta},
        model::SourceKind,
        provenance,
    },
};
use serde_json::Value;

pub fn work(cache: &Cache, t: &dyn Transport, mode: FetchMode, doi: &str) -> Res<Option<WorkMeta>> {
    let doi = normalize_doi(doi).ok_or_else(|| Failure::new("doi", "invalid DOI"))?;
    let url = format!("https://api.crossref.org/works/{}", encode_component(&doi));
    let fetched = match cache.get(SourceKind::Crossref, &url, mode, t) {
        Ok(f) => f,
        Err(e) if e.kind == "not_found" => return Ok(None),
        Err(e) => return Err(e),
    };
    let json: Value =
        serde_json::from_slice(&fetched.body).map_err(|e| Failure::new("json", e.to_string()))?;
    let m = json
        .get("message")
        .filter(|v| v.is_object())
        .ok_or_else(|| Failure::new("json", "missing Crossref message"))?;
    let string = |key: &str| {
        m.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let first = |key: &str| {
        m.get(key)
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let authors = m
        .get("author")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    let name = [
                        v.get("given").and_then(Value::as_str),
                        v.get("family").and_then(Value::as_str),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                    if !name.is_empty() {
                        Some(name)
                    } else {
                        v.get("name")
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| format!("{{{s}}}"))
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let page = string("page");
    let (first_page, last_page) = match page {
        Some(p) => match p.split_once(page_dash) {
            Some((a, b)) => (
                Some(a.trim().to_owned()),
                Some(b.trim_start_matches(page_dash).trim().to_owned()),
            ),
            None => (Some(p), None),
        },
        None => (None, None),
    };
    let record_doi = string("DOI").and_then(|s| normalize_doi(&s)).unwrap_or(doi);
    Ok(Some(WorkMeta {
        id: record_doi.clone(),
        doi: Some(record_doi),
        title: first("title"),
        authors,
        year: m
            .pointer("/issued/date-parts/0/0")
            .and_then(Value::as_i64)
            .and_then(|n| i32::try_from(n).ok()),
        venue: first("container-title"),
        volume: string("volume"),
        issue: string("issue"),
        first_page,
        last_page,
        article_number: string("article-number"),
        source: provenance::tag(SourceKind::Crossref, &fetched, &url, "", None),
    }))
}
