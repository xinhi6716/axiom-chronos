//! Shared OS locks, one per Chronos Codex turn. Files remain; only locks mean occupancy.
use crate::{
    base::{self, Failure, Res},
    council::budgets::BudgetError,
};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const ENV: &str = "AXIOM_CODEX_MAX_PARALLEL";
pub const DEFAULT: u64 = 48;
pub const POSITIVE_INTEGER: &str = "Codex parallel limit must be a positive whole number.";

pub fn parse(value: Option<String>) -> Result<u64, BudgetError> {
    match value {
        None => Ok(DEFAULT),
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or(BudgetError {
                variable: ENV,
                value,
                reason: POSITIVE_INTEGER,
            }),
    }
}
pub fn from_env() -> Res<u64> {
    parse(crate::council::budgets::env_value(ENV)).map_err(Into::into)
}
pub fn home() -> PathBuf {
    base::data_home().unwrap_or_else(|_| base::axiom_home())
}
fn failure(e: std::io::Error) -> Failure {
    Failure::new("codex_slots", e.to_string())
}
fn open(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}
fn try_lock(file: &File) -> Res<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(failure(e)),
    }
}

/// Dropping the handle (including process death) releases the OS lock. Never unlink it:
/// replacing a locked inode would let another process take the same slot on Unix.
pub struct Slot {
    _file: File,
    record: PathBuf,
}
impl Slot {
    /// Written while holding the slot, before its turn starts. Readers only trust locked slots.
    pub fn model(&self, model: &str) -> Res<()> {
        base::atomic_write(&self.record, serde_json::json!({"model":model}).to_string().as_bytes()).map_err(failure)
    }
}

#[derive(Debug, Default)]
pub struct Occupancy {
    pub total: u64,
    /// A missing or unreadable model record hides model counts, without guessing a model.
    pub models: Option<std::collections::BTreeMap<String, u64>>,
}

pub fn occupancy(home: &Path) -> Res<Occupancy> {
    let folder = home.join("codex-slots");
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Occupancy { total: 0, models: Some(Default::default()) }),
        Err(e) => return Err(failure(e)),
    };
    let mut result = Occupancy { total: 0, models: Some(Default::default()) };
    for entry in entries {
        let entry = entry.map_err(failure)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(index) = name.strip_suffix(".lock").filter(|n| n.parse::<u64>().is_ok()) else { continue };
        let file = OpenOptions::new().read(true).write(true).open(entry.path()).map_err(failure)?;
        if try_lock(&file)? { continue; }
        result.total += 1;
        let model = fs::read(folder.join(format!("{index}.json"))).ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|v| v["model"].as_str().filter(|s| !s.is_empty()).map(str::to_owned));
        match (result.models.as_mut(), model) {
            (Some(models), Some(model)) => *models.entry(model).or_default() += 1,
            (_, None) => result.models = None,
            _ => {}
        }
    }
    Ok(result)
}

#[derive(Default)]
pub struct Wait {
    pub elapsed: Duration,
    pub full: bool,
}
impl Wait {
    pub fn note(&self, limit: u64) -> Option<String> {
        self.full.then(|| {
            format!(
                "waited {:.1} s for a Codex slot ({limit} in use)",
                self.elapsed.as_secs_f64()
            )
        })
    }
}
pub fn acquire(
    home: &Path,
    limit: u64,
    wait: &mut Wait,
    mut stopped: impl FnMut() -> Res<()>,
) -> Res<Slot> {
    let folder = home.join("codex-slots");
    fs::create_dir_all(&folder).map_err(failure)?;
    let start = Instant::now();
    let mut last_notice = start;
    loop {
        if wait.full {
            wait.elapsed = start.elapsed();
        }
        stopped()?;
        for index in 0..limit {
            // Poll cancellation even with a large configured pool.
            stopped()?;
            let file = open(&folder.join(format!("{index}.lock"))).map_err(failure)?;
            if try_lock(&file)? {
                let record = folder.join(format!("{index}.json"));
                base::atomic_write(&record, b"{}").map_err(failure)?;
                return Ok(Slot { _file: file, record });
            }
        }
        wait.full = true;
        if last_notice.elapsed() >= Duration::from_secs(15) {
            if let Some(progress) = base::progress::current() {
                progress.emit("waiting for a Codex slot");
            }
            last_notice = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Probe existing locks only; no Codex process, stale-file heuristics or background timer.
pub fn in_use(home: &Path) -> Res<u64> {
    occupancy(home).map(|reading| reading.total)
}
