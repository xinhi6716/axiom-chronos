mod common;

use axiom::{
    codex::AppServer,
    engine::{
        budgets::{self, Role},
        dual_core::{judge, Decision, Kind, Verdict},
        kinds, policy,
    },
    orch::events,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fake(name: &str, script: &Path, log: &Path) -> Vec<String> {
    let mut cmd = common::fake(name);
    cmd.push(format!("--fake-script={}", script.display()));
    cmd.push(format!("--fake-log={}", log.display()));
    cmd
}
fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn decision() -> Decision {
    Decision {
        kind: Kind::PlanAcceptance,
        id: "d1".into(),
        question: "Question marker".into(),
        inputs: vec!["evidence.txt".into()],
        core_choice: "Accept".into(),
        core_reasoning: "SECRET_REASONING_SENTINEL".into(),
        irreversible: false,
        reserved: false,
    }
}
fn rows(store: &Path, kind: &str) -> Vec<Value> {
    events::read_events(store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == kind)
        .map(|e| e.data)
        .collect()
}
fn write_script(path: &Path, choice: &str, uphold: &str) {
    fs::write(path, json!([
        {"match":"AXIOM-ENGINE co_core plan_acceptance", "reply":{"choice":choice,"reason":"co-core evidence"}},
        {"match":"AXIOM-ENGINE arbiter plan_acceptance", "reply":{"uphold":uphold,"reason":"arbiter reason"}}
    ]).to_string()).unwrap();
}

// The only test in this process that sets AXIOM_CLAUDE_CMD.
#[test]
fn dual_core_contract() {
    let previous = std::env::var_os("AXIOM_CLAUDE_CMD");
    let temp = common::temp();
    let store = temp.path();
    let script = store.join("script.json");
    let codex_log = store.join("codex.jsonl");
    let member_log = store.join("member.jsonl");
    fs::write(store.join("evidence.txt"), "INPUT CONTENT MARKER").unwrap();
    write_script(&script, " accept ", "core");
    std::env::set_var(
        "AXIOM_CLAUDE_CMD",
        serde_json::to_string(&fake("fake_member", &script, &member_log)).unwrap(),
    );
    let server = AppServer::new(fake("fake_codex", &script, &codex_log))
        .with_slots_home(store.join("slots"));
    let mut d = decision();
    assert_eq!(judge(&server, store, store, &d).unwrap(), Verdict::Off);
    assert!(lines(&codex_log).is_empty() && lines(&member_log).is_empty());
    assert!(rows(store, kinds::DUAL_CORE).is_empty());
    policy::set(store, "dual_core", "on").unwrap();
    d.reserved = true;
    assert_eq!(judge(&server, store, store, &d).unwrap(), Verdict::Reserved);
    assert!(lines(&codex_log).is_empty() && lines(&member_log).is_empty());
    assert!(rows(store, kinds::DUAL_CORE).is_empty());
    d.reserved = false;
    budgets::record(store, Role::Core, "prior", Some(1.25), 0, 0, "prior").unwrap();
    budgets::record(store, Role::CoCore, "prior", Some(0.5), 0, 0, "prior").unwrap();
    budgets::record(store, Role::Arbiter, "prior", Some(0.75), 0, 0, "prior").unwrap();
    let before = budgets::spent(store, Role::CoCore).unwrap().usd;
    assert_eq!(judge(&server, store, store, &d).unwrap(), Verdict::Agree);
    let after = budgets::spent(store, Role::CoCore).unwrap().usd;
    let log = lines(&codex_log);
    let turns: Vec<_> = log.iter().filter(|v| v["method"] == "turn/start").collect();
    assert_eq!(turns.len(), 1);
    let prompt = turns[0]["params"]["input"][0]["text"].as_str().unwrap();
    for text in ["Question marker", "INPUT CONTENT MARKER", "Accept"] {
        assert!(prompt.contains(text), "missing {text}: {prompt}");
    }
    assert!(!prompt.contains("SECRET_REASONING_SENTINEL"));
    assert!(lines(&member_log).is_empty());
    let e = rows(store, kinds::DUAL_CORE);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0]["id"], "d1");
    assert_eq!(e[0]["kind"], "plan_acceptance");
    assert_eq!(e[0]["step"], "co_core");
    assert_eq!(e[0]["choice"], " accept ");
    assert_eq!(e[0]["agree"], true);
    assert_eq!(e[0]["core_usd"], 1.25);
    assert!((e[0]["usd"].as_f64().unwrap() - (after - before)).abs() < 1e-10);
    assert!(after > before);
    for (uphold, irreversible, verdict) in [
        (
            "core",
            false,
            Verdict::Upheld {
                reason: "arbiter reason".into(),
            },
        ),
        (
            "co_core",
            false,
            Verdict::Overruled {
                reason: "arbiter reason".into(),
            },
        ),
        (
            "co_core",
            true,
            Verdict::ToPerson {
                reason: "arbiter reason".into(),
            },
        ),
    ] {
        write_script(&script, "Reject", uphold);
        let server = AppServer::new(fake("fake_codex", &script, &codex_log))
            .with_slots_home(store.join("slots"));
        d.irreversible = irreversible;
        let before_co = budgets::spent(store, Role::CoCore).unwrap().usd;
        let before_arb = budgets::spent(store, Role::Arbiter).unwrap().usd;
        let n = rows(store, kinds::DUAL_CORE).len();
        let persons = rows(store, kinds::PERSON_REQUEST).len();
        let fables = lines(&member_log).len();
        assert_eq!(judge(&server, store, store, &d).unwrap(), verdict);
        let member = lines(&member_log);
        assert_eq!(member.len(), fables + 1);
        let prompt = member.last().unwrap()["prompt"].as_str().unwrap();
        for text in ["Accept", "Reject", "co-core evidence"] {
            assert!(prompt.contains(text));
        }
        assert!(!prompt.contains("SECRET_REASONING_SENTINEL"));
        assert!(member.last().unwrap()["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == axiom::council::FABLE_MODEL));
        let e = rows(store, kinds::DUAL_CORE);
        assert_eq!(e.len(), n + 2);
        assert_eq!(e[n]["step"], "co_core");
        assert_eq!(e[n]["agree"], false);
        assert_eq!(e[n + 1]["step"], "arbiter");
        assert_eq!(e[n + 1]["choice"], uphold);
        assert_eq!(e[n + 1]["agree"], uphold == "core");
        assert!(
            (e[n]["usd"].as_f64().unwrap()
                - (budgets::spent(store, Role::CoCore).unwrap().usd - before_co))
                .abs()
                < 1e-10
        );
        assert!(
            (e[n + 1]["usd"].as_f64().unwrap()
                - (budgets::spent(store, Role::Arbiter).unwrap().usd - before_arb))
                .abs()
                < 1e-10
        );
        let p = rows(store, kinds::PERSON_REQUEST);
        assert_eq!(p.len(), persons + usize::from(irreversible));
        if irreversible {
            assert_eq!(
                p.last().unwrap(),
                &json!({"id":"d1","kind":"plan_acceptance","reason":"arbiter reason"})
            );
        }
    }
    let server = AppServer::new(fake("fake_codex", &script, &codex_log))
        .with_slots_home(store.join("slots"));
    let calls = lines(&codex_log)
        .iter()
        .filter(|v| v["method"] == "turn/start")
        .count();
    policy::set(store, "budget.co_core", "0").unwrap();
    assert_eq!(
        judge(&server, store, store, &d).unwrap_err().kind,
        "budget_exceeded"
    );
    assert_eq!(
        lines(&codex_log)
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .count(),
        calls
    );
    policy::set(store, "budget.co_core", "none").unwrap();
    policy::set(store, "budget.arbiter", "0").unwrap();
    let fables = lines(&member_log).len();
    assert_eq!(
        judge(&server, store, store, &d).unwrap_err().kind,
        "budget_exceeded"
    );
    assert_eq!(lines(&member_log).len(), fables);
    policy::set(store, "budget.arbiter", "none").unwrap();
    budgets::record(store, Role::Core, "unknown", None, 0, 0, "prior").unwrap();
    budgets::record(store, Role::CoCore, "unknown", None, 0, 0, "prior").unwrap();
    write_script(&script, "Accept", "core");
    let server = AppServer::new(fake("fake_codex", &script, &codex_log))
        .with_slots_home(store.join("slots"));
    assert_eq!(judge(&server, store, store, &d).unwrap(), Verdict::Agree);
    let e = rows(store, kinds::DUAL_CORE);
    assert!(e.last().unwrap()["usd"].is_null());
    assert!(e.last().unwrap()["core_usd"].is_null());
    match previous {
        Some(v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
}

#[test]
fn kind_names() {
    for (kind, name) in [
        (Kind::PlanAcceptance, "plan_acceptance"),
        (Kind::CouncilRuling, "council_ruling"),
        (Kind::ModuleMap, "module_map"),
        (Kind::FixRounds, "fix_rounds"),
        (Kind::ScopeReduction, "scope_reduction"),
        (Kind::GoalClose, "goal_close"),
    ] {
        assert_eq!(kind.name(), name);
    }
}
