//! Focus evidence: gpui-component 0.9.0 Button and Switch enable their focus rings by
//! default; ButtonGroup renders its Button children. Button's focus_style and Switch's
//! focus_ring_style use theme.ring, which theme::apply sets to t.gold in both modes.
//! No control here disables the kit's ring. The kit's ring is translucent gold with a
//! gold border; it is not the separate theme::focus_ring shadow.
//! AUDIT WALK (not yet verified; requires a window)
//! Two cards actually paint side by side at 1400 without horizontal overflow.
//! In light and dark modes, Tab reaches each segment, switch, Re-check, Try again and
//! Open; each gold ring is visible and not clipped or obscured by adjacent segments.
//! Status glyph and version warning tooltips appear.
//! Try again clicks reach AppState::retry(Checks).
//! Real preference clicks record undo, and selecting the current choice does not.
use super::{logic::*, *};
use std::prelude::v1::test;

#[test]
fn columns_and_split() {
    assert_eq!([1100., 1399., 1400., 1920.].map(|w| settings_columns(px(w))), [1, 1, 2, 2]);
    for n in [1, 2] {
        let columns = split(n);
        assert_eq!(columns.len(), n);
        for card in [Card::Environment, Card::Versions, Card::Budgets, Card::Preferences] {
            assert_eq!(columns.iter().flatten().filter(|c| **c == card).count(), 1);
        }
        if n == 1 {
            assert_eq!(columns, vec![vec![Card::Environment, Card::Versions, Card::Budgets, Card::Preferences]]);
        } else {
            assert_eq!(columns, vec![vec![Card::Budgets, Card::Environment, Card::Versions], vec![Card::Preferences]]);
        }
    }
}
#[test]
fn panel_states_keep_messages() {
    assert_eq!(checks_view(&PanelState::Loading), ChecksView::Loading);
    assert_eq!(checks_view(&PanelState::Empty), ChecksView::Empty);
    assert_eq!(checks_view(&PanelState::Ready), ChecksView::Ready);
    assert_eq!(checks_view(&PanelState::Failed("error".into())), ChecksView::Failed("error".into()));
    assert_eq!(checks_view(&PanelState::Stale("old".into())), ChecksView::Stale("old".into()));
}
fn changes() -> Vec<(SettingPrior, SettingPrior)> {
    vec![
        (SettingPrior::Language(LanguageChoice::System), SettingPrior::Language(LanguageChoice::English)),
        (SettingPrior::Theme(ThemeChoice::System), SettingPrior::Theme(ThemeChoice::Dark)),
        (SettingPrior::ReduceMotion(None), SettingPrior::ReduceMotion(Some(true))),
        (SettingPrior::PlainWords(false), SettingPrior::PlainWords(true)),
        (SettingPrior::Density(Density::Comfortable), SettingPrior::Density(Density::Dense)),
        (SettingPrior::MiniOnTop(false), SettingPrior::MiniOnTop(true)),
        (SettingPrior::Board(BoardChoice::Luna), SettingPrior::Board(BoardChoice::Off)),
    ]
}
#[test]
fn change_detection_and_undo_order() {
    let mut state = super::super::demo::state();
    let pairs = changes();
    for &(prior, now) in &pairs {
        let mut called = false;
        if let Some(undo) = record_if_changed(prior, prior, || { called = true; now }) { state.record_undo(undo); }
        assert!(!called);
        assert_eq!(state.take_undo(), None);
    }
    for &(prior, now) in &pairs {
        let mut called = false;
        if let Some(undo) = record_if_changed(prior, now, || { called = true; now }) { state.record_undo(undo); }
        assert!(called);
    }
    for &(_, returned_prior) in pairs.iter().rev() { assert_eq!(state.take_undo(), Some(Undoable::Setting(returned_prior))); }
    assert_eq!(state.take_undo(), None);
}
fn has_frame_call(source: &str) -> bool {
    source.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains(concat!("request_", "animation_frame")))
}
fn violations(source: &str) -> Vec<&'static str> {
    let source = source.replace("\r\n", "\n");
    let code = source.split("#[cfg(test)]").next().unwrap();
    let lines = code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>();
    let mut result = Vec::new();
    if has_frame_call(code) { result.push("frame"); }
    if lines.iter().any(|l| l.contains("settings::set_") && !l.contains("record(") && !l.contains("settings::set_login_start")) { result.push("setter"); }
    if lines.iter().any(|l| l.contains(".w(px(400.))")) { result.push("width"); }
    result
}
#[test]
fn source_guard_has_negative_controls() {
    for (sample, expected) in [(concat!("window.request_", "animation_frame();"), "frame"), ("settings::set_board(x, cx);", "setter"), ("div().w(px(400.))", "width")] {
        assert!(violations(sample).contains(&expected));
    }
    assert!(violations(concat!("// settings::set_board(x, cx)\n#[cfg(test)]\nwindow.request_", "animation_frame();")).is_empty());
    assert!(violations("record(&state, current, choice, cx, |cx| settings::set_board(x, cx));").is_empty());
    assert!(violations(include_str!("../doctor.rs")).is_empty());
    for source in [include_str!("../doctor.rs"), include_str!("logic.rs"), include_str!("accept.rs")] {
        assert!(!has_frame_call(source));
    }
}

fn login_start_before_mini_on_top(source: &str) -> bool {
    let code = source.split("#[cfg(test)]").next().unwrap();
    let code = code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let Some((_, preferences)) = code.split_once("fn preferences(") else { return false; };
    let Some((before_mini, _)) = preferences.split_once(".child(preference_row(&t, d, tr(cx, T, \"Mini window always on top\")") else { return false; };
    before_mini.contains("Toggle::new(LOGIN_START_ID") && before_mini.trim_end().ends_with("}))")
}

#[test]
fn login_start_row_is_immediately_before_mini_on_top() {
    let source = include_str!("../doctor.rs");
    let row = ".child(preference_row(&t, d, tr(cx, T, \"Start at login\"), None, { Toggle::new(LOGIN_START_ID) }))";
    let mini = ".child(preference_row(&t, d, tr(cx, T, \"Mini window always on top\"), None, switch))";
    assert!(login_start_before_mini_on_top(source));
    for sample in [
        source.replace("Toggle::new(LOGIN_START_ID", "Toggle::new(other_id"),
        format!("fn preferences() {{\n{mini}\n{row}\n}}"),
        format!("fn preferences() {{\n// {row}\n{mini}\n}}"),
    ] {
        assert!(!login_start_before_mini_on_top(&sample));
    }
}

#[test]
fn login_start_switch_uses_the_default_off_feed() {
    assert!(!settings::login_start(&super::super::demo::state()));
    assert_eq!(LOGIN_START_ID, crate::desktop::settings::LOGIN_START_ID);
}

#[test]
fn new_translations_are_present() {
    for key in ["Codex parallel limit", "Codex parallel limit must be a positive whole number.", "Invalid limit: {} = {}. {}", "Advanced research budgets", "Fable dollars", "Astra tokens", "Argon tokens (one review)", "Maximum council rounds", "Fable must be a positive dollar amount, at most $200; a larger cap is probably a typo.", "Tokens and rounds must be positive whole numbers.", "Saved for the next wish. Advanced research stays off until you turn it on.", "Invalid budget: {} = {}. {}", "Summary board", "luna", "Haiku", "Off", "luna uses Codex quota, Haiku uses Claude quota, off costs nothing.", "Density", "Comfortable", "Dense", "Mini window always on top", "Keeps the mini window above other windows.", "Checking the environment…", "Try again"] {
        let zh = T.iter().find(|(en, _)| *en == key).expect(key).1;
        assert!(!zh.is_empty());
        assert_eq!(super::super::i18n::translate(super::super::i18n::Lang::En,T,key),key);
        if key != "luna" && key != "Haiku" { assert_ne!(key, zh); }
    }
}
