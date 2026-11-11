use super::board::*;
use super::super::i18n::Lang;
use super::super::model::{board_facts, board_runs, BoardEntry, BoardFacts, BoardStatus, BoardText, RunStatus};

// Estimate the full Run screen with its taller, running control bar. The real
// board always uses the measured column height, including any optional cards.
fn details_column_height(window_height: f32, d: super::theme::Density) -> f32 {
    use super::{chrome_heights, control, theme::{self, Step}};
    use gpui_kit::{px, Pixels};
    // Window title bar above the Run screen.
    const TITLE_BAR: Pixels = gpui_kit::component::TITLE_BAR_HEIGHT;
    // Height of the running message composer entry.
    const COMPOSER: Pixels = control::RUNNING_ENTRY_HEIGHT;
    // Conservative height of the control header and hint text.
    const CONTROL_HEADER: Pixels = theme::TARGET_MIN;
    // Minimum height of the density-dependent quota strip row.
    const QUOTA_ROW_MIN: Pixels = theme::TARGET_MIN;
    // Width of each border included in the control/chrome allowance.
    const BORDER: Pixels = px(1.);
    // Allowance for the two horizontal borders around the surrounding chrome.
    const BORDER_COUNT: f32 = 2.0;
    // Control bar top and bottom padding plus the gap between its rows.
    const CONTROL_PADDING_COUNT: f32 = 3.0;
    // Density-dependent spacing used for control bar padding and row gap.
    const CONTROL_PADDING_STEP: Step = Step::Sm;
    // Brief, header, and stage rail heights come from the production layout.
    let chrome = chrome_heights(d);
    let control = COMPOSER + CONTROL_HEADER
        + theme::space(d, CONTROL_PADDING_STEP) * CONTROL_PADDING_COUNT + BORDER * BORDER_COUNT;
    // Allow one quota strip row, at least the minimum target height.
    let quota_strip = theme::row_height(d).max(QUOTA_ROW_MIN);
    window_height - f32::from(TITLE_BAR)
        - f32::from(chrome.brief + chrome.header + chrome.rail)
        - f32::from(control) - f32::from(quota_strip)
}

fn facts() -> BoardFacts {
    BoardFacts { status: RunStatus::Active, verified: 2, total: 7, running: 3, failed: 1, needs_you: true }
}

fn text(next: Option<&str>) -> BoardText {
    BoardText { recent: "recent".into(), next: next.map(Into::into), written_unix: 1_000 }
}

#[test]
fn facts_report_only_supplied_counts_and_attention() {
    for lang in [Lang::En, Lang::ZhTw] {
        let f = facts();
        let sentence = facts_sentence(&f, lang);
        assert_eq!(sentence, if lang == Lang::En {
            "Running, 2 of 7 verified, 3 running, 1 failed, needs you."
        } else {
            "進行中，已驗收 2 / 7，3 個進行中，1 個失敗，需要你處理。"
        });
        assert!(sentence.contains(if lang == Lang::En { "needs you" } else { "需要你" }));
        let quiet = BoardFacts { running: 0, failed: 0, needs_you: false, ..f.clone() };
        let sentence = facts_sentence(&quiet, lang);
        assert!(!sentence.contains(if lang == Lang::En { "0 running" } else { "個進行中" }));
        assert!(!sentence.contains(if lang == Lang::En { "failed" } else { "失敗" }));
        assert!(!sentence.contains(if lang == Lang::En { "needs you" } else { "需要你" }));
        let empty = BoardFacts { verified: 0, total: 0, ..quiet };
        let sentence = facts_sentence(&empty, lang);
        assert!(sentence.contains(if lang == Lang::En { "no plan yet" } else { "尚無計畫" }));
        assert!(!sentence.contains("0 of 0") && !sentence.contains("0 / 0"));
    }
}

#[test]
fn facts_change_with_verified_count() {
    let f = facts();
    assert_ne!(facts_sentence(&f, Lang::En), facts_sentence(&BoardFacts { verified: 4, ..f }, Lang::En));
}

#[test]
fn facts_are_translated() {
    assert_ne!(facts_sentence(&facts(), Lang::En), facts_sentence(&facts(), Lang::ZhTw));
}

#[test]
fn next_step_is_explicit_even_when_unknown() {
    let run = &crate::desktop::demo::state().runs[0];
    for (lang, expected) in [(Lang::En, "Next step: unknown"), (Lang::ZhTw, "下一步：未知")] {
        let mut entry = BoardEntry { status: BoardStatus::Idle, text: Some(text(None)) };
        let lines = board_lines(run, Some(&entry), 1_000, lang);
        assert_eq!(lines.recent.as_deref(), Some("recent"));
        assert_eq!(lines.next.as_deref(), Some(expected));
        assert!(board_line(&lines, lang).contains(expected));
        let without_recent = BoardLines { recent: None, ..lines };
        let line = board_line(&without_recent, lang);
        assert_eq!(line, expected);
        entry.text = Some(text(Some("x")));
        assert_eq!(board_lines(run, Some(&entry), 1_000, lang).next.as_deref(), Some("x"));
        entry.text = None;
        let lines = board_lines(run, Some(&entry), 1_000, lang);
        assert_eq!((lines.recent, lines.next, lines.note), (None, None, None));
    }
}

#[test]
fn board_line_removes_trailing_punctuation_only_when_joining() {
    for (lang, recent, next, expected) in [
        (Lang::En, "Progress updated.", "Continue", "Progress updated — Continue"),
        (Lang::ZhTw, "工作進度已更新。", "繼續", "工作進度已更新，接下來：繼續"),
    ] {
        let mut lines = BoardLines { facts: "facts".into(), recent: Some(recent.into()), next: Some(next.into()), note: None };
        assert_eq!(board_line(&lines, lang), expected);
        lines.recent = Some(format!("{} .。,，;；!！?？  ", recent.trim_end_matches(['.', '。'])));
        assert_eq!(board_line(&lines, lang), expected);
        lines.next = None;
        lines.recent = Some(recent.into());
        assert_eq!(board_line(&lines, lang), recent);
    }
}

#[test]
fn strip_content_begins_with_project() {
    assert_eq!(strip_content("ml_hfo2", "等待批次", "工作進度"), "ml_hfo2 · 等待批次 · 工作進度");
}

#[test]
fn state_note_is_appended_only_when_present() {
    let run = &crate::desktop::demo::state().runs[0];
    for (status, board_text, expected) in [
        (BoardStatus::Writing, Some(text(Some("next"))), Some("Updating — last written 2 min ago")),
        (BoardStatus::Failed("oops".into()), Some(text(Some("next"))), Some("Could not update (oops) — last written 2 min ago")),
        (BoardStatus::Failed("oops".into()), None, Some("Could not write the summary (oops)")),
        (BoardStatus::Idle, Some(text(Some("next"))), None),
        (BoardStatus::Off, Some(text(Some("next"))), None),
    ] {
        let entry = BoardEntry { status, text: board_text };
        let lines = board_lines(run, Some(&entry), 1_120, Lang::En);
        let line = board_line(&lines, Lang::En);
        assert_eq!(lines.note.as_deref(), expected);
        let expected_line = match (line.is_empty(), expected) {
            (true, Some(note)) => note.to_owned(),
            (false, Some(note)) => format!("{line} · {note}"),
            (_, None) => line.clone(),
        };
        assert_eq!(line_with_note(&line, lines.note.as_deref()), expected_line);
        assert_eq!(card_parts(&lines, true, Lang::En).next.as_deref(), (!expected_line.is_empty()).then_some(expected_line.as_str()));
    }
}

#[test]
fn writing_failed_and_off_keep_the_right_text_and_age() {
    let run = &crate::desktop::demo::state().runs[0];
    for lang in [Lang::En, Lang::ZhTw] {
        for (status, with_prefix, without) in [
            (BoardStatus::Writing, if lang == Lang::En { "Updating" } else { "更新中" }, if lang == Lang::En { "Writing the summary…" } else { "正在撰寫摘要…" }),
            (BoardStatus::Failed("oops".into()), if lang == Lang::En { "Could not update (oops)" } else { "無法更新（oops）" }, if lang == Lang::En { "Could not write the summary (oops)" } else { "無法撰寫摘要（oops）" }),
        ] {
            let mut entry = BoardEntry { status, text: Some(text(Some("next"))) };
            let lines = board_lines(run, Some(&entry), 1_420, lang);
            assert_eq!(lines.recent.as_deref(), Some("recent"));
            assert_eq!(lines.next.as_deref(), Some("next"));
            let note = lines.note.unwrap();
            assert!(note.contains(with_prefix) && note.contains(if lang == Lang::En { "7 min ago" } else { "7 分鐘前" }), "{note}");
            entry.text = None;
            let lines = board_lines(run, Some(&entry), 1_420, lang);
            assert_eq!((lines.recent, lines.next, lines.note.as_deref()), (None, None, Some(without)));
        }
        let off = BoardEntry { status: BoardStatus::Off, text: Some(text(Some("next"))) };
        for entry in [Some(&off), None] {
            let lines = board_lines(run, entry, 1_420, lang);
            assert_eq!(lines.facts, facts_sentence(&board_facts(run), lang));
            assert_eq!((lines.recent, lines.next, lines.note), (None, None, None));
        }
    }
}

#[test]
fn ages_use_expected_units_and_clamp_future_times() {
    for (age, en, zh) in [(-1, "just now", "剛剛"), (59, "just now", "剛剛"), (60, "1 min ago", "1 分鐘前"), (3600, "1 h ago", "1 小時前"), (172800, "2 d ago", "2 天前")] {
        assert_eq!(age_text(age, Lang::En), en);
        assert_eq!(age_text(age, Lang::ZhTw), zh);
    }
}

#[test]
fn board_order_puts_attention_first_and_omits_done() {
    let mut state = crate::desktop::demo::state();
    let mut active = state.runs[0].clone();
    active.status = RunStatus::Active;
    active.needs.clear();
    active.session = "active".into();
    let mut blocked = active.clone();
    blocked.session = "blocked".into();
    blocked.status = RunStatus::Blocked;
    let mut done = active.clone();
    done.session = "done".into();
    done.status = RunStatus::Done;
    state.runs = vec![active, done, blocked];
    let ordered_facts: Vec<_> = board_runs(&state).into_iter().map(|i| facts_sentence(&board_facts(&state.runs[i]), Lang::En)).collect();
    assert_eq!(ordered_facts, vec![facts_sentence(&board_facts(&state.runs[2]), Lang::En), facts_sentence(&board_facts(&state.runs[0]), Lang::En)]);
}

#[test]
fn status_tooltips_are_translated() {
    for (status, en, zh) in [(BoardStatus::Idle, "Summary up to date", "摘要已更新"), (BoardStatus::Writing, "Writing", "撰寫中"), (BoardStatus::Failed("x".into()), "Failed", "失敗"), (BoardStatus::Off, "Off", "已關閉")] {
        let entry = BoardEntry { status, text: None };
        assert_eq!(card_tooltip(Some(&entry), Lang::En).as_ref(), en);
        assert_eq!(card_tooltip(Some(&entry), Lang::ZhTw).as_ref(), zh);
    }
    assert_eq!(card_tooltip(None, Lang::En).as_ref(), "Off");
}

#[test]
fn translation_keys_are_unique() {
    let mut keys: Vec<_> = TEXT.iter().map(|(key, _)| *key).collect();
    keys.sort_unstable();
    let total = keys.len();
    keys.dedup();
    assert_eq!(keys.len(), total);
}

#[test]
fn production_source_uses_design_tokens_and_no_animation() {
    let source = include_str!("board.rs").split("#[cfg(test)]").next().unwrap();
    for forbidden in ["text_size(px(", "rounded_md", "rounded_lg", "rounded_xl", "request_animation_frame"] {
        assert!(!source.contains(forbidden), "found {forbidden}");
    }
}


#[test]
fn chrome_fits_and_dense_is_smaller() {
    use super::{chrome_heights, theme::{self, Density, TypeSize}};
    let comfortable = chrome_heights(Density::Comfortable);
    let dense = chrome_heights(Density::Dense);
    let sum = |h: super::ChromeHeights| h.brief + h.header + h.rail;
    assert!(sum(comfortable) <= gpui_kit::px(200.));
    assert!(sum(dense) < sum(comfortable));
    for d in [Density::Comfortable, Density::Dense] {
        assert!(chrome_heights(d).brief >= theme::font(d, TypeSize::Body));
    }
}

#[test]
fn stage_rail_fits_two_caption_lines() {
    use super::{caption_line, chrome_heights, theme::{self, Density, Step, TypeSize}};
    for d in [Density::Comfortable, Density::Dense] {
        assert!(chrome_heights(d).rail >= caption_line(d) * 2. + theme::space(d, Step::Sm) * 2.);
        assert!(caption_line(d) >= theme::font(d, TypeSize::Caption));
    }
    let d = Density::Dense;
    let old_rail = theme::row_height(d) + theme::space(d, Step::Lg);
    assert!(old_rail < caption_line(d) * 2. + theme::space(d, Step::Sm) * 2.);
}

#[test]
fn panel_states_keep_content_only_for_stale() {
    use super::{panel_view, PanelView};
    use super::super::model::PanelState;
    assert_eq!(panel_view(&PanelState::Loading), PanelView::Skeleton);
    assert_eq!(panel_view(&PanelState::Empty), PanelView::Empty);
    assert_eq!(panel_view(&PanelState::Ready), PanelView::Content);
    assert_eq!(panel_view(&PanelState::Failed("failed".into())), PanelView::Error { message: "failed".into(), keep_content: false });
    assert_eq!(panel_view(&PanelState::Stale("stale".into())), PanelView::Error { message: "stale".into(), keep_content: true });
}

fn bad_run_source(source: &str) -> bool {
    if ["text_size(px(", "rounded_md", "rounded_lg", "rounded_xl"].iter().any(|s| source.contains(s)) { return true; }
    source.split("rounded(px(").skip(1).any(|after| {
        let number: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        number.parse::<f32>().is_ok_and(|n| n > 4.0)
    })
}

#[test]
fn production_source_uses_scaled_type_and_radii() {
    let source = include_str!("../run_view.rs").split_once("#[cfg(test)]").unwrap().0;
    assert!(source.contains("fn details_column"));
    assert!(!bad_run_source(source));
    assert!(bad_run_source("div().rounded(px(10.))"));
    assert!(!bad_run_source("div().rounded(px(4.))"));
}

#[test]
fn unit_glows_settle_do_not_restart_and_respect_reduced_motion() {
    use super::super::motion::{Driver, Glows, Pace};
    use std::time::Instant;
    let start = Instant::now();
    let key = gpui_kit::SharedString::from("sel:unit");
    let mut glows = Glows::new(Pace::Glow);
    glows.set(&key, 1.0, start, false);
    let middle = start + Pace::Glow.duration() / 2;
    let mut halfway = Driver::at(middle, false);
    assert!(glows.value(&key, &mut halfway) > 0.0);
    assert!(halfway.wants_frame());
    glows.set(&key, 1.0, middle, false);
    let mut done = Driver::at(start + Pace::Glow.duration(), false);
    assert_eq!(glows.value(&key, &mut done), 1.0);
    assert!(!done.wants_frame());
    let mut reduced = Glows::new(Pace::Glow);
    reduced.set(&key, 1.0, start, true);
    let mut immediate = Driver::at(start, true);
    assert_eq!(reduced.value(&key, &mut immediate), 1.0);
    assert!(!immediate.wants_frame());
}

#[test]
fn stage_tooltips_cover_roles_and_other_stages_in_both_languages() {
    use super::{stage_tip, STAGE_NAMES};
    use super::super::i18n::{role_duty, role_tooltip};
    for (name, id) in [("Astra", "astra"), ("Critic", "critic"), ("Architect", "architect"), ("Memory", "memory-keeper")] {
        for lang in [Lang::En, Lang::ZhTw] {
            assert_eq!(stage_tip(name, lang), role_tooltip(lang, id));
        }
        assert!(stage_tip(name, Lang::En).contains(role_duty(Lang::En, id).unwrap()));
    }
    for name in STAGE_NAMES {
        let en = stage_tip(name, Lang::En);
        let zh = stage_tip(name, Lang::ZhTw);
        assert!(!en.is_empty() && !zh.is_empty());
        assert_ne!(en, zh, "missing translation for {name}");
    }
    assert_eq!(stage_tip("Build", Lang::En).as_ref(), "Build implements and checks the planned units.");
    assert_eq!(stage_tip("Build", Lang::ZhTw).as_ref(), "施工並檢查規劃好的單元。");
}

#[test]
fn column_widths_restore_and_clamp() {
    use super::{restored_widths, CHECKLIST_WIDTH, DETAILS_WIDTH, CHECKLIST_RANGE, DETAILS_RANGE, GRAPH_MIN};
    use super::super::settings::ColumnWidths;
    use gpui_kit::px;
    let defaults = ColumnWidths { checklist: f32::from(CHECKLIST_WIDTH), details: f32::from(DETAILS_WIDTH) };
    assert_eq!(restored_widths(None, 1400.), defaults);
    let big = restored_widths(Some(ColumnWidths { checklist: 5000., details: 5000. }), 1400.);
    assert!((f32::from(CHECKLIST_RANGE.start)..f32::from(CHECKLIST_RANGE.end)).contains(&big.checklist));
    assert!((f32::from(DETAILS_RANGE.start)..f32::from(DETAILS_RANGE.end)).contains(&big.details));
    assert!(big.checklist + big.details + f32::from(GRAPH_MIN) <= 1400.);
    let nan = restored_widths(Some(ColumnWidths { checklist: f32::NAN, details: f32::NAN }), 1400.);
    assert_eq!(nan, ColumnWidths { checklist: f32::from(CHECKLIST_RANGE.start), details: f32::from(DETAILS_RANGE.start) });
    let small = restored_widths(None, 600.);
    assert_eq!(small, ColumnWidths { checklist: f32::from(px(240.)), details: f32::from(px(300.)) });
    assert!(small.checklist < defaults.checklist && small.details < defaults.details);
}

#[test]
fn column_keys_step_and_reset() {
    use super::{step_widths, restored_widths};
    use super::super::settings::ColumnWidths;
    let w = ColumnWidths { checklist: 300., details: 380. };
    assert_eq!(step_widths(w, "alt-right", 1400.).unwrap().checklist, 316.);
    assert_eq!(step_widths(w, "alt-left", 1400.).unwrap().checklist, 284.);
    assert_eq!(step_widths(ColumnWidths { checklist: 240., ..w }, "alt-left", 1400.).unwrap().checklist, 240.);
    assert_eq!(step_widths(w, "alt-shift-left", 1400.).unwrap().details, 396.);
    assert_eq!(step_widths(w, "alt-shift-right", 1400.).unwrap().details, 364.);
    assert_eq!(step_widths(ColumnWidths { checklist: 400., details: 500. }, "alt-0", 1400.), Some(restored_widths(None, 1400.)));
    assert_eq!(step_widths(w, "enter", 1400.), None);
}

#[test]
fn column_handler_keys_are_palette_columns_shortcuts() {
    use super::super::palette::{Area, SHORTCUTS};
    let source = include_str!("../run_view.rs");
    let handler = source.split_once(".key_context(\"RunColumns\")").unwrap().1.split_once(".child(").unwrap().0;
    let arms = regex::Regex::new(r#"(?m)^\s*((?:"[a-z0-9-]+"\s*\|\s*)*"[a-z0-9-]+")\s*=>"#).unwrap();
    let literal = regex::Regex::new(r#""([a-z0-9-]+)""#).unwrap();
    let keys: Vec<_> = arms.captures_iter(handler).flat_map(|arm| literal.captures_iter(arm.get(1).unwrap().as_str()).map(|key| key[1].to_owned()).collect::<Vec<_>>()).collect();
    let listed: Vec<_> = SHORTCUTS.iter().filter(|s| s.area == Area::Columns).flat_map(|s| s.keys.iter().copied()).collect();
    assert_eq!(keys.len(), 5);
    for key in &keys { assert!(listed.contains(&key.as_str()), "unlisted key {key}"); }
    for key in listed { assert!(keys.iter().any(|handled| handled == key), "missing key {key}"); }
}

#[test]
fn focus_target_selects_only_focus_modes() {
    use super::focus_target;
    use super::super::settings::{FocusPanel, WindowMode};
    assert_eq!(focus_target(WindowMode::Focus(FocusPanel::Graph)), Some(FocusPanel::Graph));
    assert_eq!(focus_target(WindowMode::Focus(FocusPanel::Checklist)), Some(FocusPanel::Checklist));
    assert_eq!(focus_target(WindowMode::Full), None);
    assert_eq!(focus_target(WindowMode::Mini), None);
}

#[test]
fn overview_board_share_is_bounded() {
    use super::board_card_max_height;
    assert_eq!(board_card_max_height(1000.0), 300.0);
    assert_eq!(board_card_max_height(0.0), 0.0);
    for height in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(board_card_max_height(height), 0.0);
    }
}

#[test]
fn board_appears_at_1100x700_in_both_densities() {
    use super::{board_layout, DETAIL_OVERVIEW_MIN, theme::{self, Density, Step}};
    const OLD_RESERVE: f32 = 340.0;
    // Extra space above the board's minimum threshold.
    const BOARD_MARGIN: f32 = 8.0;
    // Capture-measured column heights; update with fresh captures when chrome changes.
    const MEASURED_COMFORTABLE: f32 = 363.0;
    const MEASURED_DENSE: f32 = 413.0;
    let comfortable = details_column_height(700.0, Density::Comfortable);
    assert!(comfortable < OLD_RESERVE + STRIP_ROW_HEIGHT);
    for (d, measured) in [(Density::Comfortable, MEASURED_COMFORTABLE), (Density::Dense, MEASURED_DENSE)] {
        let h = details_column_height(700.0, d);
        assert!((h - measured).abs() <= 12.0, "{d:?}: estimated {h}, capture {measured}");
        assert!(h >= DETAIL_OVERVIEW_MIN + STRIP_ROW_HEIGHT + BOARD_MARGIN);
        for count in 1..=5 {
            let layout = board_layout(h, count, f32::from(theme::space(d, Step::Sm)))
                .expect("the board should fit at 1100x700");
            assert!(h - layout.max_height >= DETAIL_OVERVIEW_MIN);
        }
    }
    for count in 1..=5 {
        assert!(board_layout(DETAIL_OVERVIEW_MIN + STRIP_ROW_HEIGHT - 0.1, count, 8.0).is_none());
        assert!(board_layout(DETAIL_OVERVIEW_MIN + STRIP_ROW_HEIGHT, count, 8.0).is_some());
    }
}

#[test]
fn overview_board_budget_reserves_detail() {
    use super::{board_card_budget, DETAIL_OVERVIEW_MIN, theme::{self, Density, Step}};
    let height = DETAIL_OVERVIEW_MIN + 65.0;
    assert_eq!(board_card_budget(height), 65.0);
    for density in [Density::Dense, Density::Comfortable] {
        assert!(board_card_budget(height) < COMPACT_CARD_ESTIMATE + 2.0 * f32::from(theme::space(density, Step::Sm)));
    }
    assert!((board_card_budget(700.0) - 210.0).abs() < 0.001);
    assert_eq!(board_card_budget(1000.0), 300.0);
    for height in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(board_card_budget(height), 0.0);
    }
}

#[test]
fn overview_board_layout_reserves_space_for_cards_and_more_row() {
    use super::{board_layout, BoardLayout, BoardMode, DETAIL_OVERVIEW_MIN};
    assert_eq!(board_layout(DETAIL_OVERVIEW_MIN, 5, 8.0), None);
    assert_eq!(board_layout(DETAIL_OVERVIEW_MIN + 24.0, 5, 8.0), Some(BoardLayout { mode: BoardMode::Strip, visible: 1, max_height: 24.0, min_height: 24.0 }));
    assert_eq!(board_layout(DETAIL_OVERVIEW_MIN + 40.0, 5, 8.0), Some(BoardLayout { mode: BoardMode::Strip, visible: 1, max_height: 24.0, min_height: 24.0 }));
    assert_eq!(board_layout(DETAIL_OVERVIEW_MIN + 65.0, 5, 8.0), Some(BoardLayout { mode: BoardMode::Strip, visible: 1, max_height: 48.0, min_height: 48.0 }));
    for count in 1..=5 {
        let layout = board_layout(DETAIL_OVERVIEW_MIN + 65.0, count, 8.0).unwrap();
        assert_eq!(layout.mode, BoardMode::Strip);
        assert!(layout.visible >= 1);
        assert!(layout.max_height <= 65.0);
    }
    assert_eq!(board_layout(1000.0, 0, 8.0), None);
    assert_eq!(board_layout(1000.0, 3, 8.0), Some(BoardLayout {
        mode: BoardMode::Card, visible: 3, max_height: 300.0, min_height: 2.0 * 8.0 + COMPACT_CARD_ESTIMATE,
    }));
    let hidden = board_layout(700.0, 4, 8.0).unwrap();
    assert_eq!(hidden.visible, 2);
    assert!((hidden.max_height - 210.0).abs() < 0.001);
    assert_eq!(hidden.min_height, 2.0 * 8.0 + COMPACT_CARD_ESTIMATE + 8.0 + MORE_ROW_HEIGHT);
    for height in [0.0, DETAIL_OVERVIEW_MIN, DETAIL_OVERVIEW_MIN + 10.0, DETAIL_OVERVIEW_MIN + 24.0, DETAIL_OVERVIEW_MIN + 40.0, DETAIL_OVERVIEW_MIN + 65.0, DETAIL_OVERVIEW_MIN + 80.0] {
        for count in 1..=5 {
            let Some(layout) = board_layout(height, count, 8.0) else {
                assert!(height < DETAIL_OVERVIEW_MIN + STRIP_ROW_HEIGHT);
                continue;
            };
            let budget = super::board_card_budget(height);
            assert!(layout.max_height <= budget.max(STRIP_ROW_HEIGHT));
            assert!(layout.max_height + DETAIL_OVERVIEW_MIN <= height);
        }
    }
}

#[test]
fn compact_board_fit_reserves_space_for_more_row() {
    assert_eq!(cards_that_fit(&[64.0, 64.0, 64.0], 150.0), 2);
    assert_eq!(cards_that_fit(&[64.0, 64.0, 64.0], 10.0), 1);
    assert_eq!(cards_that_fit(&[], 150.0), 0);
    assert_eq!(compact_visible_count(3, 200.0, 8.0), 2);
    assert_eq!(compact_visible_count(3, 10.0, 8.0), 1);
    assert_eq!(compact_visible_count(3, 224.0, 8.0), 3);
    assert_eq!(compact_visible_count(4, 224.0, 8.0), 2);
    assert_eq!(more_text(12, Lang::En), "+12 more");
    assert_eq!(more_text(12, Lang::ZhTw), "另有 12 個");
}

#[test]
fn hidden_project_names_are_listed_in_order() {
    let projects = ["First", "Second", "Third", "Fourth"];
    assert_eq!(hidden_names(&projects, 2), "Third\nFourth");
    assert_eq!(hidden_names(&projects, 0), "First\nSecond\nThird\nFourth");
    assert_eq!(hidden_names(&projects, projects.len()), "");
    assert_eq!(hidden_names(&projects, projects.len() + 1), "");
}

#[test]
fn card_parts_place_recent_next_and_status_for_both_modes() {
    let run = &crate::desktop::demo::state().runs[0];
    for lang in [Lang::En, Lang::ZhTw] {
        let unknown = if lang == Lang::En { "Next step: unknown" } else { "下一步：未知" };
        let mut entry = BoardEntry { status: BoardStatus::Idle, text: Some(BoardText { recent: "R".into(), next: None, written_unix: 1_000 }) };
        let lines = board_lines(run, Some(&entry), 1_000, lang);
        let compact = card_parts(&lines, true, lang);
        assert_eq!(compact.facts, lines.facts);
        assert_eq!(compact.recent, None);
        assert!(compact.next.as_deref().unwrap().contains("R"));
        assert!(compact.next.as_deref().unwrap().contains(unknown));
        assert_eq!(compact.note, None);
        assert!(compact.tip.as_deref().unwrap().contains("R"));
        assert!(compact.tip.as_deref().unwrap().contains(unknown));
        assert!(compact.tip.as_deref().unwrap().contains(&lines.facts));
        let full = card_parts(&lines, false, lang);
        assert_eq!(full.recent, lines.recent);
        assert_eq!(full.next, lines.next);
        assert_eq!(full.tip, None);
        entry.text = Some(text(Some("next")));
        let fresh = board_lines(run, Some(&entry), 1_000, lang);
        let line = board_line(&fresh, lang);
        assert!(line.contains(entry.text.as_ref().unwrap().recent.as_ref()));
        assert!(line.contains(entry.text.as_ref().unwrap().next.as_ref().unwrap().as_ref()));
        assert_eq!(card_parts(&fresh, true, lang).next.as_deref(), Some(line.as_str()));
        for status in [BoardStatus::Writing, BoardStatus::Failed("oops".into())] {
            entry.status = status;
            let lines = board_lines(run, Some(&entry), 1_000, lang);
            let compact = card_parts(&lines, true, lang);
            assert_eq!(compact.note, None);
            assert_eq!(compact.next.as_deref(), Some(line_with_note(&board_line(&lines, lang), lines.note.as_deref()).as_str()));
            assert!(compact.tip.as_deref().unwrap().contains(lines.note.as_deref().unwrap()));
            let full = card_parts(&lines, false, lang);
            assert_eq!(full.note, lines.note);
            assert_eq!(full.next, lines.next);
            assert_eq!(full.tip, None);
            entry.text = None;
            let lines = board_lines(run, Some(&entry), 1_000, lang);
            let compact = card_parts(&lines, true, lang);
            assert_eq!(compact.note, None);
            assert_eq!(compact.next.as_deref(), lines.note.as_deref());
            assert!(compact.tip.as_deref().unwrap().contains(lines.note.as_deref().unwrap()));
            entry.text = Some(text(Some("next")));
        }
    }
}

#[test]
fn compact_board_shows_facts_once_for_each_status() {
    let run = &crate::desktop::demo::state().runs[0];
    let entries = [
        Some(BoardEntry { status: BoardStatus::Off, text: Some(text(Some("next"))) }),
        None,
        Some(BoardEntry { status: BoardStatus::Writing, text: None }),
        Some(BoardEntry { status: BoardStatus::Failed("oops".into()), text: None }),
        Some(BoardEntry { status: BoardStatus::Idle, text: Some(text(Some("next"))) }),
    ];
    for lang in [Lang::En, Lang::ZhTw] {
        for entry in &entries {
            let lines = board_lines(run, entry.as_ref(), 1_000, lang);
            let line = line_with_note(&board_line(&lines, lang), lines.note.as_deref());
            let strip = strip_content("P", &lines.facts, &line);
            assert_eq!(strip.matches(&lines.facts).count(), 1);
            assert_eq!(strip, if line.is_empty() { format!("P · {}", lines.facts) } else { format!("P · {} · {line}", lines.facts) });
            let card = card_parts(&lines, true, lang);
            let visible = std::iter::once(card.facts.as_str()).chain(card.recent.as_deref()).chain(card.next.as_deref()).chain(card.note.as_deref()).collect::<Vec<_>>().join("\n");
            assert_eq!(visible.matches(&lines.facts).count(), 1);
            assert_eq!(card.next.as_deref(), (!line.is_empty()).then_some(line.as_str()));
            if lines.recent.is_none() {
                assert_eq!(line, lines.note.as_deref().unwrap_or(""));
            } else {
                assert!(line.contains("recent") && line.contains("next"));
            }
        }
    }
}
