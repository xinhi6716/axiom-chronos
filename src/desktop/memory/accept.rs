//! AUDIT WALK
//! Hover tween painting and reduced-motion transitions require a GPUI window.
//! Shift-F10 menu anchoring on the selected row requires a GPUI window.
//! Immediate focus-ring painting requires a GPUI window.
//! Scroll-offset restoration and refresh stability require a GPUI window.
//! The Undo button reaching undo_memory requires a GPUI window.
//! The named confirmation opening for every Delete path requires a GPUI window.
use super::logic::*;
use super::*;
use std::prelude::v1::test;

fn entry(id: i64, title: &str) -> MemoryEntry {
    MemoryEntry { id, title: title.into(), body: format!("body {id}").into(), project: "p".into(), wish: "".into(), confirmed: "".into(), corrected: false, stale: false }
}
fn prior(body: &str) -> MemoryPrior { MemoryPrior { body: body.into(), summary: "s".into(), corrected: None, confirmed: 0.0 } }
fn undo(id: i64, prior: &MemoryPrior) -> Undoable { Undoable::Memory { id, prior: prior.clone() } }
#[test]
fn menu_resolves_ids_after_reorder_and_removal_and_delete_requires_confirmation() {
    let a = entry(4, "A"); let b = entry(8, "B"); let target = MenuTarget::Memory(4);
    assert_eq!(resolve(&[b.clone(), a.clone()], &target, MenuAction::CopyMemory), Some(MemoryEffect::Copy("A\n\nbody 4".into())));
    assert_eq!(resolve(&[b.clone(), a.clone()], &target, MenuAction::CorrectMemory), Some(MemoryEffect::Correct(4)));
    assert_eq!(resolve(&[b.clone(), a], &target, MenuAction::DeleteMemory), Some(MemoryEffect::Delete(4, "A".into())));
    assert_eq!(MenuAction::DeleteMemory.effect(), MenuEffect::Confirm);
    for action in [MenuAction::CopyMemory, MenuAction::CorrectMemory, MenuAction::DeleteMemory] { assert_eq!(resolve(&[b.clone()], &target, action), None); }
    assert_eq!(resolve(&[b], &MenuTarget::Memory(8), MenuAction::OpenRun), None);
}
#[test]
fn undo_offer_only_tracks_a_new_memory_top() {
    let mut offer = UndoOffer::default(); let a = prior("a"); let b = prior("b");
    offer.observe(None, 0, 1); assert!(offer.offered.is_none()); // Setting
    offer.remember_top(Some(undo(1, &a)));
    offer.observe(Some(undo(1, &a)), 1, 1); assert!(offer.offered.is_none());
    offer.observe(Some(undo(1, &a)), 1, 2); assert_eq!(offer.offered, Some((1, a.clone())));
    offer.observe(Some(undo(2, &b)), 2, 3); assert_eq!(offer.offered, Some((2, b.clone())));
    assert!(!offer.matches_top(Some((1, &a))));
    offer.observe(None, 3, 2); assert!(offer.take().is_none()); // global undo or deletion
    offer.observe(Some(undo(2, &b)), 2, 3);
    assert!(!can_undo(2, &b, false, Some((2, &b)))); // entry deleted before click
    assert!(!can_undo(2, &b, true, Some((1, &a))));
    assert!(can_undo(2, &b, true, Some((2, &b))));
    offer.clear(); assert!(offer.take().is_none());
}
#[test]
fn undo_offer_detects_a_push_on_a_full_stack() {
    let mut stack = UndoStack::default();
    for id in 0..20 { stack.push(undo(id, &prior("old"))); }
    let mut offer = UndoOffer::default();
    offer.remember_top(stack.clone().pop());
    offer.observe(stack.clone().pop(), 20, stack.len());
    assert!(offer.offered.is_none());
    let correction = prior("new");
    stack.push(undo(20, &correction));
    assert_eq!(stack.len(), 20);
    offer.observe(stack.clone().pop(), 20, stack.len());
    assert_eq!(offer.offered, Some((20, correction)));
}
#[test]
fn memory_fields_are_bounded_unambiguous_and_sanitized() {
    let v = encode(Some("a|b"), "搜尋|text", Some(37), 42.0);
    assert_eq!(decode(&v), (Some("a|b".into()), "搜尋|text".into(), Some(37), 42.0));
    let mut v = encode(None, &"願".repeat(121), None, f32::NAN);
    assert_eq!(v.sort.as_ref().unwrap().chars().count(), 120); assert_eq!(v.scroll, 0.0);
    v.selected = Some("abc".into()); v.scroll = -5.0; assert_eq!(decode(&v).2, None); assert_eq!(decode(&v).3, 0.0);
    assert_eq!(encode(None, "", None, f32::INFINITY).sort, None);
}
#[test]
fn restore_only_runs_once_on_usable_feed_and_can_be_cancelled() {
    let mut r = Restore::default();
    for panel in [PanelState::Loading, PanelState::Empty, PanelState::Failed("bad".into())] { assert!(!r.take(&panel)); }
    assert!(r.take(&PanelState::Stale("offline".into()))); assert!(!r.take(&PanelState::Ready));
    let mut cancelled = Restore::default(); cancelled.cancel(); assert!(!cancelled.take(&PanelState::Ready));
    let mut ready = Restore::default(); assert!(ready.take(&PanelState::Ready)); assert!(!ready.take(&PanelState::Ready));
}
#[test]
fn all_panel_states_map_and_row_levels_obey_motion_and_focus() {
    assert_eq!(screen_state(&PanelState::Loading), ScreenState::Loading);
    assert_eq!(screen_state(&PanelState::Empty), ScreenState::Empty);
    assert_eq!(screen_state(&PanelState::Ready), ScreenState::Ready);
    assert_eq!(screen_state(&PanelState::Failed("x".into())), ScreenState::Failed);
    assert_eq!(screen_state(&PanelState::Stale("x".into())), ScreenState::Stale);
    assert_eq!(levels(true, false, true, 0.4, 0.6, false), (0.4, 0.6, true));
    assert_eq!(levels(true, false, true, 0.4, 0.6, true), (0.0, 1.0, true));
    assert_eq!(levels(false, true, true, 0.4, 0.6, true), (1.0, 0.0, false));
    assert_eq!(levels(true, true, false, 0.4, 0.6, false).2, false);
}
#[test]
fn list_width_is_clamped_and_grows_with_window() {
    assert_eq!(list_width(500.0), 360.0); assert_eq!(list_width(1200.0), 408.0); assert_eq!(list_width(1920.0), 520.0);
}
#[test]
fn detail_width_caps_reading_width_plus_padding_for_both_densities() {
    for d in [theme::Density::Comfortable, theme::Density::Dense] {
        assert_eq!(detail_width(d), theme::READ_WIDTH + theme::space(d, Step::Xl) * 2.0);
    }
}

#[test]
fn selection_restores_existing_ids_and_never_saves_a_fallback_during_restore() {
    let memory = [entry(4, "A"), entry(8, "B")];
    let rows = [0, 1];
    assert_eq!(selection(&rows, &memory, Some(99), None, false, true, true), (Some(0), Some(4), false, Some(None)));
    assert_eq!(selection(&rows, &memory, Some(8), None, false, true, true), (Some(1), Some(8), true, Some(Some(8))));
    assert_eq!(selection(&rows, &memory, Some(8), None, false, false, false), (Some(0), Some(4), false, None));
}

// Fail closed if the function or its delimiters cannot be found.
fn delete_calls_only_in_confirmation(source: &str) -> bool {
    let Some(start) = source.find("fn confirm_delete(") else { return false };
    let Some(open) = source[start..].find('{').map(|i| start + i) else { return false };
    let mut depth = 0usize;
    let mut close = None;
    for (i, byte) in source.bytes().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { close = Some(i + 1); break; } }
            _ => {}
        }
    }
    let Some(close) = close else { return false };
    source[open..close].matches("delete_memory(").count() == 1
        && !source[..open].contains("delete_memory(")
        && !source[close..].contains("delete_memory(")
}

#[test]
fn deletion_is_called_only_from_confirmation() {
    let source = include_str!("../memory.rs").replace("\r\n", "\n");
    let production = source.split("\n#[cfg(test)]\nmod tests").next().unwrap();
    assert!(delete_calls_only_in_confirmation(production));
    let extra_call = format!("{production}\nfn bypass() {{ state.delete_memory(id, cx); }}");
    assert!(!delete_calls_only_in_confirmation(&extra_call));
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
    assert!(!has_animation_frame(include_str!("../memory.rs")));
}

// HANDOFF OPEN
// Hover tween painting and reduced-motion transitions require a GPUI window.
// Shift-F10 menu anchoring on the selected row requires a GPUI window.
// Immediate focus-ring painting requires a GPUI window.
// Scroll-offset restoration and refresh stability require a GPUI window.
// The Undo button reaching undo_memory requires a GPUI window.
// The named confirmation opening for every Delete path requires a GPUI window.
