mod common;
use axiom::{
    codex::AppServer,
    engine::{
        adjudicator::{self, Case, Evidence, Outcome, Ruling},
        budgets::{self, Role},
        policy,
    },
    orch::{self, events},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    store: PathBuf,
    repo: PathBuf,
    log: PathBuf,
    server: AppServer,
}
impl Fixture {
    fn new(reply: Value) -> Self {
        let temp = common::temp();
        let store = temp.path().join("store");
        let repo = temp.path().join("repo");
        fs::create_dir_all(store.join("reports")).unwrap();
        fs::create_dir_all(&repo).unwrap();
        let script = temp.path().join("script.json");
        let log = temp.path().join("log.jsonl");
        fs::write(
            &script,
            json!([{"match":"AXIOM-ENGINE adjudicator", "reply":reply}]).to_string(),
        )
        .unwrap();
        let mut cmd = common::fake("fake_codex");
        cmd.push(format!("--fake-script={}", script.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        let server = AppServer::new(cmd).with_slots_home(temp.path().join("slots"));
        Self {
            _temp: temp,
            store,
            repo,
            log,
            server,
        }
    }
    fn enable(&self) {
        policy::set(&self.store, "adjudicator", "on").unwrap();
    }
    fn run(&self, case: &Case, evidence: Vec<Evidence>, reserved: bool) -> Outcome {
        adjudicator::adjudicate(
            &self.server,
            &self.store,
            &self.repo,
            case,
            evidence,
            reserved,
        )
        .unwrap()
    }
    fn events(&self) -> Vec<events::Event> {
        events::read_events(&self.store, 0).unwrap()
    }
    fn log(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
fn item() -> Vec<Evidence> {
    vec![Evidence {
        id: "E1".into(),
        path: None,
        text: "capture confirms check".into(),
    }]
}
fn needs() -> Case {
    Case::NeedsCheck {
        unit: "u".into(),
        finding: 1,
        needs: "manual".into(),
    }
}
fn assert_person(out: Outcome, expected: &str) {
    match out {
        Outcome::Person { reason } => assert_eq!(reason, expected),
        _ => panic!("expected person"),
    }
}
fn ruling(out: Outcome) -> Ruling {
    match out {
        Outcome::Ruled(r) => r,
        _ => panic!("expected ruling"),
    }
}
fn verdict(store: &Path, findings: Value) {
    fs::write(
        store.join("reports/u.verdict.json"),
        json!({"unit":"u","tree":"abc","verdict":"fail","findings":findings}).to_string(),
    )
    .unwrap();
}

#[test]
fn pass_records_ruling_and_uses_fresh_threads() {
    let f = Fixture::new(json!({"decision":"pass","reason":"confirmed","evidence":["E1"]}));
    f.enable();
    let r = ruling(f.run(&needs(), item(), false));
    assert_eq!(r.id, "adj-1");
    assert_eq!(
        fs::read_to_string(f.store.join("decisions.md")).unwrap(),
        "- [adjudicator adj-1] needs_check u#1: pass: confirmed (evidence: E1)\n"
    );
    let event = f
        .events()
        .into_iter()
        .find(|e| e.kind == "adjudication")
        .unwrap();
    assert_eq!(event.unit.as_deref(), Some("u"));
    assert_eq!(event.data["on_behalf_of_person"], true);
    ruling(f.run(&needs(), item(), false));
    let log = f.log();
    let turns: Vec<_> = log.iter().filter(|v| v["method"] == "turn/start").collect();
    assert_eq!(turns.len(), 2);
    assert_ne!(
        turns[0]["params"]["threadId"],
        turns[1]["params"]["threadId"]
    );
    for turn in turns {
        assert_eq!(turn["params"]["effort"], "max");
        let input = turn["params"]["input"][0]["text"].as_str().unwrap();
        assert!(input.contains("[E1] capture confirms check"));
        assert!(!input.contains("SECRET BRIEF"));
        assert!(!input.contains("SECRET HANDOFF"));
    }
}
#[test]
fn missing_or_unknown_evidence_falls_back() {
    let f = Fixture::new(json!({"decision":"pass","reason":"x","evidence":["E99"]}));
    f.enable();
    assert_person(f.run(&needs(), vec![], false), "missing_evidence");
    assert!(f.log().is_empty());
    assert_person(f.run(&needs(), item(), false), "missing_evidence");
    assert_eq!(
        f.events()
            .iter()
            .filter(|e| e.kind == "person_request" && e.data["reason"] == "missing_evidence")
            .count(),
        2
    );
}
#[test]
fn policy_caps_and_reserved_do_not_call() {
    let f = Fixture::new(json!({}));
    assert_person(f.run(&needs(), item(), true), "reserved");
    for question in [
        "please PUSH",
        "merge to main",
        "change the scope",
        "password",
    ] {
        assert_person(
            f.run(
                &Case::CloseAsk {
                    question: question.into(),
                },
                item(),
                false,
            ),
            "reserved",
        );
    }
    assert_person(
        f.run(
            &Case::CouncilDispute {
                claim: "Purchase now".into(),
                positions: vec![],
            },
            item(),
            false,
        ),
        "reserved",
    );
    assert!(!adjudicator::is_reserved("reorder the payload"));
    assert_person(f.run(&needs(), item(), false), "disabled");
    f.enable();
    policy::set(&f.store, "adjudicator.max_adjudications", "0").unwrap();
    assert_person(f.run(&needs(), item(), false), "cap_count");
    policy::set(&f.store, "adjudicator.max_adjudications", "5").unwrap();
    policy::set(&f.store, "adjudicator.max_usd", "0").unwrap();
    assert_person(f.run(&needs(), item(), false), "cap_spend");
    policy::set(&f.store, "adjudicator.max_usd", "5").unwrap();
    budgets::record(&f.store, Role::Adjudicator, "unknown", None, 0, 0, "test").unwrap();
    assert_person(f.run(&needs(), item(), false), "cap_spend");
    assert!(f.log().is_empty());
    assert_eq!(
        f.events()
            .iter()
            .filter(|e| e.kind == "person_request")
            .count(),
        10
    );
}
#[test]
fn budget_failure_and_invalid_decision() {
    let f = Fixture::new(json!({"decision":"waive","reason":"x","evidence":["E1"]}));
    f.enable();
    policy::set(&f.store, "budget.adjudicator", "0").unwrap();
    assert_person(f.run(&needs(), item(), false), "budget_exceeded");
    assert!(f.log().is_empty());
    policy::set(&f.store, "budget.adjudicator", "none").unwrap();
    assert_person(f.run(&needs(), item(), false), "failed");
}
#[test]
fn council_answer_and_overturn() {
    let f = Fixture::new(
        json!({"decision":"answer","reason":"resolved","evidence":["E1"],"answer":"yes"}),
    );
    f.enable();
    let r = ruling(f.run(
        &Case::CouncilDispute {
            claim: "which design".into(),
            positions: vec!["A".into(), "B".into()],
        },
        item(),
        false,
    ));
    assert_eq!(r.answer.as_deref(), Some("yes"));
    let e = adjudicator::overturn(&f.store, &r.id, "changed facts").unwrap();
    assert_eq!(e.kind, "adjudication_overturned");
    assert_eq!(e.data["on_behalf_of_person"], false);
    assert!(fs::read_to_string(f.store.join("decisions.md"))
        .unwrap()
        .contains("- [person] overturned adj-1: changed facts\n"));
    assert_eq!(
        adjudicator::overturn(&f.store, "adj-9", "x")
            .unwrap_err()
            .kind,
        "not_found"
    );
}
#[test]
fn fix_rounds_apply_only_waives_remaining_failures() {
    let f = Fixture::new(json!({}));
    fs::write(
        f.store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t\n",
    )
    .unwrap();
    verdict(
        &f.store,
        json!([{ "severity":"major" },{"severity":"blocker"},{"severity":"minor"}]),
    );
    for (kind, exit_code) in [("run", 0), ("verify", 0), ("review", 1)] {
        orch::append_evidence(
            &f.store,
            "u",
            &orch::Evidence {
                kind: kind.into(),
                command: if kind == "review" {
                    "claude-review"
                } else {
                    "gpt-run"
                }
                .into(),
                exit_code,
                commit: "abc".into(),
                ts: "now".into(),
            },
        )
        .unwrap();
    }
    orch::append_evidence(&f.store, "u", &orch::waive_row(1, "already", "abc", false)).unwrap();
    let case = Case::FixRounds { unit: "u".into() };
    let mut r = Ruling {
        id: "adj-1".into(),
        decision: "block".into(),
        reason: "ok".into(),
        evidence: vec![],
        answer: None,
    };
    let before = orch::evidence(&f.store, "u").unwrap().len();
    adjudicator::apply(&f.store, &case, &r).unwrap();
    assert_eq!(orch::evidence(&f.store, "u").unwrap().len(), before);
    r.decision = "waive".into();
    adjudicator::apply(&f.store, &case, &r).unwrap();
    let rows = orch::evidence(&f.store, "u").unwrap();
    let ws = orch::waivers(&rows);
    assert_eq!(ws.len(), 2);
    assert_eq!(ws[1].number, 2);
    assert_eq!(ws[1].reason, "adjudicator adj-1: ok");
    assert!(ws[1].closes);
    assert_eq!(orch::read_units(&f.store).unwrap()[0].state, "verified");
}
#[test]
fn evidence_pack_limits_and_excludes_other_material() {
    let f = Fixture::new(json!({}));
    fs::write(f.repo.join("capture.log"), "CAPTURE").unwrap();
    fs::write(f.store.join("reports/u.verify.txt"), "VERIFY").unwrap();
    fs::write(f.repo.join("brief.md"), "SECRET BRIEF").unwrap();
    fs::write(f.repo.join("core.md"), "SECRET CORE").unwrap();
    fs::write(f.store.join("handoff.md"), "SECRET HANDOFF").unwrap();
    verdict(
        &f.store,
        json!([{ "severity":"major", "check":"untested", "needs":"manual", "evidence":"capture.log brief.md handoff.md core.md" }]),
    );
    events::append_event(
        &f.store,
        "test",
        None,
        Some("u"),
        json!({"message":"event"}),
    )
    .unwrap();
    events::append_event(
        &f.store,
        "test",
        None,
        Some("other"),
        json!({"message":"other"}),
    )
    .unwrap();
    let pack = adjudicator::evidence_for(&f.store, &f.repo, &needs()).unwrap();
    assert_eq!(
        pack.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["E1", "E2", "E3"]
    );
    assert_eq!(pack[0].text, "CAPTURE");
    assert_eq!(pack[1].text, "VERIFY");
    assert!(pack[2].text.contains("event"));
    assert!(pack
        .iter()
        .all(|e| !e.text.contains("SECRET") && e.text.len() <= 8192));
}

#[test]
fn count_cap_uses_adjudication_events_and_spend_cap_uses_spend() {
    let f = Fixture::new(json!({}));
    f.enable();
    policy::set(&f.store, "adjudicator.max_adjudications", "1").unwrap();
    events::append_event(&f.store, "adjudication", None, None, json!({"id":"adj-1"})).unwrap();
    assert_person(f.run(&needs(), item(), false), "cap_count");
    policy::set(&f.store, "adjudicator.max_adjudications", "2").unwrap();
    budgets::record(
        &f.store,
        Role::Adjudicator,
        "known",
        Some(5.0),
        0,
        0,
        "test",
    )
    .unwrap();
    assert_person(f.run(&needs(), item(), false), "cap_spend");
    assert!(f.log().is_empty());
}

#[test]
fn evidence_content_is_capped_and_passed_without_other_files() {
    let f = Fixture::new(json!({"decision":"pass", "reason":"confirmed", "evidence":["E1"]}));
    f.enable();
    fs::write(f.repo.join("capture.log"), "é".repeat(5000)).unwrap();
    fs::write(f.repo.join("brief.md"), "SECRET BRIEF").unwrap();
    fs::write(f.repo.join("core.md"), "SECRET CORE").unwrap();
    fs::write(f.repo.join("handoff.md"), "SECRET HANDOFF").unwrap();
    verdict(
        &f.store,
        json!([{"severity":"major", "check":"untested", "needs":"manual", "evidence":"capture.log brief.md handoff.md core.md"}]),
    );
    let pack = adjudicator::evidence_for(&f.store, &f.repo, &needs()).unwrap();
    assert_eq!(pack.len(), 1);
    assert_eq!(pack[0].text.len(), 8192);
    ruling(f.run(&needs(), pack, false));
    let log = f.log();
    let input = log.iter().find(|e| e["method"] == "turn/start").unwrap()["params"]["input"][0]
        ["text"]
        .as_str()
        .unwrap();
    assert!(!input.contains("SECRET BRIEF"));
    assert!(!input.contains("SECRET HANDOFF"));
}
