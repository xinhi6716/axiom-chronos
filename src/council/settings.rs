use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const ARGON_EVERY_ROUND_ENV: &str = "AXIOM_COUNCIL_ARGON_EVERY_ROUND";

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct CouncilSettings {
    #[serde(default = "yes")]
    pub argon_every_round: bool,
}

impl Default for CouncilSettings {
    fn default() -> Self {
        Self {
            argon_every_round: true,
        }
    }
}

pub fn load(home: &Path) -> Res<CouncilSettings> {
    let path = home.join("settings").join("council.json");
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| Failure::new("invalid_argument", e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CouncilSettings::default()),
        Err(e) => Err(Failure::new("io", e.to_string())),
    }
}

pub fn save(home: &Path, s: &CouncilSettings) -> Res<()> {
    let dir = home.join("settings");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&dir.join("council.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}

pub fn parse_argon_every_round(
    value: Option<String>,
) -> Result<Option<bool>, super::budgets::BudgetError> {
    match value.as_deref() {
        None => Ok(None),
        Some("0") => Ok(Some(false)),
        Some("1") => Ok(Some(true)),
        _ => Err(super::budgets::BudgetError {
            variable: ARGON_EVERY_ROUND_ENV,
            value: value.unwrap(),
            reason: "must be 0 or 1",
        }),
    }
}
