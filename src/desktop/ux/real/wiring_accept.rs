//! Production seam wiring and fake negative controls.
use super::wiring::production;
use crate::service::launcher::StartPath;
use crate::{
    desktop::ux::{
        celestial::DualCore,
        seams::{Draft, KeepAlive, PlazaAsk, UnitActionKind},
        Seams,
    },
    engine::{
        approvals::{self, Action, Origin},
        kinds, policy,
    },
    orch,
    storage::{Class, Ledger, Owner},
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Rig {
    home: tempfile::TempDir,
    slots: tempfile::TempDir,
    store: PathBuf,
}
impl Rig {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let slots = tempfile::tempdir().unwrap();
        let goal = crate::goal::open(
            home.path(),
            "wiring-session",
            "wish",
            Some(home.path().to_str().unwrap()),
        )
        .unwrap();
        let store = orch::store_for_goal(home.path(), &goal).unwrap();
        let r = Self { home, slots, store };
        r.orch(&["init"]);
        let brief = r.home.path().join("brief.md");
        fs::write(
            &brief,
            "GOAL test\nSCOPE src/a\nVERIFY echo ok\nRISK standard\n",
        )
        .unwrap();
        r.orch(&[
            "unit",
            "add",
            "u1",
            "--track",
            "core",
            "--brief",
            brief.to_str().unwrap(),
        ]);
        std::fs::write(
            r.store.join("modules.md"),
            "## core\nPURPOSE test\nPATHS src/a\nDEPENDS none\nCONTRACT test\nACCEPT test `echo ok`\nROUTING\n  default sol/high standard fixture\n",
        )
        .unwrap();
        policy::set(&r.store, "dual_core", "on").unwrap();
        orch::events::append_event(
            &r.store,
            kinds::DUAL_CORE,
            None,
            None,
            json!({"id":"a","kind":"module_map","step":"co_core","agree":true}),
        )
        .unwrap();
        approvals::request(
            &r.store,
            Action {
                command: "unique-wiring-command".into(),
                args: vec![],
                cwd: r.home.path().to_string_lossy().into(),
            },
            "run",
            Origin::Model,
        )
        .unwrap();
        let data = r.home.path().join("ledger-item");
        fs::write(&data, b"wiring ledger").unwrap();
        let ledger = Ledger::open(r.home.path()).unwrap();
        ledger
            .register(
                &data,
                Class::Scratch,
                &Owner {
                    project: "wiring".into(),
                    wish: None,
                },
                json!({}),
            )
            .unwrap();
        ledger.measure(&data, 5).unwrap();
        let dir = r.home.path().join("desktop/interjections");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("wiring-session.json"), json!({"session":"wiring-session", "entries":[{"client_id":"wiring-client","digest":"unused","text":"hello","sent_ms":1,"replan":false,"ij":null,"after_seq":0}]}).to_string()).unwrap();
        r
    }
    fn orch(&self, args: &[&str]) {
        let out = orch::run(
            &self.store,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            false,
            false,
        );
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
    }
    fn seams(&self) -> Seams {
        production(self.home.path(), self.slots.path())
    }
}
fn dual(s: &Seams) -> bool {
    s.dual_core.as_ref().unwrap().state("wiring-session")
        == DualCore::Agreed {
            topic: "module_map".into(),
        }
}
fn turn(s: &Seams, slots: &Path) -> bool {
    s.turn_policy
        .as_ref()
        .unwrap()
        .try_set_extreme(true)
        .unwrap();
    crate::codex::extreme::load(slots).on
}
fn interjection(s: &Seams) -> bool {
    s.interjections
        .as_ref()
        .unwrap()
        .receipt("wiring-client")
        .is_some_and(|r| r.session == "wiring-session")
}
fn approval(s: &Seams) -> bool {
    s.approvals
        .as_ref()
        .unwrap()
        .pending()
        .iter()
        .any(|c| c.session == "wiring-session" && c.action == "unique-wiring-command")
}
fn storage(s: &Seams, home: &Path) -> bool {
    s.storage
        .as_ref()
        .unwrap()
        .report()
        .unwrap()
        .items
        .iter()
        .any(|i| {
            i.path == home.join("ledger-item").to_string_lossy() && i.measured && i.bytes == 13
        })
}
fn background(s: &Seams) -> bool {
    matches!(
        s.background.as_ref().unwrap().status(),
        KeepAlive::Unavailable { .. }
    )
}
fn modes(s: &Seams) -> bool {
    let m = s.modes.as_ref().unwrap();
    m.chat_send("wiring-chat", "hello")
        .is_err_and(|e| e.kind != "unavailable")
        && m.plaza_send(
            "wiring-plaza",
            &PlazaAsk::Say {
                text: "hello".into(),
                mentions: vec![],
            },
        )
        .is_err_and(|e| e.kind != "unavailable")
}
fn unit(s: &Seams, store: &Path) -> bool {
    let accepted = s
        .unit_actions
        .as_ref()
        .unwrap()
        .act(
            "wiring-session",
            "u1",
            &UnitActionKind::Retry,
            "wiring-retry",
        )
        .is_ok_and(|r| r.accepted);
    accepted
        && orch::read_units(store)
            .unwrap()
            .iter()
            .any(|u| u.id == "u1-retry1")
}

#[test]
fn dual_core() {
    let r = Rig::new();
    assert!(dual(&r.seams()));
}
#[test]
fn turn_policy() {
    let r = Rig::new();
    assert!(turn(&r.seams(), r.slots.path()));
}
#[test]
fn interjections() {
    let r = Rig::new();
    let s = r.seams();
    assert!(interjection(&s));
    let d = Draft {
        session: "wiring-session".into(),
        client_id: "new-client".into(),
        text: "hello".into(),
        pause_first: false,
        replan: false,
    };
    assert_eq!(
        s.interjections.as_ref().unwrap().send(&d).unwrap_err().kind,
        "service_unavailable"
    );
    assert!(interjection(&production(r.home.path(), r.slots.path())));
}
#[test]
fn approvals() {
    let r = Rig::new();
    assert!(approval(&r.seams()));
}
#[test]
fn storage_wired() {
    let r = Rig::new();
    assert!(storage(&r.seams(), r.home.path()));
}
#[test]
fn background_wired() {
    let r = Rig::new();
    assert!(background(&r.seams()));
}
#[test]
fn modes_wired() {
    let r = Rig::new();
    let previous = std::env::var_os("AXIOM_CLAUDE_CMD");
    std::env::set_var(
        "AXIOM_CLAUDE_CMD",
        serde_json::to_string(&vec![r
            .home
            .path()
            .join("missing-fake-executable")
            .to_string_lossy()
            .to_string()])
        .unwrap(),
    );
    assert!(modes(&r.seams()));
    match previous {
        Some(v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
}
#[test]
fn unit_actions() {
    let r = Rig::new();
    assert!(unit(&r.seams(), &r.store));
}
#[test]
fn demo_negative_controls() {
    let r = Rig::new();
    let fake = Seams::demo();
    assert!(!dual(&fake));
    assert!(!turn(&fake, r.slots.path()));
    assert!(!interjection(&fake));
    assert!(!approval(&fake));
    assert!(!storage(&fake, r.home.path()));
    assert!(!background(&fake));
    assert!(!modes(&fake));
    assert!(!unit(&fake, &r.store));
}
#[test]
fn unavailable_seams_and_lazy_start() {
    let home = tempfile::tempdir().unwrap();
    let slots = tempfile::tempdir().unwrap();
    let s = production(home.path(), slots.path());
    assert!(s.sessions.is_some() && s.optimizer.is_none() && s.github.is_none());
    assert!(!home.path().join("desktop").exists());
    assert_eq!(
        super::wiring::LazyStart(home.path().into())
            .start(Default::default())
            .unwrap_err()
            .kind,
        "service_unavailable"
    );
}

fn settled(
    ticket: &crate::desktop::data::outbox::Ticket,
) -> crate::desktop::data::outbox::TicketState {
    use crate::desktop::data::outbox::TicketState;
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = ticket.state();
        if state != TicketState::Pending {
            return state;
        }
        assert!(Instant::now() < deadline, "ticket did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn outboxes_are_wired_and_approval_executes_once() {
    use crate::desktop::data::outbox::TicketState;
    use crate::desktop::ux::seams::{Decision, Verdict};

    let r = Rig::new();
    let (command, args) = if cfg!(windows) {
        ("cmd", vec!["/c".into(), "exit 0".into()])
    } else {
        ("sh", vec!["-c".into(), "exit 0".into()])
    };
    approvals::request(
        &r.store,
        Action {
            command: command.into(),
            args,
            cwd: r.home.path().to_string_lossy().into(),
        },
        "run",
        Origin::Model,
    )
    .unwrap();
    let s = r.seams();
    assert!(s.interjection_outbox.is_some());
    assert!(s.unit_action_outbox.is_some());
    let outbox = s.approval_outbox.as_ref().unwrap();
    let card = s
        .approvals
        .as_ref()
        .unwrap()
        .pending()
        .into_iter()
        .find(|card| card.action == command)
        .unwrap();
    let decision = Decision {
        action_id: card.action_id,
        payload_version: card.version,
        key: "wiring-key".into(),
        verdict: Verdict::Approve,
    };
    let first = outbox.submit(decision.clone(), &decision.key);
    let duplicate = s
        .clone()
        .approval_outbox
        .unwrap()
        .submit(decision.clone(), &decision.key);
    assert!(first.same_ticket(&duplicate));
    assert_eq!(settled(&first), TicketState::Applied);
    let events = orch::events::read_events(&r.store, 0).unwrap();
    for kind in [kinds::APPROVAL_EXECUTING, kinds::APPROVAL_EXECUTED] {
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == kind && event.data["id"] == decision.action_id)
                .count(),
            1
        );
    }
    assert!(!s
        .approvals
        .as_ref()
        .unwrap()
        .pending()
        .iter()
        .any(|card| card.action_id == decision.action_id));
}

#[test]
fn demo_outboxes_and_default_negative_control() {
    let default = Seams::default();
    assert!(default.approval_outbox.is_none());
    assert!(default.interjection_outbox.is_none());
    assert!(default.unit_action_outbox.is_none());
    let demo = Seams::demo();
    assert!(demo.approval_outbox.is_some());
    assert!(demo.interjection_outbox.is_some());
    assert!(demo.unit_action_outbox.is_some());

    use crate::desktop::data::outbox::TicketState;
    use crate::desktop::ux::seams::{Decision, UnitAction, Verdict};
    let card = demo.approvals.as_ref().unwrap().pending().remove(0);
    let decision = Decision {
        action_id: card.action_id.clone(),
        payload_version: card.version,
        key: "demo-approval".into(),
        verdict: Verdict::Approve,
    };
    assert_eq!(
        settled(
            &demo
                .approval_outbox
                .as_ref()
                .unwrap()
                .submit(decision.clone(), &decision.key)
        ),
        TicketState::Applied
    );
    assert!(!demo
        .approvals
        .as_ref()
        .unwrap()
        .pending()
        .iter()
        .any(|c| c.action_id == card.action_id));

    let draft = Draft {
        session: card.session.clone(),
        client_id: "demo-outbox".into(),
        text: "hello".into(),
        pause_first: false,
        replan: false,
    };
    assert_eq!(
        settled(
            &demo
                .interjection_outbox
                .as_ref()
                .unwrap()
                .submit(draft.clone(), &draft.client_id)
        ),
        TicketState::Applied
    );
    assert_eq!(
        demo.interjections
            .as_ref()
            .unwrap()
            .receipt(&draft.client_id)
            .unwrap()
            .session,
        draft.session
    );

    let run = crate::desktop::demo::sample_run();
    let action = UnitAction {
        session: run.session.to_string(),
        unit: run.units[0].id.to_string(),
        kind: UnitActionKind::Retry,
        key: "demo-unit".into(),
    };
    assert_eq!(
        settled(
            &demo
                .unit_action_outbox
                .as_ref()
                .unwrap()
                .submit(action.clone(), &action.key)
        ),
        TicketState::Applied
    );
    assert!(
        demo.unit_actions
            .as_ref()
            .unwrap()
            .act("missing", "missing", &action.kind, &action.key)
            .unwrap()
            .accepted
    );
}
