//! Settings view and facade exports.
pub use super::model::facade::settings::{
    Density, PlainWordsApply, ReadMark, UxPrefs, LanguageChoice, ThemeChoice, FocusPanel,
    WindowMode, BoardChoice, ColumnWidths, ViewMemory, BoardTokens, SettingPrior,
    WindowPlacement, BudgetInputs, Settings, Store, axiom_home, command_exists, on_path,
    login_start, set_login_start, set_language, set_theme, system_appearance_changed,
    set_reduce_motion, set_density, set_mode, set_mini_on_top, set_board, restore,
    remember_columns, columns, view_memory, remember_view, add_board_tokens, board_tokens,
    remember_window, remember_screen, remember_project, finish_first_day, first_day_done,
    has_recent_project, recent_projects, opening_bounds, save_runtime_and_models,
};

use super::components::Toggle;
use super::i18n;
use super::model::{facade, AppState};
use super::theme::{self, Step, TypeSize};
use gpui_kit::{div, App, Entity, IntoElement, ParentElement, Styled};

#[cfg(test)]
mod accept;

pub const LOGIN_START_ID: &str = "login-start";
pub const ARGON_EVERY_ROUND_ID: &str = "argon-every-round";

const TEXT: i18n::Table = &[("Start at login", "登入時啟動"), ("Argon every round", "每輪皆使用 Argon")];

pub fn install_hooks(cx: &mut App) {
    if !cx.has_global::<PlainWordsApply>() {
        cx.set_global(PlainWordsApply(facade::work::apply_plain_words));
    }
}

pub fn set_plain_words(on: bool, cx: &mut App) -> SettingPrior {
    install_hooks(cx);
    facade::settings::set_plain_words(on, cx)
}

/// A service-owned preference row for the settings panel.
pub fn login_start_row(state: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let d = theme::density(cx);
    let tokens = cx.global::<theme::Tokens>();
    let label = i18n::tr(cx, TEXT, "Start at login");
    let checked = state.read(cx).feeds.login_start;
    let state = state.clone();
    let argon_state = state.clone();
    let home = axiom_home();
    let argon_on = facade::usage::argon_every_round(&home);
    let argon_label = i18n::tr(cx, TEXT, "Argon every round");
    div().flex().flex_col()
        .child(div().child(argon_label).child(
            Toggle::new(ARGON_EVERY_ROUND_ID, argon_label, tokens).checked(argon_on).on_change(move |on, _, cx| {
                let result = facade::usage::set_argon_every_round(&home, on);
                argon_state.update(cx, |state, cx| {
                    if let Err(error) = result { state.report(super::model::Action::Engine, None, error.message); }
                    cx.notify();
                });
            }),
        ))
        .py(theme::space(d, Step::Xs))
        .gap(theme::space(d, Step::Xs))
        .child(div().flex().flex_col().min_w_0().child(div().text_size(theme::font(d, TypeSize::Body)).child(label)))
        .child(div().flex_shrink_0().child(
            Toggle::new(LOGIN_START_ID, label, tokens).checked(checked).on_change(move |on, _, cx| {
                state.update(cx, |state, cx| {
                    set_login_start(state, on);
                    cx.notify();
                });
            }),
        ))
}
