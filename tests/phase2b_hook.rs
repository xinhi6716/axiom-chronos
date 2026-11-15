use axiom::{
    engine::{
        budgets::{self, Role},
        policy,
    },
    goal, hook,
    orch::{self, events},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const ORCH: &str = "mcp__plugin_chronos_bridge__orch";

fn setup() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    let g = goal::open(home.path(), "s", "wish", Some(project.to_str().unwrap())).unwrap();
    goal::set_mode(home.path(), "s", "change").unwrap();
    let store = orch::store_for_goal(home.path(), &g).unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    (home, store)
}

fn tool(name: &str, input: Value, agent: bool) -> Value {
    let mut e = json!({"hook_event_name":"PreToolUse", "session_id":"s", "tool_name":name, "tool_input":input});
    if agent {
        e["agent_id"] = json!("a");
    }
    e
}

fn call(home: &Path, event: &Value) -> Option<Value> {
    hook::handle(event, Some(home), false)
}

fn reason(result: Option<Value>) -> Option<String> {
    let out = result?;
    (out["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
        out["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_owned()
    })
}

#[test]
fn interjections_have_receipts_and_distinct_ids() {
    let (home, store) = setup();
    for (n, text) in ["first", "second"].iter().enumerate() {
        let event = json!({"hook_event_name":"UserPromptSubmit", "session_id":"s", "prompt":text});
        let output = call(home.path(), &event).unwrap();
        let context = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        let id = format!("ij-{}", n + 1);
        for part in [&id[..], "amend", "answer", "stop", "replan", "pause_batch"] {
            assert!(context.contains(part), "{context}");
        }
        let rows = events::read_events(&store, 0).unwrap();
        for kind in ["interjection_sent", "interjection_received"] {
            assert!(rows.iter().any(|e| e.kind == kind && e.data["id"] == id));
        }
    }
}

#[test]
fn reviewer_budget_blocks_spent_and_unknown_spend() {
    for usd in [Some(0.5), None] {
        let (home, store) = setup();
        budgets::record(&store, Role::Reviewer, "model", usd, 1, 1, "previous").unwrap();
        for (name, agent) in [("Agent", false), ("Task", true)] {
            let event = tool(name, json!({"subagent_type":"chronos:reviewer"}), agent);
            assert!(reason(call(home.path(), &event)).is_none());
            policy::set(&store, "budget.reviewer", "0.5").unwrap();
            let message = reason(call(home.path(), &event)).expect("budget denial");
            assert!(message.contains("budget exceeded"), "{message}");
            assert!(events::read_events(&store, 0)
                .unwrap()
                .iter()
                .any(|e| e.kind == "budget_exceeded"
                    && e.data["call"] == "agent:chronos:reviewer"
                    && e.data["reason"]
                        == if usd.is_some() {
                            "spent"
                        } else {
                            "unknown_spend"
                        }));
            policy::set(&store, "budget.reviewer", "none").unwrap();
        }
    }
}

#[test]
fn pm_qa_dispatch_and_verdict_are_bound() {
    let (home, store) = setup();
    for name in ["Agent", "Task"] {
        let event = tool(name, json!({"subagent_type":"chronos:pm-qa"}), true);
        assert!(reason(call(home.path(), &event)).is_some());
        policy::set(&store, "pm_qa", "on").unwrap();
        assert!(reason(call(home.path(), &event)).is_none());
        policy::set(&store, "pm_qa", "off").unwrap();
    }
    fs::create_dir_all(store.join("agents")).unwrap();
    fs::write(store.join("agents/a.reviewer"), "u").unwrap();
    for (file, denied) in [
        ("u.pm-qa.verdict.json", false),
        ("u.verdict.json", true),
        ("v.pm-qa.verdict.json", true),
    ] {
        let mut event = tool(
            "Write",
            json!({"file_path":store.join("reports").join(file)}),
            true,
        );
        event["agent_type"] = json!("chronos:pm-qa");
        assert_eq!(
            reason(call(home.path(), &event)).is_some(),
            denied,
            "{file}"
        );
    }
}

#[test]
fn policy_set_is_core_only_with_or_without_json_flag() {
    let (home, _) = setup();
    for args in [
        json!(["policy", "set", "pm_qa", "on"]),
        json!(["--json", "policy", "set", "pm_qa", "on"]),
    ] {
        let input = json!({"args":args});
        let denied = reason(call(home.path(), &tool(ORCH, input.clone(), true))).unwrap();
        assert!(
            denied.contains("person") && denied.contains("core"),
            "{denied}"
        );
        assert!(reason(call(home.path(), &tool(ORCH, input, false))).is_none());
    }
    assert!(reason(call(
        home.path(),
        &tool(ORCH, json!({"args":["policy", "show"]}), true)
    ))
    .is_none());
}
