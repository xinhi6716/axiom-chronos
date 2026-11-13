use crate::base::Res;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub fn for_attempt(
    server: &super::rpc::AppServer,
    home: Option<&Path>,
    store: &Path,
    unit: &str,
    attempt_id: &str,
    role: &str,
    brief: &crate::orch::parse::Brief,
    explicit_model: Option<&str>,
) -> Res<RoutingSnapshot> {
    use crate::base::Failure;
    if let Some(s) = read_snapshot(store, unit)? {
        if s.attempt_id == attempt_id {
            return Ok(s);
        }
    }
    let invalid = |s: String| Failure::new("invalid_argument", s);
    let fix = read_fix(store, unit)?;
    let map_line = if fix.is_none() {
        let text = std::fs::read_to_string(store.join("modules.md")).unwrap_or_default();
        let track = crate::orch::store::read_units(store)?
            .into_iter()
            .find(|u| u.id == unit)
            .map(|u| u.track);
        crate::orch::parse::parse_map(&text)
            .ok()
            .and_then(|m| {
                m.modules
                    .into_iter()
                    .find(|m| Some(&m.id) == track.as_ref())
            })
            .and_then(|m| {
                m.routing
                    .iter()
                    .find(|r| r.key == unit)
                    .or_else(|| m.routing.iter().find(|r| r.key == "default"))
                    .cloned()
            })
    } else {
        None
    };
    let (tier, effort, risk, rule, reason) = if let Some(f) = fix {
        (
            Some(f.tier),
            Some(f.effort),
            None,
            Rule::FixEscalation,
            f.reason,
        )
    } else if let Some(line) = map_line {
        let risk = serde_json::from_value(serde_json::Value::String(line.risk))
            .map_err(|e| invalid(format!("{e}")))?;
        (
            Some(parse_tier(&line.tier).map_err(invalid)?),
            Some(parse_effort(&line.effort).map_err(invalid)?),
            Some(risk),
            if line.key == unit {
                Rule::MapUnit
            } else {
                Rule::MapDefault
            },
            line.reason,
        )
    } else if explicit_model.is_some() {
        (
            None,
            brief
                .effort
                .as_deref()
                .map(parse_effort)
                .transpose()
                .map_err(invalid)?,
            None,
            Rule::ExplicitModel,
            String::new(),
        )
    } else if brief.tier.is_some() || brief.effort.is_some() {
        (
            Some(parse_tier(brief.tier.as_deref().unwrap_or(role)).map_err(invalid)?),
            brief
                .effort
                .as_deref()
                .map(parse_effort)
                .transpose()
                .map_err(invalid)?,
            None,
            Rule::Brief,
            String::new(),
        )
    } else {
        (
            Some(parse_tier(role).map_err(invalid)?),
            None,
            None,
            Rule::LegacyRole,
            String::new(),
        )
    };
    if matches!(rule, Rule::FixEscalation | Rule::MapUnit | Rule::MapDefault) {
        if explicit_model.is_some() {
            return Err(invalid("explicit model conflicts with routing".into()));
        }
        if brief
            .tier
            .as_deref()
            .is_some_and(|v| parse_tier(v).ok() != tier)
            || brief
                .effort
                .as_deref()
                .is_some_and(|v| parse_effort(v).ok() != effort)
        {
            return Err(Failure::new(
                "routing_mismatch",
                "brief conflicts with routing",
            ));
        }
    }
    let settings = crate::engine::policy::effective_models(home, store)?;
    let model = match explicit_model {
        Some(m) => m.to_owned(),
        None => {
            let name = format!("{:?}", tier.unwrap()).to_lowercase();
            match settings.models.worker.get(&name) {
                Some(m) => m.clone(),
                None => server.resolve_model(&name)?,
            }
        }
    };
    let snapshot = RoutingSnapshot {
        unit: unit.into(),
        attempt_id: attempt_id.into(),
        role: role.into(),
        tier,
        effort,
        risk,
        model,
        rule,
        reason,
        settings_version: Some(settings.version),
    };
    write_snapshot(store, &snapshot)?;
    Ok(snapshot)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Terra,
    Sol,
    Astra,
    Luna,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Trivial,
    Standard,
    StateMachine,
    Concurrency,
    Migration,
    Parser,
    Security,
    Privacy,
    CrossModule,
    PriorFailure,
    Novel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routing {
    pub tier: Tier,
    pub effort: Effort,
    pub risk: Option<Risk>,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Escalation {
    Next(Routing),
    Saturated,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    FixEscalation,
    PlanException,
    MapUnit,
    MapDefault,
    Brief,
    LegacyRole,
    ExplicitModel,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingSnapshot {
    pub unit: String,
    pub attempt_id: String,
    pub role: String,
    pub tier: Option<Tier>,
    pub effort: Option<Effort>,
    pub risk: Option<Risk>,
    pub model: String,
    pub rule: Rule,
    pub reason: String,
    pub settings_version: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixRecord {
    pub unit: String,
    pub parent: String,
    pub parent_tier: Tier,
    pub parent_effort: Option<Effort>,
    pub tier: Tier,
    pub effort: Effort,
    pub rule: Rule,
    pub reason: String,
}
pub fn parse_tier(s: &str) -> Result<Tier, String> {
    match s {
        "terra" => Ok(Tier::Terra),
        "sol" => Ok(Tier::Sol),
        "astra" => Ok(Tier::Astra),
        "luna" => Ok(Tier::Luna),
        _ => Err(format!("unknown tier: {s}")),
    }
}
pub fn parse_effort(s: &str) -> Result<Effort, String> {
    match s {
        "low" => Ok(Effort::Low),
        "medium" => Ok(Effort::Medium),
        "high" => Ok(Effort::High),
        "xhigh" => Ok(Effort::Xhigh),
        "max" => Ok(Effort::Max),
        _ => Err(format!("unknown effort: {s}")),
    }
}
pub fn check_rubric(r: &Routing) -> Result<(), String> {
    let risk = r.risk.ok_or("risk is required")?;
    if r.tier == Tier::Luna {
        return Err("luna is not valid for routing".into());
    }
    if r.tier == Tier::Astra && r.effort < Effort::High {
        return Err("astra requires at least high".into());
    }
    if r.tier == Tier::Terra && risk != Risk::Trivial {
        return Err("terra requires trivial risk".into());
    }
    if !matches!(risk, Risk::Trivial | Risk::Standard)
        && !(r.tier == Tier::Astra || r.tier == Tier::Sol && r.effort >= Effort::Xhigh)
    {
        return Err("risk requires sol/xhigh or astra".into());
    }
    Ok(())
}
pub fn escalate(tier: Tier, effort: Option<Effort>) -> Escalation {
    use Effort::*;
    use Tier::*;
    let (t, e) = match (tier, effort) {
        (Terra | Luna, _) | (Sol, None | Some(Low | Medium)) => (Sol, High),
        (Sol, Some(High)) => (Sol, Xhigh),
        (Sol, Some(Xhigh)) => (Astra, Xhigh),
        (Sol, Some(Max)) => (Astra, Max),
        (Astra, Some(Max)) => return Escalation::Saturated,
        (Astra, Some(Xhigh)) => (Astra, Max),
        (Astra, _) => (Astra, Xhigh),
    };
    Escalation::Next(Routing {
        tier: t,
        effort: e,
        risk: None,
        reason: format!("escalate {tier:?}/{effort:?} to {t:?}/{e:?}"),
    })
}
pub fn fix_record_path(store: &Path, unit: &str) -> PathBuf {
    store.join("routing").join(format!("{unit}.fix.json"))
}
pub fn snapshot_path(store: &Path, unit: &str) -> PathBuf {
    store.join("routing").join(format!("{unit}.attempt.json"))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Res<Option<T>> {
    match std::fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| crate::base::Failure::new("invalid_argument", e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(crate::base::Failure::new("io", e.to_string())),
    }
}
fn write<T: Serialize>(path: &Path, value: &T) -> Res<()> {
    let data = serde_json::to_vec_pretty(value)
        .map_err(|e| crate::base::Failure::new("invalid_argument", e.to_string()))?;
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| crate::base::Failure::new("io", e.to_string()))?;
    crate::base::atomic_write(path, &data)
        .map_err(|e| crate::base::Failure::new("io", e.to_string()))
}
pub fn read_fix(store: &Path, unit: &str) -> Res<Option<FixRecord>> {
    read(&fix_record_path(store, unit))
}
pub fn write_fix(store: &Path, record: &FixRecord) -> Res<()> {
    write(&fix_record_path(store, &record.unit), record)
}
pub fn read_snapshot(store: &Path, unit: &str) -> Res<Option<RoutingSnapshot>> {
    read(&snapshot_path(store, unit))
}
pub fn write_snapshot(store: &Path, record: &RoutingSnapshot) -> Res<()> {
    write(&snapshot_path(store, &record.unit), record)
}

pub fn routed_store(store: &Path) -> Res<bool> {
    let text = match std::fs::read_to_string(store.join("modules.md")) {
        Ok(text) => text,
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                || e.kind() == std::io::ErrorKind::InvalidData =>
        {
            return Ok(false)
        }
        Err(e) => return Err(crate::base::Failure::new("io", e.to_string())),
    };
    Ok(crate::orch::parse::parse_map(&text).is_ok_and(|map| {
        !map.modules.is_empty()
            && map
                .modules
                .iter()
                .all(|m| m.routing.iter().any(|r| r.key == "default"))
    }))
}
