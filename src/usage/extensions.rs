use crate::base::{Failure, Res};
use crate::usage::price::{self, CodexUsage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

pub fn path(home: &Path) -> PathBuf {
    home.join("usage").join("extensions.jsonl")
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    #[default]
    Normal,
    Extreme,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ExtRecord {
    #[serde(default)]
    pub ts_ms: i64,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub job: Option<String>,
    #[serde(default)]
    pub turn: Option<String>,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tier: Tier,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub gpu_seconds: f64,
    #[serde(default)]
    pub compute_units: f64,
    #[serde(default)]
    pub usd: Option<f64>,
}

fn io_failure(path: &Path, error: std::io::Error) -> Failure {
    Failure::new("io", format!("{}: {error}", path.display()))
}

pub fn append(home: &Path, record: &ExtRecord) -> Res<()> {
    let path = path(home);
    let dir = home.join("usage");
    let mut line =
        serde_json::to_vec(record).map_err(|error| Failure::new("json", error.to_string()))?;
    line.push(b'\n');
    fs::create_dir_all(&dir).map_err(|error| io_failure(&dir, error))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)
        .map_err(|error| io_failure(&path, error))?;
    file.lock().map_err(|error| io_failure(&path, error))?;
    let write_result = file
        .write_all(&line)
        .map_err(|error| io_failure(&path, error));
    let unlock_result = file.unlock().map_err(|error| io_failure(&path, error));
    write_result.and(unlock_result)
}

pub fn read(home: &Path) -> Res<(Vec<ExtRecord>, usize)> {
    let path = path(home);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => return Err(io_failure(&path, error)),
    };
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut skipped = 0;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|error| io_failure(&path, error))?
            == 0
        {
            break;
        }
        match serde_json::from_slice(&line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    Ok((records, skipped))
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Totals {
    pub records: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub gpu_seconds: f64,
    pub compute_units: f64,
    pub priced_usd: f64,
    pub unpriced_records: u64,
}

impl Totals {
    pub fn gpu_hours(&self) -> f64 {
        self.gpu_seconds / 3600.0
    }

    fn add(&mut self, record: &ExtRecord) {
        self.records += 1;
        self.input_tokens += record.input_tokens;
        self.cached_input_tokens += record.cached_input_tokens;
        self.output_tokens += record.output_tokens;
        self.gpu_seconds += record.gpu_seconds;
        self.compute_units += record.compute_units;
        match record.usd {
            Some(usd) => self.priced_usd += usd,
            None => self.unpriced_records += 1,
        }
    }
}

pub fn total(records: &[ExtRecord]) -> Totals {
    let mut totals = Totals::default();
    for record in records {
        totals.add(record);
    }
    totals
}

pub fn by_module(records: &[ExtRecord]) -> BTreeMap<String, Totals> {
    let mut groups = BTreeMap::<String, Totals>::new();
    for record in records {
        if let Some(module) = &record.module {
            groups.entry(module.clone()).or_default().add(record);
        }
    }
    groups
}

pub fn by_project(records: &[ExtRecord]) -> BTreeMap<String, Totals> {
    let mut groups = BTreeMap::<String, Totals>::new();
    for record in records {
        if let Some(project) = &record.project {
            groups.entry(project.clone()).or_default().add(record);
        }
    }
    groups
}

pub fn by_tier(records: &[ExtRecord]) -> BTreeMap<Tier, Totals> {
    let mut groups = BTreeMap::<Tier, Totals>::new();
    for record in records.iter().filter(|record| record.provider == "codex") {
        groups.entry(record.tier).or_default().add(record);
    }
    groups
}

pub fn codex_usd(model: &str, tier: Tier, usage: &CodexUsage) -> Option<f64> {
    match tier {
        Tier::Normal => price::codex_usd(model, usage),
        Tier::Extreme => None,
    }
}
