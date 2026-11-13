//! Tests for modes; owned by module modes-seam.
use super::modes::{parse_brief, LazyQuick, ModesEngine};
use crate::{
    base::Res,
    desktop::ux::seams::modes::{ModeEngine, ModeRef, PlazaAsk, QuickRequest, QuickStep, Speaker},
    modes::{
        launch::{start_child, LaunchRequest, Launched, Launcher},
        plaza::router::{Pick, Router, Routing},
        providers::{Ask, Member, Provider, Reply},
        Mode,
    },
    service::{
        launcher::{ServiceLauncher, StartPath, Unavailable},
        StartRequest,
    },
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct FakeProvider(AtomicUsize);
impl Provider for FakeProvider {
    fn ask(&self, ask: &Ask) -> Res<Reply> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Reply {
            text: format!("{} says hi", ask.member.name()),
            tool_requests: vec![],
            cost_usd: Some(0.01),
            tokens: None,
        })
    }
}
struct FakeRouter;
impl Router for FakeRouter {
    fn route(&self, _: &str, _: &str) -> Res<Routing> {
        Ok(Routing {
            picks: vec![Pick {
                member: Member::Astra,
                reason: "risk".into(),
            }],
            note: "risk topic".into(),
        })
    }
}
#[derive(Default)]
struct FakeStart(Mutex<Vec<StartRequest>>);
impl StartPath for FakeStart {
    fn start(&self, req: StartRequest) -> Res<String> {
        self.0.lock().unwrap().push(req);
        Ok("h1".into())
    }
}
struct FakeQuick(Mutex<Vec<LaunchRequest>>, std::path::PathBuf);
impl Launcher for FakeQuick {
    fn launch(&self, req: &LaunchRequest) -> Res<Launched> {
        self.0.lock().unwrap().push(req.clone());
        Ok(Launched {
            run: start_child(&self.1, req)?,
            detail: serde_json::json!({"passed": true, "state": "verified"}),
        })
    }
}

struct Rig {
    _home: tempfile::TempDir,
    engine: ModesEngine,
    provider: Arc<FakeProvider>,
    start: Arc<FakeStart>,
    quick: Arc<FakeQuick>,
}
fn rig() -> Rig {
    let home = tempfile::tempdir().unwrap();
    let provider = Arc::new(FakeProvider::default());
    let start = Arc::new(FakeStart::default());
    let quick = Arc::new(FakeQuick(Mutex::default(), home.path().into()));
    let wish = Arc::new(ServiceLauncher {
        home: home.path().into(),
        start: start.clone(),
    });
    let engine = ModesEngine::new(
        home.path(),
        provider.clone(),
        Arc::new(FakeRouter),
        wish,
        quick.clone(),
    );
    Rig {
        _home: home,
        engine,
        provider,
        start,
        quick,
    }
}

#[test]
fn chat_replies_are_persisted_and_handed_off() {
    let r = rig();
    r.engine.chat_send("t1", "Explain the reader").unwrap();
    let thread = r.engine.chat("t1");
    assert_eq!(thread.messages.len(), 2);
    assert_eq!(thread.messages[0].from, Speaker::You);
    assert_eq!(thread.messages[1].from, Speaker::Core);
    let again = ModesEngine::new(
        r.engine.home.as_path(),
        r.provider.clone(),
        Arc::new(FakeRouter),
        r.engine.wish.clone(),
        r.quick.clone(),
    );
    assert_eq!(again.chat("t1").messages.len(), 2);
    let text = r.engine.handoff(&ModeRef::Chat("t1".into())).unwrap();
    assert!(text.contains("Explain the reader") && text.contains("opus says hi"));
}

#[test]
fn damaged_alias_storage_is_not_overwritten() {
    let r = rig();
    r.engine.chat_send("old", "hello").unwrap();
    let aliases = r.engine.home.join("desktop/modes/aliases.json");
    std::fs::write(&aliases, b"invalid json").unwrap();
    assert!(r.engine.chat_send("new", "hello").is_err());
    assert_eq!(std::fs::read(&aliases).unwrap(), b"invalid json");
}

#[test]
fn concurrent_engines_preserve_all_aliases() {
    let r = rig();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut threads = Vec::new();
    for i in 0..8 {
        let engine = ModesEngine::new(
            &r.engine.home,
            r.provider.clone(),
            Arc::new(FakeRouter),
            r.engine.wish.clone(),
            r.quick.clone(),
        );
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            engine.chat_send(&format!("thread-{i}"), "hello").unwrap();
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    for i in 0..8 {
        assert_eq!(r.engine.chat(&format!("thread-{i}")).messages.len(), 2);
    }
}

#[test]
fn promotion_of_unknown_chat_or_plaza_does_not_create_a_mode() {
    let r = rig();
    for from in [
        ModeRef::Chat("unknown-chat".into()),
        ModeRef::Plaza("unknown-plaza".into()),
    ] {
        assert_eq!(
            r.engine.promote_wish(&from, "Ship it").unwrap_err().kind,
            "not_found"
        );
    }
    assert!(r.start.0.lock().unwrap().is_empty());
    assert!(!r.engine.home.join("desktop/modes/aliases.json").exists());
}

#[test]
fn plaza_mentions_routing_and_pins() {
    let r = rig();
    let said = r
        .engine
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "Check this".into(),
                mentions: vec![Speaker::Argon],
            },
        )
        .unwrap();
    assert_eq!(said.picked, vec![Speaker::Argon]);
    let routed = r
        .engine
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "Any risk?".into(),
                mentions: vec![],
            },
        )
        .unwrap();
    assert_eq!(routed.picked, vec![Speaker::Astra]);
    assert_eq!(routed.reason, "risk topic");
    let room = r.engine.plaza("plaza");
    assert_eq!(room.seats.len(), 4);
    assert_eq!(
        room.last_route.as_ref().unwrap().picked,
        vec![Speaker::Astra]
    );
    assert_eq!(room.messages.len(), 4);
    assert!(room.seats.iter().any(|s| s.spent_usd.unwrap_or(0.0) > 0.0));
    let target = room.messages[1].id;
    r.engine.pin("plaza", target).unwrap();
    assert_eq!(r.engine.plaza("plaza").pins[0].sources, vec![target]);
    let handoff = r.engine.handoff(&ModeRef::Plaza("plaza".into())).unwrap();
    assert!(handoff.contains("Check this"));
}

fn wait_done(e: &ModesEngine, id: &str) -> QuickStep {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let step = e.quick(id).unwrap().step;
        if matches!(step, QuickStep::Done { .. } | QuickStep::Failed { .. }) || Instant::now() > end
        {
            return step;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn quick_requires_scope_and_verify_before_any_call() {
    let r = rig();
    for text in ["Fix it", "Fix it\nSCOPE src", "Fix it\nVERIFY true"] {
        let err = r
            .engine
            .quick_start(&QuickRequest {
                cwd: None,
                text: text.into(),
                review: false,
                context: None,
            })
            .unwrap_err();
        assert_eq!(err.kind, "invalid_argument", "{text}");
    }
    assert_eq!(r.provider.0.load(Ordering::SeqCst), 0);
    assert!(r.quick.0.lock().unwrap().is_empty());
}

#[test]
fn quick_runs_with_brief_and_promotes_through_service() {
    let r = rig();
    let project = tempfile::tempdir().unwrap();
    let id = r
        .engine
        .quick_start(&QuickRequest {
            cwd: Some(project.path().into()),
            text: "Fix it\nSCOPE src tests\nVERIFY cargo test".into(),
            review: true,
            context: Some("earlier talk".into()),
        })
        .unwrap();
    assert!(matches!(wait_done(&r.engine, &id), QuickStep::Done { .. }));
    let sent = r.quick.0.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].scope, vec!["src".to_owned(), "tests".to_owned()]);
    assert_eq!(sent[0].verify.as_deref(), Some("cargo test"));
    assert!(sent[0].review);
    drop(sent);
    let handle = r
        .engine
        .promote_wish(&ModeRef::Quick(id.clone()), "Ship the fix")
        .unwrap();
    assert_eq!(handle, "h1");
    let started = r.start.0.lock().unwrap();
    assert_eq!(started.len(), 1);
    assert!(started[0].wish.starts_with("Ship the fix"));
    assert_eq!(started[0].cwd, project.path().to_string_lossy());
    assert!(started[0].wish.contains("earlier talk"));
    assert!(started[0].wish.contains("Fix it"));
}

#[test]
fn quick_promotion_is_child_of_the_quick_run() {
    let r = rig();
    let project = tempfile::tempdir().unwrap();
    let id = r
        .engine
        .quick_start(&QuickRequest {
            cwd: Some(project.path().into()),
            text: "Fix it\nSCOPE src\nVERIFY cargo test".into(),
            review: false,
            context: None,
        })
        .unwrap();
    assert!(matches!(wait_done(&r.engine, &id), QuickStep::Done { .. }));
    r.engine
        .promote_wish(&ModeRef::Quick(id), "Ship it")
        .unwrap();
    let sent = r.quick.0.lock().unwrap();
    let runs = crate::modes::runs::session_runs(&r.engine.home, &sent[0].session).unwrap();
    let quick = runs.iter().find(|run| run.mode == Mode::Quick).unwrap();
    let wish = runs.iter().find(|run| run.mode == Mode::Wish).unwrap();
    assert_eq!(wish.parent.as_deref(), Some(quick.id.as_str()));
}

#[test]
fn unavailable_provider_refuses_chat() {
    let home = tempfile::tempdir().unwrap();
    let engine = ModesEngine::new(
        home.path(),
        Arc::new(Unavailable),
        Arc::new(Unavailable),
        Arc::new(ServiceLauncher {
            home: home.path().into(),
            start: Arc::new(FakeStart::default()),
        }),
        Arc::new(FakeQuick(Mutex::default(), home.path().into())),
    );
    assert_eq!(engine.chat_send("t", "hi").unwrap_err().kind, "unavailable");
    let err = engine
        .plaza_send(
            "plaza",
            &PlazaAsk::Say {
                text: "hi".into(),
                mentions: vec![],
            },
        )
        .unwrap_err();
    assert_eq!(err.kind, "unavailable");
}

#[test]
fn brief_parsing_splits_goal_scope_and_verify() {
    let b = parse_brief("Do x\nmore\nSCOPE a b\nVERIFY make test", false);
    assert_eq!(b.goal, "Do x\nmore");
    assert_eq!(b.scope, vec!["a", "b"]);
    assert_eq!(b.verify, "make test");
}

#[test]
fn lazy_quick_uses_engine_home_even_when_environment_home_differs() {
    if let Some(root) = std::env::var_os("AXIOM_TEST_LAZY_QUICK_ROOT") {
        let root = std::path::PathBuf::from(root);
        let home = root.join("engine");
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let session =
            crate::modes::runs::create_session(&home, Some(&project), &["src".into()]).unwrap();
        let parent =
            crate::modes::runs::start_run(&home, &session.id, Mode::Chat, None, None).unwrap();
        let req = LaunchRequest {
            kind: crate::modes::launch::LaunchKind::Quick,
            session: session.id.clone(),
            parent_run: parent.id,
            goal: "Fix it".into(),
            scope: vec!["src".into()],
            verify: Some("exit 0".into()),
            review: false,
            context: None,
            sources: vec![],
        };
        let _ = LazyQuick { home: home.clone() }.launch(&req);
        assert!(crate::modes::runs::session_runs(&home, &session.id)
            .unwrap()
            .iter()
            .any(|r| r.mode == Mode::Quick));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let fake = serde_json::to_string(&vec![
        "python".to_owned(),
        "-I".to_owned(),
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fakes/fake_codex.py")
            .to_string_lossy()
            .into_owned(),
    ])
    .unwrap();
    let output = crate::base::proc::cmd(std::env::current_exe().unwrap())
        .args(["--exact", "desktop::ux::real::modes_accept::lazy_quick_uses_engine_home_even_when_environment_home_differs", "--nocapture"])
        .env("AXIOM_TEST_LAZY_QUICK_ROOT", root.path())
        .env("AXIOM_HOME", root.path().join("other"))
        .env("AXIOM_CODEX_CMD", fake)
        .env("FAKE_CODEX_LOG", root.path().join("codex.log"))
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
