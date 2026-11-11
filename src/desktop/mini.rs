//! Compact view of unfinished work; window switching is owned by the shell.
use super::{i18n::{self, tr, Lang, Table}, model::{self, AppState}, ornament::{self, Emblem}, palette, quota, settings::{self, SettingPrior, WindowMode}, shell, theme::{self, Step, Tokened as _, TypeSize}, wishes};
use crate::base::{Failure, Res};
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::{h_flex, v_flex, Icon, TitleBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;

const MINI_WIDTH: f32 = 380.;
const MINI_HEIGHT: f32 = 520.;
const T: Table = &[
    ("Chronos", "Chronos"),
    ("Keep on top", "保持在最上層"),
    ("Stop keeping on top", "取消保持在最上層"),
    ("Nothing is in progress", "目前沒有進行中的願望"),
    ("{} wish in progress", "{} 個願望進行中"),
    ("{} wishes in progress", "{} 個願望進行中"),
    ("{}, {} needs you", "{}，{} 個需要你"),
    ("{}, {} need you", "{}，{} 個需要你"),
    ("Needs you", "需要你"),
    ("Back to full app", "返回完整應用程式"),
    ("Back to full app ({})", "返回完整應用程式（{}）"),
];

#[derive(Debug, PartialEq, Eq)]
struct MiniRow { session: SharedString, needs_you: bool }

fn rows(state: &AppState) -> Vec<MiniRow> {
    model::board_runs(state).into_iter().map(|i| {
        let run = &state.runs[i];
        MiniRow { session: run.session.clone(), needs_you: model::needs_you(run) }
    }).collect()
}

fn summary_line(open: usize, needs: usize, lang: Lang) -> String {
    if open == 0 { return i18n::translate(lang, T, "Nothing is in progress").into(); }
    let key = if open == 1 { "{} wish in progress" } else { "{} wishes in progress" };
    let line = i18n::fill(i18n::translate(lang, T, key), &[&open.to_string()]);
    if needs == 0 { line } else {
        let key = if needs == 1 { "{}, {} needs you" } else { "{}, {} need you" };
        i18n::fill(i18n::translate(lang, T, key), &[&line, &needs.to_string()])
    }
}

fn back_tip_text(keys: &str, lang: Lang) -> String {
    if keys.is_empty() { i18n::translate(lang, T, "Back to full app").into() }
    else { i18n::fill(i18n::translate(lang, T, "Back to full app ({})"), &[keys]) }
}

fn kind_for(on_top: bool) -> WindowKind {
    if on_top { WindowKind::PopUp } else { WindowKind::Normal }
}

pub fn open_mini(state: Entity<AppState>, on_top: bool, cx: &mut App) -> Res<AnyWindowHandle> {
    open_mini_at(state, on_top, false, cx)
}

pub fn open_mini_at(state: Entity<AppState>, on_top: bool, offscreen: bool, cx: &mut App) -> Res<AnyWindowHandle> {
    let window_size = size(px(MINI_WIDTH), px(MINI_HEIGHT));
    let bounds = if offscreen { Bounds { origin: point(px(-3000.), px(-3000.)), size: window_size } }
        else { Bounds::centered(None, window_size, cx) };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(320.), px(400.))),
        titlebar: Some(TitleBar::title_bar_options()),
        focus: !offscreen,
        kind: kind_for(on_top),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MiniView::new(state, window, cx))
    }).map(|(handle, _)| handle).map_err(|error| Failure::new("window_open", error.to_string()))
}

fn back_transition() -> (WindowMode, SettingPrior) { (WindowMode::Full, SettingPrior::Mode(WindowMode::Full)) }
fn pin_prior(on: bool) -> SettingPrior { SettingPrior::MiniOnTop(!on) }

pub(super) fn back_to_full(state: &Entity<AppState>, cx: &mut App) {
    let (mode, next) = back_transition();
    let prior = settings::set_mode(mode, cx);
    shell::record_change(state, prior, next, cx);
}

fn toggle_pin(state: &Entity<AppState>, on: bool, cx: &mut App) {
    let prior = settings::set_mini_on_top(!on, cx);
    shell::record_change(state, prior, pin_prior(on), cx);
}

fn open_card(state: &Entity<AppState>, session: &SharedString, cx: &mut App) {
    let opened = state.update(cx, |state, cx| {
        if let Some(index) = state.runs.iter().position(|run| run.session == *session) {
            state.open_run(index, cx);
            true
        } else { false }
    });
    if opened { back_to_full(state, cx); }
}

pub struct MiniView {
    state: Entity<AppState>,
    focus: FocusHandle,
    pin_focus: FocusHandle,
    back_focus: FocusHandle,
    card_focus: HashMap<SharedString, FocusHandle>,
}
impl MiniView {
    fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let closing_state = state.clone();
        window.on_window_should_close(cx, move |_, cx| {
            back_to_full(&closing_state, cx);
            true
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self { state, focus, pin_focus: cx.focus_handle().tab_stop(true), back_focus: cx.focus_handle().tab_stop(true), card_focus: HashMap::new() }
    }
}

impl Render for MiniView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let d = theme::density(cx);
        let lang = *cx.global::<Lang>();
        let state = self.state.read(cx);
        let list = rows(state);
        let indices = model::board_runs(state);
        self.card_focus.retain(|session, _| list.iter().any(|row| row.session == *session));
        let on = settings::Store::current(cx).mini_on_top;
        let pin_tip = if on { tr(cx, T, "Stop keeping on top") } else { tr(cx, T, "Keep on top") };
        let pin_state = self.state.clone();
        let close_state = self.state.clone();
        let back_state = self.state.clone();
        let mut content = v_flex().w_full().gap(theme::space(d, Step::Sm))
            .child(div().text_size(theme::font(d, TypeSize::Body)).text_color(t.muted)
                .child(summary_line(list.len(), list.iter().filter(|r| r.needs_you).count(), lang)));
        for (position, index) in indices.into_iter().enumerate() {
            let run = &state.runs[index];
            let focus = self.card_focus.entry(run.session.clone()).or_insert_with(|| cx.focus_handle().tab_stop(true)).clone();
            let ring = focus.is_focused(window);
            let ink = t.outcome(run.status, run.verified_end());
            let line = super::run_view::board::compact_line(state, index, cx);
            let session = run.session.clone();
            let click_session = session.clone();
            let click_state = self.state.clone();
            let key_state = self.state.clone();
            content = content.child(v_flex().id(("mini-card", position)).test_support().track_focus(&focus)
                .w_full().min_w_0().gap(theme::space(d, Step::Xs)).p(theme::space(d, Step::Sm))
                .rounded(theme::RADIUS).border_1().border_color(t.border).bg(t.raised).cursor_pointer()
                .when(ring, |this| this.border_2().border_color(t.gold).shadow(theme::focus_ring(&t)))
                .on_click(move |_, _, cx| open_card(&click_state, &click_session, cx))
                .on_key_down(move |event, _, cx| { if event.keystroke.key == "enter" { cx.stop_propagation(); open_card(&key_state, &session, cx); } })
                .child(h_flex().w_full().min_w_0().gap(theme::space(d, Step::Sm))
                    .child(ornament::glyph(wishes::glyph_state(run.status, run.verified_end()), f32::from(theme::font(d, TypeSize::Heading)), ink, 0.0))
                    .child(div().flex_1().min_w_0().truncate().child(run.project.clone().test_text(run.project.clone())))
                    .child(div().text_color(ink).text_size(theme::font(d, TypeSize::Caption)).child(tr(cx, &[], run.status_label()).test_text(tr(cx, &[], run.status_label())))))
                .child(div().w_full().text_color(t.muted).text_size(theme::font(d, TypeSize::Caption)).child(line.clone().test_text(line))));
        }
        let strip = quota::compact_strip(state, window, cx);
        let back_tip = back_tip_text(palette::keys_of(palette::Global::ToggleMini), lang);
        let pin_ring = self.pin_focus.is_focused(window);
        let back_ring = self.back_focus.is_focused(window);
        let pin_key_state = self.state.clone();
        let back_key_state = self.state.clone();
        v_flex().track_focus(&self.focus).size_full().bg(t.canvas).text_color(t.text)
            .child(TitleBar::new().on_close_window(move |_, window, cx| { back_to_full(&close_state, cx); window.remove_window(); })
                .child(h_flex().w_full().items_center().gap(theme::space(d, Step::Sm))
                    .child(ornament::emblem(Emblem::Hourglass, f32::from(theme::font(d, TypeSize::Title)), t.gold))
                    .child(div().text_size(theme::font(d, TypeSize::Title)).text_color(t.gold).child(tr(cx, T, "Chronos")))
                    .child(div().flex_1())
                    .child(div().id("mini-pin").test_support().track_focus(&self.pin_focus).cursor_pointer().px(theme::space(d, Step::Sm)).border_2().border_color(if pin_ring { t.gold } else { t.canvas }).when(pin_ring, |this| this.shadow(theme::focus_ring(&t))).text_color(if on { t.gold } else { t.muted })
                        .tooltip(move |window, cx| Tooltip::new(pin_tip).build(window, cx))
                        .on_click(move |_, _, cx| toggle_pin(&pin_state, on, cx))
                        .on_key_down(move |event, _, cx| { if event.keystroke.key == "enter" { cx.stop_propagation(); toggle_pin(&pin_key_state, on, cx); } })
                        .child(Icon::new(gpui_kit::assets::IconName::Pin).size(theme::font(d, TypeSize::Title))))))
            .child(div().id("mini-rule").w_full().h(px(1.)).bg(t.gold))
            .child(div().id("mini-body").flex_1().min_h_0().overflow_y_scroll().p(theme::space(d, Step::Md)).child(content))
            .child(div().px(theme::space(d, Step::Md)).pb(theme::space(d, Step::Sm))
                .child(div().id("mini-back").test_support().track_focus(&self.back_focus).w_full().h(theme::row_height(d)).flex().items_center().justify_center().cursor_pointer()
                    .rounded(theme::RADIUS).border_1().border_color(t.border).bg(t.raised).text_color(t.text)
                    .when(back_ring, |this| this.border_2().border_color(t.gold).shadow(theme::focus_ring(&t)))
                    .tooltip(move |window, cx| Tooltip::new(back_tip.clone()).build(window, cx))
                    .on_click(move |_, _, cx| back_to_full(&back_state, cx))
                    .on_key_down(move |event, _, cx| { if event.keystroke.key == "enter" { cx.stop_propagation(); back_to_full(&back_key_state, cx); } })
                    .child(tr(cx, T, "Back to full app"))))
            .child(strip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::{demo, model::RunStatus};
    use std::prelude::v1::test;

    #[test]
    fn rows_only_unfinished_needing_you_first() {
        let mut state = demo::state();
        demo::apply_data_scene(&mut state, "at-rest");
        assert!(rows(&state).is_empty());
        let template = state.runs[0].clone();
        state.runs = (0..6).map(|i| { let mut r = template.clone(); r.session = format!("{i}").into(); r.needs.clear(); r }).collect();
        state.runs[0].status = RunStatus::Active;
        state.runs[1].status = RunStatus::Done;
        state.runs[2].status = RunStatus::Blocked;
        state.runs[3].status = RunStatus::LimitWait;
        state.runs[4].status = RunStatus::Waiting;
        state.runs[5].status = RunStatus::Active;
        let list = rows(&state);
        assert_eq!(list.iter().map(|r| (r.session.as_ref(), r.needs_you)).collect::<Vec<_>>(), [("2", true), ("4", true), ("0", false), ("3", false), ("5", false)]);
        state.runs[2].status = RunStatus::Done;
        assert_eq!(rows(&state).iter().map(|r| (r.session.as_ref(), r.needs_you)).collect::<Vec<_>>(), [("4", true), ("0", false), ("3", false), ("5", false)]);
    }

    #[test]
    fn summaries_in_both_languages() {
        for (open, needs, en, zh) in [
            (0, 0, "Nothing is in progress", "目前沒有進行中的願望"),
            (1, 0, "1 wish in progress", "1 個願望進行中"),
            (1, 1, "1 wish in progress, 1 needs you", "1 個願望進行中，1 個需要你"),
            (2, 0, "2 wishes in progress", "2 個願望進行中"),
            (2, 1, "2 wishes in progress, 1 needs you", "2 個願望進行中，1 個需要你"),
            (2, 2, "2 wishes in progress, 2 need you", "2 個願望進行中，2 個需要你"),
        ] {
            assert_eq!(summary_line(open, needs, Lang::En), en);
            assert_eq!(summary_line(open, needs, Lang::ZhTw), zh);
        }
    }

    #[test]
    fn window_kind_and_size() {
        assert_eq!(kind_for(true), WindowKind::PopUp);
        assert_eq!(kind_for(false), WindowKind::Normal);
        assert_eq!((MINI_WIDTH, MINI_HEIGHT), (380., 520.));
    }

    #[test]
    fn back_tip_with_and_without_shortcut() {
        for (lang, plain, with_keys) in [
            (Lang::En, "Back to full app", "Back to full app (Ctrl-M)"),
            (Lang::ZhTw, "返回完整應用程式", "返回完整應用程式（Ctrl-M）"),
        ] {
            assert_eq!(back_tip_text("", lang), plain);
            assert_eq!(back_tip_text("Ctrl-M", lang), with_keys);
        }
    }

    #[test]
    fn mode_and_pin_outcomes() {
        assert_eq!(back_transition(), (WindowMode::Full, SettingPrior::Mode(WindowMode::Full)));
        assert_eq!(pin_prior(false), SettingPrior::MiniOnTop(true));
        assert_eq!(pin_prior(true), SettingPrior::MiniOnTop(false));
    }

    #[test]
    fn translation_keys() {
        for key in ["Chronos", "Stop keeping on top", "Keep on top", "Back to full app",
            "Back to full app ({})", "Nothing is in progress", "{} wish in progress",
            "{} wishes in progress", "{}, {} needs you", "{}, {} need you"] {
            assert!(T.iter().any(|(en, _)| *en == key), "missing {key}");
        }
    }
}
