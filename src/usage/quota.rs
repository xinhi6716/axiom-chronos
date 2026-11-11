//! Provider quota snapshots, independent of presentation and I/O.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    FiveHour,
    Weekly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub kind: WindowKind,
    pub used: f32,
    pub resets_at: Option<i64>,
    pub observed_at: i64,
    pub status: Option<String>,
}

impl Window {
    pub fn left(&self) -> f32 {
        (1.0 - self.used).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderQuota {
    pub five_hour: Option<Window>,
    pub weekly: Option<Window>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Quotas {
    pub claude: ProviderQuota,
    pub codex: ProviderQuota,
}

impl Quotas {
    pub fn merge(&mut self, p: Provider, update: ProviderQuota) -> bool {
        fn merge_window(current: &mut Option<Window>, update: Option<Window>) -> bool {
            let Some(update) = update else { return false };
            if let Some(old) = current {
                if old.observed_at > update.observed_at || *old == update {
                    return false;
                }
            }
            *current = Some(update);
            true
        }
        let current = match p {
            Provider::Claude => &mut self.claude,
            Provider::Codex => &mut self.codex,
        };
        let five_changed = merge_window(&mut current.five_hour, update.five_hour);
        let weekly_changed = merge_window(&mut current.weekly, update.weekly);
        five_changed || weekly_changed
    }
}

pub fn parse_claude_event(v: &Value, observed_at: i64) -> Option<ProviderQuota> {
    if v.get("type").and_then(Value::as_str) != Some("rate_limit_event") {
        return None;
    }
    fn window(v: &Value, kind: WindowKind, observed_at: i64) -> Option<Window> {
        let used = v.get("utilization")?.as_f64()?;
        if !used.is_finite() || used < 0.0 {
            return None;
        }
        Some(Window {
            kind,
            used: used.min(1.0) as f32,
            resets_at: Some(v.get("resetsAt")?.as_i64()?),
            observed_at,
            status: Some(v.get("status")?.as_str()?.to_owned()),
        })
    }
    let windows = &v["rate_limit_info"]["unifiedWindows"];
    Some(ProviderQuota {
        five_hour: window(&windows["five_hour"], WindowKind::FiveHour, observed_at),
        weekly: window(&windows["seven_day"], WindowKind::Weekly, observed_at),
    })
}

fn codex_snapshot(snapshot: &Value, observed_at: i64) -> ProviderQuota {
    let mut quota = ProviderQuota::default();
    for name in ["primary", "secondary"] {
        let v = &snapshot[name];
        let (kind, slot) = match v["windowDurationMins"].as_i64() {
            Some(300) => (WindowKind::FiveHour, &mut quota.five_hour),
            Some(10080) => (WindowKind::Weekly, &mut quota.weekly),
            _ => continue,
        };
        let Some(used) = v["usedPercent"].as_f64().filter(|n| n.is_finite()) else {
            continue;
        };
        *slot = Some(Window {
            kind,
            used: (used / 100.0).clamp(0.0, 1.0) as f32,
            resets_at: v["resetsAt"].as_i64(),
            observed_at,
            status: None,
        });
    }
    quota
}

pub fn parse_codex_read(result: &Value, observed_at: i64) -> ProviderQuota {
    let codex = &result["rateLimitsByLimitId"]["codex"];
    let snapshot = if codex.is_object() {
        codex
    } else {
        &result["rateLimits"]
    };
    codex_snapshot(snapshot, observed_at)
}

pub fn parse_codex_update(params: &Value, observed_at: i64) -> ProviderQuota {
    codex_snapshot(&params["rateLimits"], observed_at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Normal,
    Low,
    Exhausted,
}

pub fn level(left: f32) -> Level {
    if left <= 0.0 {
        Level::Exhausted
    } else if left < 0.20 {
        Level::Low
    } else {
        Level::Normal
    }
}

pub const STALE_AFTER_S: i64 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale { age_s: i64 },
}

pub fn freshness(w: &Window, now: i64) -> Freshness {
    let age_s = now.saturating_sub(w.observed_at).max(0);
    if age_s > STALE_AFTER_S || w.resets_at.is_some_and(|reset| reset < now) {
        Freshness::Stale { age_s }
    } else {
        Freshness::Fresh
    }
}

pub fn claude_log_time(line_timestamp: Option<&str>, log_start_unix: Option<i64>) -> Option<i64> {
    line_timestamp.and_then(parse_timestamp).or(log_start_unix)
}

// RFC 3339 calendar date and offset, converted to Unix seconds without a time dependency.
fn parse_timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    fn number(b: &[u8]) -> Option<i64> {
        b.iter().try_fold(0_i64, |n, c| {
            if c.is_ascii_digit() {
                Some(n * 10 + i64::from(c - b'0'))
            } else {
                None
            }
        })
    }
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let year = number(&b[0..4])?;
    let month = number(&b[5..7])?;
    let day = number(&b[8..10])?;
    let hour = number(&b[11..13])?;
    let minute = number(&b[14..16])?;
    let second = number(&b[17..19])?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut at = 19;
    if b.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
    }
    let offset = match b.get(at..)? {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let h = number(&[*h1, *h2])?;
            let m = number(&[*m1, *m2])?;
            if h > 23 || m > 59 {
                return None;
            }
            (h * 3600 + m * 60) * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    // Gregorian days since 1970-01-01, with March as the first month.
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + minute * 60 + second - offset)
}
