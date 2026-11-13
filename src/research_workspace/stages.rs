use super::{
    index::{ArtifactStatus, Index},
    manifest::{Format, GateOp, Role, StageDecl},
    read::{self, Limits},
    repro::{self, ReproResult, ReproStatus},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Retry {
    At(String),
    Unknown,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum StageState {
    NotStarted,
    Running,
    Paused { reason: String },
    Blocked { reason: String },
    RateLimited { retry: Retry },
    Stalled { since: String },
    Done,
    Failed { reason: String },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum GateResult {
    Passed,
    Failed { detail: String },
    NotRun { reason: GateNotRun },
    NotDeclared,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum GateNotRun {
    ArtifactUnreadable { artifact: String },
    MissingSetting { pointer: String },
    MissingThreshold,
    MissingPointer { pointer: String },
    NonNumeric,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum StaleCause {
    InputChanged {
        input: String,
        recorded: String,
        current: String,
    },
    CodeChanged {
        recorded: String,
        current: String,
    },
    PromptChanged {
        recorded: String,
        current: String,
    },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Staleness {
    pub artifact: String,
    pub cause: StaleCause,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub failed: u64,
    pub pending: u64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StageView {
    pub id: String,
    pub label: String,
    pub state: StageState,
    pub progress: Option<Progress>,
    pub gate: GateResult,
    pub stale: Vec<Staleness>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ResearchStatus {
    Passed,
    InProgress,
    Paused { reason: String },
    Blocked { reason: String },
    RateLimited { retry: Retry },
    Stalled,
    Stale,
    NotStarted,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Engineering {
    pub verified: usize,
    pub superseded: usize,
    pub other: usize,
    pub total: usize,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ResearchMap {
    pub status: ResearchStatus,
    pub stages: Vec<StageView>,
    pub stale: Vec<Staleness>,
    pub engineering: Option<Engineering>,
    pub repro: Option<ReproResult>,
}

pub fn staleness(ix: &Index) -> Vec<Staleness> {
    let l = ix.located();
    let lim = Limits::default();
    let mut out = Vec::new();
    for a in &l.manifest.artifacts {
        let Some(p) = &a.produced else { continue };
        for pin in &p.inputs {
            let recorded = pin.sha256.clone().or_else(|| {
                let source = pin.sha256_from.as_deref()?;
                if let Some((id, pointer)) = source.split_once('#') {
                    read::read_json(l, id, &lim)
                        .ok()?
                        .pointer(pointer)?
                        .as_str()
                        .map(str::to_owned)
                } else {
                    read::read_text(l, source, &lim)
                        .ok()
                        .map(|s| s.trim().to_owned())
                }
            });
            let current = l
                .artifact(&pin.artifact)
                .and_then(|a| l.artifact_path(a).ok())
                .and_then(|p| read::hash_file(&p).ok());
            if let (Some(recorded), Some(current)) = (recorded, current) {
                if !recorded.eq_ignore_ascii_case(&current) {
                    out.push(Staleness {
                        artifact: a.id.clone(),
                        cause: StaleCause::InputChanged {
                            input: pin.artifact.clone(),
                            recorded,
                            current,
                        },
                    });
                }
            }
        }
        if let (Some(recorded), Some(current)) = (&p.code_version, &l.manifest.versions.code) {
            if recorded != current {
                out.push(Staleness {
                    artifact: a.id.clone(),
                    cause: StaleCause::CodeChanged {
                        recorded: recorded.clone(),
                        current: current.clone(),
                    },
                });
            }
        }
        // Produced metadata does not name a prompt, so only a single declared prompt is unambiguous.
        if l.manifest.versions.prompts.len() == 1 {
            if let (Some(recorded), Some(current)) = (
                &p.prompt_version,
                l.manifest.versions.prompts.values().next(),
            ) {
                if recorded != current {
                    out.push(Staleness {
                        artifact: a.id.clone(),
                        cause: StaleCause::PromptChanged {
                            recorded: recorded.clone(),
                            current: current.clone(),
                        },
                    });
                }
            }
        }
    }
    out
}
fn gate(ix: &Index, stage: &StageDecl) -> GateResult {
    let Some(g) = &stage.gate else {
        return GateResult::NotDeclared;
    };
    let l = ix.located();
    let threshold = if let Some(s) = &g.setting {
        match l.manifest.settings.pointer(s) {
            Some(v) => v,
            None => {
                return GateResult::NotRun {
                    reason: GateNotRun::MissingSetting { pointer: s.clone() },
                }
            }
        }
    } else {
        match &g.value {
            Some(v) => v,
            None => {
                return GateResult::NotRun {
                    reason: GateNotRun::MissingThreshold,
                }
            }
        }
    };
    let Ok(value) = read::read_json(l, &g.artifact, &Limits::default()) else {
        return GateResult::NotRun {
            reason: GateNotRun::ArtifactUnreadable {
                artifact: g.artifact.clone(),
            },
        };
    };
    let Some(actual) = value.pointer(&g.pointer) else {
        return GateResult::NotRun {
            reason: GateNotRun::MissingPointer {
                pointer: g.pointer.clone(),
            },
        };
    };
    let passed = match g.op {
        GateOp::Eq => match (actual.as_f64(), threshold.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => actual == threshold,
        },
        GateOp::Ge => match (actual.as_f64(), threshold.as_f64()) {
            (Some(a), Some(b)) => a >= b,
            _ => {
                return GateResult::NotRun {
                    reason: GateNotRun::NonNumeric,
                }
            }
        },
        GateOp::Le => match (actual.as_f64(), threshold.as_f64()) {
            (Some(a), Some(b)) => a <= b,
            _ => {
                return GateResult::NotRun {
                    reason: GateNotRun::NonNumeric,
                }
            }
        },
    };
    if passed {
        GateResult::Passed
    } else {
        GateResult::Failed {
            detail: format!("{} {:?} {} failed", actual, g.op, threshold),
        }
    }
}
fn ready(ix: &Index, id: &str) -> bool {
    let l = ix.located();
    let Some(a) = l.artifact(id) else {
        return false;
    };
    match a.format {
        Format::Csv | Format::Jsonl => matches!(ix.status(id), ArtifactStatus::Ready(_)),
        Format::Json => read::read_json(l, id, &Limits::default()).is_ok(),
        Format::Text => read::read_text(l, id, &Limits::default()).is_ok(),
    }
}
// RFC 3339 dates, including fractional seconds and numeric UTC offsets.
fn timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20
        || b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || !matches!(b.get(10), Some(b'T' | b't'))
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
    {
        return None;
    }
    let n = |a, b| s.get(a..b)?.parse::<i64>().ok();
    let (year, month, day, hour, minute, second) = (
        n(0, 4)?,
        n(5, 7)?,
        n(8, 10)?,
        n(11, 13)?,
        n(14, 16)?,
        n(17, 19)?,
    );
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut pos = 19;
    let mut fraction = 0;
    if b.get(pos) == Some(&b'.') {
        pos += 1;
        let start = pos;
        while b.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
        if pos == start {
            return None;
        }
        let digits = s.get(start..pos)?;
        fraction = digits
            .chars()
            .chain("00".chars())
            .take(3)
            .collect::<String>()
            .parse::<i64>()
            .ok()?;
    }
    let offset = match b.get(pos) {
        Some(b'Z' | b'z') if pos + 1 == b.len() => 0,
        Some(sign @ (b'+' | b'-')) if pos + 6 == b.len() && b.get(pos + 3) == Some(&b':') => {
            let h = n(pos + 1, pos + 3)?;
            let m = n(pos + 4, pos + 6)?;
            if h > 23 || m > 59 {
                return None;
            }
            (h * 60 + m) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Some(
        days * 86400000 + hour * 3600000 + minute * 60000 + second * 1000 + fraction
            - offset * 60000,
    )
}
fn engineering(ix: &Index) -> Option<Engineering> {
    let path = ix.located().store.as_ref()?.join("units.tsv");
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let headers: Vec<_> = lines
        .next()
        .unwrap_or("")
        .trim_end_matches('\r')
        .split('\t')
        .collect();
    let state = headers.iter().position(|x| *x == "state");
    let verified = headers.iter().position(|x| *x == "verified");
    let mut e = Engineering {
        verified: 0,
        superseded: 0,
        other: 0,
        total: 0,
    };
    for line in lines.filter(|x| !x.trim().is_empty()) {
        let fields: Vec<_> = line.trim_end_matches('\r').split('\t').collect();
        let v = state
            .or(verified)
            .and_then(|i| fields.get(i))
            .copied()
            .unwrap_or("");
        e.total += 1;
        if (state.is_some() && v == "verified") || (state.is_none() && v == "1") {
            e.verified += 1
        } else if state.is_some() && v.starts_with("superseded:") {
            e.superseded += 1
        } else {
            e.other += 1
        }
    }
    Some(e)
}
fn retry(v: Option<&str>) -> Retry {
    match v {
        Some(s) if !s.is_empty() && s != "unknown" => Retry::At(s.into()),
        _ => Retry::Unknown,
    }
}
fn view(ix: &Index, stage: &StageDecl, stale: &[Staleness], now: i64) -> StageView {
    let l = ix.located();
    let lim = Limits::default();
    let raw = stage
        .progress
        .as_deref()
        .and_then(|id| read::read_json(l, id, &lim).ok());
    let progress = raw.as_ref().and_then(|v| {
        Some(Progress {
            done: v.get("done")?.as_u64()?,
            total: v.get("total")?.as_u64()?,
            failed: v.get("failed")?.as_u64()?,
            pending: v.get("pending")?.as_u64()?,
        })
    });
    let driver = l
        .by_role(Role::Driver)
        .into_iter()
        .filter_map(|a| read::read_json(l, &a.id, &lim).ok())
        .find(|v| v.get("stage").and_then(Value::as_str) == Some(stage.id.as_str()));
    let mut log_rate = None;
    for a in l.by_role(Role::Completed) {
        let mut later_done = BTreeSet::new();
        for i in (0..ix.len(&a.id)).rev() {
            if let Some(row) = ix.row(&a.id, i as u32) {
                if row.get("stage") != stage.id {
                    continue;
                }
                let sha = row.get("sha256");
                if row.get("status") == "done" && !sha.is_empty() {
                    later_done.insert(sha.to_owned());
                } else if row.get("status") == "rate_limited"
                    && (sha.is_empty() || !later_done.contains(sha))
                    && log_rate.is_none()
                {
                    log_rate = Some(row.get("retry_after").to_owned());
                }
            }
        }
    }
    let rate = raw
        .as_ref()
        .and_then(|v| v.get("rate_limited_until"))
        .and_then(Value::as_str);
    let driver_rate = driver
        .as_ref()
        .filter(|d| matches!(d.get("state").and_then(Value::as_str), Some("rate_limited")));
    let complete = progress
        .as_ref()
        .is_some_and(|p| p.done == p.total && p.failed == 0);
    let log_rate = if complete { None } else { log_rate };
    let has_rate = log_rate.is_some() || driver_rate.is_some();
    let gate = gate(ix, stage);
    let state = if let Some(d) = driver.as_ref().filter(|d| {
        matches!(
            d.get("state").and_then(Value::as_str),
            Some("paused" | "blocked")
        )
    }) {
        let reason = d
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if d.get("state").and_then(Value::as_str) == Some("paused") {
            StageState::Paused { reason }
        } else {
            StageState::Blocked { reason }
        }
    } else if has_rate {
        StageState::RateLimited {
            retry: retry(
                rate.filter(|s| !s.is_empty() && *s != "unknown")
                    .or_else(|| {
                        driver_rate
                            .and_then(|d| d.get("retry_at"))
                            .and_then(Value::as_str)
                    })
                    .or(log_rate.as_deref()),
            ),
        }
    } else if let Some(p) = &progress {
        if p.done == p.total && p.failed == 0 {
            StageState::Done
        } else if let Some(updated) = raw
            .as_ref()
            .and_then(|v| v.get("updated"))
            .and_then(Value::as_str)
            .filter(|s| {
                timestamp(s).is_some_and(|t| {
                    i128::from(now) - i128::from(t)
                        > i128::from(l.manifest.stall_after_s.unwrap_or(1800)) * 1000
                })
            })
        {
            StageState::Stalled {
                since: updated.into(),
            }
        } else if p.failed > 0 {
            StageState::Failed {
                reason: format!("{} failed", p.failed),
            }
        } else if p.pending > 0 || p.done > 0 {
            StageState::Running
        } else {
            StageState::NotStarted
        }
    } else if stage.progress.is_some() {
        StageState::NotStarted
    } else if !stage.outputs.is_empty() && stage.outputs.iter().all(|id| ready(ix, id)) {
        StageState::Done
    } else {
        StageState::NotStarted
    };
    let state = if let GateResult::Failed { detail } = &gate {
        if matches!(
            state,
            StageState::Done | StageState::Running | StageState::NotStarted
        ) {
            StageState::Failed {
                reason: detail.clone(),
            }
        } else {
            state
        }
    } else {
        state
    };
    StageView {
        id: stage.id.clone(),
        label: stage.label.clone(),
        state,
        progress,
        gate,
        stale: stale
            .iter()
            .filter(|s| stage.outputs.contains(&s.artifact))
            .cloned()
            .collect(),
    }
}
pub fn blocker(m: &ResearchMap) -> Option<(String, StageState)> {
    m.stages
        .iter()
        .find(|s| !matches!(s.state, StageState::Done | StageState::Running))
        .map(|s| (s.id.clone(), s.state.clone()))
}
pub fn research_map(ix: &Index, now_ms: i64) -> ResearchMap {
    let stale = staleness(ix);
    let stages: Vec<_> = ix
        .located()
        .manifest
        .stages
        .iter()
        .map(|s| view(ix, s, &stale, now_ms))
        .collect();
    let repro = ix
        .located()
        .manifest
        .groups
        .as_ref()
        .map(|_| repro::guard(ix));
    let priority = stages.iter().find_map(|s| match &s.state {
        StageState::Paused { reason } => Some(ResearchStatus::Paused {
            reason: reason.clone(),
        }),
        StageState::Blocked { reason } => Some(ResearchStatus::Blocked {
            reason: reason.clone(),
        }),
        StageState::RateLimited { retry } => Some(ResearchStatus::RateLimited {
            retry: retry.clone(),
        }),
        StageState::Stalled { .. } => Some(ResearchStatus::Stalled),
        _ => None,
    });
    let status = if let Some(p) = priority {
        p
    } else if !stale.is_empty() {
        ResearchStatus::Stale
    } else if stages.iter().any(|s| {
        matches!(s.state, StageState::Running | StageState::Failed { .. })
            || matches!(s.gate, GateResult::Failed { .. })
    }) || repro
        .as_ref()
        .is_some_and(|r| matches!(r.status, ReproStatus::Fail { .. }))
    {
        ResearchStatus::InProgress
    } else if stages.is_empty()
        || stages.iter().any(|s| {
            !matches!(s.state, StageState::Done) || matches!(s.gate, GateResult::NotRun { .. })
        })
        || repro
            .as_ref()
            .is_some_and(|r| !matches!(r.status, ReproStatus::Pass))
    {
        ResearchStatus::NotStarted
    } else {
        ResearchStatus::Passed
    };
    ResearchMap {
        status,
        stages,
        stale,
        engineering: engineering(ix),
        repro,
    }
}
