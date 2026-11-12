use crate::{
    base::{Failure, Res},
    paper::model::{
        Access, AccessWanted, Apc, FilterResult, FreeTextStatus, JournalRecord, Known, Quartile,
        ReqStatus, Requirements,
    },
    seams::sha256_hex,
};

pub fn validate(r: &Requirements) -> Res<()> {
    if r.abstract_text.trim().is_empty() {
        return Err(Failure::new("invalid_requirements", "abstract is empty"));
    }
    if r.require_no_fee && r.apc_limit.is_some() {
        return Err(Failure::new(
            "invalid_requirements",
            "no-fee requirement conflicts with APC limit",
        ));
    }
    if let Some(money) = &r.apc_limit {
        // ISO 4217 active currency codes, including precious metals and fund codes.
        const CODES: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB BOV BRL BSD BTN BWP BYN BZD CAD CDF CHE CHF CHW CLF CLP CNY COP COU CRC CUC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD MMK MNT MOP MRU MUR MVR MWK MXN MXV MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR SBD SCR SDG SEK SGD SHP SLE SLL SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD USN UYI UYU UYW UZS VED VES VND VUV WST XAF XAG XAU XBA XBB XBC XBD XCD XDR XOF XPD XPF XPT XSU XTS XUA XXX YER ZAR ZMW ZWG";
        if !CODES
            .split_whitespace()
            .any(|c| c.eq_ignore_ascii_case(money.currency.trim()))
        {
            return Err(Failure::new(
                "invalid_requirements",
                "unknown currency code",
            ));
        }
    }
    Ok(())
}
pub fn hash(r: &Requirements) -> String {
    sha256_hex(&serde_json::to_vec(r).expect("requirements serialize"))
}
fn quartile(q: &Quartile) -> u8 {
    match q {
        Quartile::Q1 => 1,
        Quartile::Q2 => 2,
        Quartile::Q3 => 3,
        Quartile::Q4 => 4,
    }
}
fn result(
    name: &str,
    status: ReqStatus,
    detail: impl Into<String>,
    source: Option<crate::paper::model::SourceTag>,
) -> FilterResult {
    FilterResult {
        name: name.into(),
        status,
        detail: detail.into(),
        source,
    }
}
pub fn check_filters(r: &Requirements, j: &JournalRecord) -> Vec<FilterResult> {
    let mut filters = Vec::new();
    if let Some(max) = &r.max_quartile {
        let best = j
            .quartiles
            .iter()
            .filter(|q| {
                q.value.scheme == r.tier_scheme
                    && (r.categories.is_empty()
                        || r.categories
                            .iter()
                            .any(|c| c.trim().eq_ignore_ascii_case(q.value.category.trim())))
            })
            .min_by_key(|q| quartile(&q.value.quartile));
        filters.push(match best {
            Some(q) => result(
                "tier",
                if quartile(&q.value.quartile) <= quartile(max) {
                    ReqStatus::Met
                } else {
                    ReqStatus::Unmet
                },
                q.value.label(),
                Some(q.source.clone()),
            ),
            None => result(
                "tier",
                ReqStatus::Unknown,
                "quartile not available for requested scheme and category",
                None,
            ),
        });
    }
    if r.access != AccessWanted::Either {
        filters.push(match &j.access {
            Known::Known(v) => {
                let open = v.value == Access::OpenAccess;
                let met = match r.access {
                    AccessWanted::OpenAccess => open,
                    AccessWanted::Subscription => !open,
                    AccessWanted::Either => true,
                };
                result(
                    "access",
                    if met {
                        ReqStatus::Met
                    } else {
                        ReqStatus::Unmet
                    },
                    format!("{:?}", v.value),
                    Some(v.source.clone()),
                )
            }
            Known::Unknown { reason } => result("access", ReqStatus::Unknown, reason, None),
        });
    }
    if r.require_no_fee || r.apc_limit.is_some() {
        filters.push(match &j.apc {
            Known::Unknown { reason } => result("apc", ReqStatus::Unknown, reason, None),
            Known::Known(v) => {
                let (status, detail) = match &v.value {
                    Apc::NoFee => (ReqStatus::Met, "no fee"),
                    Apc::Fee(_) if r.require_no_fee => (ReqStatus::Unmet, "fee charged"),
                    Apc::Fee(fee) => match &r.apc_limit {
                        Some(limit)
                            if !fee
                                .currency
                                .trim()
                                .eq_ignore_ascii_case(limit.currency.trim()) =>
                        {
                            (ReqStatus::Unknown, "currency differs")
                        }
                        Some(limit) if fee.amount_minor <= limit.amount_minor => {
                            (ReqStatus::Met, "within APC limit")
                        }
                        Some(_) => (ReqStatus::Unmet, "exceeds APC limit"),
                        None => (ReqStatus::Met, "no APC limit"),
                    },
                };
                result("apc", status, detail, Some(v.source.clone()))
            }
        });
    }
    filters
}
pub fn free_text_status(r: &Requirements, j: &JournalRecord) -> Vec<FreeTextStatus> {
    r.free_text
        .iter()
        .map(|text| {
            let found = [&j.publisher, &j.homepage]
                .into_iter()
                .find_map(|fact| match fact {
                    Known::Known(v)
                        if !text.trim().is_empty()
                            && text.trim().eq_ignore_ascii_case(v.value.trim()) =>
                    {
                        Some(v)
                    }
                    _ => None,
                });
            match found {
                Some(fact) => FreeTextStatus {
                    text: text.clone(),
                    status: ReqStatus::Met,
                    detail: "exact sourced fact".into(),
                    source: Some(fact.source.clone()),
                },
                None => FreeTextStatus {
                    text: text.clone(),
                    status: ReqStatus::Unknown,
                    detail: "no exact sourced fact".into(),
                    source: None,
                },
            }
        })
        .collect()
}
