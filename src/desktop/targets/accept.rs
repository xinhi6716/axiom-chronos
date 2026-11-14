use super::*;
use crate::desktop::{parse_options, research::Tab, ux::Page};
use std::prelude::v1::test;
#[test]
fn capture_target_is_applied_after_the_window_opens() {
 let source = include_str!("../mod.rs");
 let closure = source.split_once("gpui_kit::open_window(window, cx, |window, cx| {").unwrap().1
  .split_once(".expect(\"could not open the main window\")").unwrap().0;
 assert!(!closure.contains("targets::prepare"));
 assert!(!closure.contains("apply_target"));
 assert!(source.contains("targets::prepare"));
 assert!(source.contains("apply_target"));
}
#[test]
fn inventory_covers_pages_screens_tabs_and_ids() {
 let pages = [Page::Research, Page::Storage, Page::Chat, Page::QuickTask, Page::Plaza, Page::Session("id".into()), Page::Paper("id".into()), Page::Routine];
 let targets = all();
 for page in pages { assert!(targets.iter().any(|t| t.opening == Opening::Page(page.kind())), "{}", page.kind()); }
 for screen in [Screen::Run, Screen::Wishes, Screen::Memory, Screen::Usage, Screen::Doctor] { assert!(targets.iter().any(|t| t.opening == Opening::Screen(screen))); }
 for tab in [Tab::Map, Tab::Sources, Tab::Evidence, Tab::Dataset, Tab::Exports] { assert!(targets.iter().any(|t| t.opening == Opening::ResearchTab(tab.key()))); }
 for id in ["run", "wishes", "memory", "usage", "settings", "research", "storage", "chat", "quick", "plaza", "session", "paper", "routine", "research.map", "research.sources", "research.evidence", "research.dataset", "research.exports", "paper.form", "paper.shortlist", "paper.estimate", "paper.progress", "paper.changelog", "paper.preview", "paper.readiness", "mini", "focus.graph", "focus.checklist", "palette", "composer", "dialog.close", "dialog.control", "dialog.interjection", "dialog.composer", "dialog.evolve"] { assert_eq!(find(id).unwrap().id,id); }
 assert!(find("unknown").is_none());
 let unique: std::collections::HashSet<_> = targets.iter().map(|t| t.id).collect(); assert_eq!(unique.len(),targets.len());
}
#[test]
fn options_and_states() {
 for state in [CaptureState::Live,CaptureState::Empty,CaptureState::Error,CaptureState::Action] { assert_eq!(CaptureState::from_key(state.key()),Some(state)); }
 assert_eq!(parse_options(&[]).unwrap().capture_state,CaptureState::Live);
 let args = ["--demo", "--target", "run", "--state", "action", "--perf-log", "trace.jsonl"].map(str::to_string);
 let parsed = parse_options(&args).unwrap(); assert_eq!(parsed.target.as_deref(),Some("run")); assert_eq!(parsed.capture_state,CaptureState::Action); assert_eq!(parsed.perf_log.unwrap(),std::path::PathBuf::from("trace.jsonl"));
 for args in [["--target", "bad-target"], ["--state", "bad-state"]] { let e = parse_options(&args.map(str::to_string)).unwrap_err(); assert!(e.contains(args[1])); }
}
#[test]
fn prepare_rejects_unfilled_pairs() {
 let mut state = crate::desktop::demo::state();
 let target = find("run").unwrap(); assert!(prepare(&target,CaptureState::Live,&mut state).is_ok());
 let target = find("paper.preview").unwrap(); assert_eq!(prepare(&target,CaptureState::Action,&mut state),Err(Unregistered { target: target.id.into(), state: CaptureState::Action }));
}
#[test]
fn routine_production_is_explicitly_unsupported() {
 let home = tempfile::tempdir().unwrap(); let seams = crate::desktop::ux::real::wiring::production(home.path(),home.path());
 assert_eq!(seams.routines.unwrap().list().err().unwrap().kind,"unsupported");
}
#[test]
fn screen_empty_and_error_use_existing_feed_states() {
 use crate::desktop::model::{panel_state, PanelState};
 let target = find("memory").unwrap();
 let mut state = crate::desktop::demo::state();
 prepare(&target, CaptureState::Empty, &mut state).unwrap();
 assert_eq!(panel_state(&state.feeds.memory, state.memory.is_empty()), PanelState::Empty);
 prepare(&target, CaptureState::Error, &mut state).unwrap();
 assert!(matches!(panel_state(&state.feeds.memory, state.memory.is_empty()), PanelState::Failed(_)));
}

#[test]
fn focus_and_composer_targets_prepare_and_unavailable_states_fail() {
 let mut state = crate::desktop::demo::state();
 for id in ["mini", "focus.graph", "focus.checklist", "dialog.composer"] {
  let target = find(id).unwrap();
  assert!(target.states.contains(&CaptureState::Live));
  assert_eq!(prepare(&target, CaptureState::Live, &mut state), Ok(()));
  assert_eq!(prepare(&target, CaptureState::Action, &mut state), Err(Unregistered { target: id.into(), state: CaptureState::Action }));
 }
}

#[test]
fn declared_panel_and_action_ids_are_unique_and_namespaced() {
 assert_eq!(find("routine").unwrap().panels, &["routine.page"]);
 let mut ids = std::collections::HashSet::new();
 for t in all() {
  assert!(!t.panels.is_empty(), "{} has no panels", t.id);
  if !matches!(t.id, "routine" | "paper.progress" | "paper.changelog" | "paper.readiness") {
   assert!(!t.actions.is_empty(), "{} has no actions", t.id);
  }
  for &id in t.panels {
   if t.group != "research" { assert!(id.strip_prefix(t.id).is_some_and(|suffix| suffix.starts_with('.') && suffix.len() > 1), "{id}"); }
   assert!(ids.insert(id), "duplicate {id}");
  }
  for &id in t.actions {
   let prefix = format!("action:{}.", t.id);
   if t.group != "research" { assert!(id.strip_prefix(&prefix).is_some_and(|suffix| !suffix.is_empty()), "{id}"); }
   assert!(ids.insert(id), "duplicate {id}");
  }
 }
}

#[test]
fn declared_ids_include_existing_controls_and_routine_view_id() {
 for (target, action) in [("chat", "action:chat.send"), ("quick", "action:quick.start"),
  ("plaza", "action:plaza.vote"), ("research.evidence", "research-tab-evidence"),
  ("paper.form", "action:paper.form.shortlist"), ("mini", "action:mini.pin"),
  ("dialog.composer", "action:dialog.composer.start")] {
  assert!(find(target).unwrap().actions.contains(&action), "{action}");
 }
 assert!(include_str!("../shell/modes/routine.rs").contains(".id(\"routine.page\")"));
}

#[test]
fn research_targets_reference_real_element_ids_and_prepare_the_page() {
 let page = include_str!("../research/mod.rs");
 let root = find("research").unwrap();
 assert_eq!(root.panels, &["research-page"]);
 assert!(page.contains(&format!(".id(\"{}\")", root.panels[0])));
 let panels = [
  ("map", include_str!("../research/map.rs")),
  ("sources", include_str!("../research/sources.rs")),
  ("evidence", include_str!("../research/evidence.rs")),
  ("dataset", include_str!("../research/dataset.rs")),
  ("exports", include_str!("../research/exports.rs")),
 ];
 let mut state = crate::desktop::demo::state();
 for (key, source) in panels {
  let target = find(&format!("research.{key}")).unwrap();
  assert_eq!(target.panels, &[format!("research-{key}-panel")]);
  assert_eq!(target.actions, &[format!("research-tab-{key}")]);
  assert!(source.contains(&format!(".id(\"{}\")", target.panels[0])));
  assert!(page.contains("research-tab-{}"));
  state.ux.page = None;
  assert_eq!(prepare(&target, CaptureState::Live, &mut state), Ok(()));
  assert_eq!(state.ux.page, Some(Page::Research));
 }
}
