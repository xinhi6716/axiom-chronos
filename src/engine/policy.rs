use super::budgets::Role;
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub adjudicator: AdjudicatorPolicy,
    pub pm_qa: bool,
    pub reviewer: ReviewerMode,
    pub dual_core: bool,
    pub budgets: BTreeMap<Role, f64>,
    #[serde(default)]
    pub models: Option<Models>,
    #[serde(default)]
    pub version: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdjudicatorPolicy {
    pub enabled: bool,
    pub max_adjudications: u32,
    pub max_usd: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewerMode {
    Claude,
    AstraMax,
    Both,
}
impl Default for AdjudicatorPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_adjudications: 5,
            max_usd: 5.0,
        }
    }
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            adjudicator: AdjudicatorPolicy::default(),
            pm_qa: false,
            reviewer: ReviewerMode::Claude,
            dual_core: false,
            budgets: BTreeMap::new(),
            models: None,
            version: 0,
        }
    }
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn validate(p: &Policy) -> Res<()> {
    if !p.adjudicator.max_usd.is_finite()
        || p.adjudicator.max_usd < 0.0
        || p.budgets.values().any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err(invalid("amount must be finite and nonnegative"));
    }
    if let Some(models) = &p.models {
        models.validate()?;
    }
    Ok(())
}
pub fn load(store: &Path) -> Res<Policy> {
    let bytes = match std::fs::read(store.join("policy.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => b"{}".to_vec(),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let mut p: Policy = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    let fields: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    if fields.get("reviewer").is_none()
        && crate::codex::routing::routed_store(store).unwrap_or(false)
    {
        p.reviewer = ReviewerMode::AstraMax;
    }
    validate(&p)?;
    Ok(p)
}
pub fn save(store: &Path, p: &Policy) -> Res<()> {
    validate(p)?;
    std::fs::create_dir_all(store).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(p).map_err(|e| invalid(e.to_string()))?;
    base::atomic_write(&store.join("policy.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}
pub fn set(store: &Path, key: &str, value: &str) -> Res<Policy> {
    let mut p = load(store)?;
    let boolean = || match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(invalid("expected on or off")),
    };
    let amount = || value.parse::<f64>().map_err(|_| invalid("invalid amount"));
    match key {
        "adjudicator" => p.adjudicator.enabled = boolean()?,
        "adjudicator.max_adjudications" => {
            p.adjudicator.max_adjudications =
                value.parse().map_err(|_| invalid("invalid maximum"))?
        }
        "adjudicator.max_usd" => p.adjudicator.max_usd = amount()?,
        "pm_qa" => p.pm_qa = boolean()?,
        "dual_core" => p.dual_core = boolean()?,
        "reviewer" => {
            p.reviewer = match value {
                "claude" => ReviewerMode::Claude,
                "astra-max" => ReviewerMode::AstraMax,
                "both" => ReviewerMode::Both,
                _ => return Err(invalid("unknown reviewer")),
            }
        }
        _ if key.starts_with("models.") => {
            let models = p.models.get_or_insert_with(Models::default);
            match key {
                "models.self_check" | "models.reviewer" => {
                    let (tier, effort) = value
                        .split_once('/')
                        .ok_or_else(|| invalid("expected tier/effort"))?;
                    let stage = CodexStage {
                        tier: tier.into(),
                        effort: effort.into(),
                    };
                    validate_stage(&stage)?;
                    if key == "models.self_check" {
                        models.self_check = stage;
                    } else {
                        models.reviewer = stage;
                    }
                }
                "models.core.planning"
                | "models.core.build"
                | "models.core.escalation"
                | "models.core.close" => {
                    if !token(value) {
                        return Err(invalid("expected a single token"));
                    }
                    match key {
                        "models.core.planning" => &mut models.core.planning,
                        "models.core.build" => &mut models.core.build,
                        "models.core.escalation" => &mut models.core.escalation,
                        _ => &mut models.core.close,
                    }
                    .clone_from(&value.to_string());
                }
                _ if key.starts_with("models.worker.") => {
                    let tier = &key[14..];
                    validate_tier(tier)?;
                    if value == "none" {
                        models.worker.remove(tier);
                    } else {
                        if !token(value) {
                            return Err(invalid("expected a single token"));
                        }
                        models.worker.insert(tier.into(), value.into());
                    }
                }
                _ => return Err(invalid("unknown policy key")),
            }
            p.version = p
                .version
                .checked_add(1)
                .ok_or_else(|| invalid("version overflow"))?;
        }
        _ if key.starts_with("budget.") => {
            let role = key[7..].parse::<Role>()?;
            if value == "none" {
                p.budgets.remove(&role);
            } else {
                p.budgets.insert(role, amount()?);
            }
        }
        _ => return Err(invalid("unknown policy key")),
    }
    save(store, &p)?;
    Ok(p)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_set_and_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(load(d.path()).unwrap(), Policy::default());
        for (k, v) in [
            ("adjudicator", "on"),
            ("adjudicator.max_adjudications", "7"),
            ("adjudicator.max_usd", "2.5"),
            ("pm_qa", "on"),
            ("reviewer", "astra-max"),
            ("dual_core", "on"),
            ("budget.co_core", "1.2"),
        ] {
            set(d.path(), k, v).unwrap();
        }
        let p = load(d.path()).unwrap();
        assert!(p.adjudicator.enabled && p.pm_qa && p.dual_core);
        assert_eq!(p.adjudicator.max_adjudications, 7);
        assert_eq!(p.adjudicator.max_usd, 2.5);
        assert_eq!(p.reviewer, ReviewerMode::AstraMax);
        assert_eq!(p.budgets[&Role::CoCore], 1.2);
        set(d.path(), "budget.co_core", "none").unwrap();
        assert!(load(d.path()).unwrap().budgets.is_empty());
    }
    #[test]
    fn invalid_policy_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        for text in [
            r#"{"bad":1}"#,
            r#"{"budgets":{"bogus":1}}"#,
            r#"{"reviewer":"unknown"}"#,
            r#"{"adjudicator":{"extra":1}}"#,
            r#"{"budgets":{"core":-1}}"#,
        ] {
            std::fs::write(d.path().join("policy.json"), text).unwrap();
            assert_eq!(load(d.path()).unwrap_err().kind, "invalid_argument");
        }
        std::fs::remove_file(d.path().join("policy.json")).unwrap();
        for value in ["-1", "NaN", "inf"] {
            assert_eq!(
                set(d.path(), "budget.core", value).unwrap_err().kind,
                "invalid_argument"
            );
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexStage {
    pub tier: String,
    pub effort: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreModels {
    pub planning: String,
    pub build: String,
    pub escalation: String,
    pub close: String,
}
impl Default for CoreModels {
    fn default() -> Self {
        Self {
            planning: "opus".into(),
            build: "sonnet".into(),
            escalation: "opus".into(),
            close: "opus".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    pub worker: BTreeMap<String, String>,
    pub self_check: CodexStage,
    pub reviewer: CodexStage,
    pub core: CoreModels,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            worker: BTreeMap::new(),
            self_check: CodexStage {
                tier: "sol".into(),
                effort: "xhigh".into(),
            },
            reviewer: CodexStage {
                tier: "astra".into(),
                effort: "max".into(),
            },
            core: CoreModels::default(),
        }
    }
}
fn token(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_whitespace)
}
fn validate_tier(s: &str) -> Res<()> {
    match crate::codex::routing::parse_tier(s) {
        Ok(crate::codex::routing::Tier::Luna) => Err(invalid("luna is not allowed")),
        Ok(_) => Ok(()),
        Err(e) => Err(invalid(e)),
    }
}
fn validate_stage(s: &CodexStage) -> Res<()> {
    validate_tier(&s.tier)?;
    crate::codex::routing::parse_effort(&s.effort).map_err(invalid)?;
    Ok(())
}
impl Models {
    pub fn validate(&self) -> Res<()> {
        validate_stage(&self.self_check)?;
        validate_stage(&self.reviewer)?;
        for k in self.worker.keys() {
            validate_tier(k)?;
        }
        if [
            &self.core.planning,
            &self.core.build,
            &self.core.escalation,
            &self.core.close,
        ]
        .iter()
        .any(|v| v.is_empty())
        {
            return Err(invalid("invalid core model"));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobalModels {
    version: u64,
    models: Models,
}
fn read_global(home: &Path) -> Res<Option<GlobalModels>> {
    let bytes = match std::fs::read(home.join("settings/models.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let global: GlobalModels =
        serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    global.models.validate()?;
    Ok(Some(global))
}
pub struct EffectiveModels {
    pub models: Models,
    pub version: String,
}
pub fn effective_models(home: Option<&Path>, store: &Path) -> Res<EffectiveModels> {
    let policy = load(store)?;
    if let Some(models) = policy.models {
        return Ok(EffectiveModels {
            models,
            version: format!("policy:{}", policy.version),
        });
    }
    if let Some(home) = home {
        if let Some(global) = read_global(home)? {
            return Ok(EffectiveModels {
                models: global.models,
                version: format!("global:{}", global.version),
            });
        }
    }
    Ok(EffectiveModels {
        models: Models::default(),
        version: "default:0".into(),
    })
}
pub fn save_global_models(home: &Path, models: &Models) -> Res<u64> {
    models.validate()?;
    let version = read_global(home)?
        .map_or(0, |g| g.version)
        .checked_add(1)
        .ok_or_else(|| invalid("version overflow"))?;
    let path = home.join("settings/models.json");
    let bytes = serde_json::to_vec_pretty(&GlobalModels {
        version,
        models: models.clone(),
    })
    .map_err(|e| invalid(e.to_string()))?;
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&path, &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(version)
}
