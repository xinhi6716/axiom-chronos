use crate::{
    base::{Failure, Res},
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        model::{Access, JournalRecord, Known, SourceKind, SourceTag, Sourced},
        provenance,
    },
};
use serde_json::Value;
use std::{collections::HashSet, path::Path};
pub struct OpenAlex<'a> {
    cache: &'a Cache,
    transport: &'a dyn Transport,
    mode: FetchMode,
    mailto: Option<String>,
}
#[derive(Debug)]
pub struct WorkHit {
    pub id: String,
    pub title: String,
    pub abstract_words: Vec<String>,
    pub year: Option<i32>,
    pub source_id: Option<String>,
    pub topics: Vec<String>,
    pub doi: Option<String>,
}
#[derive(Debug)]
pub struct WorkMeta {
    pub id: String,
    pub doi: Option<String>,
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: Option<String>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    pub first_page: Option<String>,
    pub last_page: Option<String>,
    pub article_number: Option<String>,
    pub source: SourceTag,
}
pub(crate) fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char)
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn string(v: &Value, p: &str) -> Option<String> {
    v.pointer(p)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
fn year(v: &Value) -> Option<i32> {
    v.get("publication_year")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
}
fn names(v: &Value, p: &str) -> Vec<String> {
    v.pointer(p)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| string(x, "/display_name"))
                .collect()
        })
        .unwrap_or_default()
}
fn unknown<T>() -> Known<T> {
    Known::Unknown {
        reason: "Not reported by OpenAlex".into(),
    }
}
impl<'a> OpenAlex<'a> {
    pub fn new(
        cache: &'a Cache,
        transport: &'a dyn Transport,
        mode: FetchMode,
        data_home: &Path,
    ) -> Res<Self> {
        let path = data_home.join("paper/settings.json");
        let mailto = match std::fs::read(path) {
            Ok(b) => {
                let v: Value =
                    serde_json::from_slice(&b).map_err(|e| Failure::new("json", e.to_string()))?;
                v.get("contact_email")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Failure::new("io", e.to_string())),
        };
        Ok(Self {
            cache,
            transport,
            mode,
            mailto,
        })
    }
    fn url(&self, url: String) -> String {
        match &self.mailto {
            Some(m) => format!(
                "{url}{}mailto={}",
                if url.contains('?') { '&' } else { '?' },
                encode_component(m)
            ),
            None => url,
        }
    }
    pub fn search_works(&self, query: &str, from_year: i32, max: usize) -> Res<Vec<WorkHit>> {
        let mut hits = Vec::new();
        let mut cursor = "*".to_string();
        let mut seen = HashSet::new();
        while hits.len() < max {
            if !seen.insert(cursor.clone()) {
                return Err(Failure::new("json", "repeated OpenAlex cursor"));
            }
            let encoded_cursor = if cursor == "*" {
                "*".into()
            } else {
                encode_component(&cursor)
            };
            let url=self.url(format!("https://api.openalex.org/works?search={}&filter=from_publication_date:{from_year}-01-01&per-page=200&cursor={encoded_cursor}",encode_component(query)));
            let f = self
                .cache
                .get(SourceKind::Openalex, &url, self.mode, self.transport)?;
            let v: Value =
                serde_json::from_slice(&f.body).map_err(|e| Failure::new("json", e.to_string()))?;
            let results = v
                .get("results")
                .and_then(Value::as_array)
                .ok_or_else(|| Failure::new("json", "missing works results"))?;
            for w in results.iter().take(max - hits.len()) {
                let mut positions = Vec::new();
                if let Some(index) = w.get("abstract_inverted_index").and_then(Value::as_object) {
                    for (word, indices) in index {
                        if let Some(indices) = indices.as_array() {
                            for index in indices {
                                if let Some(pos) = index.as_u64() {
                                    positions.push((pos, word.clone()));
                                }
                            }
                        }
                    }
                }
                positions.sort_by_key(|p| p.0);
                hits.push(WorkHit {
                    id: string(w, "/id").unwrap_or_default(),
                    title: string(w, "/display_name").unwrap_or_default(),
                    abstract_words: positions.into_iter().map(|p| p.1).collect(),
                    year: year(w),
                    source_id: string(w, "/primary_location/source/id"),
                    topics: names(w, "/topics"),
                    doi: string(w, "/doi"),
                });
            }
            match v
                .pointer("/meta/next_cursor")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                Some(c) => cursor = c.into(),
                None => break,
            }
        }
        Ok(hits)
    }
    pub fn source(&self, id: &str) -> Res<JournalRecord> {
        let id = id.trim_end_matches('/').rsplit('/').next().unwrap_or(id);
        let url = self.url(format!(
            "https://api.openalex.org/sources/{}",
            encode_component(id)
        ));
        let f = self
            .cache
            .get(SourceKind::Openalex, &url, self.mode, self.transport)?;
        let v: Value =
            serde_json::from_slice(&f.body).map_err(|e| Failure::new("json", e.to_string()))?;
        let tag = |p| provenance::tag(SourceKind::Openalex, &f, &url, p, None);
        let known_string = |p| {
            string(&v, p)
                .map(|value| {
                    Known::Known(Sourced {
                        value,
                        source: tag(p),
                    })
                })
                .unwrap_or_else(unknown)
        };
        let access = v
            .get("is_oa")
            .and_then(Value::as_bool)
            .map(|oa| {
                Known::Known(Sourced {
                    value: if oa {
                        Access::OpenAccess
                    } else {
                        Access::Subscription
                    },
                    source: tag("/is_oa"),
                })
            })
            .unwrap_or_else(unknown);
        let works_count = v
            .get("works_count")
            .and_then(Value::as_u64)
            .map(|value| {
                Known::Known(Sourced {
                    value,
                    source: tag("/works_count"),
                })
            })
            .unwrap_or_else(unknown);
        Ok(JournalRecord {
            id: string(&v, "/id").unwrap_or_default(),
            title: string(&v, "/display_name").unwrap_or_default(),
            issns: v
                .get("issn")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            issn_l: string(&v, "/issn_l"),
            publisher: known_string("/host_organization_name"),
            homepage: known_string("/homepage_url"),
            quartiles: vec![],
            sjr: unknown(),
            access,
            apc: unknown(),
            review_weeks: unknown(),
            topics: names(&v, "/topics"),
            works_count,
        })
    }
    pub fn work(&self, id_or_doi: &str) -> Res<Option<WorkMeta>> {
        let id = if let Some(s) = id_or_doi.strip_prefix("https://openalex.org/") {
            s.to_owned()
        } else if let Some(s) = id_or_doi.strip_prefix("https://doi.org/") {
            format!("doi:{}", encode_component(s))
        } else if let Some(s) = id_or_doi.strip_prefix("doi:") {
            format!("doi:{}", encode_component(s))
        } else if id_or_doi.starts_with("10.") {
            format!("doi:{}", encode_component(id_or_doi))
        } else {
            encode_component(id_or_doi)
        };
        let url = self.url(format!("https://api.openalex.org/works/{id}"));
        let f = match self
            .cache
            .get(SourceKind::Openalex, &url, self.mode, self.transport)
        {
            Ok(f) => f,
            Err(e) if e.kind == "not_found" => return Ok(None),
            Err(e) => return Err(e),
        };
        let v: Value =
            serde_json::from_slice(&f.body).map_err(|e| Failure::new("json", e.to_string()))?;
        let authors = v
            .get("authorships")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| string(x, "/author/display_name"))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(WorkMeta {
            id: string(&v, "/id").unwrap_or_default(),
            doi: string(&v, "/doi"),
            title: string(&v, "/display_name"),
            authors,
            year: year(&v),
            venue: string(&v, "/primary_location/source/display_name"),
            volume: string(&v, "/biblio/volume"),
            issue: string(&v, "/biblio/issue"),
            first_page: string(&v, "/biblio/first_page"),
            last_page: string(&v, "/biblio/last_page"),
            article_number: string(&v, "/biblio/article_number"),
            source: provenance::tag(SourceKind::Openalex, &f, &url, "", None),
        }))
    }
}
