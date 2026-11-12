use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::base::{Failure, Res};
use crate::graph::{code, constraints, feedback, query, GraphStore};

pub const LAUNCH_BUDGET: usize = 2400;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LaunchPacket {
    pub text: String,
    pub bytes: usize,
    pub budget: usize,
    pub revision: i64,
    pub graph_state: String,
    pub reason: Option<String>,
    pub sources: Vec<String>,
    pub omitted: usize,
    pub wish_sha256: String,
}

fn io(error: impl std::fmt::Display) -> Failure {
    Failure::new("io", error.to_string())
}

fn truncate(text: &mut String, budget: usize) {
    if text.len() > budget {
        let mut end = budget;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

pub fn build(
    home: &Path,
    project: &str,
    project_dir: &Path,
    wish: &str,
    budget: usize,
    sync_deadline: Duration,
) -> Res<LaunchPacket> {
    let project_store = home.join("orchestrate").join(project);
    let block = constraints::load(
        project_dir,
        project_store.is_dir().then_some(project_store.as_path()),
        constraints::DEFAULT_BUDGET,
    )?;
    let mut sources: Vec<String> = block.sources.into_iter().map(|(path, _)| path).collect();
    let mut revision = 0;
    let mut error = None;
    let mut sync_reason = None;
    let mut payload = Value::Null;
    let mut health = None;
    match GraphStore::open(home) {
        Err(e) => error = Some(e.message),
        Ok(store) => {
            let options = code::SyncOptions {
                deadline: Instant::now().checked_add(sync_deadline),
                ..code::SyncOptions::default()
            };
            match code::sync(&store, project_dir, None, &options) {
                Ok(report) => sync_reason = report.partial_reason,
                Err(e) => error = Some(e.message),
            }
            if let Err(e) = feedback::ingest(&store, home, project, project_dir) {
                if error.is_none() {
                    error = Some(e.message);
                }
            }
            match store.revision() {
                Ok(r) => revision = r,
                Err(e) if error.is_none() => error = Some(e.message),
                Err(_) => {}
            }
            let request: query::ExploreRequest = serde_json::from_value(json!({
                "project_dir": project_dir, "project": project, "query": wish,
                "view": "mixed", "budget": query::HARD_BUDGET, "refresh": false
            }))
            .map_err(io)?;
            match query::explore(home, &request) {
                Ok(value) => {
                    if let Some(r) = value["revision"].as_i64() {
                        revision = r;
                    }
                    payload = value;
                }
                Err(e) if error.is_none() => error = Some(e.message),
                Err(_) => {}
            }
            match code::health(home, project_dir) {
                Ok(value) => health = Some(value),
                Err(e) if error.is_none() => error = Some(e.message),
                Err(_) => {}
            }
        }
    }
    let bundles = payload["bundles"].as_array().cloned().unwrap_or_default();
    let freshness = payload["freshness"]["state"].as_str().unwrap_or("missing");
    let (state, reason) = if let Some(message) = error {
        ("unavailable", Some(message))
    } else if matches!(freshness, "partial" | "stale") {
        (
            "partial",
            Some(sync_reason.unwrap_or_else(|| format!("index is {freshness}"))),
        )
    } else if bundles.is_empty()
        && health
            .as_ref()
            .is_none_or(|h| h.files == 0 || h.state == "missing")
    {
        ("empty", Some("index is missing or empty".to_owned()))
    } else if freshness == "fresh" {
        ("fresh", None)
    } else {
        ("partial", Some("graph freshness is unavailable".to_owned()))
    };
    let mut base = format!("LAUNCH CONTEXT (graph revision {revision}, {state})\n");
    if let Some(reason) = &reason {
        base.push_str(&format!("graph {state}: {reason}\n"));
    }
    base.push_str(&block.text);
    let mut rendered: Vec<(String, String)> = bundles
        .iter()
        .map(|bundle| {
            let key = bundle["key"].as_str().unwrap_or("").to_owned();
            (key, format!("{}\n", bundle))
        })
        .collect();
    let mut omitted = payload["omitted"].as_u64().unwrap_or(0) as usize;
    loop {
        let notice = if omitted > 0 {
            format!("OMITTED {omitted}\n")
        } else {
            String::new()
        };
        let size = base
            .len()
            .saturating_add(rendered.iter().map(|(_, text)| text.len()).sum::<usize>())
            .saturating_add(notice.len());
        if size <= budget || rendered.is_empty() {
            break;
        }
        rendered.pop();
        omitted += 1;
    }
    let notice = if omitted > 0 {
        format!("OMITTED {omitted}\n")
    } else {
        String::new()
    };
    for (key, text) in rendered {
        sources.push(key);
        base.push_str(&text);
    }
    // Reserve the notice when possible, even if the fixed context alone is too large.
    truncate(&mut base, budget.saturating_sub(notice.len()));
    base.push_str(&notice);
    truncate(&mut base, budget);
    Ok(LaunchPacket {
        bytes: base.len(),
        text: base,
        budget,
        revision,
        graph_state: state.to_owned(),
        reason,
        sources,
        omitted,
        wish_sha256: format!("{:x}", Sha256::digest(wish.as_bytes())),
    })
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

pub fn inject_once(home: &Path, goal: &crate::goal::Goal) -> Res<Option<LaunchPacket>> {
    let (Some(project_dir), Some(project)) = (goal.cwd.as_deref(), goal.project.as_deref()) else {
        return Ok(None);
    };
    let store = crate::orch::store_for_goal(home, goal)?;
    let destination = store.join("launch.json");
    if destination.exists() {
        return Ok(None);
    }
    let packet = build(
        home,
        project,
        Path::new(project_dir),
        &goal.wish,
        LAUNCH_BUDGET,
        Duration::from_secs(3),
    )?;
    fs::create_dir_all(&store).map_err(io)?;
    let data = serde_json::to_vec(&packet).map_err(io)?;
    let (temp, mut file) = loop {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = store.join(format!(".launch-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (path, file),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        }
    };
    let result = (|| -> Res<bool> {
        file.write_all(&data).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        // A hard link publishes the complete temp file atomically, without replacing a winner.
        match fs::hard_link(&temp, &destination) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(io(e)),
        }
    })();
    let _ = fs::remove_file(&temp);
    result.map(|created| created.then_some(packet))
}
