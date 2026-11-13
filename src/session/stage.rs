use crate::{base::{Failure, Res}, engine::{kinds, policy}, goal, orch};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs::{self, OpenOptions}, io::{BufRead, BufReader, Write}, path::{Path, PathBuf}, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage { Planning, Build, Escalation, Close }
impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Build => "build",
            Self::Escalation => "escalation",
            Self::Close => "close",
        }
    }
}
pub fn current(home: &Path, session: &str) -> Stage {
    let Ok(Some(goal)) = goal::load(home, session) else { return Stage::Planning; };
    if goal.status == "done" { return Stage::Close; }
    let Ok(store) = orch::store_for_goal(home, &goal) else { return Stage::Planning; };
    if !store.is_dir() { return Stage::Planning; }
    if let Ok(units) = orch::read_units(&store) {
        let active: Vec<_> = units.iter().filter(|u| !u.state.starts_with("superseded")).collect();
        if !active.is_empty() && active.iter().all(|u| u.state == "verified") { return Stage::Close; }
    }
    // Reuse the store parser, including its validation of the entire gates file.
    let gates = orch::store::run(&store, &["gate".into(), "list".into()], true, false);
    let open_gate = gates.exit_code == 0 && serde_json::from_str::<Vec<serde_json::Value>>(&gates.stdout)
        .is_ok_and(|rows| !rows.is_empty());
    let pending = fs::read(store.join("adjudication.pending")).is_ok_and(|bytes| !bytes.is_empty());
    if open_gate || pending { return Stage::Escalation; }
    if store.join("astra/plan-final.md").exists() && store.join("modules.md").exists() {
        Stage::Build
    } else { Stage::Planning }
}
pub fn model_for(home: &Path, session: &str, stage: Stage) -> Res<String> {
    let store = goal::load(home, session).ok().flatten()
        .and_then(|g| orch::store_for_goal(home, &g).ok()).filter(|p| p.is_dir());
    let core = if let Some(store) = store {
        policy::effective_models(Some(home), &store)?.models.core
    } else {
        // No store exists to load policy from; only global settings can override defaults.
        #[derive(Deserialize)]
        struct Global { models: policy::Models }
        match fs::read(home.join("settings/models.json")) {
            Ok(bytes) => {
                let global: Global = serde_json::from_slice(&bytes).map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                global.models.validate()?;
                global.models.core
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => policy::CoreModels::default(),
            Err(e) => return Err(io(e)),
        }
    };
    Ok(match stage {
        Stage::Planning => core.planning,
        Stage::Build => core.build,
        Stage::Escalation => core.escalation,
        Stage::Close => core.close,
    })
}
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
pub fn set_model_request(request_id: &str, model: &str) -> String {
    format!("{}\n", serde_json::json!({"type":"control_request","request_id":request_id,
        "request":{"subtype":"set_model","model":model}}))
}
pub fn parse_control_response(event: &serde_json::Value, request_id: &str) -> Option<Result<(), String>> {
    if event["type"].as_str() != Some("control_response")
        || event["response"]["request_id"].as_str() != Some(request_id) { return None; }
    let response = &event["response"];
    Some(if response["subtype"].as_str() == Some("success") { Ok(()) }
        else { Err(response["error"].as_str().unwrap_or("control error").to_owned()) })
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via { Initial, Control, Resume }
#[derive(Debug, Serialize, Deserialize)]
pub struct TurnRecord {
    pub turn: u64,
    pub stage: String,
    pub model: String,
    pub via: Via,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_error: Option<String>,
}
fn io(e: std::io::Error) -> Failure { Failure::new("io", e.to_string()) }
fn turns_path(home: &Path, session: &str) -> Res<PathBuf> {
    if session.is_empty() || session.len() > 64
        || !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(Failure::new("invalid_argument", "not a session id"));
    }
    Ok(home.join("goals").join(format!("{session}.turns.jsonl")))
}
pub fn log_turn(home: &Path, session: &str, rec: &TurnRecord) -> Res<()> {
    let path = turns_path(home, session)?;
    fs::create_dir_all(home.join("goals")).map_err(io)?;
    let mut file = OpenOptions::new().create(true).read(true).append(true).open(path).map_err(io)?;
    // Serialize before writing, and lock across the full append so calls cannot interleave.
    let mut bytes = serde_json::to_vec(rec).map_err(|e| Failure::new("invalid", e.to_string()))?;
    bytes.push(b'\n');
    file.lock().map_err(io)?;
    file.write_all(&bytes).map_err(io)
}
pub fn read_turns(home: &Path, session: &str) -> Res<Vec<TurnRecord>> {
    let file = match fs::File::open(turns_path(home, session)?) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(io(e)),
    };
    BufReader::new(file).lines().enumerate().map(|(i, line)| {
        let line = line.map_err(io)?;
        serde_json::from_str(&line).map_err(|e| Failure::new("invalid", format!("turns line {}: {e}", i + 1)))
    }).collect()
}
pub struct TraceRow { pub model: String, pub role: String, pub calls: u64 }
pub fn trace(home: &Path, session: &str) -> Res<Vec<TraceRow>> {
    let mut counts = BTreeMap::<(String, String), u64>::new();
    for rec in read_turns(home, session)? {
        if rec.ok { *counts.entry((rec.model, "core".into())).or_default() += 1; }
    }
    if let Some(goal) = goal::load(home, session)? {
        if let Ok(store) = orch::store_for_goal(home, &goal) {
            for event in orch::events::read_events(&store, 0)? {
                if event.kind != kinds::ROLE_SPEND { continue; }
                let (Some(role), Some(model)) = (event.data["role"].as_str(), event.data["model"].as_str()) else { continue; };
                if role != "core" { *counts.entry((model.into(), role.into())).or_default() += 1; }
            }
        }
    }
    Ok(counts.into_iter().map(|((model, role), calls)| TraceRow { model, role, calls }).collect())
}
