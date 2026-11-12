//! Tests for approvals; owned by module approvals-seam.
use super::approvals::EngineApprovals;
use crate::desktop::ux::seams::{
    ApprovalCard, ApprovalOutcome, ApprovalService, Decision, Verdict,
};
use crate::engine::approvals::{self, Action, Origin};
use crate::engine::kinds::*;
use crate::orch::events::read_events;
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    home: tempfile::TempDir,
    store: PathBuf,
    wish: String,
}

fn harmless() -> Action {
    let (command, args) = if cfg!(windows) {
        ("cmd", vec!["/c".to_owned(), "exit 0".to_owned()])
    } else {
        ("sh", vec!["-c".to_owned(), "exit 0".to_owned()])
    };
    Action {
        command: command.into(),
        args,
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
    }
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let goal = crate::goal::open(
            home.path(),
            "session",
            "wish",
            Some(project.path().to_str().unwrap()),
        )
        .unwrap();
        let store = crate::orch::store_for_goal(home.path(), &goal).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        approvals::request(&store, harmless(), "run", Origin::Model).unwrap();
        Self {
            home,
            wish: goal.wish_id.unwrap(),
            store,
        }
    }
    fn service(&self) -> EngineApprovals {
        EngineApprovals::new(self.home.path().to_path_buf())
    }
    fn count(&self, kind: &str) -> usize {
        read_events(&self.store, 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == kind)
            .count()
    }
}

fn decision(card: &ApprovalCard, key: &str, verdict: Verdict) -> Decision {
    Decision {
        action_id: card.action_id.clone(),
        payload_version: card.version,
        key: key.into(),
        verdict,
    }
}

#[test]
fn card_is_bound_to_the_wish_and_its_session() {
    let f = Fixture::new();
    let cards = f.service().pending();
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.action_id, "ap-1");
    assert_eq!(card.session, "session");
    assert_eq!(card.action, harmless().command);
    assert_eq!(card.scope, harmless().cwd);
    assert_eq!(card.cost_usd, None);
    assert_eq!(f.wish.len(), 22);
}

#[test]
fn double_click_executes_once() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let first = svc
        .decide(&decision(&card, "k1", Verdict::Approve))
        .unwrap();
    assert!(matches!(first.outcome, ApprovalOutcome::Executed { .. }));
    // Same key, then a new key for the same card.
    assert_eq!(
        svc.decide(&decision(&card, "k1", Verdict::Approve)).unwrap(),
        first
    );
    let again = svc
        .decide(&decision(&card, "k2", Verdict::Approve))
        .unwrap();
    assert_eq!(again.outcome, first.outcome);
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
    assert_eq!(f.count(APPROVAL_EXECUTED), 1);
    assert!(svc.pending().is_empty());
}

#[test]
fn restart_replays_without_executing() {
    let f = Fixture::new();
    let card = f.service().pending().remove(0);
    let first = f
        .service()
        .decide(&decision(&card, "k1", Verdict::Approve))
        .unwrap();
    let replay = f
        .service()
        .decide(&decision(&card, "k1", Verdict::Approve))
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn race_executes_once() {
    let f = Fixture::new();
    let card = f.service().pending().remove(0);
    let shared = Arc::new(f.service());
    let results: Vec<_> = (0..4)
        .map(|i| {
            let (svc, card) = (shared.clone(), card.clone());
            let other = i % 2 == 1;
            let home = f.home.path().to_path_buf();
            std::thread::spawn(move || {
                let key = format!("k{i}");
                if other {
                    EngineApprovals::new(home).decide(&decision(&card, &key, Verdict::Approve))
                } else {
                    svc.decide(&decision(&card, &key, Verdict::Approve))
                }
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap().unwrap())
        .collect();
    assert!(results
        .iter()
        .all(|r| matches!(r.outcome, ApprovalOutcome::Executed { .. })));
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn rejection_is_recorded_and_never_executes() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let r = svc
        .decide(&decision(
            &card,
            "k",
            Verdict::Reject {
                reason: "no".into(),
            },
        ))
        .unwrap();
    assert_eq!(r.outcome, ApprovalOutcome::Rejected);
    assert_eq!(f.count(APPROVAL_DENIED), 1);
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn stale_card_is_refused() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let mut d = decision(&card, "k", Verdict::Approve);
    d.payload_version ^= 1;
    assert_eq!(svc.decide(&d).unwrap_err().kind, "stale");
    d.action_id = "ap-9".into();
    assert_eq!(svc.decide(&d).unwrap_err().kind, "not_found");
    assert_eq!(f.count(APPROVAL_APPROVED), 0);
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn wrong_wish_is_refused() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    svc.bound.lock().unwrap()[0].wish = "w-1700000000000-abcdef".into();
    assert_eq!(
        svc.decide(&decision(&card, "k", Verdict::Approve))
            .unwrap_err()
            .kind,
        "wrong_wish"
    );
    assert_eq!(f.count(APPROVAL_APPROVED), 0);
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn modified_action_is_refused() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let file = f.store.join("events.jsonl");
    let mut event: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    event["data"]["action"]["command"] = serde_json::json!("altered");
    std::fs::write(&file, format!("{event}\n")).unwrap();
    assert_eq!(
        svc.decide(&decision(&card, "k", Verdict::Approve))
            .unwrap_err()
            .kind,
        "tampered"
    );
    assert_eq!(f.count(APPROVAL_APPROVED), 0);
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn model_origin_reply_is_refused() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let digest = svc.bound.lock().unwrap()[0].digest.clone();
    let err = approvals::reply(
        f.home.path(),
        &f.store,
        "ap-1",
        true,
        &digest,
        &f.wish,
        None,
        Origin::Model,
    )
    .unwrap_err();
    assert_eq!(err.kind, "person_only");
    assert_eq!(f.count(APPROVAL_APPROVED), 0);
    assert_eq!(svc.pending(), vec![card]);
}
