//! Static quota strip presentation.
use super::{concurrency, i18n::{self, Lang, Table}, model::AppState, ornament, theme::{self, Step, Tokened, Tokens, TypeSize}};
use crate::usage::quota::{self, Freshness, Level, ProviderQuota, Quotas, Window, WindowKind};
use crate::usage::pools::{Pool, PoolStatus, PoolStrip, Remaining};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{rc::Rc, time::{Duration, SystemTime, UNIX_EPOCH}};

const T: Table = &[
    ("Codex parallel {} / {}", "Codex 並行 {} / {}"),
    ("5 hours", "5 小時"), ("weekly", "每週"),
    ("{}% left", "剩 {}%"),
    ("{} min", "{} 分"), ("{} h", "{} 小時"), ("{} d", "{} 天"),
    ("last known · {} · {} ago", "最後已知 · {} · {}前"),
    ("{}% left ({}% used)", "剩 {}%（已用 {}%）"),
    ("{} · observed {}", "{} · 觀測 {}"), ("{} · resets {}", "{} · {} 重置"),
    ("used {}", "已用 {}"), ("remaining {}", "剩餘 {}"),
    ("unknown", "未知"), ("exhausted", "已耗盡"),
];

impl Global for PoolStrip {}

pub(super) fn unknown_pools() -> Vec<PoolStatus> {
    Pool::ALL.into_iter().map(|pool| PoolStatus {
        pool, used_tokens: None, cost_usd: None, remaining: Remaining::Unknown, exhausted_until_ms: None,
    }).collect()
}

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

fn pool_tokens(tokens: u64) -> String {
    let n = tokens as f64;
    let (value, suffix) = if tokens >= 1_000_000_000 { (n / 1e9, "B") }
        else if tokens >= 1_000_000 { (n / 1e6, "M") }
        else if tokens >= 1_000 { (n / 1e3, "k") }
        else { return format!("{tokens} tok") };
    format!("{value:.1}{suffix} tok")
}

fn pool_text(status: &PoolStatus, lang: Lang) -> String {
    let used = status.used_tokens.map(pool_tokens).unwrap_or_else(|| "—".into());
    let remaining = match status.remaining {
        Remaining::Known { tokens } => pool_tokens(tokens),
        Remaining::Unknown => tr(lang, "unknown").into(),
    };
    let mut parts = vec![fill(lang, "used {}", &[&used])];
    if let Some(cost) = status.cost_usd { parts.push(format!("${cost:.2}")); }
    parts.push(fill(lang, "remaining {}", &[&remaining]));
    if status.exhausted_until_ms.is_some() { parts.push(tr(lang, "exhausted").into()); }
    parts.join(" · ")
}

fn pool_tone(status: &PoolStatus) -> Tone {
    if status.exhausted_until_ms.is_some() { Tone::Cinnabar }
    else if status.used_tokens.is_none() && matches!(status.remaining, Remaining::Unknown) { Tone::Muted }
    else { Tone::Gold }
}

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

pub fn strip(state: &AppState, cx: &App) -> AnyElement { strip_layout(state, cx, None, None) }
pub fn compact_strip(state: &AppState, window: &gpui_kit::Window, cx: &App) -> AnyElement { strip_layout(state, cx, Some(window), None) }
pub fn strip_with(state: &Entity<AppState>, flash: f32, cx: &App) -> AnyElement {
    strip_layout(state.read(cx), cx, None, Some((state, flash)))
}
pub fn compact_strip_with(state: &Entity<AppState>, flash: f32, window: &gpui_kit::Window, cx: &App) -> AnyElement {
    strip_layout(state.read(cx), cx, Some(window), Some((state, flash)))
}

fn strip_value(w: Option<&Window>, now: i64, lang: Lang, compact: bool) -> String {
    let Some(w) = w else { return "—".into() };
    if matches!(quota::freshness(w, now), Freshness::Stale { .. }) { return "—".into(); }
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

fn visible_windows(q: &ProviderQuota, now: i64) -> Vec<(WindowKind, Option<&Window>)> {
    let observed = q.observed_at.or_else(|| [q.five_hour.as_ref(),q.weekly.as_ref()].into_iter().flatten().map(|w|w.observed_at).max());
    let fresh = observed.is_some_and(|at| now.saturating_sub(at)<=quota::STALE_AFTER_S)
        && [q.five_hour.as_ref(),q.weekly.as_ref()].into_iter().flatten()
            .filter(|w|Some(w.observed_at)==observed).all(|w|quota::freshness(w,now)==Freshness::Fresh);
    [(WindowKind::FiveHour,q.five_hour.as_ref()),(WindowKind::Weekly,q.weekly.as_ref())].into_iter()
        .enumerate().filter(|(index,(_,w))| !fresh || if q.observed_at.is_some() {q.present[*index]} else {w.is_some()})
        .map(|(_,w)|w).collect()
}

// The two quota providers occupy separate children, hence their extra gap.
fn compact_fit(available: f32, quotas: f32, models: Option<f32>, parallel: f32, toggle: Option<f32>, notice: Option<f32>, gap: f32) -> (bool, bool, bool) {
    let mut total = parallel;
    if let Some(width) = toggle { total += gap + width; }
    if let Some(width) = notice { total += gap + width; }
    if toggle.is_some() && total > available { return (false, false, false); }
    let show_models = models.is_some_and(|width| width + gap + total <= available);
    let show_quotas = quotas + gap * 2. + total + if show_models { models.unwrap_or(0.) + gap } else { 0. } <= available
        && (models.is_none() || show_models);
    (show_quotas, show_models, true)
}

fn strip_layout(state: &AppState, cx: &App, mini: Option<&gpui_kit::Window>, controls: Option<(&Entity<AppState>, f32)>) -> AnyElement {
    let compact = mini.is_some();
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
        let windows = visible_windows(q, now);
        h_flex().id(id).flex_none().gap(spacing / 2.)
            .child(div().text_color(t.gold.opacity(0.75)).child(name.test_text(name)))
            .children(windows.into_iter().enumerate().map(|(index,(kind,w))| {
                h_flex().gap(spacing / 2.)
                    .when(index>0, |row|row.child(div().text_color(t.muted).child("·")))
                    .child(meter(kind,w,if kind==WindowKind::FiveHour {five_id} else {weekly_id}))
            }))
            .test_support()
    };
    let groups = concurrency::groups(state.codex_models.iter().chain(&state.live_models),lang);
    let mut value = parallel_text(state.codex_slots, super::settings::Store::current(cx).codex_limit().ok(), lang);
    if compact { value = value.replace("Codex ",""); }
    let gap = theme::space(if compact {theme::Density::Dense} else {d},Step::Md);
    let model_gap = theme::space(d,Step::Xs);
    let space = controls.and_then(|_| state.ux.seams.storage.as_ref().and_then(|s| s.space()));
    let low_space = space.filter(super::doctor::storage::is_low);
    let (show_quotas,show_models,show_parallel) = if let Some(window)=mini {
        let measure_at = |text: &str, font_size| {
            let run = TextRun { len:text.len(),font:window.text_style().font(),color:t.text,background_color:None,underline:None,strikethrough:None };
            window.text_system().shape_line(text.to_owned().into(),font_size,&[run],None).width
        };
        let measure = |text: &str| measure_at(text, theme::font(d,TypeSize::Caption));
        let mut model_width=px(0.);
        let mut prior=None;
        for group in &groups {
            if prior.is_some() { model_width += model_gap; }
            if prior.is_some_and(|p|p!=group.provider) { model_width+=measure("·")+model_gap; }
            model_width += theme::font(d,TypeSize::Caption)+spacing/2.+measure(&group.count.to_string());
            prior=Some(group.provider);
        }
        let mut quota_width=px(0.);
        for (name,q) in [("Claude",&state.quota.claude),("Codex",&state.quota.codex)] {
            quota_width+=measure(name);
            for (index,(_,w)) in visible_windows(q,now).into_iter().enumerate() {
                quota_width+=spacing+theme::font(d,TypeSize::Caption)+measure(&strip_value(w,now,lang,true));
                if index>0 { quota_width+=spacing+measure("·"); }
            }
        }
        let available=window.viewport_size().width-spacing*2.-px(2.);
        let notice_width = low_space.map(|space| {
            let bytes = super::doctor::storage::bytes_text(space.free_bytes, lang);
            let notice = match lang { Lang::En => format!("Low disk: {bytes} free"), Lang::ZhTw => format!("磁碟空間不足：剩 {bytes}") };
            f32::from(measure_at(&notice, theme::font(d, TypeSize::Body)))
        });
        compact_fit(f32::from(available), f32::from(quota_width),
            (!groups.is_empty()).then(|| f32::from(model_width)), f32::from(measure(&value)),
            controls.map(|_| f32::from(theme::font(d, TypeSize::Title))), notice_width, f32::from(gap))
    } else { (true,!groups.is_empty(),true) };
    let mut model_row=h_flex().id("running-models").flex_none().gap(model_gap).test_support();
    let mut prior=None;
    for (index,group) in groups.into_iter().enumerate() {
        if prior.is_some_and(|p|p!=group.provider) { model_row=model_row.child(div().text_color(t.muted).child("·".test_text("·"))); }
        prior=Some(group.provider);
        let tip=group.tooltip;
        let count=group.count.to_string();
        model_row=model_row.child(h_flex().id(("running-model",index)).flex_none().gap(spacing/2.)
            .tooltip(move |window,cx|Tooltip::new(tip.clone()).build(window,cx))
            .child(ornament::celestial(&t,group.glyph,f32::from(theme::font(d,TypeSize::Caption))))
            .child(div().text_color(t.text).child(count.clone().test_text(count))).test_support());
    }
    let (toggle, current, notice) = if let Some((entity, flash)) = controls {
        let on = super::doctor::extreme::extreme_on(cx);
        let toggle_state = entity.clone();
        let open_state = entity.clone();
        (Some(super::doctor::extreme::strip_toggle(on, flash, lang, &t, d,
            Rc::new(move |on, _, cx| toggle_state.update(cx, |s, cx| s.set_extreme(on, cx))))),
         super::doctor::extreme::strip_current(on, flash, &t),
         super::doctor::storage::low_disk_notice(space, lang, &t, d,
            Rc::new(move |_, cx| open_state.update(cx, |s, cx| s.open_page(super::ux::Page::Storage, cx)))))
    } else { (None, None, None) };
    let fallback = unknown_pools();
    let pools = cx.try_global::<PoolStrip>().map(|strip| strip.0.as_slice()).unwrap_or(&fallback);
    h_flex().id("quota-strip").relative().w_full().h(theme::row_height(d)).min_h(theme::TARGET_MIN)
        .px(spacing).gap(gap)
        .border_t_1().border_color(t.border).bg(t.surface).text_size(theme::font(d,TypeSize::Caption))
        .children(current.map(|current| div().absolute().top(px(-1.)).left_0().right_0().child(current)))
        .when(show_quotas,|row|row
            .child(provider("Claude", &state.quota.claude, "quota-claude", "quota-claude-five-hour", "quota-claude-weekly"))
            .child(provider("Codex", &state.quota.codex, "quota-codex", "quota-codex-five-hour", "quota-codex-weekly")))
        .when(!compact, |row| row.children(pools.iter().map(|status| {
            let label = status.pool.label();
            let value = pool_text(status, lang);
            let id = match status.pool { Pool::HydrogenFlash => "quota-pool-hydrogen_flash", Pool::Argon => "quota-pool-argon" };
            h_flex().id(id).flex_none().gap(spacing / 2.)
                .child(div().text_color(t.gold.opacity(0.75)).child(label.test_text(label)))
                .child(div().text_color(ink(&t, pool_tone(status))).child(value.clone().test_text(value)))
                .test_support()
        })))
        .when(show_models,|row|row.child(model_row))
        .when(show_parallel, |row| row.child(div().id("codex-parallel").flex_none().text_color(t.muted).child(value.clone().test_text(value)).test_support()))
        .children(notice)
        .children(toggle)
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
    fn pool_readout_is_honest_and_localized() {
        use crate::usage::pools::{Pool, PoolStatus, Remaining};
        let mut status = PoolStatus { pool: Pool::HydrogenFlash, used_tokens: None, cost_usd: None, remaining: Remaining::Unknown, exhausted_until_ms: None };
        assert_eq!(pool_text(&status, Lang::En), "used — · remaining unknown");
        assert_eq!(pool_text(&status, Lang::ZhTw), "已用 — · 剩餘 未知");
        assert_eq!(pool_tone(&status), Tone::Muted);
        status.used_tokens = Some(0);
        status.remaining = Remaining::Known { tokens: 0 };
        assert_eq!(pool_text(&status, Lang::En), "used 0 tok · remaining 0 tok");
        status.used_tokens = Some(12_300);
        status.cost_usd = Some(1.25);
        status.remaining = Remaining::Known { tokens: 42_000 };
        assert_eq!(pool_text(&status, Lang::En), "used 12.3k tok · $1.25 · remaining 42.0k tok");
        status.exhausted_until_ms = Some(1);
        assert_eq!(pool_text(&status, Lang::En), "used 12.3k tok · $1.25 · remaining 42.0k tok · exhausted");
        assert_eq!(pool_text(&status, Lang::ZhTw), "已用 12.3k tok · $1.25 · 剩餘 42.0k tok · 已耗盡");
        assert_eq!(pool_tone(&status), Tone::Cinnabar);
    }

    #[test]
    fn compact_budget_preserves_controls_before_readouts() {
        let fit = |available| compact_fit(available, 60., Some(30.), 20., Some(10.), Some(15.), 5.);
        assert_eq!(fit(160.), (true, true, true));
        assert_eq!(fit(159.), (false, true, true));
        assert_eq!(fit(90.), (false, true, true));
        assert_eq!(fit(89.), (false, false, true));
        assert_eq!(fit(55.), (false, false, true));
        // Only the toggle and notice (including their gap) fit here.
        assert_eq!(fit(30.), (false, false, false));
        assert_eq!(compact_fit(35., 60., Some(30.), 20., Some(10.), None, 5.), (false, false, true));
        assert_eq!(compact_fit(34., 60., Some(30.), 20., Some(10.), None, 5.), (false, false, false));
    }

    #[test]
    fn compact_budget_without_controls_keeps_previous_behavior() {
        let fit = |available| compact_fit(available, 60., Some(30.), 20., None, None, 5.);
        assert_eq!(fit(125.), (true, true, true));
        assert_eq!(fit(124.), (false, true, true));
        assert_eq!(fit(54.), (false, false, true));
        assert_eq!(fit(0.), (false, false, true));
        assert_eq!(compact_fit(90., 60., None, 20., None, None, 5.), (true, false, true));
        assert_eq!(compact_fit(89., 60., None, 20., None, None, 5.), (false, false, true));
    }

    #[test]
    fn fresh_snapshots_hide_absent_windows_for_both_providers_but_stale_and_unseen_keep_unknowns() {
        let codex = |mins| quota::parse_codex_read(&serde_json::json!({"rateLimits":{"primary":{"windowDurationMins":mins,"usedPercent":42}}}), NOW);
        for (q,kind) in [(codex(10080),WindowKind::Weekly),(codex(300),WindowKind::FiveHour)] {
            let windows=visible_windows(&q,NOW);
            assert_eq!(windows.len(),1); assert_eq!(windows[0].0,kind);
            assert_eq!(visible_windows(&q,NOW+quota::STALE_AFTER_S+1).len(),2);
            assert_eq!(strip_value(windows[0].1,NOW+quota::STALE_AFTER_S+1,Lang::En,false),"—");
            assert!(tooltip(windows[0].1,NOW+quota::STALE_AFTER_S+1,0,Lang::En).starts_with("last known"));
        }
        let claude=quota::parse_claude_event(&serde_json::json!({"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.2,"resetsAt":NOW+500,"status":"allowed"}}}}),NOW).unwrap();
        assert_eq!(visible_windows(&claude,NOW).iter().map(|w|w.0).collect::<Vec<_>>(),vec![WindowKind::FiveHour]);
        assert_eq!(visible_windows(&ProviderQuota::default(),NOW).len(),2);
        let mut merged=Quotas::default();
        merged.merge(quota::Provider::Codex,codex(300));
        let newer=quota::parse_codex_read(&serde_json::json!({"rateLimits":{"secondary":{"windowDurationMins":10080,"usedPercent":30}}}),NOW+1);
        merged.merge(quota::Provider::Codex,newer);
        assert_eq!(visible_windows(&merged.codex,NOW+1).iter().map(|w|w.0).collect::<Vec<_>>(),vec![WindowKind::Weekly]);
        assert!(merged.codex.five_hour.is_some(),"last known window is retained for stale tooltips");
        let same_second=quota::parse_codex_read(&serde_json::json!({"rateLimits":{"primary":{"windowDurationMins":300,"usedPercent":20}}}),NOW+1);
        merged.merge(quota::Provider::Codex,same_second);
        assert_eq!(visible_windows(&merged.codex,NOW+1).iter().map(|w|w.0).collect::<Vec<_>>(),vec![WindowKind::FiveHour]);
    }

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
