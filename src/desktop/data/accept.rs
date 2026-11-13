use super::feed::FeedCore;
use super::outbox::{Outbox, Outcome, TicketState};
use super::*;
use crate::base::Failure;
use crate::desktop::components::panel::{PanelState, RenderGuard};
use std::prelude::v1::test;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

fn apply(
    core: &FeedCore<i32>,
    result: crate::base::Res<(Delta<i32>, Cursor)>,
    now: i64,
    fault: Option<faults::Fault>,
) {
    let generation = core.begin_poll();
    core.apply(generation, result, now, fault);
}
#[test]
fn generations_and_deltas() {
    let core = FeedCore::new(true, None);
    let old = core.begin_poll();
    let new = core.begin_poll();
    core.apply(new, Ok((Delta::Replace(2), Cursor(2))), 20, None);
    core.apply(old, Ok((Delta::Replace(1), Cursor(1))), 30, None);
    assert_eq!(*core.snapshot().value.unwrap(), 2);
    assert_eq!(core.cursor(), Cursor(2));
    let old_snapshot = core.snapshot();
    apply(
        &core,
        Ok((Delta::Patch(Box::new(|v| *v += 3)), Cursor(3))),
        40,
        None,
    );
    assert_eq!(*core.snapshot().value.unwrap(), 5);
    assert_eq!(*old_snapshot.value.unwrap(), 2);
    apply(&core, Ok((Delta::Unchanged, Cursor(4))), 50, None);
    assert_eq!(core.snapshot().state, PanelState::Ready);
}
#[test]
fn faults_errors_and_recovery() {
    use faults::Fault;
    let core = FeedCore::new(true, Some(Duration::from_millis(100)));
    for (fault, expected) in [
        (
            Fault::Corrupt,
            PanelState::Corrupt {
                reason: "injected corrupt".into(),
            },
        ),
        (Fault::Missing, PanelState::Empty),
        (
            Fault::Offline,
            PanelState::Offline {
                reason: "injected offline".into(),
            },
        ),
        (
            Fault::Stale { age_ms: 50 },
            PanelState::Stale { since_ms: 950 },
        ),
        (Fault::Slow { ms: 0 }, PanelState::Ready),
        (Fault::Panic, PanelState::Ready),
    ] {
        apply(&core, Ok((Delta::Replace(7), Cursor(1))), 900, None);
        apply(&core, Ok((Delta::Replace(8), Cursor(2))), 1000, Some(fault));
        let s = core.snapshot();
        assert_eq!(s.state, expected);
        if matches!(
            s.state,
            PanelState::Offline { .. } | PanelState::Stale { .. }
        ) {
            assert_eq!(*s.value.unwrap(), 7);
        }
        apply(&core, Ok((Delta::Replace(9), Cursor(3))), 1001, None);
        assert_eq!(core.snapshot().state, PanelState::Ready);
    }
    apply(&core, Err(Failure::new("unreachable", "gone")), 1200, None);
    assert_eq!(core.snapshot().state, PanelState::Stale { since_ms: 1001 });
    let other = FeedCore::<i32>::new(false, None);
    apply(&other, Err(Failure::new("not_found", "missing")), 1, None);
    assert!(matches!(other.snapshot().state, PanelState::Failed { .. }));
}
#[test]
fn snapshot_during_poll() {
    let core = FeedCore::<i32>::new(true, None);
    let copy = core.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    struct BlockingSource(mpsc::Sender<()>, mpsc::Receiver<()>);
    impl Source<i32> for BlockingSource {
        fn poll(&mut self, _: Cursor) -> crate::base::Res<(Delta<i32>, Cursor)> {
            self.0.send(()).unwrap();
            self.1.recv().unwrap();
            Ok((Delta::Replace(1), Cursor(1)))
        }
    }
    let worker = std::thread::spawn(move || {
        let mut source = BlockingSource(started_tx, release_rx);
        let generation = copy.begin_poll();
        let result = source.poll(copy.cursor());
        copy.apply(generation, result, 1, None);
    });
    started_rx.recv().unwrap();
    let (tx, rx) = mpsc::channel();
    let copy = core.clone();
    std::thread::spawn(move || tx.send(copy.snapshot().state).unwrap());
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        PanelState::Loading
    );
    release_tx.send(()).unwrap();
    worker.join().unwrap();
}
fn settled(ticket: &outbox::Ticket) -> TicketState {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let state = ticket.state();
        if state != TicketState::Pending {
            return state;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn outbox_deduplicates_retries_and_retains() {
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    let outbox = Outbox::with_max_attempts(
        move |action: &u8, key: &str| {
            assert!(!key.is_empty());
            let n = calls.fetch_add(1, Ordering::SeqCst);
            match action {
                1 if n == 0 => Outcome::Retry("again".into()),
                1 => Outcome::Applied,
                2 => Outcome::Refused("no".into()),
                _ => Outcome::Retry("bad".into()),
            }
        },
        2,
    );
    let a = outbox.submit(1, "a");
    let duplicate = outbox.submit(1, "a");
    assert!(a.same_ticket(&duplicate));
    assert_eq!(settled(&a), TicketState::Applied);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let b = outbox.submit(2, "b");
    let c = outbox.submit(3, "c");
    assert_eq!(settled(&b), TicketState::Refused("no".into()));
    assert_eq!(settled(&c), TicketState::Failed("bad".into()));
    assert_eq!(outbox.tickets().len(), 3);
}
#[test]
fn io_render_guard_and_negative_control() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, b"ok").unwrap();
    assert_eq!(io::read(&path).unwrap(), b"ok");
    assert!(io::metadata(&path).is_ok());
    assert!(io::read_dir(dir.path()).is_ok());
    for f in [0, 1, 2] {
        let result = std::panic::catch_unwind(|| {
            let _guard = RenderGuard::new("test-panel");
            match f {
                0 => {
                    let _ = io::read(&path);
                }
                1 => {
                    let _ = io::read_dir(dir.path());
                }
                _ => {
                    let _ = io::metadata(&path);
                }
            }
        });
        let panic = result.unwrap_err();
        assert_eq!(
            panic.downcast_ref::<String>().unwrap(),
            "io during render test-panel"
        );
    }
}

#[test]
fn error_kinds_versions_and_last_good() {
    let core = FeedCore::new(true, Some(Duration::from_millis(100)));
    assert_eq!(core.snapshot().version, 0);
    apply(&core, Ok((Delta::Replace(4), Cursor(4))), 100, None);
    let previous = core.snapshot();
    apply(&core, Err(Failure::new("offline", "down")), 150, None);
    let snapshot = core.snapshot();
    assert_eq!(
        snapshot.state,
        PanelState::Offline {
            reason: "down".into()
        }
    );
    assert_eq!(*snapshot.value.unwrap(), 4);
    assert_eq!(snapshot.updated_ms, Some(100));
    assert_eq!(snapshot.version, previous.version + 1);
    assert_eq!(core.cursor(), Cursor(4));
    apply(&core, Err(Failure::new("io", "broken")), 201, None);
    assert_eq!(core.snapshot().state, PanelState::Stale { since_ms: 100 });
    assert_eq!(*core.snapshot().value.unwrap(), 4);
    apply(
        &core,
        Ok((Delta::Replace(5), Cursor(5))),
        202,
        Some(faults::Fault::Offline),
    );
    assert!(matches!(core.snapshot().state, PanelState::Offline { .. }));
    apply(
        &core,
        Err(Failure::new("corrupt_artifact", "invalid")),
        203,
        None,
    );
    assert_eq!(
        core.snapshot().state,
        PanelState::Corrupt {
            reason: "invalid".into()
        }
    );
    apply(&core, Err(Failure::new("not_found", "missing")), 204, None);
    assert_eq!(core.snapshot().state, PanelState::Empty);
    let without_age = FeedCore::<i32>::new(true, None);
    apply(
        &without_age,
        Err(Failure::new("unreachable", "down")),
        100,
        None,
    );
    assert_eq!(
        without_age.snapshot().state,
        PanelState::Offline {
            reason: "down".into()
        }
    );
    apply(
        &without_age,
        Err(Failure::new("unexpected", "bad")),
        101,
        None,
    );
    assert_eq!(
        without_age.snapshot().state,
        PanelState::Failed {
            reason: "bad".into()
        }
    );
}

#[test]
fn outbox_once_and_fifo() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let worker_seen = seen.clone();
    let outbox = Outbox::new(move |action: &u8, key: &str| {
        worker_seen.lock().unwrap().push((*action, key.to_owned()));
        Outcome::Applied
    });
    let first = outbox.submit(1, "same");
    let duplicate = outbox.submit(9, "same");
    let last = outbox.submit(2, "next");
    assert!(first.same_ticket(&duplicate));
    assert_eq!(settled(&last), TicketState::Applied);
    assert_eq!(settled(&first), TicketState::Applied);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(1, "same".into()), (2, "next".into())]
    );
    assert_eq!(outbox.tickets().len(), 2);
}

#[test]
fn slow_fault_delay_and_source_defaults() {
    struct Defaults;
    impl Source<i32> for Defaults {
        fn poll(&mut self, _: Cursor) -> crate::base::Res<(Delta<i32>, Cursor)> {
            Ok((Delta::Unchanged, Cursor(0)))
        }
    }
    assert!(Defaults.missing_is_empty());
    assert_eq!(Defaults.stale_after(), None);
    assert_eq!(
        feed::slow_delay(&Some(faults::Fault::Slow { ms: 123 })),
        Some(Duration::from_millis(123))
    );
    assert_eq!(feed::slow_delay(&Some(faults::Fault::Panic)), None);
}

#[test]
fn outbox_default_attempt_limit_and_key_stability() {
    let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = keys.clone();
    let outbox = Outbox::new(move |_: &(), key: &str| {
        recorded.lock().unwrap().push(key.to_owned());
        Outcome::Retry("unavailable".into())
    });
    let ticket = outbox.submit((), "stable");
    assert_eq!(settled(&ticket), TicketState::Failed("unavailable".into()));
    assert_eq!(*keys.lock().unwrap(), vec!["stable"; 5]);
    assert_eq!(outbox.tickets().len(), 1);
}

#[test]
fn snapshot_while_patch_runs_and_newer_reply_wins() {
    let core = FeedCore::new(true, None);
    apply(&core, Ok((Delta::Replace(1), Cursor(1))), 1, None);
    let worker_core = core.clone();
    let generation = core.begin_poll();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        worker_core.apply(
            generation,
            Ok((
                Delta::Patch(Box::new(move |v| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    *v = 2;
                })),
                Cursor(2),
            )),
            2,
            None,
        )
    });
    started_rx.recv().unwrap();
    let (snapshot_tx, snapshot_rx) = mpsc::channel();
    let snapshot_core = core.clone();
    std::thread::spawn(move || {
        let _ = snapshot_tx.send(snapshot_core.snapshot());
    });
    let snapshot = snapshot_rx.recv_timeout(Duration::from_secs(1));
    if snapshot.is_err() {
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        panic!("snapshot blocked during patch");
    }
    assert_eq!(*snapshot.unwrap().value.unwrap(), 1);
    apply(&core, Ok((Delta::Replace(3), Cursor(3))), 3, None);
    release_tx.send(()).unwrap();
    assert!(!worker.join().unwrap());
    assert_eq!(*core.snapshot().value.unwrap(), 3);
    assert_eq!(core.cursor(), Cursor(3));
}
