//! Independent usage pools for Gemini models.

use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub const HYDROGEN_XHIGH: &str = "gemini-4-hydrogen-xhigh";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Pool {
    HydrogenFlash,
    Argon,
}
impl Pool {
    pub const ALL: [Pool; 2] = [Pool::HydrogenFlash, Pool::Argon];
    pub fn id(self) -> &'static str {
        match self {
            Self::HydrogenFlash => "hydrogen_flash",
            Self::Argon => "argon",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::HydrogenFlash => "Hydrogen/Flash",
            Self::Argon => "Argon",
        }
    }
}

pub fn pool_for_model(model: &str) -> Option<Pool> {
    if model.starts_with("gemini-4-hydrogen") || model.starts_with("gemini-3.8-flash") {
        Some(Pool::HydrogenFlash)
    } else if model.starts_with("gemini-4-argon") {
        Some(Pool::Argon)
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
    Charge {
        pool: Pool,
        model: String,
        tokens: Option<u64>,
        cost_usd: Option<f64>,
        caller: String,
    },
    Exhausted {
        pool: Pool,
        until_ms: i64,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remaining {
    Known { tokens: u64 },
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoolStatus {
    pub pool: Pool,
    pub used_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub remaining: Remaining,
    pub exhausted_until_ms: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct PoolStrip(pub Vec<PoolStatus>);

#[derive(Default, Deserialize)]
struct Limit {
    token_limit: Option<u64>,
}
#[derive(Default, Deserialize)]
struct Limits {
    #[serde(default)]
    hydrogen_flash: Limit,
    #[serde(default)]
    argon: Limit,
}

fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> Failure {
    Failure::new("invalid_argument", e.to_string())
}

fn append(home: &Path, entry: &Entry) -> Res<()> {
    let dir = home.join("usage");
    fs::create_dir_all(&dir).map_err(io)?;
    let mut line = serde_json::to_vec(entry).map_err(invalid)?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(dir.join("pools.jsonl"))
        .map_err(io)?;
    file.lock().map_err(io)?;
    file.write_all(&line).map_err(io)
}

pub fn charge(
    home: &Path,
    model: &str,
    tokens: Option<u64>,
    cost_usd: Option<f64>,
    caller: &str,
) -> Res<Option<Pool>> {
    let Some(pool) = pool_for_model(model) else {
        return Ok(None);
    };
    if cost_usd.is_some_and(|cost| !cost.is_finite() || cost < 0.0) {
        return Err(invalid("invalid cost"));
    }
    append(
        home,
        &Entry::Charge {
            pool,
            model: model.into(),
            tokens,
            cost_usd,
            caller: caller.into(),
        },
    )?;
    Ok(Some(pool))
}

/// Record a provider exhaustion mark. When the provider gives no reset time,
/// callers should pass the current time plus 60 minutes as `until_ms`.
pub fn mark_exhausted(home: &Path, pool: Pool, until_ms: i64, reason: &str) -> Res<()> {
    append(
        home,
        &Entry::Exhausted {
            pool,
            until_ms,
            reason: reason.into(),
        },
    )
}

pub fn status(home: &Path, now_ms: i64) -> Res<Vec<PoolStatus>> {
    let limits: Limits = match fs::read(home.join("settings/pools.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(invalid)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Limits::default(),
        Err(e) => return Err(io(e)),
    };
    let mut statuses: Vec<_> = Pool::ALL
        .into_iter()
        .map(|pool| PoolStatus {
            pool,
            used_tokens: Some(0),
            cost_usd: Some(0.0),
            remaining: Remaining::Unknown,
            exhausted_until_ms: None,
        })
        .collect();
    let path = home.join("usage/pools.jsonl");
    let mut contents = String::new();
    match File::open(path) {
        Ok(mut file) => {
            file.lock_shared().map_err(io)?;
            file.read_to_string(&mut contents).map_err(io)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(e)),
    }
    for line in contents.lines() {
        let entry: Entry = serde_json::from_str(line).map_err(invalid)?;
        match entry {
            Entry::Charge {
                pool,
                tokens,
                cost_usd,
                ..
            } => {
                let s = &mut statuses[if pool == Pool::HydrogenFlash { 0 } else { 1 }];
                s.used_tokens = s
                    .used_tokens
                    .zip(tokens)
                    .and_then(|(a, b)| a.checked_add(b));
                s.cost_usd = s.cost_usd.zip(cost_usd).and_then(|(a, b)| {
                    let sum = a + b;
                    (b.is_finite() && b >= 0.0 && sum.is_finite()).then_some(sum)
                });
            }
            Entry::Exhausted { pool, until_ms, .. } if until_ms > now_ms => {
                let s = &mut statuses[if pool == Pool::HydrogenFlash { 0 } else { 1 }];
                s.exhausted_until_ms = Some(
                    s.exhausted_until_ms
                        .map_or(until_ms, |old| old.max(until_ms)),
                );
            }
            Entry::Exhausted { .. } => {}
        }
    }
    for (s, limit) in statuses
        .iter_mut()
        .zip([limits.hydrogen_flash.token_limit, limits.argon.token_limit])
    {
        s.remaining = match (limit, s.used_tokens) {
            (Some(limit), Some(used)) => Remaining::Known {
                tokens: limit.saturating_sub(used),
            },
            _ => Remaining::Unknown,
        };
    }
    Ok(statuses)
}

pub fn check(home: &Path, model: &str) -> Res<()> {
    let Some(pool) = pool_for_model(model) else {
        return Ok(());
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(invalid)?
        .as_millis()
        .min(i64::MAX as u128) as i64;
    let s = status(home, now_ms)?
        .into_iter()
        .find(|s| s.pool == pool)
        .expect("all pools present");
    if s.exhausted_until_ms.is_some() || s.remaining == (Remaining::Known { tokens: 0 }) {
        return Err(Failure::new(
            "pool_exhausted",
            format!("{} pool exhausted", pool.id()),
        ));
    }
    Ok(())
}
