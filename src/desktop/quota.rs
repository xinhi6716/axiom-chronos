//! Quota strip presentation and its minute-scale repaint schedule.
use super::{i18n::{self, Lang, Table}, model::AppState, ornament, theme::{self, Step, Tokened, Tokens, TypeSize}};
use crate::usage::quota::{self, Freshness, Level, Provider, ProviderQuota, Quotas, Window, WindowKind};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::h_flex;
use gpui_kit::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const T: Table = &[
    ("5 hours", "5 小時"), ("weekly", "每週"),
    ("{}% left", "剩 {}%"), ("{}% left · resets {}", "剩 {}% · {} 重置"),
    ("used up", "已用完"), ("used up · resets in {}", "已用完 · {}後重置"),
    ("under 1 min", "不到 1 分"), ("{} h {} min", "{} 小時 {} 分"),
    ("{} min", "{} 分"), ("{} h", "{} 小時"), ("{} d", "{} 天"),
    ("last known · {} · {} ago", "最後已知 · {} · {}前"),
    ("{}% left ({}% used)", "剩 {}%（已用 {}%）"),
    ("{} · observed {}", "{}· 觀測 {}"), ("{} · resets {}", "{} · {} 重置"),
];
const HOURGLASS_SIDE: f32 = 18.0;
const MINUTE: Duration = Duration::from_secs(60);

fn tr(lang: Lang, key: &'static str) -> &'static str { i18n::translate(lang, T, key) }
fn fill(lang: Lang, key: &'static str, args: &[&str]) -> String { i18n::fill(tr(lang, key), args) }

fn window_name(kind: WindowKind, lang: Lang) -> &'static str {
    tr(lang, match kind { WindowKind::FiveHour => "5 hours", WindowKind::Weekly => "weekly" })
}

#[allow(dead_code)]
pub fn head(p: Provider, kind: WindowKind, lang: Lang) -> String {
    let provider = match p { Provider::Claude => "Claude", Provider::Codex => "Codex" };
    format!("{provider} {}{}", window_name(kind, lang), if lang == Lang::ZhTw { "：" } else { ": " })
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

fn countdown(seconds: i64, lang: Lang) -> String {
    let minutes = seconds / 60;
    if minutes == 0 { tr(lang, "under 1 min").to_string() }
    else if minutes >= 60 { fill(lang, "{} h {} min", &[&(minutes / 60).to_string(), &(minutes % 60).to_string()]) }
    else { fill(lang, "{} min", &[&minutes.to_string()]) }
}

#[allow(dead_code)]
pub fn line(w: Option<&Window>, now: i64, utc_offset_min: i32, lang: Lang) -> String {
    let Some(w) = w else { return "—".into() };
    let left = percent(w.left()).to_string();
    let share = || fill(lang, "{}% left", &[&left]);
    if let Freshness::Stale { age_s } = quota::freshness(w, now) {
        return fill(lang, "last known · {} · {} ago", &[&share(), &age(age_s, lang)]);
    }
    if quota::level(w.left()) == Level::Exhausted {
        return match w.resets_at.filter(|&reset| reset > now) {
            Some(reset) => fill(lang, "used up · resets in {}", &[&countdown(reset.saturating_sub(now), lang)]),
            None => tr(lang, "used up").to_string(),
        };
    }
    match w.resets_at {
        Some(reset) => fill(lang, "{}% left · resets {}", &[&left, &clock(reset, now, utc_offset_min)]),
        None => share(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Tone { Gold, Amber, Cinnabar, Muted }

#[allow(dead_code)]
pub fn tone(w: Option<&Window>, now: i64) -> Tone {
    match w {
        None => Tone::Muted,
        Some(w) if matches!(quota::freshness(w, now), Freshness::Stale { .. }) => Tone::Muted,
        Some(w) => match quota::level(w.left()) { Level::Normal => Tone::Gold, Level::Low => Tone::Amber, Level::Exhausted => Tone::Cinnabar },
    }
}

fn ink(t: &Tokens, tone: Tone) -> Hsla {
    match tone { Tone::Gold => t.gold.opacity(0.75), Tone::Amber => t.review, Tone::Cinnabar => t.fail, Tone::Muted => t.muted }
}

#[allow(dead_code)]
pub fn primary(p: &ProviderQuota) -> Option<&Window> {
    match (&p.five_hour, &p.weekly) {
        (Some(a), Some(b)) => Some(if a.left() <= b.left() { a } else { b }),
        (Some(a), None) => Some(a), (None, Some(b)) => Some(b), (None, None) => None,
    }
}

#[allow(dead_code)]
pub fn tooltip(_p: Provider, q: &ProviderQuota, now: i64, utc_offset_min: i32, lang: Lang) -> String {
    [(WindowKind::FiveHour, q.five_hour.as_ref()), (WindowKind::Weekly, q.weekly.as_ref())]
        .into_iter().map(|(kind, w)| {
            let detail = if let Some(w) = w {
                let share = format!("{:.1}", w.left() * 100.0);
                let used = format!("{:.1}", w.used.clamp(0.0, 1.0) * 100.0);
                let figures = fill(lang, "{}% left ({}% used)", &[&share, &used]);
                let observed = fill(lang, "{} · observed {}", &[&figures, &clock(w.observed_at, now, utc_offset_min)]);
                match w.resets_at { Some(reset) => fill(lang, "{} · resets {}", &[&observed, &clock(reset, now, utc_offset_min)]), None => observed }
            } else { "—".to_string() };
            format!("{}{}{}", window_name(kind, lang), if lang == Lang::ZhTw { "：" } else { ": " }, detail)
        }).collect::<Vec<_>>().join("\n")
}

pub struct ProviderView { pub label: String, pub tone: Tone, pub share: Option<f32>, pub tooltip: String }

pub fn provider_view(p: Provider, q: &ProviderQuota, now: i64, utc_offset_min: i32, lang: Lang) -> ProviderView {
    let w = primary(q);
    ProviderView {
        label: format!("{}{}", head(p, w.map_or(WindowKind::FiveHour, |w| w.kind), lang), line(w, now, utc_offset_min, lang)),
        tone: tone(w, now),
        share: w.map(Window::left),
        tooltip: tooltip(p, q, now, utc_offset_min, lang),
    }
}

#[allow(dead_code)]
pub fn next_wake(q: &Quotas) -> Option<Duration> {
    [&q.claude, &q.codex].into_iter().any(|p| p.five_hour.is_some() || p.weekly.is_some()).then_some(MINUTE)
}

pub fn repaint_after(q: &Quotas) -> Option<Duration> { next_wake(q) }

#[derive(Debug, PartialEq, Eq)]
enum SyncAction { Start, Stop, Keep }

fn sync_action(on: bool, running: bool) -> SyncAction {
    match (on, running) {
        (false, _) => SyncAction::Stop,
        (true, false) => SyncAction::Start,
        (true, true) => SyncAction::Keep,
    }
}

#[allow(dead_code)]
pub struct Ticker { task: Option<Task<()>> }
#[allow(dead_code)]
impl Ticker {
    pub fn new() -> Self { Self { task: None } }
    pub fn sync<V: 'static>(&mut self, on: bool, cx: &mut Context<V>) {
        match sync_action(on, self.task.is_some()) {
            SyncAction::Stop => self.task = None,
            SyncAction::Start => self.task = Some(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(MINUTE).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() { break; }
            })),
            SyncAction::Keep => {},
        }
    }
}

#[allow(dead_code)]
pub fn strip(state: &AppState, cx: &App) -> AnyElement {
    let t = cx.tokens();
    let d = theme::density(cx);
    let lang = *cx.global::<Lang>();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().min(i64::MAX as u64) as i64).unwrap_or(0);
    let offset = crate::session::local_offset_secs(now) / 60;
    let provider = |p, q: &ProviderQuota, id: &'static str| {
        let view = provider_view(p, q, now, offset, lang);
        let color = ink(&t, view.tone);
        let tip = view.tooltip;
        h_flex().id(id).min_w_0().gap(theme::space(d, Step::Xs)).text_color(color)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .child(ornament::hourglass(&t, view.share, color, HOURGLASS_SIDE))
            .child(div().min_w_0().truncate().child(view.label))
    };
    h_flex().w_full().h(theme::row_height(d)).min_h(theme::TARGET_MIN)
        .px(theme::space(d, Step::Md)).gap(theme::space(d, Step::Lg))
        .border_t_1().border_color(t.border).bg(t.surface).text_size(theme::font(d, TypeSize::Caption))
        .child(provider(Provider::Claude, &state.quota.claude, "quota-claude"))
        .child(provider(Provider::Codex, &state.quota.codex, "quota-codex"))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    const NOW: i64 = 1_704_067_200; // 2024-01-01 00:00 UTC
    fn window(used: f32, reset: Option<i64>, observed: i64) -> Window {
        Window { kind: WindowKind::FiveHour, used, resets_at: reset, observed_at: observed, status: None }
    }
    fn check(w: Option<&Window>, zh: &str, en: &str) {
        assert_eq!(line(w, NOW, 480, Lang::ZhTw), zh);
        assert_eq!(line(w, NOW, 480, Lang::En), en);
    }

    #[test]
    fn exhausted_with_passed_reset_is_explicit() {
        let exhausted_reset_just_passed = Window {
            kind: WindowKind::FiveHour, used: 1.0, resets_at: Some(NOW - 1), observed_at: NOW, status: None,
        };
        check(Some(&exhausted_reset_just_passed), "最後已知 · 剩 0% · 0 分前", "last known · 0% left · 0 min ago");
        let stale_observation = Window {
            kind: WindowKind::FiveHour, used: 1.0, resets_at: Some(NOW - 1), observed_at: NOW - 2400, status: None,
        };
        check(Some(&stale_observation), "最後已知 · 剩 0% · 40 分前", "last known · 0% left · 40 min ago");

        let above_boundary = window(0.79, None, NOW);
        let below_boundary = window(0.81, None, NOW);
        let at_boundary = window(1.0 - 0.2f32, None, NOW);
        assert_eq!(tone(Some(&above_boundary), NOW), Tone::Gold);
        assert_eq!(tone(Some(&below_boundary), NOW), Tone::Amber);
        assert_eq!(tone(Some(&at_boundary), NOW), Tone::Amber);
    }

    #[test]
    fn clock_stamp_formatting_handles_short_stamps() {
        let today = crate::session::log_stamp(NOW, 0);
        assert_eq!(format_clock_stamp("bad", &today), "—");
        assert_eq!(format_clock_stamp(&crate::session::log_stamp(NOW + 1200, 0), &today), "00:20");
        assert_eq!(format_clock_stamp(&crate::session::log_stamp(NOW + 86400, 0), &today), "1/2 00:00");
    }

    #[test]
    fn lines_cover_fresh_unknown_exhausted_and_time_zones() {
        check(None, "—", "—");
        let mut w = window(0.38, Some(NOW + 1200), NOW);
        check(Some(&w), "剩 62% · 08:20 重置", "62% left · resets 08:20");
        assert_eq!(line(Some(&w), NOW, -300, Lang::ZhTw), "剩 62% · 19:20 重置");
        assert_eq!(line(Some(&w), NOW, -300, Lang::En), "62% left · resets 19:20");
        w.resets_at = None;
        check(Some(&w), "剩 62%", "62% left");
        w.used = 0.9;
        check(Some(&w), "剩 10%", "10% left");
        w.used = 1.0;
        check(Some(&w), "已用完", "used up");
        w.resets_at = Some(NOW + 4800);
        check(Some(&w), "已用完 · 1 小時 20 分後重置", "used up · resets in 1 h 20 min");
        w.resets_at = Some(NOW + 30);
        check(Some(&w), "已用完 · 不到 1 分後重置", "used up · resets in under 1 min");
        w.resets_at = Some(NOW + 300);
        check(Some(&w), "已用完 · 5 分後重置", "used up · resets in 5 min");
        w.used = 0.38;
        w.resets_at = Some(NOW + 86400);
        check(Some(&w), "剩 62% · 1/2 08:00 重置", "62% left · resets 1/2 08:00");
        assert_eq!(head(Provider::Claude, WindowKind::FiveHour, Lang::ZhTw), "Claude 5 小時：");
        assert_eq!(head(Provider::Codex, WindowKind::Weekly, Lang::ZhTw), "Codex 每週：");
        assert_eq!(head(Provider::Claude, WindowKind::FiveHour, Lang::En), "Claude 5 hours: ");
        assert_eq!(head(Provider::Codex, WindowKind::Weekly, Lang::En), "Codex weekly: ");
    }

    #[test]
    fn percent_clamping_and_stale_snapshots_never_restore_share() {
        let mut w = window(0.001, None, NOW);
        check(Some(&w), "剩 99%", "99% left");
        w.used = 0.996;
        check(Some(&w), "剩 1%", "1% left");
        w.used = 0.0;
        check(Some(&w), "剩 100%", "100% left");
        w.used = 0.38; w.observed_at = NOW - 2400; w.resets_at = Some(NOW - 1);
        check(Some(&w), "最後已知 · 剩 62% · 40 分前", "last known · 62% left · 40 min ago");
        assert!(!line(Some(&w), NOW, 480, Lang::ZhTw).contains("重置"));
        assert!(!line(Some(&w), NOW, 480, Lang::En).contains("resets"));
        w.observed_at = NOW - 7200;
        check(Some(&w), "最後已知 · 剩 62% · 2 小時前", "last known · 62% left · 2 h ago");
        w.observed_at = NOW - 172800;
        check(Some(&w), "最後已知 · 剩 62% · 2 天前", "last known · 62% left · 2 d ago");
        w.used = 1.0; w.observed_at = NOW;
        check(Some(&w), "最後已知 · 剩 0% · 0 分前", "last known · 0% left · 0 min ago");
    }

    #[test]
    fn tones_and_primary_selection() {
        let mut w = window(0.38, None, NOW);
        assert_eq!(tone(None, NOW), Tone::Muted);
        assert_eq!(tone(Some(&w), NOW), Tone::Gold);
        w.used = 0.9; assert_eq!(tone(Some(&w), NOW), Tone::Amber);
        w.used = 1.0; assert_eq!(tone(Some(&w), NOW), Tone::Cinnabar);
        w.observed_at = NOW - 901; assert_eq!(tone(Some(&w), NOW), Tone::Muted);
        let mut q = ProviderQuota::default();
        assert!(primary(&q).is_none());
        q.five_hour = Some(window(0.2, None, NOW));
        q.weekly = Some(Window { kind: WindowKind::Weekly, ..window(0.8, None, NOW) });
        assert_eq!(primary(&q).unwrap().kind, WindowKind::Weekly);
        q.weekly.as_mut().unwrap().used = 0.2;
        assert_eq!(primary(&q).unwrap().kind, WindowKind::FiveHour);
        q.five_hour = None;
        assert_eq!(primary(&q).unwrap().kind, WindowKind::Weekly);
    }

    #[test]
    fn repaint_and_tooltip_cover_both_windows() {
        let mut quotas = Quotas::default();
        assert_eq!(repaint_after(&quotas), None);
        quotas.codex.weekly = Some(Window { kind: WindowKind::Weekly, ..window(0.38, Some(NOW + 1200), NOW) });
        assert_eq!(repaint_after(&quotas), Some(Duration::from_secs(60)));
        assert_eq!(tooltip(Provider::Codex, &quotas.codex, NOW, 480, Lang::ZhTw), "5 小時：—\n每週：剩 62.0%（已用 38.0%）· 觀測 08:00 · 08:20 重置");
        assert_eq!(tooltip(Provider::Codex, &quotas.codex, NOW, -300, Lang::En), "5 hours: —\nweekly: 62.0% left (38.0% used) · observed 19:00 · resets 19:20");
        quotas.codex.five_hour = Some(window(0.1, None, NOW - 86400));
        assert_eq!(tooltip(Provider::Codex, &quotas.codex, NOW, 480, Lang::En), "5 hours: 90.0% left (10.0% used) · observed 12/31 08:00\nweekly: 62.0% left (38.0% used) · observed 08:00 · resets 08:20");
    }

    #[test]
    fn ticker_actions_and_minute_wake_rule() {
        for (on, running, expected) in [
            (false, false, SyncAction::Stop), (false, true, SyncAction::Stop),
            (true, false, SyncAction::Start), (true, true, SyncAction::Keep),
        ] { assert_eq!(sync_action(on, running), expected); }

        assert_eq!(MINUTE, Duration::from_secs(60));
        let mut q = Quotas::default();
        assert_eq!(next_wake(&q), None);
        assert_eq!(repaint_after(&q), None);
        for provider in [Provider::Claude, Provider::Codex] {
            for kind in [WindowKind::FiveHour, WindowKind::Weekly] {
                let mut single = Quotas::default();
                let target = match provider { Provider::Claude => &mut single.claude, Provider::Codex => &mut single.codex };
                let w = Some(Window { kind, ..window(0.38, None, NOW) });
                match kind { WindowKind::FiveHour => target.five_hour = w, WindowKind::Weekly => target.weekly = w }
                assert_eq!(next_wake(&single), Some(MINUTE));
                assert_eq!(repaint_after(&single), Some(MINUTE));
                assert!(repaint_after(&single).unwrap() >= MINUTE);
            }
        }
        q.claude.five_hour = Some(window(0.38, None, NOW - 2400));
        assert_eq!(next_wake(&q), Some(MINUTE));
        assert_eq!(repaint_after(&q), Some(MINUTE));
        assert!(repaint_after(&q).unwrap() >= MINUTE);
        q.codex.weekly = Some(Window { kind: WindowKind::Weekly, ..window(0.1, None, NOW) });
        assert_eq!(next_wake(&q), Some(MINUTE));
        assert_eq!(repaint_after(&q), Some(MINUTE));
    }

    #[test]
    fn provider_view_composes_strip_content() {
        let mut q = ProviderQuota::default();
        let view = provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw);
        assert_eq!(view.label, "Claude 5 小時：—");
        assert_eq!(view.tone, Tone::Muted);
        assert_eq!(view.share, None);

        q.five_hour = Some(window(0.38, Some(NOW + 1200), NOW));
        let view = provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw);
        assert_eq!(view.label, "Claude 5 小時：剩 62% · 08:20 重置");
        assert_eq!(view.tone, Tone::Gold);
        assert_eq!(view.share, Some(0.62));
        let en = provider_view(Provider::Claude, &q, NOW, 480, Lang::En);
        assert_eq!(en.label, "Claude 5 hours: 62% left · resets 08:20");

        q.five_hour.as_mut().unwrap().used = 0.9;
        assert_eq!(provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw).tone, Tone::Amber);
        q.five_hour.as_mut().unwrap().used = 1.0;
        let view = provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw);
        assert_eq!(view.tone, Tone::Cinnabar);
        assert_eq!(view.label, "Claude 5 小時：已用完 · 20 分後重置");

        q.five_hour = Some(window(0.38, None, NOW - 2400));
        let view = provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw);
        assert_eq!(view.label, "Claude 5 小時：最後已知 · 剩 62% · 40 分前");
        assert_eq!(view.tone, Tone::Muted);
        assert_eq!(view.share, Some(0.62));
        q.weekly = Some(Window { kind: WindowKind::Weekly, ..window(0.8, None, NOW - 60) });
        let view = provider_view(Provider::Claude, &q, NOW, 480, Lang::ZhTw);
        assert_eq!(view.tooltip, "5 小時：剩 62.0%（已用 38.0%）· 觀測 07:20\n每週：剩 20.0%（已用 80.0%）· 觀測 07:59");
        assert!(view.label.starts_with("Claude 每週："));
        q.five_hour = None;
        let view = provider_view(Provider::Codex, &q, NOW, 480, Lang::ZhTw);
        assert!(view.label.starts_with("Codex 每週："));
        assert!((view.share.unwrap() - 0.2).abs() < 0.000_001);
    }

    #[test]
    fn presentation_does_not_request_frames_or_use_literal_font_sizes() {
        let source = include_str!("quota.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in ["request_animation_frame", "on_next_frame", "text_size(px("] {
            assert!(!source.contains(forbidden), "found {forbidden}");
        }
    }
}
