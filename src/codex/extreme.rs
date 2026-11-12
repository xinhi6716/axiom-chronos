use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

pub const TIER: &str = "fast";
pub fn path(home: &Path) -> PathBuf {
    home.join("extreme-mode.json")
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Setting {
    pub on: bool,
    pub updated: Option<String>,
    pub last_check: Option<Check>,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TierStatus {
    Accepted,
    NotAdvertised,
    Rejected,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Check {
    pub status: TierStatus,
    pub detail: String,
    pub at: String,
    pub turn: Option<String>,
}

pub fn load(home: &Path) -> Setting {
    fs::read(path(home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn update(home: &Path, change: impl FnOnce(&mut Setting)) -> Res<Setting> {
    let io = |e: std::io::Error| Failure::new("io", e.to_string());
    fs::create_dir_all(home).map_err(io)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join("extreme-mode.lock"))
        .map_err(io)?;
    lock.lock().map_err(io)?;
    let mut setting = load(home);
    change(&mut setting);
    let bytes = serde_json::to_vec(&setting).map_err(|e| Failure::new("json", e.to_string()))?;
    base::atomic_write(&path(home), &bytes).map_err(io)?;
    Ok(setting)
}
pub fn set(home: &Path, on: bool) -> Res<Setting> {
    update(home, |s| {
        s.on = on;
        s.updated = Some(base::now_iso());
    })
}
pub fn record_check(home: &Path, check: &Check) -> Res<()> {
    update(home, |s| s.last_check = Some(check.clone())).map(|_| ())
}
pub fn classify(turn_start: &Result<Value, Failure>, stderr: &[String]) -> (TierStatus, String) {
    if let Err(e) = turn_start {
        let message = e.message.to_lowercase();
        if message.contains("service_tier")
            || message.contains("servicetier")
            || message.contains("fast")
        {
            return (TierStatus::Rejected, e.message.clone());
        }
    }
    if let Some(line) = stderr
        .iter()
        .find(|line| line.to_lowercase().contains("not advertised"))
    {
        return (TierStatus::NotAdvertised, line.clone());
    }
    (TierStatus::Accepted, "service tier requested".into())
}
pub fn report(status: TierStatus, detail: &str) -> Value {
    json!({"requested": TIER, "status": status, "applied": status == TierStatus::Accepted, "detail": detail})
}
