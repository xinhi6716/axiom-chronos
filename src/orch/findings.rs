use super::{standing_unit, Evidence, Unit, WAIVE_KIND};
use crate::base::{now_iso, Failure, Res};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Blocker,
    Major,
    Minor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needs {
    Smoke,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub number: usize,
    pub check: String,
    pub severity: Severity,
    pub location: String,
    pub evidence: String,
    pub fix: String,
    pub needs: Option<Needs>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub unit: String,
    pub tree: String,
    pub verdict: String,
    pub findings: Vec<Finding>,
}

pub fn parse_verdict(text: &str) -> Res<Verdict> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    let required = |name: &str| -> Res<String> {
        value
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Failure::new("invalid_argument", format!("missing {name}")))
    };
    let verdict = required("verdict")?;
    let items = value
        .get("findings")
        .and_then(Value::as_array)
        .ok_or_else(|| Failure::new("invalid_argument", "missing findings"))?;
    let unit = required("unit")?;
    let tree = required("tree")?;
    let findings = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let number = index + 1;
            let string = |key: &str| {
                item.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            };
            let severity = match string("severity").to_ascii_lowercase().as_str() {
                "blocker" => Severity::Blocker,
                "major" => Severity::Major,
                _ => Severity::Minor,
            };
            let needs = match item.get("needs") {
                None => None,
                Some(Value::String(s)) if s == "smoke" => Some(Needs::Smoke),
                Some(Value::String(s)) if s == "manual" => Some(Needs::Manual),
                Some(other) => {
                    return Err(Failure::new(
                        "invalid_argument",
                        format!("finding {number}: invalid needs {other}"),
                    ))
                }
            };
            Ok(Finding {
                number,
                check: string("check"),
                severity,
                location: string("location"),
                evidence: string("evidence"),
                fix: string("fix"),
                needs,
            })
        })
        .collect::<Res<Vec<_>>>()?;
    Ok(Verdict {
        unit,
        tree,
        verdict,
        findings,
    })
}

pub fn verdict_path(store: &Path, unit: &str) -> PathBuf {
    store.join("reports").join(format!("{unit}.verdict.json"))
}

pub fn read_verdict(store: &Path, unit: &str) -> Res<Option<Verdict>> {
    match std::fs::read_to_string(verdict_path(store, unit)) {
        Ok(text) => parse_verdict(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}

impl Finding {
    pub fn is_serious(&self) -> bool {
        matches!(self.severity, Severity::Blocker | Severity::Major)
    }
    pub fn needs_check(&self) -> bool {
        self.check == "untested" && self.needs.is_some()
    }
    pub fn fails(&self) -> bool {
        self.is_serious() && !self.needs_check()
    }
}

pub fn passes(v: &Verdict, waived: &BTreeSet<usize>) -> bool {
    v.findings
        .iter()
        .all(|f| !f.fails() || waived.contains(&f.number))
        && (v.verdict.eq_ignore_ascii_case("pass") || v.findings.iter().any(Finding::is_serious))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiver {
    pub number: usize,
    pub reason: String,
    pub tree: String,
    pub closes: bool,
}

pub fn waivers(rows: &[Evidence]) -> Vec<Waiver> {
    let Some(index) = rows.iter().rposition(|row| row.kind == "review") else {
        return Vec::new();
    };
    let review = &rows[index];
    rows[index + 1..]
        .iter()
        .filter(|row| row.kind == WAIVE_KIND && row.commit == review.commit)
        .filter_map(|row| {
            let (number, reason) = row.command.split_once(' ')?;
            if number.is_empty()
                || !number.bytes().all(|c| c.is_ascii_digit())
                || reason.trim().is_empty()
            {
                return None;
            }
            let number = number.parse().ok().filter(|&n| n > 0)?;
            Some(Waiver {
                number,
                reason: reason.to_owned(),
                tree: row.commit.clone(),
                closes: row.exit_code == 0,
            })
        })
        .collect()
}

pub fn waive_row(number: usize, reason: &str, tree: &str, closes: bool) -> Evidence {
    Evidence {
        kind: WAIVE_KIND.into(),
        command: format!("{number} {reason}"),
        exit_code: if closes { 0 } else { 1 },
        commit: tree.into(),
        ts: now_iso(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    Open,
    Answered,
    NeedsCheck,
    Ruled(String),
    Resolved,
    Minor,
}

pub fn classify(units: &[Unit], unit: &Unit, f: &Finding, waivers: &[Waiver]) -> Class {
    if let Some(waiver) = waivers.iter().find(|w| w.number == f.number) {
        return Class::Ruled(waiver.reason.clone());
    }
    if f.needs_check() {
        return Class::NeedsCheck;
    }
    if !f.is_serious() {
        return Class::Minor;
    }
    match standing_unit(units, unit) {
        Some(standing) if standing.state == "verified" && standing.id == unit.id => Class::Resolved,
        Some(standing) if standing.state == "verified" => Class::Answered,
        _ => Class::Open,
    }
}
