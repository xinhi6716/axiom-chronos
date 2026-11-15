use crate::{
    base::{Failure, Res},
    paper::{
        cache::{Cache, FetchMode},
        http::Transport,
        model::{Access, Apc, Known, Money, SourceKind, Sourced},
        provenance,
    },
};
#[derive(Debug)]
pub struct DoajRecord {
    pub apc: Known<Apc>,
    pub access: Sourced<Access>,
    pub review_weeks: Known<u32>,
    pub homepage: Known<String>,
    pub ai_policy: Known<String>,
}
fn unknown<T>() -> Known<T> {
    Known::Unknown {
        reason: "Not reported by DOAJ".into(),
    }
}
pub fn journal(
    cache: &Cache,
    t: &dyn Transport,
    mode: FetchMode,
    issn: &str,
) -> Res<Option<DoajRecord>> {
    let url = format!(
        "https://doaj.org/api/search/journals/issn%3A{}",
        super::openalex::encode_component(issn)
    );
    let f = cache.get(SourceKind::Doaj, &url, mode, t)?;
    let v: serde_json::Value =
        serde_json::from_slice(&f.body).map_err(|e| Failure::new("json", e.to_string()))?;
    let Some(b) = v.pointer("/results/0/bibjson").filter(|v| v.is_object()) else {
        return Ok(None);
    };
    let source = |p: &str| {
        provenance::tag(
            SourceKind::Doaj,
            &f,
            &url,
            &format!("/results/0/bibjson{p}"),
            None,
        )
    };
    let apc = match b.pointer("/apc/has_apc").and_then(|x| x.as_bool()) {
        Some(false) => Known::Known(Sourced {
            value: Apc::NoFee,
            source: source("/apc/has_apc"),
        }),
        Some(true) => {
            let price = b.pointer("/apc/max/0/price").and_then(|v| v.as_f64());
            let currency = b
                .pointer("/apc/max/0/currency")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            match (price, currency) {
                (Some(p), Some(c)) if p >= 0.0 && p.is_finite() && p * 100.0 <= u64::MAX as f64 => {
                    Known::Known(Sourced {
                        value: Apc::Fee(Money {
                            amount_minor: (p * 100.0).round() as u64,
                            currency: c.into(),
                        }),
                        source: source("/apc/max/0"),
                    })
                }
                _ => unknown(),
            }
        }
        None => unknown(),
    };
    let review_weeks = b
        .get("publication_time_weeks")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .map(|value| {
            Known::Known(Sourced {
                value,
                source: source("/publication_time_weeks"),
            })
        })
        .unwrap_or_else(unknown);
    let string = |p: &str| {
        b.pointer(p)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                Known::Known(Sourced {
                    value: s.to_owned(),
                    source: source(p),
                })
            })
            .unwrap_or_else(unknown)
    };
    Ok(Some(DoajRecord {
        apc,
        access: Sourced {
            value: Access::OpenAccess,
            source: source(""),
        },
        review_weeks,
        homepage: string("/ref/journal"),
        ai_policy: string("/ai_policy/url"),
    }))
}
