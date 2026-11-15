pub mod triggers;

use super::{
    launch::{LaunchKind, LaunchRequest, Launched, Launcher},
    runs, Mode, Run,
};
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoutineConfig {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
    pub trigger: Trigger,
    pub launch: LaunchKind,
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: Option<String>,
    pub project: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Trigger {
    Cron {
        expr: String,
    },
    Event {
        kind: triggers::EventKind,
        repo: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct Fired {
    pub routine: String,
    pub run: Run,
    pub child: Launched,
}

fn io_error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn load(home: &Path) -> Res<Vec<RoutineConfig>> {
    let bytes = match fs::read(home.join("modes/routines.json")) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_error(e)),
    };
    let configs: Vec<RoutineConfig> = serde_json::from_slice(&bytes)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    for cfg in &configs {
        base::require_slug("routine", &cfg.id)?;
        if cfg.scope.is_empty()
            || cfg
                .scope
                .iter()
                .any(|s| s.trim().is_empty() || !runs::relative(s))
        {
            return Err(Failure::new(
                "invalid_argument",
                "scope must contain non-empty relative paths without parent components",
            ));
        }
        if cfg.launch == LaunchKind::Quick
            && cfg.verify.as_deref().map_or(true, |v| v.trim().is_empty())
        {
            return Err(Failure::new(
                "invalid_argument",
                "quick routines require verification",
            ));
        }
        if let Trigger::Cron { expr } = &cfg.trigger {
            triggers::parse_cron(expr)?;
        }
    }
    Ok(configs)
}

fn fire(home: &Path, cfg: &RoutineConfig, launcher: &dyn Launcher) -> Res<Fired> {
    let session = runs::create_session(home, Some(&cfg.project), &cfg.scope)?;
    let run = runs::start_run(home, &session.id, Mode::Routine, None, Some(&cfg.id))?;
    let request = LaunchRequest {
        kind: cfg.launch,
        session: session.id,
        parent_run: run.id.clone(),
        goal: cfg.goal.clone(),
        scope: cfg.scope.clone(),
        verify: cfg.verify.clone(),
        review: false,
        context: None,
        sources: vec![],
    };
    let child = match launcher.launch(&request) {
        Ok(child) => child,
        Err(e) => {
            let _ = runs::finish_run(home, &run.id, &e.to_string());
            return Err(e);
        }
    };
    Ok(Fired {
        routine: cfg.id.clone(),
        run,
        child,
    })
}

#[derive(Serialize, Deserialize)]
struct Seen {
    routine: String,
    event: String,
}

pub fn fire_event(
    home: &Path,
    event: &triggers::Event,
    launcher: &dyn Launcher,
) -> Res<Vec<Fired>> {
    let configs = load(home)?;
    let path = home.join("modes/routines/seen.jsonl");
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_error(e)),
    };
    let mut seen = HashSet::new();
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let record: Seen = serde_json::from_str(line).map_err(io_error)?;
        seen.insert((record.routine, record.event));
    }
    let mut fired = Vec::new();
    for cfg in configs.iter().filter(|cfg| cfg.enabled) {
        if let Trigger::Event { kind, repo } = &cfg.trigger {
            let key = (cfg.id.clone(), event.id.clone());
            if *kind != event.kind
                || repo.as_ref().is_some_and(|r| r != &event.repo)
                || seen.contains(&key)
            {
                continue;
            }
            let result = fire(home, cfg, launcher)?;
            fs::create_dir_all(path.parent().unwrap()).map_err(io_error)?;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(io_error)?;
            let mut line = serde_json::to_vec(&Seen {
                routine: key.0.clone(),
                event: key.1.clone(),
            })
            .map_err(io_error)?;
            line.push(b'\n');
            file.write_all(&line).map_err(io_error)?;
            seen.insert(key);
            fired.push(result);
        }
    }
    Ok(fired)
}

pub fn tick(home: &Path, now_unix: i64, launcher: &dyn Launcher) -> Res<Vec<Fired>> {
    let configs = load(home)?;
    let path = home.join("modes/routines/cron.json");
    let mut seen: HashMap<String, i64> = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io_error)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => return Err(io_error(e)),
    };
    let minute = now_unix / 60;
    let mut fired = Vec::new();
    for cfg in configs.iter().filter(|cfg| cfg.enabled) {
        if let Trigger::Cron { expr } = &cfg.trigger {
            if seen.get(&cfg.id) == Some(&minute) || !triggers::parse_cron(expr)?.due(now_unix) {
                continue;
            }
            let result = fire(home, cfg, launcher)?;
            seen.insert(cfg.id.clone(), minute);
            fs::create_dir_all(path.parent().unwrap()).map_err(io_error)?;
            base::atomic_write(&path, &serde_json::to_vec(&seen).map_err(io_error)?)
                .map_err(io_error)?;
            fired.push(result);
        }
    }
    Ok(fired)
}
