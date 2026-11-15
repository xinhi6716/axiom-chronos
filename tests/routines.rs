use axiom::{
    base::{Failure, Res},
    routines::{
        runner::{due, tick},
        store::{Outcome, Store},
        validate, Cause, Event, EventKind, FakeLauncher, LaunchRequest, Routine, Schedule, Trigger,
        WishLauncher,
    },
    seams::{sha256_hex, FakeApproval, FakeClock},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    },
};

fn routine() -> Routine {
    Routine {
        id: "my-routine".into(),
        project_dir: PathBuf::from("project"),
        wish: "Fix it".into(),
        enabled: true,
        schedule: Some(Schedule {
            every_minutes: 1,
            start_ms: 1_000,
        }),
        triggers: vec![Trigger {
            kind: EventKind::GithubIssue,
            repo: "Owner/Repo".into(),
        }],
    }
}
fn event() -> Event {
    Event {
        id: "123".into(),
        kind: EventKind::GithubIssue,
        repo: "owner/repo".into(),
        title: "Broken thing".into(),
        url: "https://example.com/123".into(),
    }
}

#[test]
fn schedule_coalesces_and_persists_across_restart() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let mut r = routine();
    r.triggers.clear();
    store.put(&r).unwrap();
    let clock = FakeClock::new(999);
    let launcher = FakeLauncher::new();
    let approval = FakeApproval::approving();
    assert_eq!(due(&r, None, 999), None);
    assert!(tick(&store, &clock, &[], &launcher, &approval)
        .unwrap()
        .is_empty());
    clock.set(1_000);
    let first = tick(&store, &clock, &[], &launcher, &approval).unwrap();
    assert_eq!(first[0].cause, Cause::Schedule { due_ms: 1_000 });
    clock.set(250_000);
    let next = tick(&store, &clock, &[], &launcher, &approval).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].cause, Cause::Schedule { due_ms: 241_000 });
    assert_eq!(launcher.launched().len(), 2);
    let reopened = Store::open(home.path()).unwrap();
    assert!(tick(&reopened, &clock, &[], &launcher, &approval)
        .unwrap()
        .is_empty());
    assert_eq!(reopened.state().unwrap().log.len(), 2);
    assert_eq!(due(&r, None, i64::MAX), Some(9_223_372_036_854_721_000));
}

#[test]
fn event_redelivery_restart_matching_and_approval_request() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let mut r = routine();
    r.schedule = None;
    store.put(&r).unwrap();
    let clock = FakeClock::new(12);
    let launcher = FakeLauncher::new();
    let approval = FakeApproval::approving();
    let mut wrong_repo = event();
    wrong_repo.repo = "other/repo".into();
    let mut wrong_kind = event();
    wrong_kind.kind = EventKind::GithubPr;
    assert!(tick(
        &store,
        &clock,
        &[wrong_repo, wrong_kind],
        &launcher,
        &approval
    )
    .unwrap()
    .is_empty());
    let e = event();
    assert_eq!(
        tick(
            &store,
            &clock,
            &[e.clone(), e.clone()],
            &launcher,
            &approval
        )
        .unwrap()
        .len(),
        1
    );
    let request = &launcher.launched()[0];
    assert!(request.wish.contains(&e.title));
    assert!(request.wish.contains(&e.url));
    let asked = approval.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].subject, "routine:my-routine");
    assert_eq!(asked[0].action, "launch_wish");
    assert_eq!(
        asked[0].digest,
        sha256_hex(&serde_json::to_vec(request).unwrap())
    );
    let reopened = Store::open(home.path()).unwrap();
    assert!(tick(&reopened, &clock, &[e], &launcher, &approval)
        .unwrap()
        .is_empty());
    assert!(reopened.state().unwrap().seen.contains("my-routine/123"));
}

#[test]
fn disabled_does_not_consume_event() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let mut r = routine();
    r.schedule = None;
    r.enabled = false;
    store.put(&r).unwrap();
    let clock = FakeClock::new(1000);
    let launcher = FakeLauncher::new();
    let approval = FakeApproval::approving();
    let e = event();
    assert!(tick(&store, &clock, &[e.clone()], &launcher, &approval)
        .unwrap()
        .is_empty());
    assert!(store.state().unwrap().seen.is_empty());
    r.enabled = true;
    store.put(&r).unwrap();
    assert_eq!(
        tick(&store, &clock, &[e], &launcher, &approval)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn denied_is_claimed_and_not_retried() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    store.put(&routine()).unwrap();
    let clock = FakeClock::new(1_000);
    let launcher = FakeLauncher::new();
    let denial = FakeApproval::denying();
    let records = tick(&store, &clock, &[event()], &launcher, &denial).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.outcome == Outcome::Denied));
    assert!(launcher.launched().is_empty());
    assert_eq!(store.state().unwrap().log, records);
    assert!(tick(
        &store,
        &clock,
        &[event()],
        &launcher,
        &FakeApproval::approving()
    )
    .unwrap()
    .is_empty());
}

struct FailingLauncher(AtomicUsize);
impl WishLauncher for FailingLauncher {
    fn launch(&self, _: &LaunchRequest) -> Res<String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Failure::new("launch", "failure"))
    }
}
#[test]
fn failed_launch_is_logged_and_not_retried() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let mut r = routine();
    r.schedule = None;
    store.put(&r).unwrap();
    let clock = FakeClock::new(1000);
    let launcher = FailingLauncher(AtomicUsize::new(0));
    let approval = FakeApproval::approving();
    let records = tick(&store, &clock, &[event()], &launcher, &approval).unwrap();
    assert!(
        matches!(&records[0].outcome, Outcome::Failed { message } if message.contains("failure"))
    );
    assert_eq!(store.state().unwrap().log, records);
    assert!(tick(&store, &clock, &[event()], &launcher, &approval)
        .unwrap()
        .is_empty());
    assert_eq!(launcher.0.load(Ordering::SeqCst), 1);
}

#[test]
fn validation_and_store_operations() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    let mut r = routine();
    assert!(validate(&r).is_ok());
    for id in ["", "Bad", "under_score", "a/b", &"a".repeat(65)] {
        r.id = id.into();
        assert!(validate(&r).is_err());
        assert!(store.put(&r).is_err());
    }
    r = routine();
    r.wish = " \n".into();
    assert!(validate(&r).is_err());
    r = routine();
    r.schedule.as_mut().unwrap().every_minutes = 0;
    assert!(validate(&r).is_err());
    r = routine();
    r.schedule = None;
    r.triggers.clear();
    assert!(validate(&r).is_err());
    r = routine();
    store.put(&r).unwrap();
    r.wish = "new".into();
    store.put(&r).unwrap();
    assert_eq!(store.routines().unwrap(), vec![r]);
    let state = store.state().unwrap();
    store.save_state(&state).unwrap();
    assert_eq!(store.state().unwrap(), state);
    store.remove("my-routine").unwrap();
    assert!(store.routines().unwrap().is_empty());
}

#[test]
fn concurrent_ticks_claim_once() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(home.path()).unwrap();
    store.put(&routine()).unwrap();
    let launcher = Arc::new(FakeLauncher::new());
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let store = store.clone();
            let launcher = launcher.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                tick(
                    &store,
                    &FakeClock::new(1000),
                    &[event()],
                    &*launcher,
                    &FakeApproval::approving(),
                )
                .unwrap();
            });
        }
    });
    assert_eq!(launcher.launched().len(), 2);
    assert_eq!(store.state().unwrap().log.len(), 2);
}
