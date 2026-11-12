use crate::{
    base::Res,
    paper::{
        journals::{
            doaj::DoajRecord,
            jcr::JcrImport,
            openalex::{OpenAlex, WorkHit},
            requirements,
            sjr::SjrTable,
        },
        model::{
            Access, Bucket, Excluded, FitTerm, JournalRecord, Known, RankedJournal, ReqStatus,
            Requirements, Shortlist, Sourced,
        },
    },
    seams::sha256_hex,
};
use std::collections::{BTreeSet, HashSet};

pub const FIT_WEIGHT_ABSTRACT: f64 = 0.35;
pub const FIT_WEIGHT_KEYWORDS: f64 = 0.20;
pub const FIT_WEIGHT_TOPICS: f64 = 0.25;
pub const FIT_WEIGHT_RELATED: f64 = 0.20;
pub const RELATED_FROM_YEARS: i32 = 5;

pub struct JournalSources<'a> {
    pub openalex: &'a OpenAlex<'a>,
    pub sjr: &'a SjrTable,
    pub doaj: &'a dyn Fn(&str) -> Res<Option<DoajRecord>>,
    pub jcr: Option<&'a JcrImport>,
}
pub fn gather(
    r: &Requirements,
    s: &JournalSources,
    max_candidates: usize,
) -> Res<(Vec<JournalRecord>, Vec<WorkHit>)> {
    if max_candidates == 0 {
        return Ok((vec![], vec![]));
    }
    let year = crate::base::now_iso()
        .get(..4)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(1970)
        - RELATED_FROM_YEARS;
    let mut works = Vec::new();
    let mut work_ids = HashSet::new();
    let mut queries = HashSet::new();
    for query in std::iter::once(&r.abstract_text).chain(r.keywords.iter()) {
        let query = query.trim();
        if query.is_empty() || !queries.insert(query.to_lowercase()) {
            continue;
        }
        for work in s.openalex.search_works(query, year, max_candidates)? {
            if work_ids.insert(work.id.clone()) {
                works.push(work);
            }
        }
    }
    let mut ids = HashSet::new();
    let mut journals = Vec::new();
    for id in works.iter().filter_map(|w| w.source_id.as_deref()) {
        if !ids.insert(id.to_owned()) {
            continue;
        }
        if journals.len() >= max_candidates {
            break;
        }
        let mut journal = s.openalex.source(id)?;
        let mut issns = journal.issns.clone();
        if let Some(issn) = &journal.issn_l {
            if !issns.contains(issn) {
                issns.push(issn.clone());
            }
        }
        if let Some(row) = s.sjr.lookup(&issns) {
            journal.sjr = Known::Known(row.sjr);
            journal.quartiles.extend(row.quartiles);
            if matches!(journal.publisher, Known::Unknown { .. }) {
                if let Some(publisher) = row.publisher {
                    journal.publisher = Known::Known(publisher);
                }
            }
            if matches!(journal.access, Known::Unknown { .. }) {
                if let Some(oa) = row.open_access {
                    journal.access = Known::Known(Sourced {
                        value: if oa.value {
                            Access::OpenAccess
                        } else {
                            Access::Subscription
                        },
                        source: oa.source,
                    });
                }
            }
        }
        for issn in &issns {
            if let Some(record) = (s.doaj)(issn)? {
                journal.apc = record.apc;
                journal.access = Known::Known(record.access);
                if matches!(record.review_weeks, Known::Known(_)) {
                    journal.review_weeks = record.review_weeks;
                }
                if matches!(journal.homepage, Known::Unknown { .. })
                    && matches!(record.homepage, Known::Known(_))
                {
                    journal.homepage = record.homepage;
                }
                break;
            }
        }
        if let Some(jcr) = s.jcr {
            journal.quartiles.extend(jcr.quartiles_for(&issns, None));
        }
        journals.push(journal);
    }
    Ok((journals, works))
}
fn words(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}
fn term(
    name: &str,
    weight: f64,
    wanted: &BTreeSet<String>,
    available: &BTreeSet<String>,
) -> FitTerm {
    let matched: Vec<_> = wanted.intersection(available).cloned().collect();
    FitTerm {
        name: name.into(),
        weight,
        score: if wanted.is_empty() {
            0.0
        } else {
            matched.len() as f64 / wanted.len() as f64
        },
        evidence: matched.join(", "),
    }
}
pub fn rank(
    r: &Requirements,
    journals: &[JournalRecord],
    related: &[WorkHit],
    now: &str,
) -> Shortlist {
    let abstract_words = words(&r.abstract_text);
    let keyword_words = words(&r.keywords.join(" "));
    let topic_words = words(&format!("{} {}", r.abstract_text, r.keywords.join(" ")));
    let max_related = journals
        .iter()
        .map(|j| {
            related
                .iter()
                .filter(|w| w.source_id.as_deref() == Some(j.id.as_str()))
                .count()
        })
        .max()
        .unwrap_or(0);
    let mut ranked = Vec::new();
    let mut excluded = Vec::new();
    for j in journals {
        let filters = requirements::check_filters(r, j);
        if filters.iter().any(|f| f.status == ReqStatus::Unmet) {
            excluded.push(Excluded {
                journal: j.clone(),
                filters,
            });
            continue;
        }
        let bucket = if filters.iter().any(|f| f.status == ReqStatus::Unknown) {
            Bucket::NeedsCheck
        } else {
            Bucket::Passes
        };
        let associated: Vec<_> = related
            .iter()
            .filter(|w| w.source_id.as_deref() == Some(j.id.as_str()))
            .collect();
        let mut available = words(&format!("{} {}", j.title, j.topics.join(" ")));
        let mut topics = words(&j.topics.join(" "));
        for work in &associated {
            available.extend(words(&format!(
                "{} {} {}",
                work.title,
                work.abstract_words.join(" "),
                work.topics.join(" ")
            )));
            topics.extend(words(&work.topics.join(" ")));
        }
        let ids: BTreeSet<_> = associated.iter().map(|w| w.id.clone()).collect();
        let terms = vec![
            term("abstract", FIT_WEIGHT_ABSTRACT, &abstract_words, &available),
            term("keywords", FIT_WEIGHT_KEYWORDS, &keyword_words, &available),
            term("topics", FIT_WEIGHT_TOPICS, &topic_words, &topics),
            FitTerm {
                name: "related".into(),
                weight: FIT_WEIGHT_RELATED,
                score: if max_related == 0 {
                    0.0
                } else {
                    associated.len() as f64 / max_related as f64
                },
                evidence: ids.into_iter().collect::<Vec<_>>().join(", "),
            },
        ];
        let fit = terms.iter().map(|t| t.weight * t.score).sum();
        ranked.push(RankedJournal {
            journal: j.clone(),
            bucket,
            filters,
            fit,
            terms,
            free_text: requirements::free_text_status(r, j),
        });
    }
    ranked.sort_by(|a, b| {
        let bucket_order = |b: &Bucket| match b {
            Bucket::Passes => 0,
            Bucket::NeedsCheck => 1,
        };
        bucket_order(&a.bucket)
            .cmp(&bucket_order(&b.bucket))
            .then_with(|| b.fit.total_cmp(&a.fit))
            .then_with(|| a.journal.id.cmp(&b.journal.id))
    });
    excluded.sort_by(|a, b| a.journal.id.cmp(&b.journal.id));
    let mut result = Shortlist {
        requirements_hash: requirements::hash(r),
        ranked,
        excluded,
        generated_at: now.into(),
        hash: String::new(),
    };
    result.hash = sha256_hex(&serde_json::to_vec(&result).expect("shortlist serializes"));
    result
}
