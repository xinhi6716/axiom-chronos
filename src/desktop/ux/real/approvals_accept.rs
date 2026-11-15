//! Tests for approvals; owned by module approvals-seam.
use super::approvals::EngineApprovals;
use crate::desktop::ux::seams::{
    ApprovalCard, ApprovalOutcome, ApprovalService, Decision, Verdict,
};
use crate::engine::approvals::{self, Action, Origin};
use crate::engine::kinds::*;
use crate::orch::events::read_events;
use std::{path::PathBuf, sync::Arc};

fn settled(
    ticket: &crate::desktop::data::outbox::Ticket,
) -> crate::desktop::data::outbox::TicketState {
    use crate::desktop::data::outbox::TicketState;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = ticket.state();
        if state != TicketState::Pending {
            return state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "ticket did not settle"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

struct FlakyApproval {
    inner: EngineApprovals,
    attempts: std::sync::atomic::AtomicUsize,
    failures: usize,
}
impl ApprovalService for FlakyApproval {
    fn pending(&self) -> Vec<ApprovalCard> {
        self.inner.pending()
    }
    fn decide(&self, d: &Decision) -> crate::base::Res<crate::desktop::ux::seams::ApprovalReceipt> {
        if self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            < self.failures
        {
            Err(crate::base::Failure::new(
                "service_unavailable",
                "temporarily offline",
            ))
        } else {
            self.inner.decide(d)
        }
    }
}

#[test]
fn outboxes_share_adapter_and_execute_duplicate_only_once() {
    use crate::desktop::data::{approval_outbox_for, outbox::TicketState};
    let f = Fixture::new();
    let service = Arc::new(f.service());
    let card = service.pending().remove(0);
    let d = decision(&card, "same", Verdict::Approve);
    let first = approval_outbox_for(service.clone());
    let second = approval_outbox_for(service);
    let a = first.submit(d.clone(), &d.key);
    let duplicate = first.submit(d.clone(), &d.key);
    let b = second.submit(d.clone(), &d.key);
    assert!(a.same_ticket(&duplicate));
    assert_eq!(settled(&a), TicketState::Applied);
    assert_eq!(settled(&b), TicketState::Applied);
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
    assert_eq!(f.count(APPROVAL_EXECUTED), 1);
}

#[test]
fn outbox_retries_refuses_and_retains_failed_tickets() {
    use crate::desktop::data::{approval_outbox_for, outbox::TicketState};
    let f = Fixture::new();
    let service = Arc::new(FlakyApproval {
        inner: f.service(),
        attempts: 0.into(),
        failures: 1,
    });
    let card = service.pending().remove(0);
    let d = decision(&card, "retry", Verdict::Approve);
    let outbox = approval_outbox_for(service.clone());
    assert_eq!(
        settled(&outbox.submit(d.clone(), &d.key)),
        TicketState::Applied
    );
    assert_eq!(
        service.attempts.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);

    let other = Fixture::new();
    let service = Arc::new(FlakyApproval {
        inner: other.service(),
        attempts: 0.into(),
        failures: usize::MAX,
    });
    let card = service.pending().remove(0);
    let outbox = approval_outbox_for(service.clone());
    let d = decision(&card, "failed", Verdict::Approve);
    let ticket = outbox.submit(d.clone(), &d.key);
    assert!(matches!(settled(&ticket), TicketState::Failed(reason) if reason.contains("offline")));
    assert!(outbox.tickets().iter().any(|t| t.same_ticket(&ticket)));
    assert_eq!(
        service.attempts.load(std::sync::atomic::Ordering::SeqCst),
        5
    );
    assert_eq!(other.count(APPROVAL_EXECUTING), 0);

    let outbox = approval_outbox_for(Arc::new(other.service()));
    let mut stale = decision(&card, "stale", Verdict::Approve);
    stale.payload_version ^= 1;
    assert!(
        matches!(settled(&outbox.submit(stale.clone(), &stale.key)), TicketState::Refused(reason) if reason.contains("stale"))
    );
    let rejected = decision(
        &card,
        "reject",
        Verdict::Reject {
            reason: "no".into(),
        },
    );
    assert_eq!(
        settled(&outbox.submit(rejected.clone(), &rejected.key)),
        TicketState::Applied
    );
    assert_eq!(other.count(APPROVAL_DENIED), 1);
}

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
    assert_eq!(card.target, harmless().args.join(" "));
    assert_eq!(card.scope, harmless().cwd);
    assert_eq!(card.cost_usd, None);
    let digest = approvals::digest(&f.wish, &harmless());
    assert_eq!(
        card.version,
        u64::from_str_radix(&digest[..16], 16).unwrap()
    );
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
        svc.decide(&decision(&card, "k1", Verdict::Approve))
            .unwrap(),
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
    assert_eq!(svc.decide(&d).unwrap_err().kind, "wrong_wish");
    assert_eq!(f.count(APPROVAL_APPROVED), 0);
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn replay_still_checks_version_and_stored_action() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let mut d = decision(&card, "same-key", Verdict::Approve);
    svc.decide(&d).unwrap();
    d.payload_version ^= 1;
    assert_eq!(svc.decide(&d).unwrap_err().kind, "stale");
    d.payload_version = card.version;
    let file = f.store.join("events.jsonl");
    let lines = std::fs::read_to_string(&file).unwrap();
    let mut events: Vec<serde_json::Value> = lines
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    events[0]["data"]["action"]["command"] = serde_json::json!("altered");
    std::fs::write(
        &file,
        events.iter().map(|e| format!("{e}\n")).collect::<String>(),
    )
    .unwrap();
    assert_eq!(svc.decide(&d).unwrap_err().kind, "tampered");
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn key_replays_across_restart_for_another_card() {
    let f = Fixture::new();
    approvals::request(&f.store, harmless(), "second", Origin::Model).unwrap();
    let svc = f.service();
    let cards = svc.pending();
    let first = svc
        .decide(&decision(&cards[0], "same-key", Verdict::Approve))
        .unwrap();
    let replay = f
        .service()
        .decide(&decision(&cards[1], "same-key", Verdict::Approve))
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn concurrent_same_key_for_different_cards_executes_only_one() {
    let f = Fixture::new();
    approvals::request(&f.store, harmless(), "second", Origin::Model).unwrap();
    let cards = f.service().pending();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = cards
        .into_iter()
        .map(|card| {
            let home = f.home.path().to_path_buf();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                EngineApprovals::new(home)
                    .decide(&decision(&card, "same-key", Verdict::Approve))
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results[0], results[1]);
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn pending_refresh_preserves_old_binding_with_colliding_ids() {
    let f = Fixture::new();
    let other_store = f
        .home
        .path()
        .join("orchestrate")
        .join("other")
        .join("w-1700000000000-abcdef");
    approvals::request(&other_store, harmless(), "other", Origin::Model).unwrap();
    let svc = f.service();
    let cards = svc.pending();
    let first = cards.iter().find(|c| c.session == "session").unwrap();
    svc.decide(&decision(first, "first", Verdict::Approve))
        .unwrap();
    svc.pending();
    let replay = svc
        .decide(&decision(first, "again", Verdict::Approve))
        .unwrap();
    assert!(matches!(replay.outcome, ApprovalOutcome::Executed { .. }));
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn already_approved_action_is_settled_even_if_replay_says_reject() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let digest = svc.bound.lock().unwrap()[0].digest.clone();
    approvals::reply(
        f.home.path(),
        &f.store,
        &card.action_id,
        true,
        &digest,
        &f.wish,
        None,
        Origin::Person,
    )
    .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        tx.send(svc.decide(&decision(
            &card,
            "replay",
            Verdict::Reject {
                reason: "no".into(),
            },
        )))
        .unwrap();
    });
    let result = rx.recv_timeout(std::time::Duration::from_secs(2));
    if result.is_err() {
        approvals::execute(&f.store, "ap-1").unwrap();
    }
    handle.join().unwrap();
    assert!(matches!(
        result.unwrap().unwrap().outcome,
        ApprovalOutcome::Executed { .. }
    ));
    assert_eq!(f.count(APPROVAL_EXECUTING), 1);
}

#[test]
fn unfinished_execution_does_not_invent_a_receipt() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let binding = svc.bound.lock().unwrap()[0].clone();
    crate::orch::events::append_event(
        &f.store,
        APPROVAL_EXECUTING,
        None,
        None,
        serde_json::json!({"id": card.action_id}),
    )
    .unwrap();
    let result = svc.settle_until(
        &binding,
        &decision(&card, "k", Verdict::Approve),
        std::time::Instant::now(),
    );
    assert_eq!(result.unwrap_err().kind, "timeout");
    assert_eq!(f.count(APPROVAL_EXECUTED), 0);
}

#[test]
fn execution_claim_io_error_is_reported_without_spinning() {
    let f = Fixture::new();
    let svc = f.service();
    let card = svc.pending().remove(0);
    let binding = svc.bound.lock().unwrap()[0].clone();
    approvals::reply(
        f.home.path(),
        &f.store,
        &card.action_id,
        true,
        &binding.digest,
        &f.wish,
        None,
        Origin::Person,
    )
    .unwrap();
    let lock = f.store.join("events.lock");
    std::fs::remove_file(&lock).unwrap();
    std::fs::create_dir(&lock).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        tx.send(svc.settle_until(
            &binding,
            &decision(&card, "k", Verdict::Approve),
            std::time::Instant::now() + std::time::Duration::from_secs(10),
        ))
        .unwrap();
    });
    let result = rx.recv_timeout(std::time::Duration::from_secs(2));
    std::fs::remove_dir(&lock).unwrap();
    thread.join().unwrap();
    assert_eq!(result.unwrap().unwrap_err().kind, "io");
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
fn refreshing_does_not_rebind_a_modified_request() {
    let f = Fixture::new();
    let svc = f.service();
    let original = svc.pending().remove(0);
    let file = f.store.join("events.jsonl");
    let mut event: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    let mut action = harmless();
    action.args.push("changed".into());
    event["data"]["action"] = serde_json::to_value(&action).unwrap();
    event["data"]["digest"] = approvals::digest(&f.wish, &action).into();
    std::fs::write(&file, format!("{event}\n")).unwrap();
    let changed = svc.pending().remove(0);
    assert_eq!(
        svc.decide(&decision(&changed, "new", Verdict::Approve))
            .unwrap_err()
            .kind,
        "stale"
    );
    assert_eq!(
        svc.decide(&decision(&original, "old", Verdict::Approve))
            .unwrap_err()
            .kind,
        "tampered"
    );
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn invalid_stored_action_is_tampered_even_after_restart() {
    let f = Fixture::new();
    let card = f.service().pending().remove(0);
    let file = f.store.join("events.jsonl");
    let mut event: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    event["data"]["action"] = serde_json::Value::Null;
    std::fs::write(&file, format!("{event}\n")).unwrap();
    assert_eq!(
        f.service()
            .decide(&decision(&card, "k", Verdict::Approve))
            .unwrap_err()
            .kind,
        "tampered"
    );
    assert_eq!(f.count(APPROVAL_EXECUTING), 0);
}

#[test]
fn missing_digest_is_tampered_even_after_restart() {
    let f = Fixture::new();
    let card = f.service().pending().remove(0);
    let file = f.store.join("events.jsonl");
    let mut event: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim()).unwrap();
    event["data"].as_object_mut().unwrap().remove("digest");
    std::fs::write(&file, format!("{event}\n")).unwrap();
    assert_eq!(
        f.service()
            .decide(&decision(&card, "k", Verdict::Approve))
            .unwrap_err()
            .kind,
        "tampered"
    );
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
