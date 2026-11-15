use axiom::{
    engine::interjections::{self, Links},
    goal,
    orch::{self, events, lifecycle::scheduler},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn setup() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let g = goal::open(home.path(), "s", "wish", Some("project")).unwrap();
    let store = orch::store_for_goal(home.path(), &g).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS x\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    (home, store)
}
fn links() -> Links {
    Links {
        plan_revision: None,
        units: vec!["u1".into()],
        scope: vec!["src/x".into()],
    }
}
fn submit(home: &Path, text: &str) -> String {
    interjections::submit(home, "s", text).unwrap().unwrap().0
}

#[test]
fn ordered_receipt_with_links_and_digest() {
    let (home, store) = setup();
    let id = submit(home.path(), "變更 abc");
    assert_eq!(id, "ij-1");
    interjections::classify(home.path(), "s", "amend").unwrap();
    let before = interjections::receipts(&store).unwrap();
    assert_eq!(
        before[0]
            .states
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        [
            "interjection_sent",
            "interjection_received",
            "interjection_classified"
        ]
    );
    let event = interjections::apply(home.path(), "s", &id, links()).unwrap();
    assert_eq!(event.data["links"]["units"], json!(["u1"]));
    assert_eq!(event.data["links"]["scope"], json!(["src/x"]));
    let rows = events::read_events(&store, 0).unwrap();
    assert_eq!(rows[1].data["chars"], 6);
    assert_eq!(
        rows[1].data["digest"],
        format!("{:x}", Sha256::digest("變更 abc".as_bytes()))
    );
    let receipt = &interjections::receipts(&store).unwrap()[0];
    assert_eq!(
        receipt
            .states
            .iter()
            .map(|s| s.0.as_str())
            .collect::<Vec<_>>(),
        [
            "interjection_sent",
            "interjection_received",
            "interjection_classified",
            "interjection_applied"
        ]
    );
    assert!(receipt.states.windows(2).all(|w| w[0].1 < w[1].1));
    assert_eq!(
        interjections::apply(home.path(), "s", &id, links())
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        interjections::decline(home.path(), "s", &id, "late")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        interjections::apply(home.path(), "s", "ij-99", links())
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}

#[test]
fn decline_and_replan_requirements() {
    let (home, store) = setup();
    let id = submit(home.path(), "replan");
    interjections::classify(home.path(), "s", "replan").unwrap();
    assert_eq!(
        interjections::apply(home.path(), "s", &id, links())
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        interjections::decline(home.path(), "s", &id, "  ")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let e = interjections::decline(home.path(), "s", &id, "not needed").unwrap();
    assert_eq!(e.data["reason"], "not needed");
    assert_eq!(
        interjections::receipts(&store).unwrap()[0]
            .states
            .last()
            .unwrap()
            .0,
        "interjection_declined"
    );
    assert_eq!(
        interjections::decline(home.path(), "s", &id, "again")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let next = submit(home.path(), "replan again");
    interjections::classify(home.path(), "s", "replan").unwrap();
    let mut revision = links();
    revision.plan_revision = Some("rev-2".into());
    interjections::apply(home.path(), "s", &next, revision).unwrap();
}

#[test]
fn queued_promotes_in_submission_order_and_persists_counter() {
    let (home, store) = setup();
    for (n, prompt) in ["one", "two", "three"].iter().enumerate() {
        assert_eq!(submit(home.path(), prompt), format!("ij-{}", n + 1));
    }
    let g = goal::load(home.path(), "s").unwrap().unwrap();
    assert_eq!(g.interjection.as_ref().unwrap().prompt, "three");
    assert_eq!(
        g.queued
            .iter()
            .map(|p| p.prompt.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(g.amendments, ["one", "two", "three"]);
    for (n, prompt) in ["three", "two", "one"].iter().enumerate() {
        let g = goal::load(home.path(), "s").unwrap().unwrap();
        assert_eq!(g.interjection.unwrap().prompt, *prompt);
        interjections::classify(home.path(), "s", "amend").unwrap();
        assert_eq!(
            interjections::receipts(&store).unwrap()[2 - n]
                .states
                .last()
                .unwrap()
                .0,
            "interjection_classified"
        );
    }
    assert!(goal::load(home.path(), "s")
        .unwrap()
        .unwrap()
        .interjection
        .is_none());
    assert_eq!(submit(home.path(), "four"), "ij-4");
}

#[test]
fn pause_blocks_dispatch_until_applied_or_declined() {
    let (home, store) = setup();
    let id = submit(home.path(), "pause");
    interjections::classify(home.path(), "s", "pause_batch").unwrap();
    assert_eq!(
        interjections::paused(&store).unwrap().as_deref(),
        Some(id.as_str())
    );
    assert_eq!(scheduler::claim(&store, "m").unwrap_err().kind, "held");
    let reply = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(reply.held.as_deref(), Some("interjection ij-1 paused"));
    assert!(reply.start.is_empty());
    interjections::apply(home.path(), "s", &id, links()).unwrap();
    assert_eq!(interjections::paused(&store).unwrap(), None);
    let id2 = submit(home.path(), "pause again");
    interjections::classify(home.path(), "s", "pause_batch").unwrap();
    assert_eq!(scheduler::claim(&store, "m").unwrap_err().kind, "held");
    interjections::decline(home.path(), "s", &id2, "skip").unwrap();
    assert_eq!(
        scheduler::next(&store, None, Duration::ZERO).unwrap().start,
        ["m"]
    );
}

#[test]
fn answer_and_stop_apply_when_classified() {
    let (home, store) = setup();
    for (kind, status) in [("answer", "active"), ("stop", "blocked")] {
        let id = submit(home.path(), kind);
        let goal = interjections::classify(home.path(), "s", kind).unwrap();
        assert_eq!(goal.status, status);
        let e = events::read_events(&store, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "interjection_applied" && e.data["id"] == id)
            .unwrap();
        assert_eq!(e.data["links"]["status"], status);
    }
}

#[test]
fn legacy_goal_without_fields_loads() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("goals")).unwrap();
    fs::write(home.path().join("goals/s.json"), r#"{"wish":"old","status":"active","interjection":{"prompt":"old prompt","prior_status":"active","prior_note":null},"extra":42}"#).unwrap();
    let g = goal::load(home.path(), "s").unwrap().unwrap();
    assert_eq!(g.interjection.unwrap().id, None);
    assert!(g.queued.is_empty());
    assert_eq!(g.interjection_counter, 0);
    assert_eq!(g.other["extra"], 42);
}

#[test]
fn answering_duplicate_prompt_preserves_previous_amendments() {
    let (home, _) = setup();
    submit(home.path(), "same");
    interjections::classify(home.path(), "s", "amend").unwrap();
    submit(home.path(), "other");
    interjections::classify(home.path(), "s", "amend").unwrap();
    submit(home.path(), "same");
    submit(home.path(), "same");
    let g = interjections::classify(home.path(), "s", "answer").unwrap();
    assert_eq!(g.amendments, ["same", "other", "same"]);
    assert_eq!(g.interjection.unwrap().prompt, "same");
}

#[test]
fn answering_older_duplicate_preserves_newer_classified_amendment() {
    let (home, _) = setup();
    submit(home.path(), "same");
    submit(home.path(), "middle");
    submit(home.path(), "same");
    interjections::classify(home.path(), "s", "amend").unwrap();
    interjections::classify(home.path(), "s", "amend").unwrap();
    let g = interjections::classify(home.path(), "s", "answer").unwrap();
    assert_eq!(g.amendments, ["middle", "same"]);
}

#[test]
fn new_interjection_inherits_oldest_outstanding_prior_state() {
    let (home, _) = setup();
    goal::set_status(home.path(), "s", "blocked", Some("original")).unwrap();
    for prompt in ["one", "two", "three"] {
        submit(home.path(), prompt);
    }
    interjections::classify(home.path(), "s", "stop").unwrap();
    submit(home.path(), "four");
    let g = goal::load(home.path(), "s").unwrap().unwrap();
    let pending = g.interjection.unwrap();
    assert_eq!(pending.prior_status, "blocked");
    assert_eq!(pending.prior_note.as_deref(), Some("original"));
    let g = interjections::classify(home.path(), "s", "answer").unwrap();
    assert_eq!(g.note.as_deref(), Some("original"));
    let next = g.interjection.unwrap();
    assert_eq!(next.prompt, "two");
    assert_eq!(next.prior_note.as_deref(), Some("original"));
}

#[test]
fn pause_does_not_replay_an_earlier_claim_as_a_start() {
    let (home, store) = setup();
    scheduler::claim(&store, "m").unwrap();
    let id = submit(home.path(), "pause");
    interjections::classify(home.path(), "s", "pause_batch").unwrap();
    let reply = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(reply.running, ["m"]);
    assert!(reply.start.is_empty());
    interjections::apply(home.path(), "s", &id, links()).unwrap();
    assert_eq!(
        scheduler::next(&store, None, Duration::ZERO).unwrap().start,
        ["m"]
    );
}
