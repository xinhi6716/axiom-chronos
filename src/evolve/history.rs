use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path},
};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MetricValue {
    pub problem: String,
    pub value: f64,
    pub samples: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RunRecord {
    pub id: String,
    pub started_ms: i64,
    pub repo: String,
    pub base_commit: String,
    pub branch: String,
    pub since_days: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub problems: Vec<String>,
    pub metrics: Vec<MetricValue>,
    pub state: String,
    pub session: Option<String>,
    pub goal_status: Option<String>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

pub fn new_id(started_ms: i64) -> String {
    let seconds = started_ms.div_euclid(1000);
    let days = seconds.div_euclid(86400);
    let time = seconds.rem_euclid(86400);
    // Gregorian civil date from days since the Unix epoch.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{}",
        time / 3600,
        time % 3600 / 60,
        time % 60,
        base::unique()
    )
}

fn io(error: impl std::fmt::Display) -> Failure {
    Failure::new("io", error.to_string())
}

pub fn save(home: &Path, record: &RunRecord) -> Res<()> {
    let mut parts = Path::new(&record.id).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        return Err(Failure::new("invalid_argument", "invalid run id"));
    }
    let dir = home.join("evolve/runs");
    fs::create_dir_all(&dir).map_err(io)?;
    let bytes = serde_json::to_vec_pretty(record).map_err(io)?;
    base::atomic_write(&dir.join(format!("{}.json", record.id)), &bytes).map_err(io)
}

pub fn list(home: &Path) -> Res<Vec<RunRecord>> {
    let entries = match fs::read_dir(home.join("evolve/runs")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(e)),
    };
    let mut records: Vec<RunRecord> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    records.sort_by(|a, b| (b.started_ms, &b.id).cmp(&(a.started_ms, &a.id)));
    Ok(records)
}
