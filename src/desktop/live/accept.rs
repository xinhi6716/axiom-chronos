use super::tests::{allow_test_wish_storage, memory_entries, rig, state_with};
use super::*;

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
    rig.engine
        .handle(Command::CorrectMemory(id, "new body".into()));
    let updates = rig.updates();
    let recorded = updates
        .iter()
        .find_map(|u| match u {
            Update::Corrected { id: found, prior } if *found == id => Some(prior),
            _ => None,
        })
        .expect("successful correction records undo");
    assert_eq!(
        (
            &recorded.body,
            &recorded.summary,
            recorded.corrected,
            recorded.confirmed
        ),
        (
            &prior.body,
            &prior.summary,
            prior.corrected,
            prior.confirmed
        )
    );
    let mut state = state_with(vec![]);
    apply(&mut state, updates);
    assert_eq!(state.undo.len(), 1);
    let Undoable::Memory { id, prior: undo } = state.take_undo().unwrap() else {
        panic!("memory undo")
    };
    rig.engine
        .handle(Command::RestoreMemory { id, prior: undo });
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
    rig.engine
        .handle(Command::CorrectMemory(-1, "new body".into()));
    let updates = rig.updates();
    assert!(updates.iter().any(|u| matches!(
        u,
        Update::Failed {
            action: Action::CorrectMemory,
            ..
        }
    )));
    assert!(!updates
        .iter()
        .any(|u| matches!(u, Update::Corrected { .. })));
    let mut state = state_with(vec![]);
    apply(&mut state, updates);
    assert!(state.undo.is_empty());
}

#[test]
fn memory_failure_preserves_data_and_distinguishes_stale_and_failed() {
    let mut rig = rig();
    let home = rig.engine.home.clone();
    memory_entries(&home);
    // These replies are supplied directly; do not launch a concurrent initial read.
    rig.engine.memory_dirty = false;
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
    let feed = updates
        .iter()
        .find_map(|u| match u {
            Update::Feed {
                kind: FeedKind::Memory,
                error: Some(e),
            } => Some(e.clone()),
            _ => None,
        })
        .expect("failure status");
    apply(&mut state, updates);
    assert_eq!(state.memory, before);
    assert_eq!(
        panel_state(&state.feeds.memory, false),
        PanelState::Stale(feed.clone().into())
    );
    let mut never = state_with(vec![]);
    never.feeds.memory = FeedStatus::default();
    apply(
        &mut never,
        vec![Update::Feed {
            kind: FeedKind::Memory,
            error: Some(feed.clone()),
        }],
    );
    assert_eq!(
        panel_state(&never.feeds.memory, true),
        PanelState::Failed(feed.into())
    );
    rig.engine.handle(Command::Memory(Err(failure)));
    assert!(!rig.updates().iter().any(|u| matches!(
        u,
        Update::Feed {
            kind: FeedKind::Memory,
            ..
        }
    )));
}

#[test]
fn quit_service_returns_connection_failure_synchronously() {
    let home = tempfile::tempdir().unwrap();
    let (commands, receiver) = mpsc::channel();
    let (_done, finished) = mpsc::channel();
    let backend = LiveBackend {
        commands,
        home: home.path().to_path_buf(),
        finished: Mutex::new(finished),
    };
    assert!(backend.quit_service(true).is_err());
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
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
    assert!(matches!(
        rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
        Command::Memory(Ok(_))
    ));
}

#[test]
fn reload_during_memory_read_schedules_a_fresh_read() {
    let mut rig = rig();
    rig.engine.schedule_memory();
    rig.engine.handle(Command::Reload(FeedKind::Memory));
    let old_reply = rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    rig.engine.handle(old_reply);
    assert!(rig.engine.memory_busy);
    assert!(!rig.updates().iter().any(|u| matches!(
        u,
        Update::Feed {
            kind: FeedKind::Memory,
            ..
        }
    )));
    let fresh_reply = rig._receiver.recv_timeout(Duration::from_secs(10)).unwrap();
    rig.engine.handle(fresh_reply);
    assert!(rig.updates().iter().any(|u| matches!(
        u,
        Update::Feed {
            kind: FeedKind::Memory,
            error: None
        }
    )));
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
    assert_eq!(
        panel_state(&state.feeds.runs, true),
        PanelState::Stale("unowned warning".into())
    );
    rig.engine.report_warnings();
    assert!(!rig
        .updates()
        .iter()
        .any(|u| matches!(u, Update::Feed { .. })));
    rig.engine.snapshot.as_mut().unwrap().warnings.clear();
    rig.engine.report_warnings();
    rig.engine.schedule_ledger();
    rig.engine.handle(Command::Checks(vec![]));
    apply(&mut state, rig.updates());
    assert_eq!(panel_state(&state.feeds.runs, true), PanelState::Empty);
    assert!(state.feeds.usage.loaded);
    assert!(state.feeds.checks.loaded);
    rig.engine.handle(Command::Ledger {
        session: "x".into(),
        activity: None,
        result: Err(Failure::new("io", "ledger broken")),
    });
    apply(&mut state, rig.updates());
    assert_eq!(
        panel_state(&state.feeds.usage, true),
        PanelState::Stale("ledger broken".into())
    );
    rig.engine.schedule_ledger();
    assert!(!rig.updates().iter().any(|u| matches!(
        u,
        Update::Feed {
            kind: FeedKind::Usage,
            ..
        }
    )));
    rig.engine.handle(Command::Ledger {
        session: "x".into(),
        activity: None,
        result: Err(Failure::new("not_found", "absent")),
    });
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
    rig.engine.handle(Command::RestoreMemory {
        id: -1,
        prior: MemoryPrior {
            body: prior.body.clone(),
            summary: prior.summary.clone(),
            corrected: prior.corrected,
            confirmed: prior.confirmed,
        },
    });
    let updates = rig.updates();
    assert_eq!(
        updates
            .iter()
            .filter(|u| matches!(
                u,
                Update::Failed {
                    action: Action::CorrectMemory,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(!updates
        .iter()
        .any(|u| matches!(u, Update::Corrected { .. } | Update::Memory(_))));
    assert_eq!(memory.prior(id).unwrap(), prior);
    assert_eq!(rig.engine.memory, cached);
}

fn test_python() -> String {
    if let Ok(python) = std::env::var("AXIOM_TEST_PYTHON") {
        return python;
    }
    let names = if cfg!(windows) {
        ["python", "python3"]
    } else {
        ["python3", "python"]
    };
    names
        .into_iter()
        .find(|name| {
            crate::base::proc::cmd(name)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success())
        })
        .expect("Python not found")
        .into()
}

#[test]
fn service_snapshot_rebuild_gap_resync_and_disk_pause() {
    use crate::service::{login, tray, Server, ServerOptions};
    let mut rig = rig();
    let home = rig.engine.home.clone();
    allow_test_wish_storage(&home);
    let config_home = home.clone();
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake_claude.py");
    let record = home.join("fake-record.json");
    let calls = || Arc::new(Mutex::new(Vec::new()));
    let server = Server::start(ServerOptions {
        home: home.clone(),
        config: Arc::new(move || Config {
            claude: Ok(vec![test_python(), "-I".into(), "-c".into(),
                "import os,runpy,sys; os.environ['FAKE_CLAUDE_SCENARIO']='tick'; os.environ['FAKE_CLAUDE_RECORD']=sys.argv[2]; sys.argv=[sys.argv[1]]+sys.argv[3:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(),
                fake.to_string_lossy().into_owned(), record.to_string_lossy().into_owned()]),
            home: config_home.clone(), bin: config_home.join("bin"), clock: session::Clock::system(),
        }),
        tray: Box::new(tray::Recording::new(calls())),
        desktop: Box::new(tray::Recording::new(calls())),
        login: Box::new(login::Recording::new(calls())), login_launch: false,
    }).unwrap();
    let client = Client::connect(&home).unwrap();
    let handle = client
        .start(service::StartRequest {
            wish: "test wish".into(),
            cwd: home.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let snapshot = loop {
        let snapshot = client.snapshot().unwrap();
        if snapshot
            .wishes
            .iter()
            .any(|w| w.handle == handle && !w.log.is_empty())
        {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "service did not produce a log");
        std::thread::sleep(Duration::from_millis(50));
    };
    rig.engine.service_enabled = true;
    rig.engine.service = Some(client.clone());
    rig.engine.rebuild_service(snapshot);
    let run = rig.engine.compose().pop().unwrap();
    assert!(run.hosted);
    assert!(run.log.iter().any(|l| l.text.contains("tick")));
    rig.engine.hosted.clear();
    rig.engine
        .subscription_failure(Failure::new("gap", "events lost"));
    assert_eq!(rig.engine.hosted.len(), 1);
    assert!(rig.engine.subscription.is_some());
    rig.engine.wire_event(service::WireEvent {
        seq: 0,
        instance: rig.engine.service_instance.clone(),
        handle: handle.clone(),
        session: None,
        kind: "disk_paused".into(),
        data: serde_json::json!({"line":"disk full"}),
    });
    rig.engine.publish();
    let mut state = state_with(vec![]);
    apply(&mut state, rig.updates());
    assert!(state
        .runs
        .iter()
        .flat_map(|r| &r.log)
        .any(|l| l.level == LogLevel::Warn && l.text.contains("disk full")));
    assert!(state.errors.iter().any(|e| e.message.contains("disk full")));
    rig.engine.shutdown();
    assert!(!client.snapshot().unwrap().wishes.is_empty());
    server.quit(true).unwrap();
    server.wait();
}

#[test]
fn confirm_quit_option_is_recognized() {
    assert!(
        super::super::parse_options(&["--confirm-quit".into()])
            .unwrap()
            .confirm_quit
    );
    assert!(!super::super::parse_options(&[]).unwrap().confirm_quit);
}

#[test]
fn quit_requested_event_updates_state_without_a_hosted_wish() {
    let mut rig = rig();
    let mut state = state_with(vec![]);
    assert!(!state.feeds.quit_requested);
    rig.engine.wire_event(service::WireEvent {
        seq: 1,
        instance: "instance".into(),
        handle: String::new(),
        session: None,
        kind: "quit_requested".into(),
        data: serde_json::json!({"running": true}),
    });
    apply(&mut state, rig.updates());
    assert!(state.feeds.quit_requested);
}

#[test]
fn service_snapshots_set_and_clear_login_start() {
    let mut rig = rig();
    let mut state = state_with(vec![]);
    assert!(!state.feeds.login_start);
    for on in [true, false] {
        rig.engine.rebuild_service(service::Snapshot {
            instance: "instance".into(),
            cursor: 0,
            login_start: on,
            wishes: vec![],
        });
        apply(&mut state, rig.updates());
        assert_eq!(state.feeds.login_start, on);
    }
}

#[derive(Default)]
struct Recording;
impl Backend for Recording {
    fn start_wish(&self, _: WishRequest) -> Res<()> {
        Ok(())
    }
    fn interject(&self, _: &str, _: &str) -> Res<()> {
        Ok(())
    }
    fn stop(&self, _: &str) -> Res<()> {
        Ok(())
    }
    fn resume(&self, _: &str, _: Option<&str>) -> Res<()> {
        Ok(())
    }
    fn delete_memory(&self, _: i64) -> Res<()> {
        Ok(())
    }
    fn correct_memory(&self, _: i64, _: &str) -> Res<()> {
        Ok(())
    }
    fn refresh(&self) -> Res<()> {
        Ok(())
    }
    fn recheck(&self) -> Res<()> {
        Ok(())
    }
    fn follow(&self, _: &str) -> Res<()> {
        Ok(())
    }
}

fn running_sessions(state: &AppState) -> Vec<String> {
    let sessions: Vec<_> = crate::desktop::shell::running_here(&state.runs)
        .iter()
        .map(|run| run.session.to_string())
        .collect();
    assert!(!sessions.is_empty());
    sessions
}

#[test]
/// The service owns the wish, so it keeps running while the service stays up and can reattach.
fn service_backed_real_marker_closes_at_once_and_reattaches() {
    use crate::desktop::shell::{self, ClosePlan};
    use crate::service::{login, tray, Server, ServerOptions};
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    allow_test_wish_storage(&home);
    let config_home = home.clone();
    let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake_claude.py");
    let record = home.join("fake-record.json");
    let calls = || Arc::new(Mutex::new(Vec::new()));
    let server = Server::start(ServerOptions {
        home: home.clone(),
        config: Arc::new(move || Config {
            claude: Ok(vec![test_python(), "-I".into(), "-c".into(),
                "import os,runpy,sys; os.environ['FAKE_CLAUDE_SCENARIO']='tick'; os.environ['FAKE_CLAUDE_RECORD']=sys.argv[2]; sys.argv=[sys.argv[1]]+sys.argv[3:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(),
                fake.to_string_lossy().into_owned(), record.to_string_lossy().into_owned()]),
            home: config_home.clone(), bin: config_home.join("bin"), clock: session::Clock::system(),
        }),
        tray: Box::new(tray::Recording::new(calls())),
        desktop: Box::new(tray::Recording::new(calls())),
        login: Box::new(login::Recording::new(calls())), login_launch: false,
    }).unwrap();
    let client = Client::connect(&home).unwrap();
    let handle = client
        .start(service::StartRequest {
            wish: "test wish".into(),
            cwd: home.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let session = loop {
        let snapshot = client.snapshot().unwrap();
        if let Some(session) = snapshot
            .wishes
            .iter()
            .find(|w| w.handle == handle)
            .and_then(|w| w.session.clone())
        {
            break session;
        }
        let wish = snapshot.wishes.iter().find(|w| w.handle == handle);
        assert!(
            Instant::now() < deadline,
            "service did not report a session: state={:?}, message={:?}",
            wish.map(|w| &w.state),
            wish.and_then(|w| w.disk_line.as_ref())
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut state = crate::desktop::demo::state();
    let run = state
        .runs
        .iter_mut()
        .find(|run| run.hosted && run.status == RunStatus::Active)
        .expect("demo hosted active run");
    run.session = session.clone().into();
    let sessions = running_sessions(&state);
    let background: Arc<dyn BackgroundService> = Arc::new(ServiceBackground { home: home.clone() });
    install_service_background(&mut state, Box::new(Recording), background);
    assert!(shell::service_backed(&state));
    assert_eq!(
        shell::close_plan_for(&state, &sessions),
        ClosePlan::CloseNow
    );
    assert!(
        client
            .snapshot()
            .unwrap()
            .wishes
            .iter()
            .any(|w| w.handle == handle && w.state != "ended"),
        "the service-owned wish keeps going while the service stays up"
    );
    assert_eq!(
        ServiceBackground { home }.reattach().unwrap(),
        vec![session]
    );
    server.quit(true).unwrap();
    server.wait();
}

#[test]
fn service_backed_fake_replaced_dead_and_missing_go_through_confirm() {
    use crate::desktop::{
        shell::{self, ClosePlan},
        ux::seams::FakeBackground,
    };
    let home = tempfile::tempdir().unwrap();
    let real = || {
        Arc::new(ServiceBackground {
            home: home.path().to_path_buf(),
        }) as Arc<dyn BackgroundService>
    };
    let mut state = crate::desktop::demo::state();
    let sessions = running_sessions(&state);
    state.backend = Some(Box::new(Recording));
    state.ux.seams.background = Some(Arc::new(FakeBackground::new(true)));
    assert!(!shell::service_backed(&state));
    assert_eq!(
        shell::close_plan_for(&state, &sessions),
        ClosePlan::Continue {
            sessions: sessions.clone()
        }
    );

    let original = real();
    install_service_background(&mut state, Box::new(Recording), original.clone());
    assert!(shell::service_backed(&state));
    state.ux.seams.background = Some(real());
    assert!(state
        .service_background
        .as_ref()
        .unwrap()
        .upgrade()
        .is_some());
    assert!(!shell::service_backed(&state));
    assert_eq!(
        shell::close_plan_for(&state, &sessions),
        ClosePlan::Continue {
            sessions: sessions.clone()
        }
    );
    drop(original);

    install_service_background(&mut state, Box::new(Recording), real());
    state.ux.seams.background = None;
    assert!(state
        .service_background
        .as_ref()
        .unwrap()
        .upgrade()
        .is_none());
    assert!(!shell::service_backed(&state));
    assert_eq!(
        shell::close_plan_for(&state, &sessions),
        ClosePlan::AskStop {
            sessions: sessions.clone()
        }
    );

    install_service_background(&mut state, Box::new(Recording), real());
    state.backend = None;
    assert!(!shell::service_backed(&state));
    assert_eq!(
        shell::close_plan_for(&state, &sessions),
        ClosePlan::Continue { sessions }
    );
}
