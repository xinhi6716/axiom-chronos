//! Tests for unit_actions; owned by module engine-views-seam.
use super::unit_actions::OrchUnitActions;
use crate::{
    desktop::ux::seams::unit_actions::{UnitActionKind, UnitActions},
    orch::{self, Evidence},
};
use std::path::{Path, PathBuf};

struct Fixture {
    home: tempfile::TempDir,
    _project: tempfile::TempDir,
    store: PathBuf,
    brief: String,
}
fn call(store: &Path, args: &[&str]) -> orch::Output {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    orch::run(store, &args, false, false)
}
fn row(kind: &str, command: &str, exit: i32, commit: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: exit,
        commit: commit.into(),
        ts: "2026-01-01T00:00:00Z".into(),
    }
}
fn fixture() -> Fixture {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let goal = crate::goal::open(
        home.path(),
        "s1",
        "wish",
        Some(project.path().to_str().unwrap()),
    )
    .unwrap();
    let store = orch::store_for_goal(home.path(), &goal).unwrap();
    assert_eq!(call(&store, &["init"]).exit_code, 0);
    let brief = project.path().join("brief.md");
    std::fs::write(&brief, "GOAL g\nSCOPE src/a\nVERIFY echo ok\nRISK standard\n").unwrap();
    let brief = brief.to_str().unwrap().to_owned();
    let out = call(&store, &["unit", "add", "u1", "--track", "core", "--brief", &brief]);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    std::fs::write(
        store.join("modules.md"),
        "## core\nPURPOSE test\nPATHS src/a\nDEPENDS none\nCONTRACT test\nACCEPT test `echo ok`\nROUTING\n  default sol/high standard fixture\n",
    )
    .unwrap();
    Fixture {
        home,
        _project: project,
        store,
        brief,
    }
}
fn actions(f: &Fixture) -> OrchUnitActions {
    OrchUnitActions::new(f.home.path().to_path_buf())
}
fn verdict(f: &Fixture, unit: &str, severities: &[&str]) {
    let findings: Vec<_> = severities
        .iter()
        .map(|s| serde_json::json!({"severity":s,"check":"bug","location":"a.rs:1","evidence":"e","fix":"f"}))
        .collect();
    let v = serde_json::json!({"unit":unit,"tree":"T1","verdict":"fail","findings":findings});
    std::fs::create_dir_all(f.store.join("reports")).unwrap();
    std::fs::write(orch::verdict_path(&f.store, unit), v.to_string()).unwrap();
}
fn review(f: &Fixture, unit: &str) {
    orch::append_evidence(&f.store, unit, &row("run", "gpt-5", 0, "T1")).unwrap();
    orch::append_evidence(&f.store, unit, &row("review", "chronos:reviewer", 1, "T1")).unwrap();
}
fn waive(reason: &str, finding: Option<usize>) -> UnitActionKind {
    UnitActionKind::Waive {
        reason: reason.into(),
        finding,
    }
}

#[test]
fn outbox_applies_and_refuses_real_unit_actions() {
    use crate::desktop::{
        data::{outbox::TicketState, unit_action_outbox_for},
        ux::seams::unit_actions::UnitAction,
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let f = fixture();
    let adapter = Arc::new(actions(&f));
    let outbox = unit_action_outbox_for(adapter.clone());
    let action = UnitAction {
        session: "s1".into(),
        unit: "u1".into(),
        kind: UnitActionKind::Retry,
        key: "retry".into(),
    };
    let ticket = outbox.submit(action.clone(), &action.key);
    let duplicate = outbox.submit(action.clone(), &action.key);
    assert!(ticket.same_ticket(&duplicate));
    let deadline = Instant::now() + Duration::from_secs(10);
    while ticket.state() == TicketState::Pending {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(ticket.state(), TicketState::Applied);
    assert!(
        adapter
            .act("s1", "u1", &action.kind, &action.key)
            .unwrap()
            .accepted
    );
    assert_eq!(
        orch::read_units(&f.store)
            .unwrap()
            .iter()
            .filter(|u| u.id == "u1-retry1")
            .count(),
        1
    );
    let refused = UnitAction {
        key: "refused".into(),
        kind: waive("why", None),
        ..action
    };
    let ticket = outbox.submit(refused.clone(), &refused.key);
    let deadline = Instant::now() + Duration::from_secs(10);
    while ticket.state() == TicketState::Pending {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(matches!(ticket.state(), TicketState::Refused(reason) if reason.contains("no review")));
    assert!(outbox.tickets().iter().any(|t| t.same_ticket(&ticket)));
}

#[test]
fn evidence_reports_diff_review_usage_and_failure() {
    let f = fixture();
    let a = actions(&f);
    assert_eq!(a.evidence("s1", "nope").unwrap_err().kind, "not_found");
    let empty = a.evidence("s1", "u1").unwrap();
    assert_eq!(
        (empty.diff, empty.review, empty.cost_usd, empty.tokens, empty.failure),
        (None, None, None, None, None)
    );
    review(&f, "u1");
    verdict(&f, "u1", &["major", "minor"]);
    std::fs::write(
        f.store.join("reports/u1.json"),
        r#"{"usage":{"inputTokens":1000000,"cachedInputTokens":0,"outputTokens":0}}"#,
    )
    .unwrap();
    std::fs::write(f.store.join("reports/u1.changed.json"), r#"["src/a.rs"]"#).unwrap();
    let e = a.evidence("s1", "u1").unwrap();
    assert!(e.diff.unwrap().contains("src/a.rs"));
    assert!(e.review.unwrap().contains("1 serious"));
    assert_eq!(e.tokens, Some(1_000_000));
    assert_eq!(e.cost_usd, Some(1.25));
    assert!(e.failure.unwrap().starts_with("review"));
}

#[test]
fn waive_resolves_a_single_serious_finding_and_is_idempotent() {
    let f = fixture();
    let a = actions(&f);
    review(&f, "u1");
    verdict(&f, "u1", &["major", "minor"]);
    assert_eq!(
        a.act("s1", "u1", &waive("  ", None), "k0").unwrap_err().kind,
        "reason_required"
    );
    let r = a.act("s1", "u1", &waive("accepted risk", None), "k1").unwrap();
    assert!(r.accepted, "{}", r.message);
    assert!(orch::waivers(&orch::evidence(&f.store, "u1").unwrap())
        .iter()
        .any(|w| w.number == 1));
    // The same key replays the receipt, also through a new adapter, without a second waive.
    let again = OrchUnitActions::new(f.home.path().to_path_buf())
        .act("s1", "u1", &waive("accepted risk", None), "k1")
        .unwrap();
    assert_eq!(again, r);
    let rows = orch::evidence(&f.store, "u1").unwrap();
    assert_eq!(rows.iter().filter(|r| r.kind == "waive").count(), 1);
}

#[test]
fn waive_without_a_finding_needs_exactly_one_serious() {
    let f = fixture();
    let a = actions(&f);
    review(&f, "u1");
    verdict(&f, "u1", &["blocker", "major"]);
    let r = a.act("s1", "u1", &waive("why", None), "k1").unwrap();
    assert!(!r.accepted);
    assert_eq!(r.message, "choose a finding");
    let r = a.act("s1", "u1", &waive("why", Some(2)), "k2").unwrap();
    assert!(r.accepted, "{}", r.message);
    let r = a.act("s1", "u1", &waive("why", Some(1)), "k3").unwrap();
    assert!(r.accepted, "{}", r.message);
    // Negative control: a minor finding cannot be waived.
    verdict(&f, "u1", &["blocker", "minor"]);
    let r = a.act("s1", "u1", &waive("why", Some(2)), "k4").unwrap();
    assert!(!r.accepted);
}

#[test]
fn waive_against_a_stale_review_is_refused() {
    let f = fixture();
    let a = actions(&f);
    review(&f, "u1");
    verdict(&f, "u1", &["major"]);
    orch::append_evidence(&f.store, "u1", &row("run", "gpt-5", 0, "T2")).unwrap();
    let r = a.act("s1", "u1", &waive("why", None), "k1").unwrap();
    assert!(!r.accepted);
    assert!(r.message.contains("stale"), "{}", r.message);
    assert!(orch::evidence(&f.store, "u1")
        .unwrap()
        .iter()
        .all(|r| r.kind != "waive"));
}

#[test]
fn retry_adds_a_fix_unit_but_not_while_claimed_or_at_fix_rounds() {
    let f = fixture();
    let a = actions(&f);
    // Claimed: refused, nothing added.
    std::fs::write(
        f.store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\nu1\ta1\tsrc/a\tnow\tcore\twish\t1\t\n",
    )
    .unwrap();
    let r = a.act("s1", "u1", &UnitActionKind::Retry, "k1").unwrap();
    assert!(!r.accepted);
    assert!(r.message.contains("busy"), "{}", r.message);
    assert_eq!(orch::read_units(&f.store).unwrap().len(), 1);
    std::fs::write(
        f.store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\n",
    )
    .unwrap();
    let r = a.act("s1", "u1", &UnitActionKind::Retry, "k2").unwrap();
    assert!(r.accepted, "{}", r.message);
    let units = orch::read_units(&f.store).unwrap();
    assert!(units.iter().any(|u| u.id == "u1-retry1"));
    assert_eq!(
        units.iter().find(|u| u.id == "u1").unwrap().superseded_by(),
        Some("u1-retry1")
    );
    let r = a.act("s1", "u1-retry1", &UnitActionKind::Retry, "k3").unwrap();
    assert!(r.accepted, "{}", r.message);
    // A third round in the chain hits fix_rounds.
    let r = a
        .act("s1", "u1-retry1-retry1", &UnitActionKind::Retry, "k4")
        .unwrap();
    assert!(!r.accepted);
    assert!(r.message.contains("fix_rounds"), "{}", r.message);
    let _ = &f.brief;
}

#[test]
fn review_uses_the_latest_reviewers_matching_verdict() {
    let f = fixture();
    review(&f, "u1");
    verdict(&f, "u1", &["major"]);
    let pm = serde_json::json!({"unit":"u1","tree":"T1","verdict":"fail","findings":[{"severity":"blocker","check":"pm-bug","location":"b.rs:2"}]} );
    std::fs::write(orch::review_verdict_path(&f.store, "u1", "chronos:pm-qa"), pm.to_string()).unwrap();
    orch::append_evidence(&f.store, "u1", &row("review", "chronos:pm-qa", 1, "T1")).unwrap();
    let text = actions(&f).evidence("s1", "u1").unwrap().review.unwrap();
    assert!(text.contains("pm-bug"), "{text}");
    assert!(!text.contains("a.rs:1"), "{text}");
}

#[test]
fn waive_refuses_a_verdict_from_a_different_review() {
    let f = fixture();
    review(&f, "u1");
    verdict(&f, "u1", &["major"]);
    orch::append_evidence(&f.store, "u1", &row("review", "chronos:reviewer", 1, "T2")).unwrap();
    let a = actions(&f);
    let r = a.act("s1", "u1", &waive("why", Some(1)), "stale-tree").unwrap();
    assert!(!r.accepted, "{}", r.message);
    assert!(r.message.contains("stale"), "{}", r.message);
    assert!(orch::evidence(&f.store, "u1").unwrap().iter().all(|r| r.kind != "waive"));
}

#[test]
fn waive_refuses_another_reviewers_verdict_even_on_the_same_tree() {
    let f = fixture();
    review(&f, "u1");
    verdict(&f, "u1", &["major"]);
    orch::append_evidence(&f.store, "u1", &row("review", "chronos:pm-qa", 1, "T1")).unwrap();
    let r = actions(&f).act("s1", "u1", &waive("why", Some(1)), "stale-reviewer").unwrap();
    assert!(!r.accepted, "{}", r.message);
    assert!(orch::evidence(&f.store, "u1").unwrap().iter().all(|r| r.kind != "waive"));
}

#[test]
fn review_can_come_from_a_verdict_without_a_review_row() {
    let f = fixture();
    verdict(&f, "u1", &["major"]);
    assert!(actions(&f).evidence("s1", "u1").unwrap().review.unwrap().contains("1 serious"));
}

#[test]
fn usage_exposes_known_tokens_and_uses_the_report_model() {
    let f = fixture();
    review(&f, "u1");
    let path = f.store.join("reports/u1.json");
    std::fs::write(&path, r#"{"model":"gpt-5-mini","usage":{"inputTokens":1000000,"outputTokens":0}}"#).unwrap();
    let e = actions(&f).evidence("s1", "u1").unwrap();
    assert_eq!(e.tokens, Some(1_000_000));
    assert_eq!(e.cost_usd, None);
    std::fs::write(&path, r#"{"model":"gpt-5-mini","usage":{"inputTokens":1000000,"cachedInputTokens":0,"outputTokens":0}}"#).unwrap();
    let e = actions(&f).evidence("s1", "u1").unwrap();
    assert_eq!(e.cost_usd, Some(0.25));
}
