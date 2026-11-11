//! AUDIT WALK
//! Two cost tables actually paint side by side at the breakpoint.
//! The page scroll offset restores after the selected run gate, including with no selection and an unknown selection.
//! Tooltips appear on all icon and role hosts.
//! Focus rings paint immediately on focused run rows.
//! Retry button clicks reach AppState::retry for Usage.
use super::{logic::*, Scope};
use crate::desktop::{i18n::Lang, model::PanelState, settings::{BoardTokens, ViewMemory}, theme};

#[test] fn compact_order_and_warning_text() {
    assert!(one_column_order(1));
    assert!(!one_column_order(2));
    for (lang, n, expected) in [(Lang::En, 1, "1 warning"), (Lang::En, 3, "3 warnings"), (Lang::ZhTw, 1, "1 個警告"), (Lang::ZhTw, 3, "3 個警告")] {
        assert_eq!(warning_summary_text(n, |key| crate::desktop::i18n::translate(lang, super::TEXT, key)), expected);
    }
}

#[test] fn columns_and_cap() {
    assert_eq!([cost_columns(1100.), cost_columns(1399.), cost_columns(1400.)], [1, 1, 2]);
    assert!(page_cap() > theme::WIDE_BREAKPOINT && page_cap() < gpui_kit::px(1920.));
}
#[test] fn board_cases() {
    assert_eq!(board_line(BoardTokens::default()), BoardLine::None);
    assert_eq!(board_line(BoardTokens { unreported: 2, ..Default::default() }), BoardLine::OnlyUnreported(2));
    assert_eq!(board_line(BoardTokens { input: 1, output: 0, unreported: 0 }), BoardLine::Tokens { input: 1, output: 0, unreported: 0 });
    assert_eq!(board_line(BoardTokens { input: 1, output: 2, unreported: 3 }), BoardLine::Tokens { input: 1, output: 2, unreported: 3 });
}
#[test] fn memory_cases() {
    let v = encode(Scope::ThisRun, Some("session"), 42.);
    assert_eq!(decode(&v), (Some(Scope::ThisRun), Some("session".into()), 42.));
    assert_eq!(decode(&encode(Scope::AllRuns, None, 0.)).0, Some(Scope::AllRuns));
    for bad in [f32::NAN, -1., f32::INFINITY] {
        let v = ViewMemory { sort: Some("garbage".into()), selected: Some("x".repeat(201)), scroll: bad, filter: None };
        assert_eq!(decode(&v), (None, None, 0.));
    }
}
#[test] fn board_text_cases() {
    let text = |board| board_text(board, |key| crate::desktop::i18n::translate(Lang::En, super::TEXT, key), |n| format!("{n}"));
    assert_eq!(text(BoardLine::None), "No summary calls yet");
    assert_eq!(text(BoardLine::OnlyUnreported(1)), "1 call without usage");
    assert_eq!(text(BoardLine::OnlyUnreported(2)), "2 calls without usage");
    assert_eq!(text(BoardLine::Tokens { input: 1, output: 0, unreported: 0 }), "1 in · 0 out");
    assert_eq!(text(BoardLine::Tokens { input: 1, output: 2, unreported: 1 }), "1 in · 2 out · 1 call without usage");
    assert_eq!(text(BoardLine::Tokens { input: 1, output: 2, unreported: 3 }), "1 in · 2 out · 3 calls without usage");
    assert_eq!(board_text(BoardLine::OnlyUnreported(2), |key| crate::desktop::i18n::translate(Lang::ZhTw, super::TEXT, key), |n| n.to_string()), "2 次呼叫未回報用量");
}
#[test] fn role_tips() {
    let one = role_tip(Lang::En, "dispatcher", &["dispatcher"]);
    assert_eq!(one, "Dispatcher — Assigns and coordinates tasks.");
    assert_eq!(role_tip(Lang::En, "dispatcher", &["dispatcher", "chronos:dispatcher"]), format!("{one}\ndispatcher, chronos:dispatcher"));
}
#[test] fn restore_once_and_cancel() {
    let mut gate = Restore::default();
    for panel in [PanelState::Loading, PanelState::Empty, PanelState::Failed("x".into())] { assert_eq!(gate.take(&panel, true, Some(true), 42.), None); }
    assert_eq!(gate.take(&PanelState::Stale("x".into()), false, Some(false), 42.), None);
    assert_eq!(gate.take(&PanelState::Stale("x".into()), true, Some(false), 42.), None);
    assert_eq!(gate.take(&PanelState::Stale("x".into()), true, Some(true), 42.), Some(42.));
    assert_eq!(gate.take(&PanelState::Ready, true, Some(true), 42.), None);
    let mut no_selection = Restore::default();
    assert_eq!(no_selection.take(&PanelState::Ready, true, None, 42.), Some(42.));
    let mut cancelled = Restore::default(); cancelled.cancel(); assert_eq!(cancelled.take(&PanelState::Ready, true, None, 42.), None);
}
#[test] fn states_are_distinct() {
    let panels = [PanelState::Loading, PanelState::Empty, PanelState::Ready, PanelState::Failed("x".into()), PanelState::Stale("x".into())];
    let states: Vec<_> = panels.iter().map(|p| screen_state(p, true)).collect();
    assert_eq!(states, [ScreenState::Loading, ScreenState::Empty, ScreenState::Ready, ScreenState::Failed, ScreenState::Stale]);
    assert_eq!(screen_state(&PanelState::Ready, false), ScreenState::Empty);
    assert_eq!(screen_state(&PanelState::Stale("x".into()), false), ScreenState::Stale);
}
fn forbidden(source: &str) -> bool {
    let normalized = source.replace("\r\n", "\n");
    let code = normalized.split("#[cfg(test)]\nmod tests").next().unwrap();
    let code = code.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let re = regex::Regex::new(r"rounded\(px\(\s*([0-9]+(?:\.[0-9]*)?)\s*\)\)").unwrap();
    ["text_size(px(", "rounded_md", "rounded_lg", "rounded_xl", ".text_xs()", ".text_sm()"].iter().any(|s| code.contains(s)) || re.captures_iter(&code).any(|c| c[1].parse::<f32>().unwrap() > 4.)
}
#[test] fn scale_source_scan_and_negative_control() {
    for sample in ["x.text_size(px(13.))", "x.rounded(px(5.))", "x.rounded_md()", "x.rounded_lg()", "x.rounded_xl()", "x.text_xs()", "x.text_sm()"] { assert!(forbidden(sample), "{sample}"); }
    assert!(!forbidden("x.rounded(px(4.))"));
    assert!(!forbidden("let ok = true;\r\n#[cfg(test)]\r\nmod tests { x.rounded_xl(); }"));
    assert!(!forbidden(include_str!("../usage.rs")));
}
#[test] fn animation_frame_source_scan_and_negative_control() {
    let has_animation_frame = |source: &str| {
        let normalized = source.replace("\r\n", "\n");
        let code = normalized.split("#[cfg(test)]\nmod tests").next().unwrap();
        code.lines().filter(|line| !line.trim_start().starts_with("//"))
            .any(|line| line.contains("request_animation_frame"))
    };
    assert!(has_animation_frame("window.request_animation_frame();"));
    assert!(!has_animation_frame("// request_animation_frame\r\n#[cfg(test)]\r\nmod tests { window.request_animation_frame(); }"));
    assert!(!has_animation_frame(include_str!("../usage.rs")));
}
