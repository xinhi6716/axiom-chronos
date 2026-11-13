//! View-free usage, quota and council helpers.
use crate::base::Res;
use crate::desktop::{
    concurrency,
    i18n::{self, Lang, Table},
    model::{Run, RunningModel, Usage},
    ornament::Celestial,
};
use crate::usage::pools::{Pool, PoolStatus, Remaining};
use crate::usage::quota::{self, Freshness, Level, ProviderQuota, Quotas, Window, WindowKind};
use gpui_kit::SharedString;
use std::{
    path::{Path, PathBuf},
    thread::{self, JoinHandle},
};

pub fn warning_texts(run: &Run, lang: Lang) -> Vec<SharedString> {
    run.other_warnings()
        .cloned()
        .chain(run.notes.iter().map(|note| note.text(lang).into()))
        .collect()
}
pub const CONTEXT_WARN: u64 = crate::usage::CONTEXT_WARN;
pub fn log_stamp(unix_secs: i64, offset_secs: i32) -> String {
    crate::session::log_stamp(unix_secs, offset_secs)
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageSession {
    pub wish: SharedString,
    pub session: SharedString,
    pub label: SharedString,
    pub usage: Usage,
    pub complete: bool,
    pub started_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolRemaining {
    Known(u64),
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PoolRow {
    pub id: &'static str,
    pub label: &'static str,
    pub used_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub remaining: PoolRemaining,
    pub exhausted: bool,
}
pub fn pool_rows(statuses: &[PoolStatus]) -> Vec<PoolRow> {
    statuses
        .iter()
        .map(|s| PoolRow {
            id: s.pool.id(),
            label: s.pool.label(),
            used_tokens: s.used_tokens,
            cost_usd: s.cost_usd,
            remaining: match s.remaining {
                Remaining::Known { tokens } => PoolRemaining::Known(tokens),
                Remaining::Unknown => PoolRemaining::Unknown,
            },
            exhausted: s.exhausted_until_ms.is_some(),
        })
        .collect()
}
pub fn unknown_pool_rows() -> Vec<PoolRow> {
    Pool::ALL
        .into_iter()
        .map(|pool| PoolRow {
            id: pool.id(),
            label: pool.label(),
            used_tokens: None,
            cost_usd: None,
            remaining: PoolRemaining::Unknown,
            exhausted: false,
        })
        .collect()
}
pub fn read_pools(home: &Path, now_ms: i64) -> Res<Vec<PoolRow>> {
    crate::usage::pools::status(home, now_ms).map(|statuses| pool_rows(&statuses))
}
const T: Table = &[
    ("Codex parallel {} / {}", "Codex 並行 {} / {}"),
    ("5 hours", "5 小時"),
    ("weekly", "每週"),
    ("{}% left", "剩 {}%"),
    ("{} min", "{} 分"),
    ("{} h", "{} 小時"),
    ("{} d", "{} 天"),
    ("last known · {} · {} ago", "最後已知 · {} · {}前"),
    ("{}% left ({}% used)", "剩 {}%（已用 {}%）"),
    ("{} · observed {}", "{} · 觀測 {}"),
    ("{} · resets {}", "{} · {} 重置"),
    ("used {}", "已用 {}"),
    ("remaining {}", "剩餘 {}"),
    ("unknown", "未知"),
    ("exhausted", "已耗盡"),
];

fn tr(lang: Lang, key: &'static str) -> &'static str {
    i18n::translate(lang, T, key)
}
fn fill(lang: Lang, key: &'static str, args: &[&str]) -> String {
    i18n::fill(tr(lang, key), args)
}

fn window_name(kind: WindowKind, lang: Lang) -> &'static str {
    tr(
        lang,
        match kind {
            WindowKind::FiveHour => "5 hours",
            WindowKind::Weekly => "weekly",
        },
    )
}

// log_stamp already implements local Gregorian calendar arithmetic without a time dependency.
fn clock(at: i64, now: i64, offset_min: i32) -> String {
    let offset = offset_min.saturating_mul(60);
    let stamp = log_stamp(at, offset);
    let today = log_stamp(now, offset);
    format_clock_stamp(&stamp, &today)
}

fn format_clock_stamp(stamp: &str, today: &str) -> String {
    let formatted = || -> Option<String> {
        let time = format!("{}:{}", stamp.get(9..11)?, stamp.get(11..13)?);
        if stamp.get(..8)? == today.get(..8)? {
            Some(time)
        } else {
            let month: u32 = stamp.get(4..6)?.parse().ok()?;
            let day: u32 = stamp.get(6..8)?.parse().ok()?;
            Some(format!("{month}/{day} {time}"))
        }
    };
    formatted().unwrap_or_else(|| "—".into())
}

fn percent(left: f32) -> u32 {
    if left <= 0.0 {
        0
    } else if left >= 1.0 {
        100
    } else {
        ((left * 100.0).round() as u32).clamp(1, 99)
    }
}

fn age(age_s: i64, lang: Lang) -> String {
    let (n, key) = if age_s >= 86400 {
        (age_s / 86400, "{} d")
    } else if age_s >= 3600 {
        (age_s / 3600, "{} h")
    } else {
        (age_s / 60, "{} min")
    };
    fill(lang, key, &[&n.to_string()])
}

fn meter_text(w: Option<&Window>, lang: Lang) -> String {
    match w {
        Some(w) => fill(lang, "{}% left", &[&percent(w.left()).to_string()]),
        None => "—".into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaTone {
    Gold,
    Amber,
    Cinnabar,
    Muted,
}

pub fn pool_tokens(tokens: u64) -> String {
    let n = tokens as f64;
    let (value, suffix) = if tokens >= 1_000_000_000 {
        (n / 1e9, "B")
    } else if tokens >= 1_000_000 {
        (n / 1e6, "M")
    } else if tokens >= 1_000 {
        (n / 1e3, "k")
    } else {
        return format!("{tokens} tok");
    };
    format!("{value:.1}{suffix} tok")
}

pub fn pool_text(status: &PoolRow, lang: Lang) -> String {
    let used = status
        .used_tokens
        .map(pool_tokens)
        .unwrap_or_else(|| "—".into());
    let remaining = match status.remaining {
        PoolRemaining::Known(tokens) => pool_tokens(tokens),
        PoolRemaining::Unknown => tr(lang, "unknown").into(),
    };
    let mut parts = vec![fill(lang, "used {}", &[&used])];
    if let Some(cost) = status.cost_usd {
        parts.push(format!("${cost:.2}"));
    }
    parts.push(fill(lang, "remaining {}", &[&remaining]));
    if status.exhausted {
        parts.push(tr(lang, "exhausted").into());
    }
    parts.join(" · ")
}

pub fn pool_tone(status: &PoolRow) -> QuotaTone {
    if status.exhausted {
        QuotaTone::Cinnabar
    } else if status.used_tokens.is_none() && matches!(status.remaining, PoolRemaining::Unknown) {
        QuotaTone::Muted
    } else {
        QuotaTone::Gold
    }
}

fn tone(w: Option<&Window>, now: i64) -> QuotaTone {
    match w {
        None => QuotaTone::Muted,
        Some(w) if matches!(quota::freshness(w, now), Freshness::Stale { .. }) => QuotaTone::Muted,
        Some(w) => match quota::level(w.left()) {
            Level::Normal => QuotaTone::Gold,
            Level::Low => QuotaTone::Amber,
            Level::Exhausted => QuotaTone::Cinnabar,
        },
    }
}

fn tooltip(w: Option<&Window>, now: i64, utc_offset_min: i32, lang: Lang) -> String {
    let Some(w) = w else { return "—".into() };
    let share = format!("{:.1}", w.left() * 100.0);
    let used = format!("{:.1}", w.used.clamp(0.0, 1.0) * 100.0);
    let figures = fill(lang, "{}% left ({}% used)", &[&share, &used]);
    let observed = fill(
        lang,
        "{} · observed {}",
        &[&figures, &clock(w.observed_at, now, utc_offset_min)],
    );
    let detail = match w.resets_at {
        Some(reset) => fill(
            lang,
            "{} · resets {}",
            &[&observed, &clock(reset, now, utc_offset_min)],
        ),
        None => observed,
    };
    match quota::freshness(w, now) {
        Freshness::Stale { age_s } => format!(
            "{} · {detail}",
            fill(
                lang,
                "last known · {} · {} ago",
                &[&meter_text(Some(w), lang), &age(age_s, lang)]
            )
        ),
        Freshness::Fresh => detail,
    }
}

fn strip_value(w: Option<&Window>, now: i64, lang: Lang, compact: bool) -> String {
    let Some(w) = w else { return "—".into() };
    if matches!(quota::freshness(w, now), Freshness::Stale { .. }) {
        return "—".into();
    }
    let value = format!("{}%", percent(w.left()));
    match (quota::level(w.left()), w.resets_at) {
        (Level::Exhausted, Some(reset)) if reset > now => {
            let remaining = reset.saturating_sub(now).saturating_add(59);
            let time = age(remaining, lang);
            if compact {
                let short = time
                    .replace(" min", "m")
                    .replace(" h", "h")
                    .replace(" d", "d")
                    .replace(' ', "");
                format!("{value} ↻{short}")
            } else {
                match lang {
                    Lang::ZhTw => format!("{value} · {time}後重置"),
                    Lang::En => format!("{value} · resets in {time}"),
                }
            }
        }
        _ => value,
    }
}

fn visible_windows(q: &ProviderQuota, now: i64) -> Vec<(WindowKind, Option<&Window>)> {
    let observed = q.observed_at.or_else(|| {
        [q.five_hour.as_ref(), q.weekly.as_ref()]
            .into_iter()
            .flatten()
            .map(|w| w.observed_at)
            .max()
    });
    let fresh = observed.is_some_and(|at| now.saturating_sub(at) <= quota::STALE_AFTER_S)
        && [q.five_hour.as_ref(), q.weekly.as_ref()]
            .into_iter()
            .flatten()
            .filter(|w| Some(w.observed_at) == observed)
            .all(|w| quota::freshness(w, now) == Freshness::Fresh);
    [
        (WindowKind::FiveHour, q.five_hour.as_ref()),
        (WindowKind::Weekly, q.weekly.as_ref()),
    ]
    .into_iter()
    .enumerate()
    .filter(|(index, (_, w))| {
        !fresh
            || if q.observed_at.is_some() {
                q.present[*index]
            } else {
                w.is_some()
            }
    })
    .map(|(_, w)| w)
    .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuotaRow {
    pub provider: &'static str,
    pub window_id: &'static str,
    pub name: String,
    pub left: Option<f32>,
    pub value: String,
    pub tooltip: String,
    pub tone: QuotaTone,
}
pub fn quota_rows(
    q: &Quotas,
    now: i64,
    utc_offset_min: i32,
    lang: Lang,
    compact: bool,
) -> Vec<QuotaRow> {
    [("Claude", &q.claude), ("Codex", &q.codex)]
        .into_iter()
        .flat_map(|(provider, quota)| {
            visible_windows(quota, now)
                .into_iter()
                .map(move |(kind, w)| QuotaRow {
                    provider,
                    window_id: match kind {
                        WindowKind::FiveHour => "five-hour",
                        WindowKind::Weekly => "weekly",
                    },
                    name: window_name(kind, lang).to_owned(),
                    left: w.map(Window::left),
                    value: strip_value(w, now, lang, compact),
                    tooltip: tooltip(w, now, utc_offset_min, lang),
                    tone: tone(w, now),
                })
        })
        .collect()
}
pub fn concurrency_groups<'a>(
    models: impl Iterator<Item = &'a RunningModel>,
    lang: Lang,
) -> Vec<concurrency::Group> {
    concurrency::groups(models, lang)
}
pub fn parallel_text(used: Option<u64>, cap: Option<u64>, lang: Lang) -> String {
    let word = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_else(|| "—".into());
    fill(lang, "Codex parallel {} / {}", &[&word(used), &word(cap)])
}

pub fn argon_every_round(home: &Path) -> bool {
    crate::council::settings::load(home)
        .unwrap_or_default()
        .argon_every_round
}
pub fn set_argon_every_round(home: &Path, on: bool) -> Res<()> {
    let mut settings = crate::council::settings::load(home).unwrap_or_default();
    settings.argon_every_round = on;
    crate::council::settings::save(home, &settings)
}
pub fn validate_budgets(fable: &str, astra: &str, argon: &str, rounds: &str) -> Result<(), String> {
    use crate::council::budgets::{ARGON_CAP, ASTRA_CAP, FABLE_CAP, ROUNDS};
    crate::council::Budgets::parse(|key| match key {
        FABLE_CAP => Some(fable.to_owned()),
        ASTRA_CAP => Some(astra.to_owned()),
        ARGON_CAP => Some(argon.to_owned()),
        ROUNDS => Some(rounds.to_owned()),
        _ => None,
    })
    .map(|_| ())
    .map_err(|e| e.to_string())
}
pub fn budget_tokens(n: u64) -> String {
    crate::council::budgets::tokens(n)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CouncilMember {
    Astra,
    Fable,
    Argon,
}
pub fn council_member(m: CouncilMember) -> (&'static str, Celestial, usize) {
    match m {
        CouncilMember::Astra => ("Astra", Celestial::Star, 0),
        CouncilMember::Fable => ("Fable", Celestial::Jupiter, 1),
        CouncilMember::Argon => ("Argon", Celestial::Gemini, 2),
    }
}
pub fn spawn_read_pools(
    home: PathBuf,
    now_ms: i64,
    deliver: impl FnOnce(Res<Vec<PoolRow>>) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || deliver(read_pools(&home, now_ms)))
}
pub fn spawn_read_argon(
    home: PathBuf,
    deliver: impl FnOnce(bool) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || deliver(argon_every_round(&home)))
}
pub fn spawn_set_argon(
    home: PathBuf,
    on: bool,
    deliver: impl FnOnce(Res<()>) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || deliver(set_argon_every_round(&home, on)))
}

#[cfg(test)]
#[path = "usage_accept.rs"]
mod accept;
