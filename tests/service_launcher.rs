mod common;
use axiom::{
    base::Res,
    modes::{
        chat::{Chat, ChatOptions},
        launch::{LaunchKind, LaunchRequest, Launcher},
        plaza::router::Router,
        providers::{Ask, Member, Provider},
        Mode,
    },
    routines::{runner::tick, store::Store, validate, Event, EventKind, Routine, Trigger},
    seams::{FakeApproval, FakeClock},
    service::{
        launcher::{ServiceLauncher, StartPath, Unavailable},
        StartRequest,
    },
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct FakeStart(Mutex<Vec<StartRequest>>);
impl StartPath for FakeStart {
    fn start(&self, req: StartRequest) -> Res<String> {
        let mut all = self.0.lock().unwrap();
        all.push(req);
        Ok(format!("handle-{}", all.len()))
    }
}

fn launcher(home: &std::path::Path) -> (ServiceLauncher, Arc<FakeStart>) {
    let start = Arc::new(FakeStart::default());
    (
        ServiceLauncher {
            home: home.into(),
            start: start.clone(),
        },
        start,
    )
}

#[test]
fn routine_dedup_and_promotion_reach_the_same_start_path() {
    let home = common::temp();
    let project = common::temp();
    let (l, start) = launcher(home.path());
    let routine = Routine {
        id: "r1".into(),
        project_dir: project.path().into(),
        wish: "Triage".into(),
        enabled: true,
        schedule: None,
        triggers: vec![Trigger {
            kind: EventKind::GithubIssue,
            repo: "o/r".into(),
        }],
    };
    validate(&routine).unwrap();
    let store = Store::open(home.path()).unwrap();
    store.put(&routine).unwrap();
    let event = Event {
        id: "1".into(),
        kind: EventKind::GithubIssue,
        repo: "o/r".into(),
        title: "Bug".into(),
        url: "https://example.com/1".into(),
    };
    let clock = FakeClock::new(5);
    let approval = FakeApproval::approving();
    for _ in 0..2 {
        tick(&store, &clock, &[event.clone()], &l, &approval).unwrap();
    }
    assert_eq!(start.0.lock().unwrap().len(), 1);
    assert!(start.0.lock().unwrap()[0].wish.starts_with("Triage"));

    let chat = Chat::open(
        home.path(),
        Some(project.path()),
        &["src".into()],
        ChatOptions::default(),
    )
    .unwrap();
    let promotion = chat.promote(&l, LaunchKind::Wish, "Ship it", None).unwrap();
    let all = start.0.lock().unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[1].wish.starts_with("Ship it"));
    assert_eq!(all[1].cwd, project.path().to_string_lossy());
    assert_eq!(promotion.child.detail["handle"], "handle-2");
    assert_eq!(promotion.child.run.mode, Mode::Wish);
}

#[test]
fn quick_request_is_invalid_and_never_starts() {
    let home = common::temp();
    let (l, start) = launcher(home.path());
    let chat = Chat::open(home.path(), None, &["src".into()], ChatOptions::default()).unwrap();
    let req = LaunchRequest {
        kind: LaunchKind::Quick,
        session: chat.session.id.clone(),
        parent_run: chat.run.id.clone(),
        goal: "x".into(),
        scope: vec!["src".into()],
        verify: Some("true".into()),
        review: false,
        context: None,
        sources: vec![],
    };
    assert_eq!(
        Launcher::launch(&l, &req).unwrap_err().kind,
        "invalid_argument"
    );
    assert!(start.0.lock().unwrap().is_empty());
}

#[test]
fn invalid_promotion_parent_never_starts_service() {
    let home = common::temp();
    let project = common::temp();
    let (l, start) = launcher(home.path());
    let chat = Chat::open(
        home.path(),
        Some(project.path()),
        &[],
        ChatOptions::default(),
    )
    .unwrap();
    let req = LaunchRequest {
        kind: LaunchKind::Wish,
        session: chat.session.id,
        parent_run: "missing".into(),
        goal: "Ship it".into(),
        scope: vec![],
        verify: None,
        review: false,
        context: None,
        sources: vec![],
    };
    assert!(Launcher::launch(&l, &req).is_err());
    assert!(start.0.lock().unwrap().is_empty());
}

#[test]
fn unavailable_refuses_provider_and_router() {
    let ask = Ask {
        member: Member::Opus,
        mode: Mode::Chat,
        session: "s".into(),
        prompt: "p".into(),
        context: String::new(),
        tools: vec![],
    };
    assert_eq!(Unavailable.ask(&ask).unwrap_err().kind, "unavailable");
    assert_eq!(Unavailable.route("t", "c").unwrap_err().kind, "unavailable");
}
