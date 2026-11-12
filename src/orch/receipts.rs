use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RunRecord {
    pub unit: String,
    pub attempt: String,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub status: String,
}
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn parse_runs(bytes: &[u8]) -> Res<(Vec<RunRecord>, usize)> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let end = offset + line.len();
        let content = line.strip_suffix(b"\n").unwrap_or(line);
        match serde_json::from_slice(content) {
            Ok(row) => rows.push(row),
            Err(_) if end == bytes.len() => break,
            Err(e) => return Err(invalid(format!("invalid run receipt: {e}"))),
        }
        offset = end;
    }
    Ok((rows, offset))
}
fn load(store: &Path) -> Res<(Vec<RunRecord>, usize)> {
    match fs::read(store.join("receipts/runs.jsonl")) {
        Ok(bytes) => parse_runs(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((vec![], 0)),
        Err(e) => Err(io(e)),
    }
}
pub fn record_run(store: &Path, record: &RunRecord) -> Res<()> {
    let dir = store.join("receipts");
    fs::create_dir_all(&dir).map_err(io)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(dir.join("runs.lock"))
        .map_err(io)?;
    lock.lock().map_err(io)?;
    let (_, len) = load(store)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(dir.join("runs.jsonl"))
        .map_err(io)?;
    if file.metadata().map_err(io)?.len() != len as u64 {
        file.set_len(len as u64).map_err(io)?;
    }
    let needs_newline = if len > 0 {
        file.seek(SeekFrom::End(-1)).map_err(io)?;
        let mut last = [0];
        file.read_exact(&mut last).map_err(io)?;
        last[0] != b'\n'
    } else {
        false
    };
    file.seek(SeekFrom::End(0)).map_err(io)?;
    let mut line = Vec::new();
    if needs_newline {
        line.push(b'\n');
    }
    serde_json::to_writer(&mut line, record).map_err(|e| invalid(e.to_string()))?;
    line.push(b'\n');
    file.write_all(&line).map_err(io)
}
pub fn read_runs(store: &Path) -> Res<Vec<RunRecord>> {
    Ok(load(store)?.0)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Concurrency {
    pub mean: f64,
    pub peak: u64,
    pub cap: u64,
    pub window_ms: i64,
}
pub fn concurrency(runs: &[RunRecord], window: (i64, i64), cap: u64) -> Concurrency {
    let length = window.1.saturating_sub(window.0).max(0);
    let mut result = Concurrency {
        mean: 0.0,
        peak: 0,
        cap,
        window_ms: length,
    };
    if length == 0 {
        return result;
    }
    let mut changes = Vec::new();
    let mut area = 0_i128;
    for run in runs {
        let start = run.started_ms.max(window.0);
        let end = run.ended_ms.min(window.1);
        if end > start {
            area += i128::from(end) - i128::from(start);
            changes.push((start, 1_i64));
            changes.push((end, -1_i64));
        }
    }
    changes.sort_unstable(); // ends precede starts at equal timestamps
    let mut current = 0_i64;
    for (_, change) in changes {
        current += change;
        result.peak = result.peak.max(current as u64);
    }
    result.mean = area as f64 / length as f64;
    result
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UnitClock {
    pub unit: String,
    pub first_ms: i64,
    pub verified_ms: i64,
    pub wall_ms: i64,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Receipts {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub total_ms: i64,
    pub verified_units: usize,
    pub per_unit_ms: Option<f64>,
    pub units: Vec<UnitClock>,
    pub concurrency: Concurrency,
    pub runs: usize,
}

// Parse the UTC millisecond timestamps emitted by base::now_iso.
pub(crate) fn parse_timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 24
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'.'
        || b[23] != b'Z'
    {
        return None;
    }
    let number = |start: usize, end: usize| -> Option<i64> {
        b[start..end].iter().try_fold(0_i64, |n, c| {
            if c.is_ascii_digit() {
                Some(n * 10 + i64::from(c - b'0'))
            } else {
                None
            }
        })
    };
    let (y, m, d, h, min, sec, ms) = (
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
        number(20, 23)?,
    );
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if d < 1 || d > mdays || h > 23 || min > 59 || sec > 59 {
        return None;
    }
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = m + if m > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Some(((days * 24 + h) * 60 * 60 + min * 60 + sec) * 1000 + ms)
}
pub fn compute(store: &Path, cap: u64) -> Res<Receipts> {
    let all = super::read_units(store)?;
    let runs = read_runs(store)?;
    let mut verified = Vec::new();
    for unit in &all {
        if unit.superseded_by().is_some() {
            continue;
        }
        let rows = super::evidence(store, &unit.id)?;
        if super::derive_state(&rows, super::needs_accept(store, &unit.id)?) == "verified" {
            let ts = &rows.last().expect("verified requires evidence").ts;
            let time = parse_timestamp(ts)
                .ok_or_else(|| invalid(format!("invalid evidence timestamp: {ts}")))?;
            verified.push((unit, time));
        }
    }
    let n = verified.len();
    let mut units = Vec::new();
    for (unit, time) in &verified {
        let first = runs
            .iter()
            .filter(|r| {
                all.iter().any(|u| {
                    u.id == r.unit
                        && super::standing_unit(&all, u)
                            .is_some_and(|standing| standing.id == unit.id)
                })
            })
            .map(|r| r.started_ms)
            .min();
        if let Some(first) = first {
            units.push(UnitClock {
                unit: unit.id.clone(),
                first_ms: first,
                verified_ms: *time,
                wall_ms: time.saturating_sub(first),
            });
        }
    }
    let (start, end) = if runs.is_empty() {
        (0, 0)
    } else {
        let start = runs.iter().map(|r| r.started_ms).min().unwrap();
        let end = runs
            .iter()
            .map(|r| r.ended_ms)
            .chain(verified.iter().map(|(_, t)| *t))
            .max()
            .unwrap()
            .max(start);
        (start, end)
    };
    let total = end.saturating_sub(start);
    Ok(Receipts {
        window_start_ms: start,
        window_end_ms: end,
        total_ms: total,
        verified_units: n,
        per_unit_ms: (n > 0).then(|| total as f64 / n as f64),
        units,
        concurrency: concurrency(&runs, (start, end), cap),
        runs: runs.len(),
    })
}
pub fn write_receipts(store: &Path, cap: u64) -> Res<PathBuf> {
    let receipts = compute(store, cap)?;
    let path = store.join("receipts.json");
    let data = serde_json::to_vec_pretty(&receipts).map_err(|e| invalid(e.to_string()))?;
    base::atomic_write(&path, &data).map_err(io)?;
    Ok(path)
}
