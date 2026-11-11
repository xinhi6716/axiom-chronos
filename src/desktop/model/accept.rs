use super::*;
use crate::desktop::{demo, settings::SettingPrior};
use crate::usage::quota::{freshness, level, Level, Quotas};
use std::{cell::{Cell, RefCell}, rc::Rc, time::SystemTime};
use std::prelude::v1::test;

const KINDS: [FeedKind; 5] = [FeedKind::Runs, FeedKind::Memory, FeedKind::Checks, FeedKind::Usage, FeedKind::Quota];

fn prior(n: i64) -> MemoryPrior { MemoryPrior { body: n.to_string(), summary: "summary".into(), corrected: None, confirmed: 0.0 } }

#[test]
fn panel_outcomes_and_priority() {
    let mut feed = FeedStatus::default();
    assert_eq!(panel_state(&feed, false), PanelState::Loading);
    feed.error = Some(("oops".into(), SystemTime::now()));
    assert_eq!(panel_state(&feed, false), PanelState::Failed("oops".into()));
    feed.loaded = true;
    assert_eq!(panel_state(&feed, true), PanelState::Stale("oops".into()));
    feed.error = None;
    assert_eq!(panel_state(&feed, true), PanelState::Empty);
    assert_eq!(panel_state(&feed, false), PanelState::Ready);
}

#[test]
fn undo_capacity_order_and_labels() {
    let mut stack = UndoStack::default();
    assert!(stack.is_empty());
    assert_eq!(stack.pop(), None);
    stack.push(Undoable::Setting(SettingPrior::PlainWords(true)));
    assert_eq!(stack.peek_label(), Some("Undo setting change"));
    stack.pop();
    for id in 0..25 { stack.push(Undoable::Memory { id, prior: prior(id) }); }
    assert_eq!(stack.len(), 20);
    assert_eq!(stack.peek_label(), Some("Undo memory correction"));
    for id in (5..25).rev() { assert_eq!(stack.pop(), Some(Undoable::Memory { id, prior: prior(id) })); }
    assert_eq!(stack.pop(), None);
    assert_eq!(stack.peek_label(), None);
}

#[test]
fn menus_resolve_ids_and_action_availability() {
    use MenuAction::*;
    let mut state = demo::state();
    let session = state.runs[0].session.clone();
    let target = MenuTarget::Run(session.clone());
    assert_eq!(state.menu_entries(&target), vec![(OpenRun, true), (Resume, false), (OpenFolder, true), (CopySessionId, true)]);
    assert!(!state.menu_allowed(&target, Resume));
    let blocked = MenuTarget::Run(state.runs[2].session.clone());
    assert_eq!(state.menu_entries(&blocked), vec![(OpenRun, true), (Resume, true), (OpenFolder, true), (CopySessionId, true)]);
    assert!(state.menu_allowed(&blocked, Resume));
    let no_folder = MenuTarget::Run(state.runs[4].session.clone());
    assert!(!state.menu_allowed(&no_folder, OpenFolder));
    let module = MenuTarget::Module { session: session.clone(), module: state.runs[0].modules[0].id.clone() };
    let unit = MenuTarget::Unit { session: session.clone(), unit: state.runs[0].units[0].id.clone() };
    for t in [&module, &unit] { assert_eq!(state.menu_entries(t), vec![(OpenDetail, true), (CopyText, true), (CommentOn, true)]); }
    let other = MenuTarget::Unit { session: state.runs[1].session.clone(), unit: state.runs[1].units[0].id.clone() };
    assert_eq!(state.menu_entries(&other), vec![(OpenDetail, true), (CopyText, true), (CommentOn, false)]);
    assert!(!state.menu_allowed(&other, CommentOn));
    let unknown = MenuTarget::Module { session: session.clone(), module: "missing".into() };
    assert!(state.menu_entries(&unknown).is_empty());
    assert!(!state.menu_allowed(&unknown, OpenDetail));
    let memory = MenuTarget::Memory(41);
    assert_eq!(state.menu_entries(&memory), vec![(CopyMemory, true), (CorrectMemory, true), (DeleteMemory, true)]);
    assert_eq!(DeleteMemory.effect(), MenuEffect::Confirm);
    assert_eq!(CorrectMemory.effect(), MenuEffect::Dialog);
    assert_eq!(CopyMemory.effect(), MenuEffect::Now);
    state.runs.reverse();
    assert_eq!(state.menu_entries(&target), vec![(OpenRun, true), (Resume, false), (OpenFolder, true), (CopySessionId, true)]);
    state.runs.retain(|r| r.session != session);
    assert!(state.menu_entries(&target).is_empty());
    assert!(!state.menu_allowed(&target, OpenRun));
    state.memory.retain(|m| m.id != 41);
    assert!(state.menu_entries(&memory).is_empty());
    assert!(!state.menu_allowed(&memory, CopyMemory));
}

#[test]
fn menu_resume_is_enabled_for_waiting_and_interrupted() {
    let mut state = demo::state();
    let target = MenuTarget::Run(state.runs[0].session.clone());
    for (status, expected) in [
        (RunStatus::Active, false),
        (RunStatus::Waiting, true),
        (RunStatus::LimitWait, false),
        (RunStatus::Interrupted, true),
        (RunStatus::Blocked, true),
        (RunStatus::Done, false),
    ] {
        state.runs[0].status = status;
        assert_eq!(state.menu_allowed(&target, MenuAction::Resume), expected, "{status:?}");
    }
}

#[test]
fn board_is_derived_only_from_runs() {
    let mut state = demo::state();
    let expected = [(13, 21, 2, 1), (13, 21, 0, 1), (13, 21, 2, 1), (8, 8, 0, 0), (0, 0, 0, 0)];
    for (run, (verified, total, running, failed)) in state.runs.iter().zip(expected) {
        let facts = board_facts(run);
        assert_eq!((facts.verified, facts.total, facts.running, facts.failed), (verified, total, running, failed));
        assert_eq!(facts.status, run.status);
        assert_eq!(facts.needs_you, needs_you(run));
    }
    let facts = board_facts(&state.runs[0]);
    state.board.clear();
    assert_eq!(board_facts(&state.runs[0]), facts);
    let mut run = state.runs[0].clone();
    let need = state.runs[1].needs[0].clone();
    for status in [RunStatus::Active, RunStatus::LimitWait, RunStatus::Blocked, RunStatus::Waiting, RunStatus::Interrupted, RunStatus::Done] {
        run.status = status;
        run.needs.clear();
        assert_eq!(needs_you(&run), matches!(status, RunStatus::Blocked | RunStatus::Waiting | RunStatus::Interrupted));
        run.needs.push(need.clone());
        assert_eq!(needs_you(&run), status != RunStatus::Done);
    }
    let indices = board_runs(&state);
    assert_eq!(indices, vec![1, 2, 0]);
    assert!(demo::apply_data_scene(&mut state, "at-rest"));
    assert!(board_runs(&state).is_empty());
    assert!(state.runs.iter().all(|r| r.status == RunStatus::Done && r.needs.is_empty() && r.units.iter().all(|u| u.state == UnitState::Verified)));
}

#[derive(Default)]
struct Calls { restores: RefCell<Vec<(i64, MemoryPrior)>>, reloads: RefCell<Vec<FeedKind>>, refresh: Cell<usize>, fail: bool }
struct Recording(Rc<Calls>);
impl Backend for Recording {
    fn start_wish(&self, _: WishRequest) -> Res<()> { Ok(()) }
    fn interject(&self, _: &str, _: &str) -> Res<()> { Ok(()) }
    fn stop(&self, _: &str) -> Res<()> { Ok(()) }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> { Ok(()) }
    fn delete_memory(&self, _: i64) -> Res<()> { Ok(()) }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> { Ok(()) }
    fn refresh(&self) -> Res<()> { self.0.refresh.set(self.0.refresh.get() + 1); Ok(()) }
    fn recheck(&self) -> Res<()> { Ok(()) }
    fn follow(&self, _: &str) -> Res<()> { Ok(()) }
    fn restore_memory(&self, id: i64, prior: &MemoryPrior) -> Res<()> {
        self.0.restores.borrow_mut().push((id, prior.clone()));
        if self.0.fail { Err(Failure::new("failure", "restore failed")) } else { Ok(()) }
    }
    fn reload(&self, f: FeedKind) -> Res<()> { self.0.reloads.borrow_mut().push(f); Ok(()) }
}
struct DefaultReload(Rc<Calls>);
impl Backend for DefaultReload {
    fn start_wish(&self, _: WishRequest) -> Res<()> { Ok(()) }
    fn interject(&self, _: &str, _: &str) -> Res<()> { Ok(()) }
    fn stop(&self, _: &str) -> Res<()> { Ok(()) }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> { Ok(()) }
    fn delete_memory(&self, _: i64) -> Res<()> { Ok(()) }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> { Ok(()) }
    fn refresh(&self) -> Res<()> { self.0.refresh.set(self.0.refresh.get() + 1); Ok(()) }
    fn recheck(&self) -> Res<()> { Ok(()) }
    fn follow(&self, _: &str) -> Res<()> { Ok(()) }
}

#[test]
fn undo_and_retry_work_locally_and_via_backend() {
    let mut state = demo::state();
    let original = state.memory.iter().find(|m| m.id == 41).unwrap().clone();
    state.apply_memory_correction(41, " changed ");
    assert_eq!(state.undo.len(), 1);
    let undo = state.take_undo().unwrap();
    let Undoable::Memory { id, prior: saved } = undo else { panic!("wrong undo") };
    assert_eq!(saved.body, original.body.as_ref());
    assert_eq!(saved.summary, original.title.as_ref());
    assert_eq!(saved.corrected, Some(0.0));
    state.memory.iter_mut().find(|m| m.id == id).unwrap().corrected = false;
    state.restore_memory(id, saved);
    assert_eq!(state.memory.iter().find(|m| m.id == id).unwrap(), &original);
    state.apply_memory_correction(-1, "missing");
    assert!(state.undo.is_empty());
    state.feeds.runs = FeedStatus { loaded: false, error: Some(("error".into(), SystemTime::now())) };
    state.retry_feed(FeedKind::Runs);
    assert_eq!(panel_state(&state.feeds.runs, false), PanelState::Ready);

    let calls = Rc::new(Calls::default());
    state.backend = Some(Box::new(Recording(calls.clone())));
    state.restore_memory(41, prior(42));
    assert_eq!(*calls.restores.borrow(), vec![(41, prior(42))]);
    state.retry_feed(FeedKind::Quota);
    assert_eq!(*calls.reloads.borrow(), vec![FeedKind::Quota]);
    state.apply_memory_correction(41, "backend");
    assert!(state.undo.is_empty());
    assert_eq!(state.memory.iter().find(|m| m.id == 41).unwrap(), &original);
    let failed = Rc::new(Calls { fail: true, ..Default::default() });
    state.backend = Some(Box::new(Recording(failed.clone())));
    state.restore_memory(41, prior(5));
    assert_eq!(*failed.restores.borrow(), vec![(41, prior(5))]);
    assert_eq!(state.errors.last().unwrap().action, Action::CorrectMemory);
    assert_eq!(state.memory.iter().find(|m| m.id == 41).unwrap(), &original);
    state.backend = Some(Box::new(DefaultReload(calls.clone())));
    state.retry_feed(FeedKind::Checks);
    assert_eq!(calls.refresh.get(), 1);
}

#[test]
fn undo_of_uncorrected_memory_restores_corrected_false() {
    let mut state = demo::state();
    let original = state.memory.iter().find(|m| m.id == 48).unwrap().clone();
    assert!(!original.corrected);
    state.apply_memory_correction(48, " new ");
    assert!(state.memory.iter().find(|m| m.id == 48).unwrap().corrected);
    let Undoable::Memory { id, prior } = state.take_undo().unwrap() else { panic!("wrong undo") };
    assert_eq!(id, 48);
    assert_eq!(prior.corrected, None);
    let mut corrected_prior = prior.clone();
    corrected_prior.corrected = Some(0.0);
    state.restore_memory(id, prior);
    assert_eq!(state.memory.iter().find(|m| m.id == 48).unwrap(), &original);
    state.restore_memory(id, corrected_prior);
    assert!(state.memory.iter().find(|m| m.id == 48).unwrap().corrected);
}

#[test]
fn quota_unknown_scene_has_no_figures() {
    let mut state = demo::state();
    let figures = |quota: &Quotas| {
        [&quota.claude.five_hour, &quota.claude.weekly, &quota.codex.five_hour, &quota.codex.weekly]
            .into_iter().flatten().map(|window| (level(window.left()), freshness(window, window.observed_at))).collect::<Vec<_>>().len()
    };
    assert_eq!(figures(&state.quota), 3);
    assert!(demo::apply_data_scene(&mut state, "quota-unknown"));
    assert_eq!(figures(&state.quota), 0);
}

fn close(actual: f32, expected: f32) { assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}"); }

#[test]
fn demo_data_and_scenes() {
    let initial = demo::state();
    for kind in KINDS { assert_eq!(panel_state(initial.feeds.get(kind), false), PanelState::Ready); }
    close(initial.quota.claude.five_hour.as_ref().unwrap().left(), 0.62);
    close(initial.quota.claude.weekly.as_ref().unwrap().left(), 0.41);
    assert!(initial.quota.codex.five_hour.is_none());
    close(initial.quota.codex.weekly.as_ref().unwrap().left(), 0.58);
    assert!(initial.undo.is_empty());
    assert_eq!(initial.board.len(), board_runs(&initial).len());
    for i in board_runs(&initial) {
        let run = &initial.runs[i];
        let entry = &initial.board[&run.session];
        assert_eq!(entry.status, BoardStatus::Idle);
        let text = entry.text.as_ref().unwrap();
        assert!(!text.recent.is_empty());
        assert_eq!(text.next.is_none(), run.status == RunStatus::Blocked);
    }
    for scene in ["loading", "feed-error", "stale"] {
        let mut state = demo::state();
        assert!(demo::apply_data_scene(&mut state, scene));
        for kind in KINDS {
            let outcome = panel_state(state.feeds.get(kind), true);
            match scene {
                "loading" => assert_eq!(outcome, PanelState::Loading),
                "feed-error" => assert!(matches!(outcome, PanelState::Failed(_))),
                _ => assert!(matches!(outcome, PanelState::Stale(_))),
            }
        }
        if scene == "stale" {
            assert_eq!(state.runs, initial.runs); assert_eq!(state.memory, initial.memory); assert_eq!(state.checks, initial.checks);
        } else { assert!(state.runs.is_empty() && state.memory.is_empty() && state.checks.is_empty()); }
    }
    for (scene, claude, codex, expected) in [("quota-low", 0.12, 0.08, Level::Low), ("quota-out", 0.0, 0.0, Level::Exhausted)] {
        let mut state = demo::state();
        assert!(demo::apply_data_scene(&mut state, scene));
        let c = state.quota.claude.five_hour.as_ref().unwrap();
        let x = state.quota.codex.five_hour.as_ref().unwrap();
        close(c.left(), claude); close(x.left(), codex);
        assert_eq!(level(c.left()), expected); assert_eq!(level(x.left()), expected);
        if scene == "quota-out" { assert!(c.resets_at.is_some()); }
    }
    let mut state = demo::state();
    assert!(demo::apply_data_scene(&mut state, "quota-unknown"));
    assert_eq!(state.quota, Quotas::default());
    assert!(demo::apply_data_scene(&mut state, "quota-low"));
    close(state.quota.claude.five_hour.as_ref().unwrap().left(), 0.12);
    close(state.quota.codex.five_hour.as_ref().unwrap().left(), 0.08);
    for scene in ["board-writing", "board-failed"] {
        let mut state = demo::state();
        assert!(demo::apply_data_scene(&mut state, scene));
        for (id, entry) in &state.board {
            assert_eq!(entry.text, initial.board[id].text);
            if scene == "board-writing" { assert_eq!(entry.status, BoardStatus::Writing); }
            else { assert!(matches!(entry.status, BoardStatus::Failed(_))); }
        }
    }
    let mut state = demo::state();
    assert!(!demo::apply_data_scene(&mut state, "unknown"));
    assert_eq!(state.runs, initial.runs); assert_eq!(state.memory, initial.memory); assert_eq!(state.checks, initial.checks);
    assert_eq!(state.feeds, initial.feeds); assert_eq!(state.quota, initial.quota); assert_eq!(state.board, initial.board);
}
