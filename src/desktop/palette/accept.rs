use super::*;
use crate::desktop::{demo, i18n::{role_duty, role_name, role_tooltip, ROLES}};
use std::prelude::v1::test;

const GLOBALS: [(Global, &str); 8] = [
    (Global::Palette, "OpenPalette"),
    (Global::NewWish, "NewWish"),
    (Global::Shortcuts, "ShowShortcuts"),
    (Global::Undo, "Undo"),
    (Global::ToggleFocus, "ToggleFocus"),
    (Global::ToggleMini, "ToggleMini"),
    (Global::ToggleDensity, "ToggleDensity"),
    (Global::ContextMenu, "ContextMenu"),
];

#[test]
fn every_global_row_has_exactly_its_bindings_and_context() {
    let bindings = global_bindings();
    let mut total = 0;
    for (global, action) in GLOBALS {
        let rows: Vec<_> = SHORTCUTS.iter().filter(|row| row.global == Some(global)).collect();
        assert_eq!(rows.len(), 1, "{global:?}");
        for key in rows[0].keys {
            total += 1;
            let stroke = Keystroke::parse(key).unwrap();
            let matching: Vec<_> = bindings.iter().filter(|binding| {
                let bound = binding.keystrokes();
                bound.len() == 1 && bound[0].inner().key == stroke.key && bound[0].inner().modifiers == stroke.modifiers
            }).collect();
            assert_eq!(matching.len(), 1, "{key}");
            let binding = matching[0];
            assert!(binding.action().name().ends_with(action), "{key}: {}", binding.action().name());
            let predicate = binding.predicate().map(|p| p.to_string());
            let expected = match global {
                Global::Undo | Global::ContextMenu | Global::Shortcuts => Some("!Input"),
                _ => None,
            };
            assert_eq!(predicate.as_deref(), expected, "{global:?}");
        }
    }
    assert_eq!(bindings.len(), total);
}

fn overlay_has_bindings(rows: &[Vec<&'static str>], bindings: &[KeyBinding]) -> bool {
    bindings.iter().all(|binding| binding.keystrokes().iter().all(|stroke| {
        rows.iter().flatten().any(|key| Keystroke::parse(key).is_ok_and(|shown| {
            shown.key == stroke.inner().key && shown.modifiers == stroke.inner().modifiers
        }))
    }))
}

#[test]
fn overlay_contains_all_bindings_and_detects_a_missing_key() {
    let mut rows: Vec<Vec<&'static str>> = overlay_rows().into_iter().map(|(_, rows)| {
        rows.into_iter().flat_map(|(keys, _)| keys.iter().copied()).collect()
    }).collect();
    let bindings = global_bindings();
    assert!(overlay_has_bindings(&rows, &bindings));
    let key = keys_of(Global::Palette);
    for area in &mut rows { area.retain(|shown| *shown != key); }
    assert!(!overlay_has_bindings(&rows, &bindings));
}

#[test]
fn every_area_has_parsable_shortcuts() {
    let sections = overlay_rows();
    assert_eq!(sections.len(), 12);
    for (area, rows) in sections {
        assert!(!rows.is_empty(), "{area:?}");
        for (keys, _) in rows {
            assert!(!keys.is_empty());
            for key in keys { assert!(Keystroke::parse(key).is_ok(), "{key}"); }
        }
    }
}

#[test]
fn new_commands_are_listed_and_translated() {
    let state = demo::state();
    let english = index(&state, Lang::En, true);
    let chinese = index(&state, Lang::ZhTw, true);
    for (target, keys) in [
        (Command::Focus(FocusPanel::Graph), Some(keys_of(Global::ToggleFocus))),
        (Command::Focus(FocusPanel::Checklist), Some(keys_of(Global::ToggleFocus))),
        (Command::FullWindow, None),
        (Command::Mini, Some(keys_of(Global::ToggleMini))),
        (Command::ToggleDensity, Some(keys_of(Global::ToggleDensity))),
        (Command::ToggleOnTop, None),
        (Command::Undo, Some(keys_of(Global::Undo))),
    ] {
        let find = |items: &Vec<Item>| items.iter().position(|item| item.kind == Kind::Command && item.target == Target::Command(target)).unwrap();
        let en = &english[find(&english)];
        let zh = &chinese[find(&chinese)];
        assert_ne!(en.title, zh.title, "{target:?}");
        assert_eq!(zh.keywords, en.title, "{target:?}");
        assert_eq!(en.keys, keys, "{target:?}");
    }
}

#[test]
fn all_roles_have_bilingual_duties_and_tooltips() {
    for (ids, _, _, _, _) in ROLES {
        for id in ids {
            let en = role_duty(Lang::En, id).unwrap();
            let zh = role_duty(Lang::ZhTw, id).unwrap();
            assert!(!en.trim().is_empty() && !zh.trim().is_empty(), "{id}");
            assert_ne!(en, zh, "{id}");
            for lang in [Lang::En, Lang::ZhTw] {
                let expected = format!("{} — {}", role_name(lang, id), role_duty(lang, id).unwrap());
                assert_eq!(role_tooltip(lang, id).as_ref(), expected);
            }
        }
    }
    assert_eq!(role_duty(Lang::En, "chronos:dispatcher"), role_duty(Lang::En, "dispatcher"));
    assert_eq!(role_duty(Lang::En, "unknown-role"), None);
    assert_eq!(role_tooltip(Lang::En, "unknown-role").as_ref(), "unknown-role");
}

#[test]
fn context_menu_entries_follow_run_status() {
    let mut state = demo::state();
    let original = state.runs[0].clone();
    let target = MenuTarget::Run(original.session.clone());
    for (status, enabled) in [
        (RunStatus::Done, false),
        (RunStatus::Blocked, true),
        (RunStatus::Waiting, true),
        (RunStatus::Interrupted, true),
    ] {
        let mut run = original.clone();
        run.status = status;
        state.runs[0] = run;
        let entries = menu::entries_for(&state, &target);
        assert_eq!(entries, state.menu_entries(&target));
        assert!(entries.contains(&(MenuAction::Resume, enabled)), "{status:?}");
        assert_eq!(menu::click_runs(&state, &target, MenuAction::Resume), enabled);
    }
}

#[test]
fn context_menu_click_resolves_run_by_session_after_changes() {
    let mut state = demo::state();
    let session = state.runs[0].session.clone();
    let target = MenuTarget::Run(session.clone());
    let opened = menu::entries_for(&state, &target);
    assert!(opened.contains(&(MenuAction::OpenRun, true)));
    assert!(menu::click_runs(&state, &target, MenuAction::OpenRun));

    let mut other = state.runs[0].clone();
    other.session = "inserted-run".into();
    state.runs.insert(0, other);
    assert!(menu::click_runs(&state, &target, MenuAction::OpenRun));

    state.runs.retain(|run| run.session != session);
    assert!(!menu::click_runs(&state, &target, MenuAction::OpenRun));
    assert!(menu::entries_for(&state, &target).is_empty());
    assert!(!menu::click_runs(&state, &MenuTarget::Run("missing".into()), MenuAction::OpenRun));
}

#[test]
fn context_menu_rejects_removed_unit() {
    let mut state = demo::state();
    let run = state.runs.iter_mut().find(|run| !run.units.is_empty()).unwrap();
    let unit = run.units[0].id.clone();
    let target = MenuTarget::Unit { session: run.session.clone(), unit: unit.clone() };
    assert!(menu::click_runs(&state, &target, MenuAction::OpenDetail));
    let run = state.runs.iter_mut().find(|run| matches!(&target, MenuTarget::Unit { session, .. } if &run.session == session)).unwrap();
    run.units.retain(|item| item.id != unit);
    assert!(menu::entries_for(&state, &target).is_empty());
    assert!(!menu::click_runs(&state, &target, MenuAction::OpenDetail));
}

#[test]
fn context_menu_actions_have_effects_and_translated_labels() {
    assert_eq!(MenuAction::CorrectMemory.effect(), MenuEffect::Dialog);
    assert_eq!(MenuAction::DeleteMemory.effect(), MenuEffect::Confirm);
    for action in [
        MenuAction::OpenRun, MenuAction::Resume, MenuAction::OpenFolder,
        MenuAction::CopySessionId, MenuAction::OpenDetail, MenuAction::CopyText,
        MenuAction::CommentOn, MenuAction::CopyMemory, MenuAction::CorrectMemory,
        MenuAction::DeleteMemory,
    ] {
        let label = menu::action_label(action);
        assert!(!label.trim().is_empty(), "{action:?}");
        let chinese = menu::T.iter().find(|(en, _)| *en == label).map(|(_, zh)| *zh).expect(label);
        assert!(!chinese.trim().is_empty(), "{action:?}");
        assert_ne!(label, chinese, "{action:?}");
    }
}
