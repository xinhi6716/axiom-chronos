//! Shadow evaluation, promotion, and drift rollback; routing remains unchanged.
use crate::base::{Failure, Res};
use crate::laya::{
    curate::Outcome,
    infer::Abstain,
    router::{DecisionKind, DecisionRecord},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const THRESHOLDS: &[u8] = include_bytes!("thresholds.json");
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn json(e: impl std::fmt::Display) -> Failure {
    Failure::new("json", e.to_string())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Thresholds {
    pub min_samples: u64,
    pub min_agreement: f64,
    pub max_ece: f64,
    pub max_abstain_rate: f64,
    pub max_drift: f64,
    pub ece_bins: usize,
}
pub fn load_thresholds(path: Option<&Path>) -> Res<(Thresholds, String)> {
    let bytes = match path {
        Some(p) => fs::read(p).map_err(io)?,
        None => THRESHOLDS.to_vec(),
    };
    Ok((serde_json::from_slice(&bytes).map_err(json)?, hash(&bytes)))
}
pub fn thresholds_sha256(t: &Thresholds) -> String {
    if load_thresholds(None).is_ok_and(|(committed, _)| committed == *t) {
        hash(THRESHOLDS)
    } else {
        hash(&serde_json::to_vec(t).expect("threshold serialization"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TypeMetrics {
    pub kind: DecisionKind,
    pub samples: u64,
    pub agreement: Option<f64>,
    pub verified_samples: u64,
    pub verified_accuracy: Option<f64>,
    pub ece: Option<f64>,
    pub abstain_rate: Option<f64>,
}
fn empty(kind: DecisionKind) -> TypeMetrics {
    TypeMetrics {
        kind,
        samples: 0,
        agreement: None,
        verified_samples: 0,
        verified_accuracy: None,
        ece: None,
        abstain_rate: None,
    }
}
/// ECE compares each predicted top label with its verified outcome, or with Claude's label if no outcome is available. All predictions, including abstentions, contribute.
pub fn metrics(
    records: &[DecisionRecord],
    outcomes: &[Outcome],
    bins: usize,
) -> BTreeMap<DecisionKind, TypeMetrics> {
    let mut latest = BTreeMap::<&str, &Outcome>::new();
    for o in outcomes {
        if latest
            .get(o.record_id.as_str())
            .is_none_or(|old| o.ts_ms >= old.ts_ms)
        {
            latest.insert(&o.record_id, o);
        }
    }
    DecisionKind::ALL
        .into_iter()
        .map(|kind| {
            let mut m = empty(kind);
            let mut agreed = 0u64;
            let mut verified_correct = 0u64;
            let mut abstained = 0u64;
            let mut buckets = vec![(0u64, 0.0f64, 0u64); bins.max(1)];
            for r in records.iter().filter(|r| r.kind == kind) {
                let Some(side) = &r.laya else { continue };
                let p = &side.prediction;
                let top = p.labels.get(p.top).map(String::as_str);
                let answer = matches!(p.abstain, Abstain::Answer);
                m.samples += 1;
                agreed += u64::from(answer && top == Some(r.claude.as_str()));
                abstained += u64::from(!answer);
                let outcome = latest.get(r.id.as_str());
                if let Some(o) = outcome {
                    m.verified_samples += 1;
                    verified_correct += u64::from(top == Some(o.label.as_str()));
                }
                let label = outcome.map_or(r.claude.as_str(), |o| o.label.as_str());
                let confidence = if p.confidence.is_finite() {
                    p.confidence.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let index = ((confidence * buckets.len() as f64) as usize).min(buckets.len() - 1);
                let b = &mut buckets[index];
                b.0 += 1;
                b.1 += confidence;
                b.2 += u64::from(top == Some(label));
            }
            if m.samples > 0 {
                let n = m.samples as f64;
                m.agreement = Some(agreed as f64 / n);
                m.abstain_rate = Some(abstained as f64 / n);
                m.ece = Some(
                    buckets
                        .iter()
                        .filter(|b| b.0 > 0)
                        .map(|b| (b.1 - b.2 as f64).abs() / n)
                        .sum(),
                );
            }
            if m.verified_samples > 0 {
                m.verified_accuracy = Some(verified_correct as f64 / m.verified_samples as f64);
            }
            (kind, m)
        })
        .collect()
}
/// Select a confidence cutoff using the development split only.
pub fn choose_cutoff(dev: &[(f64, bool)], max_abstain_rate: f64) -> Option<f64> {
    if dev.is_empty() {
        return None;
    }
    let mut candidates: Vec<f64> = dev.iter().map(|x| x.0).filter(|x| x.is_finite()).collect();
    candidates.sort_by(f64::total_cmp);
    candidates.dedup();
    let mut best: Option<(f64, usize, usize)> = None;
    for cutoff in candidates {
        let answered: Vec<_> = dev.iter().filter(|x| x.0 >= cutoff).collect();
        if answered.is_empty()
            || (dev.len() - answered.len()) as f64 / dev.len() as f64 > max_abstain_rate
        {
            continue;
        }
        let correct = answered.iter().filter(|x| x.1).count();
        if best.is_none_or(|(_, c, n)| {
            (correct as u128) * (n as u128) > (c as u128) * (answered.len() as u128)
        }) {
            best = Some((cutoff, correct, answered.len()));
        }
    }
    best.map(|x| x.0)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EvalReport {
    pub model_hash: String,
    pub split: String,
    pub thresholds_sha256: String,
    pub metrics: BTreeMap<DecisionKind, TypeMetrics>,
    pub ts_ms: i64,
}
pub fn save_report(home: &Path, r: &EvalReport) -> Res<PathBuf> {
    let dir = crate::laya::laya_dir(home).join("eval");
    fs::create_dir_all(&dir).map_err(io)?;
    let path = dir.join(format!("{}-{}.json", r.model_hash, r.split));
    let data = serde_json::to_vec_pretty(r).map_err(json)?;
    if r.split == "acceptance" {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    Failure::new("acceptance_already_scored", "acceptance already scored")
                } else {
                    io(e)
                }
            })?;
        f.write_all(&data).map_err(io)?;
    } else {
        crate::base::atomic_write(&path, &data).map_err(io)?;
    }
    Ok(path)
}
pub fn passes(m: &TypeMetrics, t: &Thresholds) -> Result<(), Vec<String>> {
    let mut reasons = Vec::new();
    if m.samples < t.min_samples {
        reasons.push("min_samples".into());
    }
    if !m.agreement.is_some_and(|x| x >= t.min_agreement) {
        reasons.push("min_agreement".into());
    }
    if m.verified_accuracy.is_none_or(|x| !x.is_finite()) {
        reasons.push("verified_accuracy unavailable".into());
    }
    if !m.ece.is_some_and(|x| x <= t.max_ece) {
        reasons.push("max_ece".into());
    }
    if !m.abstain_rate.is_some_and(|x| x <= t.max_abstain_rate) {
        reasons.push("max_abstain_rate".into());
    }
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(reasons)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TypeState {
    pub decider: String,
    pub model_hash: Option<String>,
    pub agreement_at_promotion: Option<f64>,
    pub history: Vec<String>,
    pub since_ms: i64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct PromotionState {
    pub types: BTreeMap<DecisionKind, TypeState>,
}
pub fn load_state(home: &Path) -> Res<PromotionState> {
    match fs::read(crate::laya::laya_dir(home).join("promotion.json")) {
        Ok(data) => serde_json::from_slice(&data).map_err(json),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PromotionState::default()),
        Err(e) => Err(io(e)),
    }
}
fn save_state(home: &Path, state: &PromotionState) -> Res<()> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    crate::base::atomic_write(
        &dir.join("promotion.json"),
        &serde_json::to_vec_pretty(state).map_err(json)?,
    )
    .map_err(io)
}
pub struct Confirmation {
    pub kind: DecisionKind,
    pub model_hash: String,
}
pub fn promote(
    home: &Path,
    repo: &Path,
    kinds: &[DecisionKind],
    acceptance: &EvalReport,
    t: &Thresholds,
    confirm: Option<&Confirmation>,
    date: &str,
) -> Res<PromotionState> {
    if kinds.len() != 1 {
        return Err(Failure::new(
            "one_type_at_a_time",
            "select exactly one type",
        ));
    }
    if acceptance.split != "acceptance" {
        return Err(Failure::new(
            "acceptance_required",
            "acceptance report required",
        ));
    }
    if acceptance.thresholds_sha256 != thresholds_sha256(t) {
        return Err(Failure::new("stale_thresholds", "thresholds differ"));
    }
    let kind = kinds[0];
    let metric = acceptance
        .metrics
        .get(&kind)
        .filter(|m| m.kind == kind)
        .filter(|m| passes(m, t).is_ok())
        .ok_or_else(|| Failure::new("below_threshold", "metrics do not pass thresholds"))?;
    if !confirm.is_some_and(|c| c.kind == kind && c.model_hash == acceptance.model_hash) {
        return Err(Failure::new(
            "confirmation_required",
            "matching confirmation required",
        ));
    }
    let mut info = crate::evolve::repo::check(repo, date)?;
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    let _lock = crate::orch::store::lock_store(&dir)?;
    let mut state = load_state(home)?;
    info.branch = format!("laya/{}-{date}", kind.as_str());
    crate::evolve::repo::create_branch(&info)?;
    let old = state.types.remove(&kind);
    let mut history = old.as_ref().map_or_else(Vec::new, |s| s.history.clone());
    if let Some(hash) = old.and_then(|s| s.model_hash) {
        history.push(hash);
    }
    state.types.insert(
        kind,
        TypeState {
            decider: "laya".into(),
            model_hash: Some(acceptance.model_hash.clone()),
            agreement_at_promotion: metric.agreement,
            history,
            since_ms: now_ms(),
        },
    );
    save_state(home, &state)?;
    Ok(state)
}
pub fn check_drift(
    home: &Path,
    kind: DecisionKind,
    recent: &TypeMetrics,
    t: &Thresholds,
) -> Res<Option<TypeState>> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    let _lock = crate::orch::store::lock_store(&dir)?;
    let mut state = load_state(home)?;
    let Some(s) = state.types.get_mut(&kind) else {
        return Ok(None);
    };
    let drift = s
        .agreement_at_promotion
        .zip(recent.agreement)
        .is_some_and(|(base, current)| base - current > t.max_drift)
        || recent.ece.is_some_and(|ece| ece > t.max_ece);
    if s.decider != "laya" || !drift {
        return Ok(None);
    }
    s.model_hash = s.history.pop();
    if s.model_hash.is_none() {
        s.decider = "claude".into();
        s.agreement_at_promotion = None;
    }
    s.since_ms = now_ms();
    let result = s.clone();
    save_state(home, &state)?;
    Ok(Some(result))
}
