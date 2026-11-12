//! Per-run mode receipts.
use super::Mode;
use crate::{
    base::{self, require_slug, Failure, Res},
    usage::price::{codex_usd, CodexUsage},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Cost {
    Usd(f64),
    Unknown,
}

pub fn sum(costs: &[Cost]) -> Cost {
    let mut total = 0.0;
    for cost in costs {
        match cost {
            Cost::Usd(value) => total += value,
            Cost::Unknown => return Cost::Unknown,
        }
    }
    Cost::Usd(total)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Part {
    pub label: String,
    pub model: String,
    pub usage: Option<Usage>,
    pub cost: Cost,
    pub wall_ms: u64,
}

pub fn codex_part(label: &str, model: &str, usage: &serde_json::Value, wall_ms: u64) -> Part {
    let parsed = (|| {
        let object = usage.as_object()?;
        Some(Usage {
            input: object.get("inputTokens")?.as_u64()?,
            cached_input: match object.get("cachedInputTokens") {
                None => 0,
                Some(value) => value.as_u64()?,
            },
            output: object.get("outputTokens")?.as_u64()?,
        })
    })();
    let cost = parsed
        .as_ref()
        .and_then(|u| {
            codex_usd(
                model,
                &CodexUsage {
                    input: u.input,
                    cached_input: u.cached_input,
                    output: u.output,
                },
            )
        })
        .map(Cost::Usd)
        .unwrap_or(Cost::Unknown);
    Part {
        label: label.into(),
        model: model.into(),
        usage: parsed,
        cost,
        wall_ms,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Receipt {
    pub run: String,
    pub session: String,
    pub mode: Mode,
    pub outcome: String,
    pub started: String,
    pub finished: String,
    pub wall_ms: u64,
    pub cost: Cost,
    pub parts: Vec<Part>,
}

impl Receipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run: impl Into<String>,
        session: impl Into<String>,
        mode: Mode,
        outcome: impl Into<String>,
        started: impl Into<String>,
        finished: impl Into<String>,
        wall_ms: u64,
        parts: Vec<Part>,
    ) -> Self {
        let cost = sum(&parts.iter().map(|part| part.cost).collect::<Vec<_>>());
        Self {
            run: run.into(),
            session: session.into(),
            mode,
            outcome: outcome.into(),
            started: started.into(),
            finished: finished.into(),
            wall_ms,
            cost,
            parts,
        }
    }
}

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

fn path(home: &Path, run: &str) -> Res<PathBuf> {
    require_slug("run", run)?;
    Ok(home
        .join("modes")
        .join("receipts")
        .join(format!("{run}.json")))
}

pub fn write(home: &Path, r: &Receipt) -> Res<PathBuf> {
    let path = path(home, &r.run)?;
    fs::create_dir_all(path.parent().expect("receipt has a parent")).map_err(error)?;
    let bytes = serde_json::to_vec(r).map_err(error)?;
    base::atomic_write(&path, &bytes).map_err(error)?;
    Ok(path)
}

pub fn load(home: &Path, run: &str) -> Res<Receipt> {
    let path = path(home, run)?;
    let bytes = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Failure::new("not_found", format!("not found: {}", path.display()))
        } else {
            error(e)
        }
    })?;
    serde_json::from_slice(&bytes).map_err(error)
}

pub fn render(r: &Receipt) -> String {
    let cost = match r.cost {
        Cost::Usd(value) => format!("${value:.4}"),
        Cost::Unknown => "cost unknown".into(),
    };
    format!(
        "mode {}\noutcome {}\n{:.1} s\n{}",
        r.mode.as_str(),
        r.outcome,
        r.wall_ms as f64 / 1000.0,
        cost
    )
}
