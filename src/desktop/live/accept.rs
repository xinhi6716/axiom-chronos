use super::*;
use super::tests::{memory_entries, rig, state_with};

#[test]
fn correction_records_the_actual_prior_and_undo_restores_it() {
    let mut rig = rig();
    let home = rig.engine.home.clone();
    let (id, _) = memory_entries(&home);
    let memory = Memory::open(&home).unwrap();
    memory.correct(id, "previous correction").unwrap();
    let prior = memory.prior(id).unwrap();
    rig.engine.handle(Command::Memory(read_memory(&home)));
    rig.updates();
    rig.engine.handle(Command::CorrectMemory(id, "new body".into()));
    let updates = rig.updates();
    let recorded = updates.iter().find_map(|u| match u {
        Update::Corrected { id: found, prior } if *found == id => Some(prior),
        _ => None,
    }).expect("successful correction records undo");
    assert_eq!((&recorded.body, &recorded.summary, recorded.corrected, recorded.confirmed),
        (&prior.body, &prior.summary, prior.corrected, prior.confirmed));
    let mut state = state_with(vec![]);
    apply(&mut state, updates);
    assert_eq!(state.undo.len(), 1);
    let Undoable::Memory { id, prior: undo } = state.take_undo().unwrap() else { panic!("memory undo") };
    rig.engine.handle(Command::RestoreMemory { id, prior: undo });
    apply(&mut state, rig.updates());
    assert_eq!(memory.prior(id).unwrap(), prior);
    let entry = state.memory.iter().find(|e| e.id == id).unwrap();
    assert_eq!(entry.body.as_ref(), prior.body);
    assert_eq!(entry.corrected, prior.corrected.is_some());
    assert_eq!(entry.confirmed.as_ref(), confirmed_text(prior.confirmed));
}

#[test]
fn failed_correction_does_not_record_undo() {
    let mut rig = rig();
    rig.engine.handle(Command::CorrectMemory(-1, "new body".into()));
    let updates = rig.updates();
    assert!(updates.iter().any(|u| matches!(u, Update::Failed { action: Action::CorrectMemory, .. })));
    assert!(!updates.iter().any(|u| matches!(u, Update::Corrected { .. })));
    let mut state = state_with(vec![]);
    apply(&mut state, updates);
    assert!(state.undo.is_empty());
}

#[test]
fn memory_failure_preserves_data_and_distinguishes_stale_and_failed() {
    let mut rig = rig();
    let home = rig.engine.home.clone();
    memory_entries(&home);
    rig.engine.handle(Command::Memory(read_memory(&home)));
    let mut state = state_with(vec![]);
    state.feeds.memory = FeedStatus::default();
    apply(&mut state, rig.updates());
    let before = state.memory.clone();
    assert!(state.feeds.memory.loaded);
    std::fs::write(home.join("memory/memory.db"), b"not a database").unwrap();
    let failure = read_memory(&home).unwrap_err();
    rig.engine.handle(Command::Memory(Err(failure.clone())));
    let updates = rig.updates();
    let feed = updates.iter().find_map(|u| match u {
        Update::Feed { kind: FeedKind::Memory, error: Some(e) } => Some(e.clone()), _ => None,
    }).expect("failure status");
    apply(&mut state, updates);
    assert_eq!(state.memory, before);
    assert_eq!(panel_state(&state.feeds.memory, false), PanelState::Stale(feed.clone().into()));
    let mut never = state_with(vec![]);
    never.feeds.memory = FeedStatus::default();
    apply(&mut never, vec![Update::Feed { kind: FeedKind::Memory, error: Some(feed.clone()) }]);
    assert_eq!(panel_state(&never.feeds.memory, true), PanelState::Failed(feed.into()));
    rig.engine.handle(Command::Memory(Err(failure)));
    assert!(!rig.updates().iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Memory, .. })));
}

#[test]
fn reload_memory_schedules_another_read() {
    let mut rig = rig();
    rig.engine.schedule_memory();
    let reply = rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    rig.engine.handle(reply);
    assert!(!rig.engine.memory_busy);
    rig.engine.handle(Command::Reload(FeedKind::Memory));
    assert!(rig.engine.memory_busy);
    assert!(matches!(rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap(), Command::Memory(Ok(_))));
}

#[test]
fn reload_during_memory_read_schedules_a_fresh_read() {
    let mut rig = rig();
    rig.engine.schedule_memory();
    rig.engine.handle(Command::Reload(FeedKind::Memory));
    let old_reply = rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    rig.engine.handle(old_reply);
    assert!(rig.engine.memory_busy);
    assert!(!rig.updates().iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Memory, .. })));
    let fresh_reply = rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    rig.engine.handle(fresh_reply);
    assert!(rig.updates().iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Memory, error: None })));
}

#[test]
fn runs_usage_and_checks_report_and_deduplicate_status() {
    let mut rig = rig();
    let mut snapshot = rig.engine.poller.poll();
    snapshot.warnings = vec!["unowned warning".into()];
    assert!(snapshot.wishes.is_empty());
    rig.engine.snapshot = Some(snapshot);
    rig.engine.report_warnings();
    let updates = rig.updates();
    let mut state = state_with(vec![]);
    state.feeds = Feeds::default();
    apply(&mut state, updates);
    assert_eq!(panel_state(&state.feeds.runs, true), PanelState::Stale("unowned warning".into()));
    rig.engine.report_warnings();
    assert!(!rig.updates().iter().any(|u| matches!(u, Update::Feed { .. })));
    rig.engine.snapshot.as_mut().unwrap().warnings.clear();
    rig.engine.report_warnings();
    rig.engine.schedule_ledger();
    rig.engine.handle(Command::Checks(vec![]));
    apply(&mut state, rig.updates());
    assert_eq!(panel_state(&state.feeds.runs, true), PanelState::Empty);
    assert!(state.feeds.usage.loaded);
    assert!(state.feeds.checks.loaded);
    rig.engine.handle(Command::Ledger { session: "x".into(), activity: None, result: Err(Failure::new("io", "ledger broken")) });
    apply(&mut state, rig.updates());
    assert_eq!(panel_state(&state.feeds.usage, true), PanelState::Stale("ledger broken".into()));
    rig.engine.schedule_ledger();
    assert!(!rig.updates().iter().any(|u| matches!(u, Update::Feed { kind: FeedKind::Usage, .. })));
    rig.engine.handle(Command::Ledger { session: "x".into(), activity: None, result: Err(Failure::new("not_found", "absent")) });
    apply(&mut state, rig.updates());
    assert_eq!(panel_state(&state.feeds.usage, true), PanelState::Empty);
    assert!(!state.feeds.quota.loaded);
}

#[test]
fn invalid_correction_and_failed_restore_leave_memory_and_undo_unchanged() {
    let mut rig = rig();
    let home = rig.engine.home.clone();
    let (id, _) = memory_entries(&home);
    let memory = Memory::open(&home).unwrap();
    let prior = memory.prior(id).unwrap();
    rig.engine.handle(Command::Memory(read_memory(&home)));
    rig.updates();
    let cached = rig.engine.memory.clone();
    rig.engine.handle(Command::CorrectMemory(id, "   ".into()));
    rig.engine.handle(Command::RestoreMemory { id: -1, prior: MemoryPrior {
        body: prior.body.clone(), summary: prior.summary.clone(), corrected: prior.corrected, confirmed: prior.confirmed,
    }});
    let updates = rig.updates();
    assert_eq!(updates.iter().filter(|u| matches!(u, Update::Failed { action: Action::CorrectMemory, .. })).count(), 2);
    assert!(!updates.iter().any(|u| matches!(u, Update::Corrected { .. } | Update::Memory(_))));
    assert_eq!(memory.prior(id).unwrap(), prior);
    assert_eq!(rig.engine.memory, cached);
}
