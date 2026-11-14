//! Static quota strip presentation.
use super::{components, i18n::Lang, model::{AppState, facade::{settings, usage::{self, QuotaRow, QuotaTone}}}, ornament, theme::{self, Step, Tokened, Tokens, TypeSize}};
use super::doctor::adapter_quota;
pub(super) use super::doctor::adapter_quota::unknown_pools;
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::h_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{rc::Rc, time::{Duration, SystemTime, UNIX_EPOCH}};

fn ink(t: &Tokens, tone: QuotaTone) -> Hsla {
    match tone {
        QuotaTone::Gold => t.gold.opacity(theme::superswitch_art::ALPHA_0_75),
        QuotaTone::Amber => t.review,
        QuotaTone::Cinnabar => t.fail,
        QuotaTone::Muted => t.muted,
    }
}

// The shell still calls these hooks; quota changes repaint through state updates.
pub fn repaint_after<T>(_: &T) -> Option<Duration> { None }
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
    let offset = adapter_quota::local_offset_minutes(now);
    let rows = usage::quota_rows(&state.quota, now, offset, lang, compact);
    let meter = |row: &QuotaRow, id: &'static str| {
        let color = ink(&t, row.tone);
        let tip = row.tooltip.clone();
        let name = row.name.clone();
        let value = row.value.clone();
        let meter_tokens = Tokens { gold: color, ..t };
        h_flex().id(id).flex_none().gap(spacing / 2.).text_color(color)
            .tooltip(move |window, cx| components::tooltip("quota-tooltip", tip.clone(), None, window, cx))
            .child(ornament::hourglass(&meter_tokens, row.left, color, f32::from(theme::font(d, TypeSize::Caption))))
            .when(!compact, |this| this.child(name.clone().test_text(name)))
            .child(value.clone().test_text(value))
            .test_support()
    };
    let provider = |name: &'static str, id: &'static str, five_id, weekly_id| {
        h_flex().id(id).flex_none().gap(spacing / 2.)
            .child(div().text_color(t.gold.opacity(theme::superswitch_art::ALPHA_0_75)).child(name.test_text(name)))
            .children(rows.iter().filter(|row| row.provider == name).enumerate().map(|(index, row)| {
                h_flex().gap(spacing / 2.)
                    .when(index>0, |row|row.child(div().text_color(t.muted).child("·")))
                    .child(meter(row, if row.window_id == "five-hour" {five_id} else {weekly_id}))
            }))
            .test_support()
    };
    let groups = usage::concurrency_groups(state.codex_models.iter().chain(&state.live_models),lang);
    let mut value = usage::parallel_text(state.codex_slots, settings::Store::current(cx).codex_limit().ok(), lang);
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
        let mut model_width=Pixels::default();
        let mut prior=None;
        for group in &groups {
            if prior.is_some() { model_width += model_gap; }
            if prior.is_some_and(|p|p!=group.provider) { model_width+=measure("·")+model_gap; }
            model_width += theme::font(d,TypeSize::Caption)+spacing/2.+measure(&group.count.to_string());
            prior=Some(group.provider);
        }
        let mut quota_width=Pixels::default();
        for name in ["Claude", "Codex"] {
            quota_width += measure(name);
            for (index, row) in rows.iter().filter(|row| row.provider == name).enumerate() {
                quota_width += spacing + theme::font(d, TypeSize::Caption) + measure(&row.value);
                if index > 0 { quota_width += spacing + measure("·"); }
            }
        }
        let available=window.viewport_size().width-spacing*2.-theme::ornament_tokens::DIVIDER_LINE_HEIGHT*2.;
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
            .tooltip(move |window,cx|components::tooltip("model-tooltip", tip.clone(), None, window, cx))
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
    let pools = adapter_quota::pool_rows(cx);
    h_flex().id("quota-strip").relative().w_full().h(theme::row_height(d)).min_h(theme::TARGET_MIN)
        .px(spacing).gap(gap)
        .border_t_1().border_color(t.border).bg(t.surface).text_size(theme::font(d,TypeSize::Caption))
        .children(current.map(|current| div().absolute().top(-theme::ornament_tokens::DIVIDER_LINE_HEIGHT).left_0().right_0().child(current)))
        .when(show_quotas,|row|row
            .child(provider("Claude", "quota-claude", "quota-claude-five-hour", "quota-claude-weekly"))
            .child(provider("Codex", "quota-codex", "quota-codex-five-hour", "quota-codex-weekly")))
        .when(!compact, |row| row.children(pools.iter().map(|status| {
            let label = status.label;
            let value = usage::pool_text(status, lang);
            let id = format!("quota-pool-{}", status.id);
            h_flex().id(SharedString::from(id)).flex_none().gap(spacing / 2.)
                .child(div().text_color(t.gold.opacity(theme::superswitch_art::ALPHA_0_75)).child(label.test_text(label)))
                .child(div().text_color(ink(&t, usage::pool_tone(status))).child(value.clone().test_text(value)))
                .test_support()
        })))
        .when(show_models,|row|row.child(model_row))
        .when(show_parallel, |row| row.child(div().id("codex-parallel").flex_none().text_color(t.muted).child(value.clone().test_text(value)).test_support()))
        .children(notice)
        .children(toggle)
        .test_support().into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn pool_readout_is_honest_and_localized() {
        let mut row = usage::unknown_pool_rows().remove(0);
        assert_eq!(usage::pool_text(&row, Lang::En), "used — · remaining unknown");
        assert_eq!(usage::pool_text(&row, Lang::ZhTw), "已用 — · 剩餘 未知");
        assert_eq!(usage::pool_tone(&row), QuotaTone::Muted);
        row.used_tokens = Some(12_300);
        row.cost_usd = Some(1.25);
        row.remaining = usage::PoolRemaining::Known(42_000);
        row.exhausted = true;
        assert_eq!(usage::pool_text(&row, Lang::En), "used 12.3k tok · $1.25 · remaining 42.0k tok · exhausted");
        assert_eq!(usage::pool_tone(&row), QuotaTone::Cinnabar);
    }

    #[test]
    fn fresh_snapshots_hide_absent_windows_but_stale_keeps_unknowns() {
        use crate::usage::quota::{self, Quotas};
        const NOW: i64 = 1_704_067_200;
        for (minutes, id) in [(300, "five-hour"), (10080, "weekly")] {
            let mut q = Quotas::default();
            q.codex = quota::parse_codex_read(&serde_json::json!({"rateLimits":{"primary":{"windowDurationMins":minutes,"usedPercent":42}}}), NOW);
            let rows = usage::quota_rows(&q, NOW, 0, Lang::En, false);
            let codex: Vec<_> = rows.iter().filter(|r| r.provider == "Codex").collect();
            assert_eq!(codex.len(), 1);
            assert_eq!(codex[0].window_id, id);
            let stale = usage::quota_rows(&q, NOW + quota::STALE_AFTER_S + 1, 0, Lang::En, false);
            let codex: Vec<_> = stale.iter().filter(|r| r.provider == "Codex").collect();
            assert_eq!(codex.len(), 2);
            let last = codex.iter().find(|r| r.window_id == id).unwrap();
            assert_eq!(last.value, "—");
            assert!(last.tooltip.starts_with("last known"));
            assert_eq!(last.tone, QuotaTone::Muted);
        }
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
    fn presentation_does_not_request_frames_or_use_literal_font_sizes() {
        let source = include_str!("quota.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in ["request_animation_frame", "on_next_frame", ".timer(", "text_size(px("] {
            assert!(!source.contains(forbidden), "found {forbidden}");
        }
    }
}
