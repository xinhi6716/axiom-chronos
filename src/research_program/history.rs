use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

pub fn path(home: &Path) -> PathBuf {
    home.join("research").join("history.jsonl")
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ProviderSpend {
    pub usd: f64,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunSummary {
    pub run_id: String,
    pub program_type: String,
    pub models: Vec<String>,
    pub per_provider: BTreeMap<String, ProviderSpend>,
    pub completed: bool,
    pub at: String,
}

fn io_failure(path: &Path, error: std::io::Error) -> Failure {
    Failure::new("io", format!("{}: {error}", path.display()))
}

pub fn append(home: &Path, summary: &RunSummary) -> Res<()> {
    let path = path(home);
    let dir = home.join("research");
    let mut line = serde_json::to_vec(summary).map_err(|e| Failure::new("json", e.to_string()))?;
    line.push(b'\n');
    fs::create_dir_all(&dir).map_err(|e| io_failure(&dir, e))?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)
        .map_err(|e| io_failure(&path, e))?;
    file.lock().map_err(|e| io_failure(&path, e))?;
    let result = file.write_all(&line).map_err(|e| io_failure(&path, e));
    let unlock = file.unlock().map_err(|e| io_failure(&path, e));
    result.and(unlock)
}

pub fn read(home: &Path) -> Res<Vec<RunSummary>> {
    let path = path(home);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_failure(&path, e)),
    };
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut records = Vec::new();
    loop {
        line.clear();
        if reader
            .read_until(b'\n', &mut line)
            .map_err(|e| io_failure(&path, e))?
            == 0
        {
            break;
        }
        if let Ok(record) = serde_json::from_slice(&line) {
            records.push(record);
        }
    }
    Ok(records)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderAvg {
    pub priced_usd: f64,
    pub unpriced_tokens: u64,
    pub has_unpriced: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub program_type: String,
    pub models: Vec<String>,
    pub samples: usize,
    pub per_provider: BTreeMap<String, ProviderAvg>,
    pub priced_usd: Option<f64>,
}

pub fn estimate(home: &Path, program_type: &str, models: &[String]) -> Res<Estimate> {
    let mut models = models.to_vec();
    models.sort();
    let mut result = Estimate {
        program_type: program_type.to_owned(),
        models,
        samples: 0,
        per_provider: BTreeMap::new(),
        priced_usd: None,
    };
    for run in read(home)? {
        let mut run_models = run.models;
        run_models.sort();
        if run.program_type != program_type || run_models != result.models {
            continue;
        }
        result.samples += 1;
        for (provider, spend) in run.per_provider {
            let avg = result.per_provider.entry(provider).or_default();
            avg.priced_usd += spend.usd;
            avg.unpriced_tokens += spend.unpriced_tokens;
            avg.has_unpriced |= spend.unpriced_tokens > 0;
        }
    }
    if result.samples > 0 {
        for avg in result.per_provider.values_mut() {
            avg.priced_usd /= result.samples as f64;
            avg.unpriced_tokens /= result.samples as u64;
        }
        result.priced_usd = Some(result.per_provider.values().map(|avg| avg.priced_usd).sum());
    }
    Ok(result)
}

impl Estimate {
    pub fn line(&self) -> String {
        if self.samples == 0 {
            return format!(
                "no history for {} with {}: no estimate",
                self.program_type,
                self.models.join(", ")
            );
        }
        let providers = self
            .per_provider
            .iter()
            .map(|(provider, avg)| {
                let unknown = if avg.has_unpriced {
                    format!(" + unknown ({} tokens unpriced)", avg.unpriced_tokens)
                } else {
                    String::new()
                };
                format!("{provider} ${:.2}{unknown}", avg.priced_usd)
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "estimate from {} past runs: {}; priced total ${:.2}",
            self.samples,
            providers,
            self.priced_usd.unwrap_or_default()
        )
    }
}
