//! Static quota strip presentation.
use super::{i18n::{self, Lang, Table}, model::AppState, ornament, theme::{self, Step, Tokened, Tokens, TypeSize}};
use crate::usage::quota::{self, Freshness, Level, ProviderQuota, Quotas, Window, WindowKind};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const T: Table = &[
    ("Codex parallel {} / {}", "Codex 並行 {} / {}"),
    ("5 hours", "5 小時"), ("weekly", "每週"),
    ("{}% left", "剩 {}%"),
    ("{} min", "{} 分"), ("{} h", "{} 小時"), ("{} d", "{} 天"),
    ("last known · {} · {} ago", "最後已知 · {} · {}前"),
    ("{}% left ({}% used)", "剩 {}%（已用 {}%）"),
    ("{} · observed {}", "{} · 觀測 {}"), ("{} · resets {}", "{} · {} 重置"),
];

fn tr(lang: Lang, key: &'static str) -> &'static str { i18n::translate(lang, T, key) }
fn fill(lang: Lang, key: &'static str, args: &[&str]) -> String { i18n::fill(tr(lang, key), args) }

fn window_name(kind: WindowKind, lang: Lang) -> &'static str {
    tr(lang, match kind { WindowKind::FiveHour => "5 hours", WindowKind::Weekly => "weekly" })
}

// log_stamp already implements local Gregorian calendar arithmetic without a time dependency.
fn clock(at: i64, now: i64, offset_min: i32) -> String {
    let offset = offset_min.saturating_mul(60);
    let stamp = crate::session::log_stamp(at, offset);
    let today = crate::session::log_stamp(now, offset);
    format_clock_stamp(&stamp, &today)
}

fn format_clock_stamp(stamp: &str, today: &str) -> String {
    let formatted = || -> Option<String> {
        let time = format!("{}:{}", stamp.get(9..11)?, stamp.get(11..13)?);
        if stamp.get(..8)? == today.get(..8)? { Some(time) } else {
            let month: u32 = stamp.get(4..6)?.parse().ok()?;
            let day: u32 = stamp.get(6..8)?.parse().ok()?;
            Some(format!("{month}/{day} {time}"))
        }
    };
    formatted().unwrap_or_else(|| "—".into())
}

fn percent(left: f32) -> u32 {
    if left <= 0.0 { 0 } else if left >= 1.0 { 100 } else { ((left * 100.0).round() as u32).clamp(1, 99) }
}

fn age(age_s: i64, lang: Lang) -> String {
    let (n, key) = if age_s >= 86400 { (age_s / 86400, "{} d") }
        else if age_s >= 3600 { (age_s / 3600, "{} h") }
        else { (age_s / 60, "{} min") };
    fill(lang, key, &[&n.to_string()])
}

fn meter_text(w: Option<&Window>, lang: Lang) -> String {
    match w {
        Some(w) => fill(lang, "{}% left", &[&percent(w.left()).to_string()]),
        None => "—".into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone { Gold, Amber, Cinnabar, Muted }

fn tone(w: Option<&Window>, now: i64) -> Tone {
    match w {
        None => Tone::Muted,
        Some(w) if matches!(quota::freshness(w, now), Freshness::Stale { .. }) => Tone::Muted,
        Some(w) => match quota::level(w.left()) { Level::Normal => Tone::Gold, Level::Low => Tone::Amber, Level::Exhausted => Tone::Cinnabar },
    }
}

fn ink(t: &Tokens, tone: Tone) -> Hsla {
    match tone { Tone::Gold => t.gold.opacity(0.75), Tone::Amber => t.review, Tone::Cinnabar => t.fail, Tone::Muted => t.muted }
}

fn meter_paint(t: &Tokens, w: Option<&Window>, now: i64) -> (Option<f32>, Hsla) {
    (w.map(Window::left), ink(t, tone(w, now)))
}

fn tooltip(w: Option<&Window>, now: i64, utc_offset_min: i32, lang: Lang) -> String {
    let Some(w) = w else { return "—".into() };
    let share = format!("{:.1}", w.left() * 100.0);
    let used = format!("{:.1}", w.used.clamp(0.0, 1.0) * 100.0);
    let figures = fill(lang, "{}% left ({}% used)", &[&share, &used]);
    let observed = fill(lang, "{} · observed {}", &[&figures, &clock(w.observed_at, now, utc_offset_min)]);
    let detail = match w.resets_at {
        Some(reset) => fill(lang, "{} · resets {}", &[&observed, &clock(reset, now, utc_offset_min)]),
        None => observed,
    };
    match quota::freshness(w, now) {
        Freshness::Stale { age_s } => format!("{} · {detail}", fill(lang, "last known · {} · {} ago", &[&meter_text(Some(w), lang), &age(age_s, lang)])),
        Freshness::Fresh => detail,
    }
}

// The shell still calls these hooks; quota changes repaint through state updates.
pub fn repaint_after(_: &Quotas) -> Option<Duration> { None }
pub struct Ticker;
impl Ticker {
    pub fn new() -> Self { Self }
    pub fn sync<V: 'static>(&mut self, _: bool, _: &mut Context<V>) {}
}

pub fn strip(state: &AppState, cx: &App) -> AnyElement { strip_layout(state, cx, false) }
pub fn compact_strip(state: &AppState, cx: &App) -> AnyElement { strip_layout(state, cx, true) }

fn strip_value(w: Option<&Window>, now: i64, lang: Lang, compact: bool) -> String {
    let Some(w) = w else { return "—".into() };
    let value = format!("{}%", percent(w.left()));
    match (quota::level(w.left()), w.resets_at) {
        (Level::Exhausted, Some(reset)) if reset > now => {
            let remaining = reset.saturating_sub(now).saturating_add(59);
            let time = age(remaining, lang);
            if compact {
                let short = time.replace(" min", "m").replace(" h", "h").replace(" d", "d").replace(' ', "");
                format!("{value} ↻{short}")
            }
            else { match lang {
                Lang::ZhTw => format!("{value} · {time}後重置"),
                Lang::En => format!("{value} · resets in {time}"),
            }}
        }
        _ => value,
    }
}

fn strip_layout(state: &AppState, cx: &App, compact: bool) -> AnyElement {
    let t = cx.tokens();
    let d = theme::density(cx);
    let lang = *cx.global::<Lang>();
    let spacing = theme::space(if compact { theme::Density::Dense } else { d }, Step::Xs);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0);
    let offset = crate::session::local_offset_secs(now) / 60;
    let meter = |kind, w: Option<&Window>, id: &'static str| {
        let (share, color) = meter_paint(&t, w, now);
        let tip = tooltip(w, now, offset, lang);
        let name = window_name(kind, lang);
        let value = strip_value(w, now, lang, compact);
        // The hourglass uses the window's ink for its sand as well as its outline.
        let meter_tokens = Tokens { gold: color, ..t };
        h_flex().id(id).flex_none().gap(spacing / 2.).text_color(color)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(ornament::hourglass(&meter_tokens, share, color, f32::from(theme::font(d, TypeSize::Caption))))
            .when(!compact, |this| this.child(name.test_text(name)))
            .child(value.clone().test_text(value))
            .test_support()
    };
    let provider = |name: &'static str, q: &ProviderQuota, id: &'static str, five_id, weekly_id| {
        h_flex().id(id).flex_none().gap(spacing / 2.)
            .child(div().text_color(t.gold.opacity(0.75)).child(name.test_text(name)))
            .child(meter(WindowKind::FiveHour, q.five_hour.as_ref(), five_id))
            .child(div().text_color(t.muted).child("·"))
            .child(meter(WindowKind::Weekly, q.weekly.as_ref(), weekly_id))
            .test_support()
    };
    h_flex().id("quota-strip").w_full().h(theme::row_height(d)).min_h(theme::TARGET_MIN)
        .px(spacing).gap(theme::space(if compact { theme::Density::Dense } else { d }, if compact { Step::Md } else { Step::Xl }))
        .border_t_1().border_color(t.border).bg(t.surface).text_size(theme::font(d, TypeSize::Caption))
        .child(provider("Claude", &state.quota.claude, "quota-claude", "quota-claude-five-hour", "quota-claude-weekly"))
        .child(provider("Codex", &state.quota.codex, "quota-codex", "quota-codex-five-hour", "quota-codex-weekly"))
        .when(!compact, |row| {
            let value = parallel_text(state.codex_slots, super::settings::Store::current(cx).codex_limit().ok(), lang);
            row.child(div().id("codex-parallel").flex_none().text_color(t.muted).child(value.clone().test_text(value)).test_support())
        })
        .test_support().into_any_element()
}

fn parallel_text(used: Option<u64>, cap: Option<u64>, lang: Lang) -> String {
    let word = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_else(|| "—".into());
    fill(lang, "Codex parallel {} / {}", &[&word(used), &word(cap)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn parallel_readout_has_both_languages_and_unknown_counts() {
        assert_eq!(parallel_text(Some(7), Some(48), Lang::ZhTw), "Codex 並行 7 / 48");
        assert_eq!(parallel_text(Some(2), Some(3), Lang::En), "Codex parallel 2 / 3");
        assert_eq!(parallel_text(None, Some(48), Lang::En), "Codex parallel — / 48");
        assert_eq!(parallel_text(None, None, Lang::ZhTw), "Codex 並行 — / —");
    }

    const NOW: i64 = 1_704_067_200;
    fn window(kind: WindowKind, used: f32, reset: Option<i64>, observed: i64) -> Window {
        Window { kind, used, resets_at: reset, observed_at: observed, status: None }
    }

    #[test]
    fn meter_paint_fresh_share_and_ink() {
        for t in theme::both() {
            for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
                let fresh = window(kind, 0.38, None, NOW);
                let (share, color) = meter_paint(&t, Some(&fresh), NOW);
                assert_eq!(share, Some(fresh.left()));
                assert_eq!(color, t.gold.opacity(0.75));
            }
        }
    }

    #[test]
    fn meter_paint_missing_has_no_share() {
        for t in theme::both() {
            assert_eq!(meter_paint(&t, None, NOW).0, None);
            assert_eq!(meter_paint(&t, None, NOW).1, t.muted);
        }
    }

    #[test]
    fn meter_paint_stale_uses_muted_ink() {
        for t in theme::both() {
            for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
                let fresh = window(kind, 0.38, None, NOW);
                let stale = window(kind, 0.38, None, NOW - 2400);
                let color = meter_paint(&t, Some(&stale), NOW).1;
                assert_eq!(color, t.muted);
                assert_ne!(color, meter_paint(&t, Some(&fresh), NOW).1);
            }
        }
    }

    #[test]
    fn meter_paint_exhausted_uses_fail_ink() {
        for t in theme::both() {
            for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
                let fresh = window(kind, 0.38, None, NOW);
                let exhausted = window(kind, 1.0, None, NOW);
                let color = meter_paint(&t, Some(&exhausted), NOW).1;
                assert_eq!(tone(Some(&exhausted), NOW), Tone::Cinnabar);
                assert_eq!(color, t.fail);
                assert_ne!(color, meter_paint(&t, Some(&fresh), NOW).1);
            }
        }
    }

    #[test]
    fn meter_paint_low_uses_review_ink() {
        for t in theme::both() {
            for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
                let low = window(kind, 0.9, None, NOW);
                assert_eq!(meter_paint(&t, Some(&low), NOW).1, t.review);
            }
        }
    }

    #[test]
    fn meter_values_and_tones_for_both_windows() {
        for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
            let fresh = window(kind, 0.38, Some(NOW + 1200), NOW);
            let empty = window(kind, 1.0, Some(NOW + 1200), NOW);
            let full = window(kind, 0.0, None, NOW);
            let low = window(kind, 0.9, None, NOW);
            let stale = window(kind, 0.38, Some(NOW - 1), NOW - 2400);
            for (w, zh, en, expected_tone) in [
                (None, "—", "—", Tone::Muted),
                (Some(&fresh), "剩 62%", "62% left", Tone::Gold),
                (Some(&empty), "剩 0%", "0% left", Tone::Cinnabar),
                (Some(&full), "剩 100%", "100% left", Tone::Gold),
                (Some(&low), "剩 10%", "10% left", Tone::Amber),
                (Some(&stale), "剩 62%", "62% left", Tone::Muted),
            ] {
                assert_eq!((meter_text(w, Lang::ZhTw), meter_text(w, Lang::En), tone(w, NOW)), (zh.into(), en.into(), expected_tone));
            }
        }
        let mut q = ProviderQuota::default();
        for lang in [Lang::ZhTw, Lang::En] {
            assert_eq!([meter_text(q.five_hour.as_ref(), lang), meter_text(q.weekly.as_ref(), lang)], ["—", "—"]);
            q.five_hour = Some(window(WindowKind::FiveHour, 0.0, None, NOW));
            assert_eq!(meter_text(q.weekly.as_ref(), lang), "—");
            q.five_hour = None;
            q.weekly = Some(window(WindowKind::Weekly, 1.0, None, NOW));
            assert_eq!(meter_text(q.five_hour.as_ref(), lang), "—");
            q.weekly = None;
        }
        assert_eq!(percent(0.001), 1);
        assert_eq!(percent(0.999), 99);
    }

    #[test]
    fn stale_tooltip_includes_age_and_times() {
        let w = window(WindowKind::FiveHour, 0.38, Some(NOW - 1), NOW - 2400);
        assert_eq!(tooltip(Some(&w), NOW, 0, Lang::En), "last known · 62% left · 40 min ago · 62.0% left (38.0% used) · observed 12/31 23:20 · resets 12/31 23:59");
        assert_eq!(tooltip(Some(&w), NOW, 0, Lang::ZhTw), "最後已知 · 剩 62% · 40 分前 · 剩 62.0%（已用 38.0%） · 觀測 12/31 23:20 · 12/31 23:59 重置");
        let fresh = window(WindowKind::Weekly, 0.38, Some(NOW + 1200), NOW);
        assert_eq!(tooltip(Some(&fresh), NOW, 480, Lang::En), "62.0% left (38.0% used) · observed 08:00 · resets 08:20");
        assert_eq!(tooltip(None, NOW, 0, Lang::En), "—");
    }

    #[test]
    fn clock_stamp_formatting_handles_short_stamps() {
        let today = crate::session::log_stamp(NOW, 0);
        assert_eq!(format_clock_stamp("bad", &today), "—");
        assert_eq!(format_clock_stamp(&crate::session::log_stamp(NOW + 1200, 0), &today), "00:20");
        assert_eq!(format_clock_stamp(&crate::session::log_stamp(NOW + 86400, 0), &today), "1/2 00:00");
    }

    #[test]
    fn strip_values_include_static_reset_countdown() {
        let exhausted = window(WindowKind::FiveHour, 1.0, Some(NOW + 1200), NOW);
        assert_eq!(strip_value(Some(&exhausted), NOW, Lang::ZhTw, false), "0% · 20 分後重置");
        assert_eq!(strip_value(Some(&exhausted), NOW, Lang::En, false), "0% · resets in 20 min");
        assert_eq!(strip_value(Some(&exhausted), NOW, Lang::En, true), "0% ↻20m");
        assert_eq!(strip_value(None, NOW, Lang::En, false), "—");
        assert_eq!(repaint_after(&Quotas::default()), None);
    }

    #[test]
    fn presentation_does_not_request_frames_or_use_literal_font_sizes() {
        let source = include_str!("quota.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in ["request_animation_frame", "on_next_frame", ".timer(", "text_size(px("] {
            assert!(!source.contains(forbidden), "found {forbidden}");
        }
    }
}
