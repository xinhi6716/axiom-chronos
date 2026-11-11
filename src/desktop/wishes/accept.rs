//! AUDIT WALK
//! Hover tween painting and reduced-motion transitions require a GPUI window.
//! Shift-F10 menu anchoring on the selected row requires a GPUI window.
//! Immediate focus-ring painting requires a GPUI window.
//! Scroll-offset restoration and refresh stability require a GPUI window.
//! Verify filter and search event wiring cancels restore only for changes, including
//! same-filter early return, Escape clearing, and dropping an unavailable saved selection.
use super::super::demo::sample_run;
use super::logic::*;
use super::*;
use std::prelude::v1::test;

#[test]
fn session_resolution_survives_reorder_and_removal_and_resume_rechecks() {
    let mut a = sample_run();
    a.session = "a".into();
    a.status = RunStatus::Blocked;
    let mut b = sample_run();
    b.session = "b".into();
    let target = MenuTarget::Run("a".into());
    assert_eq!(
        resolve(&[b.clone(), a.clone()], &target, MenuAction::OpenRun),
        Some(RowEffect::Open(1))
    );
    assert_eq!(resolve(&[b], &target, MenuAction::OpenRun), None);
    assert_eq!(
        resolve(&[a.clone()], &target, MenuAction::Resume),
        Some(RowEffect::Resume(0))
    );
    for status in [RunStatus::Waiting, RunStatus::Interrupted] {
        a.status = status;
        assert_eq!(resolve(&[a.clone()], &target, MenuAction::Resume), Some(RowEffect::Resume(0)));
    }
    for status in [RunStatus::Active, RunStatus::Done] {
        a.status = status;
        assert_eq!(resolve(&[a.clone()], &target, MenuAction::Resume), None);
    }
    a.cwd = None;
    assert_eq!(resolve(&[a.clone()], &target, MenuAction::OpenFolder), None);
    let path = std::path::PathBuf::from("work/project");
    a.cwd = Some(path.clone());
    assert_eq!(resolve(&[a.clone()], &target, MenuAction::OpenFolder), Some(RowEffect::Folder(path)));
    assert_eq!(resolve(&[a], &target, MenuAction::CopySessionId), Some(RowEffect::Copy("a".into())));
}

#[test]
fn selection_keeps_visible_session_or_falls_back_in_group_order() {
    let mut a = sample_run();
    a.session = "a".into();
    let mut b = sample_run();
    b.session = "b".into();
    let runs = [a, b];
    let groups = vec![RunGroup { status: runs[1].status, runs: vec![1, 0] }];
    assert_eq!(logic::selected_index(&groups, &runs, Some(&"a".into())), Some(0));
    assert_eq!(logic::selected_index(&groups, &runs, Some(&"missing".into())), Some(1));
    assert_eq!(logic::selected_index(&groups, &runs, None), Some(1));
    assert_eq!(logic::selected_index(&[], &runs, Some(&"a".into())), None);
}

#[test]
fn restore_cancels_only_for_actual_filter_or_search_changes() {
    assert!(cancels_restore(Filter::All, Filter::Running, "x", "x"));
    assert!(cancels_restore(Filter::All, Filter::All, "x", "y"));
    assert!(cancels_restore(Filter::All, Filter::All, "x", ""));
    assert!(!cancels_restore(Filter::All, Filter::All, "x", "x"));
    assert!(!cancels_restore(Filter::All, Filter::All, "", ""));
}

#[test]
fn row_meta_uses_folder_or_project_followed_by_started_label() {
    let mut run = sample_run();
    run.project = "project".into();
    run.cwd = Some("work/project".into());
    assert_eq!(row_meta(&run, "started"), format!("{} · started", run.cwd.as_ref().unwrap().display()));
    run.cwd = None;
    assert_eq!(row_meta(&run, "started"), "project · started");
}
#[test]
fn memory_encoding_is_bounded_and_unambiguous() {
    let encoded = encode(Filter::NeedsYou, "a|b");
    assert_eq!(decode(Some(&encoded)), (Filter::NeedsYou, "a|b".into()));
    assert_eq!(decode(Some("bogus|x")), (Filter::All, "x".into()));
    assert_eq!(decode(None), (Filter::All, String::new()));
    assert_eq!(decode(Some("nonsense")), (Filter::All, String::new()));
    assert_eq!(
        decode(Some(&format!("all|{}", "a".repeat(130)))).1.len(),
        120
    );
    assert_eq!(
        decode(Some(&encode(Filter::Running, &"願".repeat(130))))
            .1
            .chars()
            .count(),
        120
    );
    assert_eq!(scroll(f32::INFINITY), 0.0);
    assert_eq!(scroll(f32::NAN), 0.0);
    assert_eq!(scroll(-1.0), 0.0);
    assert_eq!(scroll(12.0), 12.0);
}
#[test]
fn restore_only_once_after_data_and_can_be_cancelled() {
    let mut gate = Restore::default();
    for p in [
        PanelState::Loading,
        PanelState::Failed("x".into()),
        PanelState::Empty,
    ] {
        assert!(!gate.take(&p));
    }
    assert!(gate.take(&PanelState::Stale("x".into())));
    assert!(!gate.take(&PanelState::Ready));
    let mut gate = Restore::default();
    gate.cancel();
    assert!(!gate.take(&PanelState::Ready));
}
#[test]
fn all_feed_states_map_distinctly() {
    for (panel, expected) in [
        (PanelState::Loading, ScreenState::Loading),
        (PanelState::Empty, ScreenState::Empty),
        (PanelState::Ready, ScreenState::Ready),
        (PanelState::Failed("x".into()), ScreenState::Failed),
        (PanelState::Stale("x".into()), ScreenState::Stale),
    ] {
        assert_eq!(screen_state(&panel), expected);
    }
}
#[test]
fn glow_levels_keep_focus_immediate_and_reduce_motion_snaps() {
    assert_eq!(levels(true, false, true, 0.4, 0.2, false), (0.4, 0.2, true));
    assert_eq!(
        levels(false, true, true, 0.4, 0.2, false),
        (0.4, 0.2, false)
    );
    assert_eq!(levels(true, false, true, 0.4, 0.2, true), (0.0, 1.0, true));
    assert_eq!(
        levels(false, true, false, 0.4, 0.2, true),
        (1.0, 0.0, false)
    );
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
    assert!(!has_animation_frame(include_str!("../wishes.rs")));
}
