use super::super::{
    live::events::summarize_text,
    ux::seams::{ApprovalCard, ApprovalService, Book, Controller, FakeApprovals, Verdict},
};
use super::{actions::can_waive, sessions::input_enabled};
use std::prelude::v1::test;

fn card() -> ApprovalCard {
    ApprovalCard {
        action_id: "a".into(),
        version: 1,
        session: "s".into(),
        action: "publish".into(),
        target: "report".into(),
        scope: "files".into(),
        cost_usd: None,
    }
}
#[test]
fn decision_keys_are_stable_and_only_one_decision_is_sent() {
    let c = card();
    let service = FakeApprovals::new(vec![c.clone()]);
    let mut book = Book::default();
    let key = book.key_for(&c);
    assert_eq!(key, book.key_for(&c));
    let decision = book.begin(&c, Verdict::Approve).unwrap();
    assert!(book.begin(&c, Verdict::Approve).is_none());
    book.finish(&key, service.decide(&decision));
    assert!(book.begin(&c, Verdict::Approve).is_none());
    assert_eq!(service.executions("a"), 1);
    let mut changed = c.clone();
    changed.version += 1;
    assert_ne!(key, book.key_for(&changed));
}
#[test]
fn decided_snapshots_merge_with_pending_without_duplicates() {
    let decided = card();
    let mut changed = decided.clone();
    changed.target = "changed".into();
    let mut other = card();
    other.version += 1;
    let merged = super::approvals::merge_cards(
        vec![changed, other.clone(), other.clone()],
        vec![decided.clone()],
    );
    assert_eq!(merged, vec![decided, other]);
}
#[test]
fn stale_result_discards_snapshot_and_draws_refreshed_pending_card() {
    use super::approvals::{merge_cards, remove_failed_snapshot};
    use std::collections::HashMap;
    let old = card();
    let id = (old.action_id.clone(), old.version);
    let service = FakeApprovals::new(vec![old.clone()]);
    let mut book = Book::default();
    let decision = book.begin(&old, Verdict::Approve).unwrap();
    let mut snapshots = HashMap::from([(id.clone(), old)]);
    service.bump_version("a");
    let result = service.decide(&decision);
    assert_eq!(result.as_ref().unwrap_err().kind, "stale");
    remove_failed_snapshot(&mut snapshots, &id, &result);
    let pending = service.pending();
    assert_eq!(
        merge_cards(pending.clone(), snapshots.into_values().collect()),
        pending
    );
    assert_eq!(pending[0].version, 2);
}
#[test]
fn rejection_and_stale_handling() {
    let c = card();
    let service = FakeApprovals::new(vec![c.clone()]);
    let mut book = Book::default();
    assert!(book
        .begin(
            &c,
            Verdict::Reject {
                reason: "  ".into()
            }
        )
        .is_none());
    let decision = book.begin(&c, Verdict::Approve).unwrap();
    service.bump_version("a");
    book.finish(&decision.key, service.decide(&decision));
    assert!(!book.keys.contains_key(&(c.action_id.clone(), c.version)));
    assert_eq!(
        book.errors.get(&decision.key).map(String::as_str),
        Some("stale")
    );
    assert_ne!(book.key_for(&c), decision.key);
    let mut current = c;
    current.version += 1;
    let rejected = book
        .begin(
            &current,
            Verdict::Reject {
                reason: "not needed".into(),
            },
        )
        .unwrap();
    book.finish(&rejected.key, service.decide(&rejected));
    assert_eq!(service.executions("a"), 0);
}
#[test]
fn waiver_requires_nonblank_reason() {
    assert!(!can_waive(" \n\t"));
    assert!(can_waive("reviewed"));
}
#[test]
fn control_requires_the_pages_own_confirmed_ticket() {
    assert!(!input_enabled(&Controller::Worker, None));
    assert!(!input_enabled(
        &Controller::Requested { ticket: "t".into() },
        Some("t")
    ));
    assert!(!input_enabled(
        &Controller::Refused {
            reason: "no".into()
        },
        Some("t")
    ));
    let you = Controller::You { ticket: "t".into() };
    assert!(!input_enabled(&you, None));
    assert!(!input_enabled(&you, Some("other")));
    assert!(input_enabled(&you, Some("t")));
}
#[test]
fn events_preserve_plain_messages_and_summarize_json() {
    let plain = summarize_text("ordinary text");
    assert_eq!(plain.en, "ordinary text");
    assert!(plain.raw.is_none());
    let event = summarize_text(r#"{"type":"result"}"#);
    assert!(!event.en.starts_with("{\""));
    assert!(!event.zh.starts_with("{\""));
    assert!(event.raw.is_some());
    let source = include_str!("events.rs");
    assert!(source.contains(".when_some(s.raw.clone()"));
    assert!(source.contains("event-raw-toggle"));
    assert!(include_str!("../detail.rs").contains("summarize_text(&line.text)"));
}
#[test]
fn fake_unit_actions_deduplicate_and_reject_blank_waivers() {
    use super::super::ux::seams::{FakeUnitActions, UnitActionKind, UnitActions, UnitEvidence};
    let service = FakeUnitActions::new();
    service.set_evidence("s", "u", UnitEvidence::default());
    assert!(service
        .act("s", "u", &UnitActionKind::Waive { reason: " ".into() }, "w")
        .is_err());
    service.act("s", "u", &UnitActionKind::Retry, "r").unwrap();
    service.act("s", "u", &UnitActionKind::Retry, "r").unwrap();
    assert_eq!(service.count("u", "retry"), 1);
    assert_eq!(service.count("u", "waive"), 0);
}
#[test]
fn fake_session_requires_confirmation_and_owns_the_ticket() {
    use super::super::ux::seams::{
        Connection, FakeSessions, SessionKind, SessionStream, WorkerSession,
    };
    let service = FakeSessions::new(vec![WorkerSession {
        id: "terminal".into(),
        run: "s".into(),
        unit: Some("u".into()),
        title: "worker".into(),
        kind: SessionKind::Terminal,
    }]);
    let ticket = service.request_takeover("terminal").unwrap();
    assert!(!input_enabled(
        &service.controller("terminal"),
        Some(&ticket)
    ));
    assert!(service.send_input("terminal", &ticket, "before").is_err());
    assert!(service.request_takeover("terminal").is_err());
    service.confirm("terminal");
    assert!(input_enabled(
        &service.controller("terminal"),
        Some(&ticket)
    ));
    service.send_input("terminal", &ticket, "hello").unwrap();
    service.hand_back("terminal", &ticket).unwrap();
    assert!(!input_enabled(
        &service.controller("terminal"),
        Some(&ticket)
    ));
    service.disconnect("terminal");
    assert_eq!(service.connection("terminal"), Connection::Lost);
    service.reconnect("terminal").unwrap();
    assert_eq!(service.connection("terminal"), Connection::Live);
    let next = service.request_takeover("terminal").unwrap();
    service.refuse("terminal", "busy");
    assert!(!input_enabled(&service.controller("terminal"), Some(&next)));
    assert_eq!(service.inputs("terminal"), vec!["hello"]);
}
