use super::{history, Collected, Gap, Source, SourceStatus};
use crate::usage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PROBLEMS: usize = 10;
pub const MAX_SOURCES_PER_PROBLEM: usize = 3;
pub const MAX_DIGEST_BYTES: usize = 16 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Metric {
    pub value: f64,
    pub samples: u64,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Problem {
    pub id: String,
    pub title: String,
    pub family: String,
    pub count: u64,
    pub cost_usd: Option<f64>,
    pub unpriced: u64,
    pub snapshot: bool,
    pub metric: Metric,
    pub sources: Vec<Source>,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CostRow {
    pub role: String,
    pub model: String,
    pub calls: u64,
    pub priced_usd: f64,
    pub usd: Option<f64>,
    pub unpriced_calls: u64,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Estimate {
    pub median_usd: Option<f64>,
    pub samples: u64,
    pub unpriced_runs: u64,
    pub method: String,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Comparison {
    pub problem: String,
    pub earlier_run: String,
    pub earlier_value: f64,
    pub earlier_samples: u64,
    pub current_value: f64,
    pub current_samples: u64,
    pub difference: f64,
}
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Digest {
    pub now_ms: i64,
    pub since_days: u32,
    pub start_ms: i64,
    pub problems: Vec<Problem>,
    pub gaps: Vec<Gap>,
    pub sources: Vec<SourceStatus>,
    pub cost: Vec<CostRow>,
    pub estimate: Estimate,
    pub comparisons: Vec<Comparison>,
}

fn family(id: &str) -> &str {
    id.split('.').next().unwrap_or(id)
}
fn samples(sources: &[SourceStatus], family: &str) -> u64 {
    sources
        .iter()
        .find(|s| s.family == family)
        .map_or(0, |s| s.samples)
}

pub fn build(
    collected: Collected,
    period: &usage::Period,
    earlier: &[history::RunRecord],
    now_ms: i64,
    since_days: u32,
) -> Digest {
    let Collected {
        events,
        gaps,
        sources,
    } = collected;
    let mut groups: BTreeMap<String, Problem> = BTreeMap::new();
    for e in events {
        let p = groups.entry(e.problem.clone()).or_insert_with(|| Problem {
            id: e.problem.clone(),
            title: e.title.clone(),
            family: family(&e.problem).into(),
            count: 0,
            cost_usd: None,
            unpriced: 0,
            snapshot: false,
            metric: Metric {
                value: 0.0,
                samples: samples(&sources, family(&e.problem)),
            },
            sources: vec![],
        });
        p.count = p.count.saturating_add(e.count);
        if let Some(cost) = e.cost_usd {
            p.cost_usd = Some(p.cost_usd.unwrap_or(0.0) + cost);
        }
        p.unpriced = p.unpriced.saturating_add(e.unpriced);
        p.snapshot |= e.snapshot;
        if !p.sources.iter().any(|s| s.key == e.source.key) {
            p.sources.push(e.source);
        }
    }
    let mut problems: Vec<_> = groups.into_values().collect();
    for p in &mut problems {
        p.metric.value = p.count as f64;
    }
    problems.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| match (a.cost_usd, b.cost_usd) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut costs: BTreeMap<(String, String), CostRow> = BTreeMap::new();
    for row in period.runs.iter().flat_map(|r| &r.ledger.by_agent) {
        let c = costs
            .entry((row.agent_type.clone(), row.model.clone()))
            .or_insert_with(|| CostRow {
                role: row.agent_type.clone(),
                model: row.model.clone(),
                calls: 0,
                priced_usd: 0.0,
                usd: Some(0.0),
                unpriced_calls: 0,
            });
        c.calls = c.calls.saturating_add(row.tally.calls);
        c.priced_usd += row.tally.priced_usd;
        c.unpriced_calls = c.unpriced_calls.saturating_add(row.tally.unpriced_calls);
        c.usd = (c.unpriced_calls == 0).then_some(c.priced_usd);
    }
    let mut cost: Vec<_> = costs.into_values().collect();
    cost.sort_by(|a, b| {
        b.priced_usd
            .total_cmp(&a.priced_usd)
            .then_with(|| a.role.cmp(&b.role))
            .then_with(|| a.model.cmp(&b.model))
    });
    let mut values = Vec::new();
    let mut unpriced_runs = 0u64;
    for run in period
        .history
        .iter()
        .filter(|r| r.skill.as_deref() == Some("wish"))
    {
        match run.usd {
            Some(v) => values.push(v),
            None => unpriced_runs += 1,
        }
    }
    values.sort_by(f64::total_cmp);
    let median_usd = if values.is_empty() {
        None
    } else {
        let mid = values.len() / 2;
        Some(if values.len() % 2 == 1 {
            values[mid]
        } else {
            values[mid - 1] / 2.0 + values[mid] / 2.0
        })
    };
    let estimate = Estimate {
        median_usd,
        samples: values.len() as u64,
        unpriced_runs,
        method: "Median of priced wish runs in the period; unpriced runs are excluded.".into(),
    };
    let mut previous: BTreeMap<&str, (&history::RunRecord, &history::MetricValue)> =
        BTreeMap::new();
    for run in earlier {
        for m in &run.metrics {
            let entry = previous.entry(&m.problem).or_insert((run, m));
            if (run.started_ms, &run.id) > (entry.0.started_ms, &entry.0.id) {
                *entry = (run, m);
            }
        }
    }
    let comparisons = previous
        .into_iter()
        .map(|(id, (run, m))| {
            let current = problems
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.metric.clone())
                .unwrap_or_else(|| Metric {
                    value: 0.0,
                    samples: samples(&sources, family(id)),
                });
            Comparison {
                problem: id.into(),
                earlier_run: run.id.clone(),
                earlier_value: m.value,
                earlier_samples: m.samples,
                current_value: current.value,
                current_samples: current.samples,
                difference: current.value - m.value,
            }
        })
        .collect();
    Digest {
        now_ms,
        since_days,
        start_ms: period.start_ms,
        problems,
        gaps,
        sources,
        cost,
        estimate,
        comparisons,
    }
}

// Bound untrusted labels without splitting UTF-8, and keep forbidden wording out of output.
fn label(s: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(ch) = rest.chars().next() {
        if rest
            .get(..6)
            .is_some_and(|s| s.eq_ignore_ascii_case("saving"))
        {
            if out.len() + 3 > limit {
                break;
            }
            out.push_str("...");
            rest = &rest[6..];
        } else {
            let c = if ch.is_control() { ' ' } else { ch };
            if out.len() + c.len_utf8() > limit {
                break;
            }
            out.push(c);
            rest = &rest[ch.len_utf8()..];
        }
    }
    if !rest.is_empty() {
        out.push('…');
    }
    out
}
fn date(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86400);
    let time = secs.rem_euclid(86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    y += i64::from(month <= 2);
    format!(
        "{y:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}
fn number(v: f64) -> String {
    if v.abs() >= 1e12 {
        format!("{v:.4e}")
    } else {
        format!("{v:.4}")
    }
}
fn money(value: Option<f64>, unpriced: u64) -> String {
    match value {
        Some(v) if unpriced > 0 => format!("${} known ({unpriced} unpriced)", number(v)),
        Some(v) => format!("${}", number(v)),
        None => format!("unknown ({unpriced} unpriced)"),
    }
}
// Each section has a fixed budget, leaving room for the mandatory summary and problem lines.
fn section(out: &mut String, title: &str, lines: impl Iterator<Item = String>, budget: usize) {
    out.push_str(title);
    out.push('\n');
    let mut used = 0;
    let mut omitted = 0;
    for line in lines {
        if used + line.len() + 1 <= budget {
            used += line.len() + 1;
            out.push_str(&line);
            out.push('\n');
        } else {
            omitted += 1;
        }
    }
    if omitted > 0 {
        out.push_str(&format!("{omitted} entries omitted\n"));
    }
}
pub fn render(d: &Digest) -> String {
    let mut out = format!(
        "Period: {} to {} ({} days)\nGaps: {}\n",
        date(d.start_ms),
        date(d.now_ms),
        d.since_days,
        d.gaps.len()
    );
    let shown = d.problems.len().min(MAX_PROBLEMS);
    let omitted_sources: usize = d
        .problems
        .iter()
        .take(shown)
        .map(|p| p.sources.len().saturating_sub(MAX_SOURCES_PER_PROBLEM))
        .sum();
    out.push_str(&format!(
        "{} problems omitted; {} sources omitted (among shown problems)\n",
        d.problems.len() - shown,
        omitted_sources
    ));
    let hook = |s: &&SourceStatus| s.family == "hook" && s.state == "not logged";
    section(
        &mut out,
        "Source states:",
        d.sources
            .iter()
            .filter(hook)
            .chain(d.sources.iter().filter(|s| !hook(s)))
            .map(|s| {
                if s.family == "hook" && s.state == "not logged" {
                    "hook refusals: not logged".to_string()
                } else {
                    format!(
                        "{}: {} (samples: {})",
                        label(&s.family, 80),
                        label(&s.state, 80),
                        s.samples
                    )
                }
            }),
        2000,
    );
    if d.problems.is_empty() {
        out.push_str("There are no recorded problems in the period.\n");
    }
    for p in d.problems.iter().take(MAX_PROBLEMS) {
        let snapshot = if p.snapshot {
            if p.family == "lint" {
                " [current snapshot]"
            } else {
                " [latest snapshot]"
            }
        } else {
            ""
        };
        let locations = if p.sources.is_empty() {
            "(no source location)".to_string()
        } else {
            p.sources
                .iter()
                .take(MAX_SOURCES_PER_PROBLEM)
                .map(|s| label(&s.location, 120))
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push_str(&format!(
            "{}: {}{}; count: {}; cost: {}; samples: {}; sources: {}\n",
            label(&p.id, 120),
            label(&p.title, 120),
            snapshot,
            p.count,
            money(p.cost_usd, p.unpriced),
            p.metric.samples,
            locations
        ));
    }
    section(
        &mut out,
        "Cost by role/model:",
        d.cost.iter().map(|c| {
            format!(
                "{} / {}: calls: {}; priced: ${}; total: {}",
                label(&c.role, 80),
                label(&c.model, 80),
                c.calls,
                number(c.priced_usd),
                money(c.usd, c.unpriced_calls)
            )
        }),
        2500,
    );
    match d.estimate.median_usd {
        Some(v) => out.push_str(&format!(
            "Estimate: median ${}; samples: {}; unpriced runs: {}\n",
            number(v),
            d.estimate.samples,
            d.estimate.unpriced_runs
        )),
        None => out.push_str(&format!(
            "Estimate: cannot be estimated; samples: {}; unpriced runs: {}\n",
            d.estimate.samples, d.estimate.unpriced_runs
        )),
    }
    out.push_str(&format!("{}\n", label(&d.estimate.method, 200)));
    section(
        &mut out,
        "Earlier comparisons:",
        d.comparisons.iter().map(|c| {
            format!(
                "{} ({}): earlier: {} (samples: {}); current: {} (samples: {}); difference: {}",
                label(&c.problem, 80),
                label(&c.earlier_run, 80),
                c.earlier_value,
                c.earlier_samples,
                c.current_value,
                c.current_samples,
                c.difference
            )
        }),
        2500,
    );
    debug_assert!(out.len() <= MAX_DIGEST_BYTES);
    out
}
