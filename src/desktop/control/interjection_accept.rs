use super::interjection::*;
use crate::desktop::ux::seams::{Draft, FakeInterjections, Receipt, ReceiptStage};

#[test]
fn declared_dialog_targets_are_used_by_their_views() {
    let control = include_str!("../control.rs");
    let interjection = include_str!("interjection.rs");
    for (source, constructor) in [
        (control, "DialogFrame::new(\"dialog.control.panel\""),
        (control, "Button::new(\"action:dialog.control.send\""),
        (control, "Button::new(\"action:dialog.control.stop\""),
        (
            interjection,
            "DialogFrame::new(\"dialog.interjection.prompt\"",
        ),
        (
            interjection,
            "Button::new(\"action:dialog.interjection.send\"",
        ),
    ] {
        let has_target = |view: &str| view.contains(constructor);
        assert!(has_target(source), "missing {constructor}");
        assert!(!has_target(&source.replace(constructor, "removed_target")));
    }
}
#[test]
fn receipt_order_and_finality() {
    let r = |stage| Receipt {
        client_id: "id".into(),
        session: "s".into(),
        stage,
        at_ms: 1,
    };
    let sent = r(ReceiptStage::Sent);
    let received = r(ReceiptStage::Received);
    let applied = r(ReceiptStage::Applied {
        first_action: "x".into(),
    });
    let rejected = r(ReceiptStage::Rejected { reason: "x".into() });
    assert_eq!(
        (
            rank(&sent.stage),
            rank(&received.stage),
            rank(&applied.stage),
            rank(&rejected.stage)
        ),
        (0, 1, 2, 2)
    );
    assert_eq!(merge(Some(&received), &sent), received);
    assert_eq!(merge(Some(&applied), &rejected), applied);
    assert_eq!(merge(None, &sent), sent);
}
#[test]
fn pause_failure_prevents_send_and_retry_is_deduplicated() {
    let seam = FakeInterjections::new();
    seam.set_pause_failure(true);
    let d = Draft {
        session: "s".into(),
        client_id: new_client_id("s", 1, 2),
        text: "hello".into(),
        pause_first: true,
        replan: true,
    };
    assert!(matches!(send(&seam, &d), Err(SendError::PauseFailed(_))));
    assert!(seam.sent().is_empty());
    seam.set_pause_failure(false);
    let a = send(&seam, &d).unwrap();
    let b = send(&seam, &d).unwrap();
    assert_eq!(a, b);
    assert_eq!(seam.sent(), vec![d]);
}

#[test]
fn outbox_deduplicates_same_draft_before_calling_seam() {
    use crate::desktop::data::outbox::{Outbox, Outcome, TicketState};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let seam = Arc::new(FakeInterjections::new());
    let service = seam.clone();
    let outbox = Outbox::new(move |draft: &Draft, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        crate::desktop::ux::seams::Interjections::send(service.as_ref(), draft).unwrap();
        Outcome::Applied
    });
    let draft = Draft {
        session: "s".into(),
        client_id: "id".into(),
        text: "hello".into(),
        pause_first: false,
        replan: false,
    };
    let first = submit_outbox(&outbox, &draft);
    let second = submit_outbox(&outbox, &draft);
    assert!(first.same_ticket(&second));
    for _ in 0..1000 {
        if first.state() != TicketState::Pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(first.state(), TicketState::Applied);
    let exactly_once = |calls| calls == 1;
    assert!(exactly_once(calls.load(Ordering::SeqCst)));
    let broken_calls = Arc::new(AtomicUsize::new(0));
    let counter = broken_calls.clone();
    let service = seam.clone();
    let broken_outbox = Outbox::new(move |draft: &Draft, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        crate::desktop::ux::seams::Interjections::send(service.as_ref(), draft).unwrap();
        Outcome::Applied
    });
    let broken_submit = |key: &str| submit_outbox_with(&broken_outbox, &draft, |_| key.to_owned()); // Broken: each submission gets a fresh key.
    let first = broken_submit("first");
    let second = broken_submit("second");
    for _ in 0..1000 {
        if first.state() != TicketState::Pending && second.state() != TicketState::Pending { break; }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(first.state(), TicketState::Applied);
    assert_eq!(second.state(), TicketState::Applied);
    assert!(!exactly_once(broken_calls.load(Ordering::SeqCst)));
    assert_eq!(seam.sent(), vec![draft]);
}

#[test]
fn failing_outbox_retains_the_original_draft_and_key() {
    use crate::desktop::data::outbox::{Outbox, Outcome, TicketState};
    use std::sync::{Arc, Mutex};
    let attempts = Arc::new(Mutex::new(Vec::new()));
    let recorded = attempts.clone();
    let outbox = Outbox::with_max_attempts(
        move |draft: &Draft, key| {
            recorded
                .lock()
                .unwrap()
                .push((draft.clone(), key.to_owned()));
            Outcome::Retry("offline".into())
        },
        1,
    );
    let draft = Draft {
        session: "s".into(),
        client_id: "recover".into(),
        text: "keep me".into(),
        pause_first: false,
        replan: false,
    };
    let ticket = submit_outbox(&outbox, &draft);
    for _ in 0..1000 {
        if ticket.state() != TicketState::Pending {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(ticket.state(), TicketState::Failed("offline".into()));
    assert_eq!(
        *attempts.lock().unwrap(),
        vec![(draft.clone(), draft.client_id.clone())]
    );
    let retry = submit_outbox(&outbox, &draft);
    assert!(!ticket.same_ticket(&retry));
    for _ in 0..1000 {
        if retry.state() != TicketState::Pending { break; }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(attempts.lock().unwrap().len(), 2);
}

#[test]
fn reopening_reuses_draft_identity_with_negative_control() {
    use crate::desktop::model::facade::settings::UxPrefs;
    let mut prefs = UxPrefs::default();
    let first = open_draft_id(&mut prefs, "s", || "first".into());
    let reopened = open_draft_id(&mut prefs, "s", || "second".into());
    let same = |a: &str, b: &str| a == b;
    assert!(same(&first, &reopened));
    let mut broken = prefs.clone();
    broken.draft_ids.remove("s");
    let broken_reopen = open_draft_id(&mut broken, "s", || "second".into());
    assert!(!same(&first, &broken_reopen));
}
#[test]
fn editing_after_send_rotates_identity_with_negative_control() {
    use crate::desktop::model::facade::settings::UxPrefs;
    let mut prefs = UxPrefs::default();
    prefs.drafts.insert("s".into(), "old".into());
    prefs.rotate_draft_client_id("s", "old-id".into());
    let mut broken = prefs.clone();
    edit_draft_with(&mut broken, "s", "new", || "new-id".into(), |_, _, _| {}); // Broken editor skips rotation.
    let new_id = edit_draft(&mut prefs, "s", "new", || "new-id".into()).unwrap();
    let rotated = |id: &str| id != "old-id";
    assert!(rotated(&new_id));
    assert!(!rotated(broken.draft_ids.get("s").unwrap()));
    assert_eq!(prefs.drafts.get("s").unwrap(), "new");
}
#[test]
fn applied_clears_only_matching_draft_with_negative_controls() {
    use crate::desktop::model::facade::settings::UxPrefs;
    let draft = Draft { session: "s".into(), client_id: "id".into(), text: "text".into(), pause_first: false, replan: false };
    let mut prefs = UxPrefs::default();
    prefs.drafts.insert("s".into(), "text".into());
    prefs.rotate_draft_client_id("s", "id".into());
    let mut receipt = Receipt { client_id: "id".into(), session: "s".into(), stage: ReceiptStage::Sent, at_ms: 0 };
    applied_draft(&mut prefs, &receipt, &draft);
    assert!(prefs.drafts.contains_key("s"));
    receipt.stage = ReceiptStage::Applied { first_action: "ok".into() };
    let mut edited = prefs.clone();
    edited.rotate_draft_client_id("s", "new".into());
    applied_draft(&mut edited, &receipt, &draft);
    assert!(edited.drafts.contains_key("s"));
    let mut broken = prefs.clone();
    applied_draft_with(&mut broken, &receipt, &draft, |_, _| {}); // Broken handler skips clearing.
    applied_draft(&mut prefs, &receipt, &draft);
    let cleared = |p: &UxPrefs| !p.drafts.contains_key("s") && !p.draft_ids.contains_key("s");
    assert!(cleared(&prefs));
    assert!(!cleared(&broken));
}
#[test]
fn failed_and_refused_retry_reach_runner_with_same_key() {
    use crate::desktop::data::outbox::{Outbox, Outcome, TicketState};
    use std::sync::{Arc, Mutex};
    for refused in [false, true] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let outbox = Outbox::with_max_attempts(move |d: &Draft, key| {
            recorded.lock().unwrap().push((d.clone(), key.to_owned()));
            if refused { Outcome::Refused("no".into()) } else { Outcome::Retry("offline".into()) }
        }, 1);
        let draft = Draft { session: "s".into(), client_id: "id".into(), text: "keep".into(), pause_first: false, replan: false };
        let mut prefs = crate::desktop::model::facade::settings::UxPrefs::default();
        prefs.drafts.insert("s".into(), draft.text.clone());
        prefs.rotate_draft_client_id("s", draft.client_id.clone());
        for _ in 0..2 {
            let ticket = submit_outbox(&outbox, &draft);
            for _ in 0..1000 { if ticket.state() != TicketState::Pending { break; } std::thread::sleep(std::time::Duration::from_millis(1)); }
            let state = ticket.state();
            assert!(matches!(state, TicketState::Failed(_) | TicketState::Refused(_)));
            ticket_draft(&mut prefs, &state, None, &draft);
        }
        let attempts = calls.lock().unwrap();
        let expected = vec![(draft.clone(), "id".into()), (draft.clone(), "id".into())];
        assert_eq!(*attempts, expected);
        assert_eq!(prefs.drafts.get("s"), Some(&draft.text));
        assert_eq!(prefs.draft_ids.get("s"), Some(&draft.client_id));
    }
}
