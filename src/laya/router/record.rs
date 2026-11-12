use crate::base::{Failure, Res};
use crate::laya::infer::Prediction;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Interjection,
    Needs,
    Triage,
    Routing,
}
impl DecisionKind {
    pub const ALL: [Self; 4] = [Self::Interjection, Self::Needs, Self::Triage, Self::Routing];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interjection => "interjection",
            Self::Needs => "needs",
            Self::Triage => "triage",
            Self::Routing => "routing",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Interjection => &["amend", "answer", "stop"],
            Self::Needs => &["smoke", "manual", "none"],
            Self::Triage => &["blocker", "major", "minor"],
            Self::Routing => &["terra", "sol", "astra"],
        }
    }
    pub fn instructions(self) -> &'static str {
        match self {
            Self::Interjection => "Classify the user's interjection.",
            Self::Needs => "Choose the verification needed for this finding.",
            Self::Triage => "Choose the severity of this finding.",
            Self::Routing => "Choose the role for this task.",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub kind: DecisionKind,
    pub wish_id: Option<String>,
    pub subject: String,
    pub context: String,
    pub claude: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LayaSide {
    pub model_version: String,
    pub prediction: Prediction,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DecisionRecord {
    pub schema: u32,
    pub id: String,
    pub ts_ms: i64,
    pub kind: DecisionKind,
    pub wish_id: Option<String>,
    pub subject: String,
    pub context: String,
    pub claude: String,
    pub laya: Option<LayaSide>,
    pub laya_error: Option<String>,
    pub policy_version: String,
    pub mode: String,
    pub executed: String,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Off,
    Shadow,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct RouterConfig {
    pub mode: Mode,
    pub model_dir: Option<PathBuf>,
    pub calibration: Option<PathBuf>,
    pub runtime_lib: Option<PathBuf>,
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
pub fn load_config(home: &Path) -> Res<RouterConfig> {
    match fs::read(crate::laya::laya_dir(home).join("router.json")) {
        Ok(data) => {
            serde_json::from_slice(&data).map_err(|e| Failure::new("invalid_json", e.to_string()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(RouterConfig::default()),
        Err(e) => Err(io(e)),
    }
}
pub fn save_config(home: &Path, c: &RouterConfig) -> Res<()> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    let data = serde_json::to_vec_pretty(c).map_err(|e| Failure::new("json", e.to_string()))?;
    crate::base::atomic_write(&dir.join("router.json"), &data).map_err(io)
}
pub fn kill(home: &Path) -> Res<()> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    fs::write(dir.join("KILL"), b"").map_err(io)
}
pub fn unkill(home: &Path) -> Res<()> {
    match fs::remove_file(crate::laya::laya_dir(home).join("KILL")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}
pub fn killed(home: &Path) -> bool {
    std::env::var_os("AXIOM_LAYA_KILL").is_some_and(|v| v == "1")
        || crate::laya::laya_dir(home).join("KILL").exists()
}
pub fn append_record(home: &Path, r: &DecisionRecord) -> Res<()> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io)?;
    let _lock = crate::orch::store::lock_store(&dir)?;
    let mut bytes = serde_json::to_vec(r).map_err(|e| Failure::new("json", e.to_string()))?;
    bytes.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("decisions.jsonl"))
        .map_err(io)?
        .write_all(&bytes)
        .map_err(io)
}
pub fn read_records(home: &Path) -> Res<(Vec<DecisionRecord>, u64)> {
    let file = match File::open(crate::laya::laya_dir(home).join("decisions.jsonl")) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((vec![], 0)),
        Err(e) => return Err(io(e)),
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut records = Vec::new();
    let mut skipped = 0;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(io)? == 0 {
            break;
        }
        match serde_json::from_slice(&line) {
            Ok(r) => records.push(r),
            Err(_) => skipped += 1,
        }
    }
    Ok((records, skipped))
}
