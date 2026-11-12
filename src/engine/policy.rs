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
    Ok(())
}
pub fn load(store: &Path) -> Res<Policy> {
    let bytes = match std::fs::read(store.join("policy.json")) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Policy::default()),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let p: Policy = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
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
