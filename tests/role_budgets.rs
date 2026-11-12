mod common;

use axiom::{
    codex::AppServer,
    engine::{
        budgets::{self, Role, Spend},
        calls::{self, EngineCall},
        kinds, policy,
    },
    orch::events,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fake(name: &str, script: &Path, log: &Path) -> Vec<String> {
    let mut command = common::fake(name);
    command.push(format!("--fake-script={}", script.display()));
    command.push(format!("--fake-log={}", log.display()));
    command
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn call(role: Role, cwd: &Path) -> EngineCall<'_> {
    EngineCall {
        role,
        case: "test",
        pack: "evidence".into(),
        cwd,
        schema: json!({"type":"object", "required":["answer"]}),
        timeout_s: 10,
    }
}

fn script(path: &Path) {
    fs::write(
        path,
        json!([{"match":"AXIOM-ENGINE", "reply":{"answer":"ok"}}]).to_string(),
    )
    .unwrap();
}

fn exceeded(store: &Path, expected: Value) {
    let rows: Vec<_> = events::read_events(store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == kinds::BUDGET_EXCEEDED)
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].data, expected);
}

// Only this test changes AXIOM_CLAUDE_CMD in this integration-test process.
#[test]
fn spent_budget_blocks_each_role_before_the_call() {
    let old_command = std::env::var_os("AXIOM_CLAUDE_CMD");
    for role in [
        Role::Core,
        Role::Dispatcher,
        Role::Reviewer,
        Role::Verifier,
        Role::Adjudicator,
        Role::CouncilMember,
        Role::CoCore,
        Role::Arbiter,
    ] {
        let temp = common::temp();
        let store = temp.path();
        let script_path = store.join("script.json");
        let codex_log = store.join("codex.jsonl");
        let member_log = store.join("member.jsonl");
        script(&script_path);
        std::env::set_var(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&fake("fake_member", &script_path, &member_log)).unwrap(),
        );
        let server = AppServer::new(fake("fake_codex", &script_path, &codex_log))
            .with_slots_home(store.join("slots"));
        policy::set(store, &format!("budget.{}", role.name()), "0.5").unwrap();
        budgets::record(store, role, "model", Some(0.5), 1, 1, "previous").unwrap();
        let c = call(role, store);
        let invoke = || match role {
            Role::Core => budgets::check(store, role, c.case).map(|()| Value::Null),
            Role::Dispatcher | Role::Verifier => calls::codex_low(&server, store, &c),
            Role::Reviewer | Role::CouncilMember => {
                calls::claude_quick(store, &c, "claude-experimental-x")
            }
            Role::Adjudicator | Role::CoCore | Role::Arbiter => {
                calls::astra_max(&server, store, &c)
            }
        };
        assert_eq!(
            invoke().unwrap_err().kind,
            "budget_exceeded",
            "{}",
            role.name()
        );
        assert!(lines(&codex_log).is_empty(), "{}", role.name());
        assert!(lines(&member_log).is_empty(), "{}", role.name());
        exceeded(
            store,
            json!({"role":role.name(), "limit_usd":0.5, "spent_usd":0.5, "reason":"spent", "call":"test"}),
        );

        // Positive control: the same invocation reaches its fake without a limit.
        policy::set(store, &format!("budget.{}", role.name()), "none").unwrap();
        if role == Role::Core {
            assert!(invoke().is_ok());
        } else {
            assert_eq!(invoke().unwrap(), json!({"answer":"ok"}));
            if matches!(role, Role::Reviewer | Role::CouncilMember) {
                assert_eq!(lines(&member_log).len(), 1);
            } else {
                assert_eq!(
                    lines(&codex_log)
                        .iter()
                        .filter(|v| v["method"] == "turn/start")
                        .count(),
                    1
                );
            }
        }
    }
    match old_command {
        Some(value) => std::env::set_var("AXIOM_CLAUDE_CMD", value),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
}

#[test]
fn unknown_spend_blocks_only_when_budget_is_set() {
    let temp = common::temp();
    let store = temp.path();
    let script_path = store.join("script.json");
    let log = store.join("codex.jsonl");
    script(&script_path);
    let server =
        AppServer::new(fake("fake_codex", &script_path, &log)).with_slots_home(store.join("slots"));
    let c = call(Role::Verifier, store);
    budgets::record(store, Role::Verifier, "unpriced", None, 1, 1, "previous").unwrap();
    policy::set(store, "budget.verifier", "2").unwrap();
    assert_eq!(
        calls::astra_max(&server, store, &c).unwrap_err().kind,
        "budget_exceeded"
    );
    assert!(lines(&log).is_empty());
    exceeded(
        store,
        json!({"role":"verifier", "limit_usd":2.0, "spent_usd":null, "reason":"unknown_spend", "call":"test"}),
    );
    policy::set(store, "budget.verifier", "none").unwrap();
    assert_eq!(
        calls::astra_max(&server, store, &c).unwrap(),
        json!({"answer":"ok"})
    );
    assert_eq!(
        lines(&log)
            .iter()
            .filter(|v| v["method"] == "turn/start")
            .count(),
        1
    );
}

#[test]
fn invalid_policies_are_rejected_and_missing_policy_defaults() {
    let temp = common::temp();
    let store = temp.path();
    assert_eq!(policy::load(store).unwrap(), policy::Policy::default());
    for (key, value) in [
        ("budget.core", "-1"),
        ("budget.core", "NaN"),
        ("budget.unknown", "1"),
        ("reviewer", "unknown"),
        ("unknown", "on"),
    ] {
        assert_eq!(
            policy::set(store, key, value).unwrap_err().kind,
            "invalid_argument",
            "{key}={value}"
        );
    }
    for invalid in [
        r#"{"budgets":{"core":-1}}"#,
        r#"{"budgets":{"core":"NaN"}}"#,
        r#"{"budgets":{"unknown":1}}"#,
        r#"{"reviewer":"unknown"}"#,
        r#"{"unknown":true}"#,
    ] {
        fs::write(store.join("policy.json"), invalid).unwrap();
        assert_eq!(
            policy::load(store).unwrap_err().kind,
            "invalid_argument",
            "{invalid}"
        );
    }
}

#[test]
fn routing_policy_preserves_read_only_and_role_mapping() {
    let temp = common::temp();
    let store = temp.path();
    let script_path = store.join("script.json");
    let log = store.join("codex.jsonl");
    script(&script_path);
    policy::set(store, "reviewer", "astra-max").unwrap();
    policy::set(store, "budget.reviewer", "1").unwrap();
    policy::set(store, "budget.core", "1").unwrap();
    assert_eq!(
        policy::load(store).unwrap().reviewer,
        policy::ReviewerMode::AstraMax
    );
    let server =
        AppServer::new(fake("fake_codex", &script_path, &log)).with_slots_home(store.join("slots"));
    assert_eq!(
        calls::astra_max(&server, store, &call(Role::Reviewer, store)).unwrap(),
        json!({"answer":"ok"})
    );
    let log = lines(&log);
    let starts: Vec<_> = log
        .iter()
        .filter(|v| v["method"] == "thread/start")
        .collect();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["params"]["sandbox"], "read-only");
    for (name, role) in [
        ("terra", Role::Dispatcher),
        ("sol", Role::Dispatcher),
        ("astra", Role::Core),
    ] {
        assert_eq!(budgets::role_for_codex(name), role);
    }
    for (name, role) in [
        ("chronos:reviewer", Some(Role::Reviewer)),
        ("chronos:pm-qa", Some(Role::Reviewer)),
        ("chronos:dispatcher", Some(Role::Dispatcher)),
        ("chronos:architect", Some(Role::Core)),
        ("chronos:other", Some(Role::Core)),
        ("other:reviewer", None),
        ("reviewer", None),
    ] {
        assert_eq!(budgets::role_for_agent(name), role, "{name}");
    }
}

#[test]
fn spent_sums_only_the_requested_roles_events() {
    let temp = common::temp();
    let store = temp.path();
    budgets::record(store, Role::Reviewer, "a", Some(0.25), 1, 2, "first").unwrap();
    budgets::record(store, Role::Core, "b", None, 3, 4, "other").unwrap();
    budgets::record(store, Role::Reviewer, "c", Some(0.5), 5, 6, "second").unwrap();
    events::append_event(
        store,
        "other",
        None,
        None,
        json!({"role":"reviewer", "usd":100}),
    )
    .unwrap();
    assert_eq!(
        budgets::spent(store, Role::Reviewer).unwrap(),
        Spend {
            usd: 0.75,
            unknown: false
        }
    );
    assert_eq!(
        budgets::spent(store, Role::Core).unwrap(),
        Spend {
            usd: 0.0,
            unknown: true
        }
    );
    assert_eq!(
        budgets::spent(store, Role::Arbiter).unwrap(),
        Spend {
            usd: 0.0,
            unknown: false
        }
    );
}
