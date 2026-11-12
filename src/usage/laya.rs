use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UsageEvent {
    pub id: String,
    pub ts_ms: i64,
    pub model_version: String,
    pub kind: String,
    pub backend: String,
    pub tokens: Option<u64>,
    pub gpu_hours: Option<f64>,
    pub compute_units: Option<f64>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ModelUsage {
    pub model_version: String,
    pub events: u64,
    pub tokens: u64,
    pub tokens_unknown: u64,
    pub gpu_hours: f64,
    pub gpu_hours_unknown: u64,
    pub compute_units: f64,
    pub compute_units_unknown: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LayaUsage {
    pub models: Vec<ModelUsage>,
    pub skipped_lines: u64,
}

fn io_error(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn record(home: &Path, e: &UsageEvent) -> Res<()> {
    let dir = crate::laya::laya_dir(home);
    fs::create_dir_all(&dir).map_err(io_error)?;
    let _lock = crate::orch::store::lock_store(&dir)?;
    let mut line = serde_json::to_vec(e).map_err(|e| Failure::new("json", e.to_string()))?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("usage.jsonl"))
        .map_err(io_error)?;
    file.write_all(&line).map_err(io_error)
}

pub fn summarize(home: &Path) -> Res<LayaUsage> {
    let file = match File::open(crate::laya::laya_dir(home).join("usage.jsonl")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LayaUsage {
                models: Vec::new(),
                skipped_lines: 0,
            });
        }
        Err(e) => return Err(io_error(e)),
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut seen = HashSet::new();
    let mut models = BTreeMap::<String, ModelUsage>::new();
    let mut skipped_lines = 0;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(io_error)? == 0 {
            break;
        }
        let event: UsageEvent = match serde_json::from_slice(&line) {
            Ok(event) => event,
            Err(_) => {
                skipped_lines += 1;
                continue;
            }
        };
        if !seen.insert(event.id) {
            continue;
        }
        let model = models
            .entry(event.model_version.clone())
            .or_insert_with(|| ModelUsage {
                model_version: event.model_version,
                events: 0,
                tokens: 0,
                tokens_unknown: 0,
                gpu_hours: 0.0,
                gpu_hours_unknown: 0,
                compute_units: 0.0,
                compute_units_unknown: 0,
            });
        model.events += 1;
        match event.tokens {
            Some(value) => {
                model.tokens = model
                    .tokens
                    .checked_add(value)
                    .ok_or_else(|| Failure::new("invalid_data", "Laya token total overflow"))?
            }
            None => model.tokens_unknown += 1,
        }
        match event.gpu_hours {
            Some(value) => model.gpu_hours += value,
            None => model.gpu_hours_unknown += 1,
        }
        match event.compute_units {
            Some(value) => model.compute_units += value,
            None => model.compute_units_unknown += 1,
        }
    }
    Ok(LayaUsage {
        models: models.into_values().collect(),
        skipped_lines,
    })
}

fn amount(value: impl std::fmt::Display, unknown: u64, events: u64) -> String {
    if unknown == events {
        "unknown".to_string()
    } else if unknown > 0 {
        format!("{value} (+{unknown} unknown)")
    } else {
        value.to_string()
    }
}

pub fn render(u: &LayaUsage) -> String {
    let mut text = String::from("Laya:\n");
    for model in &u.models {
        text.push_str(&format!(
            "  {}: tokens {}, GPU hours {}, compute units {}\n",
            model.model_version,
            amount(model.tokens, model.tokens_unknown, model.events),
            amount(model.gpu_hours, model.gpu_hours_unknown, model.events),
            amount(
                model.compute_units,
                model.compute_units_unknown,
                model.events
            ),
        ));
    }
    text
}

pub fn cli_text(home: &Path) -> Res<String> {
    let usage = summarize(home)?;
    Ok(if usage.models.is_empty() {
        String::new()
    } else {
        render(&usage)
    })
}

pub fn cli_json(home: &Path) -> Res<Option<serde_json::Value>> {
    let usage = summarize(home)?;
    if usage.models.is_empty() {
        Ok(None)
    } else {
        serde_json::to_value(usage)
            .map(Some)
            .map_err(|e| Failure::new("json", e.to_string()))
    }
}
