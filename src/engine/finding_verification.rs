use super::{
    budgets::Role,
    calls::{self, EngineCall},
    kinds,
};
use crate::{
    base::{self, Failure, Family, Res},
    codex::AppServer,
    orch::{self, events, Evidence, Finding, Severity},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Confirmed,
    Rejected,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub review_id: String,
    pub unit: String,
    pub tree: String,
    pub finding: usize,
    pub location: String,
    pub severity: String,
    pub status: Status,
    pub reason: String,
    pub failed: bool,
    pub reviewer: String,
    pub verifier_model: String,
}

type Key = (String, String, usize);
fn key(c: &Check) -> Key {
    (c.review_id.clone(), c.tree.clone(), c.finding)
}
fn all_checks(store: &Path) -> Res<BTreeMap<Key, Check>> {
    let mut latest = BTreeMap::new();
    for event in events::read_events(store, 0)? {
        if event.kind == kinds::FINDING_VERIFIED {
            if let Ok(check) = serde_json::from_value::<Check>(event.data) {
                latest.insert(key(&check), check);
            }
        }
    }
    Ok(latest)
}
pub fn checks(store: &Path, unit: &str) -> Res<Vec<Check>> {
    Ok(all_checks(store)?
        .into_values()
        .filter(|c| c.unit == unit)
        .collect())
}
pub fn rejection_rate(store: &Path) -> Res<(u64, u64)> {
    let checks = all_checks(store)?;
    let valid = checks.values().filter(|c| !c.failed);
    let mut rejected = 0;
    let mut verified = 0;
    for c in valid {
        verified += 1;
        if c.status == Status::Rejected {
            rejected += 1;
        }
    }
    Ok((rejected, verified))
}

struct Review {
    id: String,
    row: Evidence,
    findings: Vec<Finding>,
    waived: BTreeSet<usize>,
    has_verdict: bool,
}
fn current_reviews(store: &Path, unit: &str, rows: &[Evidence]) -> Res<Vec<Review>> {
    let Some(last) = rows.iter().rev().find(|r| r.kind == "review") else {
        return Ok(vec![]);
    };
    let mut reviews = Vec::new();
    let mut number = 0;
    for (i, row) in rows.iter().enumerate() {
        if row.kind != "review" {
            continue;
        }
        number += 1;
        if row.commit != last.commit {
            continue;
        }
        let reviewer = row
            .command
            .split_once(orch::CARRIED_MARK)
            .map_or(row.command.as_str(), |(s, _)| s);
        let path = orch::review_verdict_path(store, unit, reviewer);
        let verdict = match std::fs::read_to_string(path) {
            Ok(text) => Some(orch::parse_verdict(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Failure::new("io", e.to_string())),
        }
        .filter(|v| v.tree == row.commit);
        let end = rows[i + 1..]
            .iter()
            .position(|r| r.kind == "review")
            .map_or(rows.len(), |n| i + 1 + n);
        let waived = orch::waivers(&rows[i..end])
            .into_iter()
            .map(|w| w.number)
            .collect();
        reviews.push(Review {
            id: orch::review_id(unit, number),
            row: row.clone(),
            findings: verdict
                .as_ref()
                .map(|v| v.findings.iter().filter(|f| f.fails()).cloned().collect())
                .unwrap_or_default(),
            waived,
            has_verdict: verdict.is_some(),
        });
    }
    Ok(reviews)
}
fn severity(f: &Finding) -> &'static str {
    match f.severity {
        Severity::Blocker => "blocker",
        Severity::Major => "major",
        Severity::Minor => "minor",
    }
}
fn answer_for(value: &Value, number: usize) -> Option<(Status, String)> {
    let items = value.get("findings")?.as_array()?;
    let mut matched = items
        .iter()
        .filter(|v| v.get("finding").and_then(Value::as_u64) == Some(number as u64));
    let item = matched.next()?;
    if matched.next().is_some() {
        return None;
    }
    let status = match item.get("status")?.as_str()? {
        "confirmed" => Status::Confirmed,
        "rejected" => Status::Rejected,
        "inconclusive" => Status::Inconclusive,
        _ => return None,
    };
    let reason = item.get("reason")?.as_str()?.trim();
    if reason.is_empty() {
        return None;
    }
    Some((status, reason.to_owned()))
}

pub fn verify_review(codex: &AppServer, store: &Path, unit: &str, repo: &Path) -> Res<Vec<Check>> {
    let rows = orch::evidence(store, unit)?;
    let reviews = current_reviews(store, unit, &rows)?;
    let mut latest = all_checks(store)?;
    let mut result = Vec::new();
    for review in &reviews {
        let missing: Vec<_> = review
            .findings
            .iter()
            .filter(|f| {
                !latest
                    .get(&(review.id.clone(), review.row.commit.clone(), f.number))
                    .is_some_and(|c| !c.failed)
            })
            .collect();
        if !missing.is_empty() {
            let reviewer = review
                .row
                .command
                .split_once(orch::CARRIED_MARK)
                .map_or(review.row.command.as_str(), |(s, _)| s);
            let family = if reviewer.starts_with("engine-astra:") {
                Family::OpenAi
            } else {
                base::family_of(reviewer)
            };
            let mut pack = format!(
                "{}\nTree: {}\n",
                include_str!("../../engine/roles/verifier.md"),
                review.row.commit
            );
            for f in &missing {
                pack.push_str(&format!(
                    "\nFinding {}\nLocation: {}\nCheck: {}\nEvidence: {}\nFix: {}\n",
                    f.number, f.location, f.check, f.evidence, f.fix
                ));
            }
            let call = EngineCall {
                role: Role::Verifier,
                case: "finding",
                pack,
                cwd: repo,
                timeout_s: 120,
                schema: json!({"type":"object","required":["findings"],"properties":{"findings":{"type":"array","items":{"type":"object","required":["finding","status","reason"],"properties":{"finding":{"type":"integer"},"status":{"type":"string","enum":["confirmed","rejected","inconclusive"]},"reason":{"type":"string"}}}}}}),
            };
            let (reply, model) = match family {
                Family::Anthropic => {
                    let before = events::read_events(store, 0)?.last().map_or(0, |e| e.seq);
                    let answer = calls::codex_low(codex, store, &call);
                    let model = events::read_events(store, before)?
                        .into_iter()
                        .rev()
                        .find(|e| {
                            e.kind == kinds::ROLE_SPEND
                                && e.data["role"] == "verifier"
                                && e.data["call"] == "finding"
                        })
                        .and_then(|e| e.data["model"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "terra".into());
                    (answer, model)
                }
                Family::OpenAi => (calls::claude_quick(store, &call, "sonnet"), "sonnet".into()),
                _ => (
                    Err(Failure::new("unknown_family", "unknown reviewer family")),
                    String::new(),
                ),
            };
            for f in missing {
                let (status, reason, failed) = match &reply {
                    Ok(value) => match answer_for(value, f.number) {
                        Some((status, reason)) => (status, reason, false),
                        None => (
                            Status::Inconclusive,
                            "missing or invalid verifier result".into(),
                            true,
                        ),
                    },
                    Err(e) => (
                        Status::Inconclusive,
                        format!("{}: {}", e.kind, e.message),
                        true,
                    ),
                };
                let check = Check {
                    review_id: review.id.clone(),
                    unit: unit.into(),
                    tree: review.row.commit.clone(),
                    finding: f.number,
                    location: f.location.clone(),
                    severity: severity(f).into(),
                    status,
                    reason,
                    failed,
                    reviewer: reviewer.into(),
                    verifier_model: model.clone(),
                };
                events::append_event(
                    store,
                    kinds::FINDING_VERIFIED,
                    None,
                    Some(unit),
                    serde_json::to_value(&check)
                        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?,
                )?;
                latest.insert(key(&check), check);
            }
        }
        for f in &review.findings {
            if let Some(check) =
                latest.get(&(review.id.clone(), review.row.commit.clone(), f.number))
            {
                result.push(check.clone());
            }
        }
    }
    // Checked evidence has no review identifier. A successful row must therefore be safe
    // for every current review that derive_state could close with that row.
    let closed = reviews.iter().all(|r| {
        (r.row.exit_code == 0 || (r.has_verdict && !r.findings.is_empty()))
            && r.findings.iter().all(|f| {
                r.waived.contains(&f.number)
                    || latest
                        .get(&(r.id.clone(), r.row.commit.clone(), f.number))
                        .is_some_and(|c| !c.failed && c.status == Status::Rejected)
            })
    });
    for check in result
        .iter()
        .filter(|c| !c.failed && c.status == Status::Rejected)
    {
        let command = format!("{} rejected {}", check.finding, check.reason);
        let code = if closed { 0 } else { 1 };
        if !rows
            .iter()
            .rposition(|r| r.kind == "review")
            .is_some_and(|i| {
                rows[i + 1..].iter().any(|r| {
                    r.kind == orch::CHECKED_KIND
                        && r.command == command
                        && r.commit == check.tree
                        && r.exit_code == code
                })
            })
        {
            orch::append_evidence(
                store,
                unit,
                &Evidence {
                    kind: orch::CHECKED_KIND.into(),
                    command,
                    exit_code: code,
                    commit: check.tree.clone(),
                    ts: base::now_iso(),
                },
            )?;
        }
    }
    Ok(result)
}

pub fn fix_gate(codex: &AppServer, store: &Path, old_unit: &str, repo: &Path) -> Res<()> {
    let checks = verify_review(codex, store, old_unit, repo)?;
    let rows = orch::evidence(store, old_unit)?;
    let reviews = current_reviews(store, old_unit, &rows)?;
    let mut remaining = Vec::new();
    for r in reviews {
        for f in r.findings.iter().filter(|f| !r.waived.contains(&f.number)) {
            remaining.push(
                checks.iter().find(|c| {
                    c.review_id == r.id && c.tree == r.row.commit && c.finding == f.number
                }),
            );
        }
    }
    if !remaining.is_empty()
        && remaining
            .iter()
            .all(|c| c.is_some_and(|c| !c.failed && c.status == Status::Rejected))
    {
        let reasons = remaining
            .into_iter()
            .flatten()
            .map(|c| format!("{} #{}: {}", c.review_id, c.finding, c.reason.clone()))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(Failure::new("findings_rejected", reasons));
    }
    Ok(())
}
