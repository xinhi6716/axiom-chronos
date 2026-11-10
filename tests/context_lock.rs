use axiom::{goal, hook, hook::context};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

use context::ORCH;
const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";

struct Run {
    _dir: tempfile::TempDir,
    home: PathBuf,
    main: PathBuf,
    own: PathBuf,
}

fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    goal::open(&home, "s", "wish", None).unwrap();
    goal::set_mode(&home, "s", "change").unwrap();
    let main = dir.path().join("projects/sess.jsonl");
    let own = dir.path().join("projects/sess/subagents/agent-d1.jsonl");
    fs::create_dir_all(own.parent().unwrap()).unwrap();
    fs::write(&main, assistant(900_000)).unwrap();
    Run {
        _dir: dir,
        home,
        main,
        own,
    }
}

fn assistant(context: u64) -> String {
    format!(
        "{}\n",
        json!({"type":"assistant","message":{"id":format!("msg_{context}"),"model":"claude-sonnet-4-5","usage":{"input_tokens":10,"cache_creation_input_tokens":90,"cache_read_input_tokens":context - 100,"output_tokens":7}}})
    )
}
fn transcript(path: &Path, contexts: &[u64]) {
    fs::write(
        path,
        contexts.iter().map(|c| assistant(*c)).collect::<String>(),
    )
    .unwrap();
}
fn tool(r: &Run, name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":name,"tool_input":input,"transcript_path":r.main})
}
fn in_dispatcher(mut e: Value) -> Value {
    e["agent_id"] = json!("d1");
    e["agent_type"] = json!("chronos:dispatcher");
    e
}
fn refusal(r: &Run, e: &Value) -> Option<String> {
    hook::handle(e, Some(&r.home), false).and_then(|v| {
        (v["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
            v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .to_string()
        })
    })
}

fn cases() -> Vec<(&'static str, Value, bool)> {
    let mut cases = vec![
        ("Read", json!({"file_path":"missing"}), false),
        ("Grep", json!({}), false),
        ("Glob", json!({}), false),
        ("WebFetch", json!({}), false),
        ("Bash", json!({"command":"pwd"}), false),
        ("PowerShell", json!({"command":"Get-Location"}), false),
        ("Write", json!({"file_path":"project.txt"}), false),
        ("Edit", json!({"file_path":"project.txt"}), false),
        ("Agent", json!({"subagent_type":"chronos:reviewer"}), false),
        ("Task", json!({"subagent_type":"chronos:reviewer"}), false),
        (CODEX, json!({"sandbox":"read-only"}), false),
        (
            "mcp__plugin_chronos_bridge__memory_search",
            json!({}),
            false,
        ),
        ("unknown_tool", json!({}), false),
        ("orch", json!({"args":["handoff"]}), false),
        (ORCH, json!({"args":["unit", "add", "x"]}), false),
        (ORCH, json!({}), false),
        (ORCH, json!({"args":"handoff"}), false),
        (ORCH, json!({"args":[]}), false),
        (ORCH, json!({"args":[42]}), false),
    ];
    for action in ["handoff", "report", "status", "resume"] {
        cases.push((ORCH, json!({"args":[action]}), true));
    }
    cases
}

#[test]
fn dispatcher_lock_applies_to_every_tool_and_only_past_the_limit() {
    let r = run();
    transcript(&r.own, &[30_000, 150_000]);
    let mut failures = Vec::new();
    for (name, input, allowed) in cases() {
        let event = in_dispatcher(tool(&r, name, input.clone()));
        let why = refusal(&r, &event);
        if allowed {
            if why.is_some() {
                failures.push(format!("{name} {input}: {why:?}"));
            }
        } else {
            if !why.as_deref().is_some_and(|s| s.contains("handoff")) {
                failures.push(format!("{name} {input}: {why:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // Compare the complete hook output, including unrelated denials, against the
    // below-limit result for each caller and input.
    for (name, input, _) in cases() {
        let core = tool(&r, name, input);
        let dispatcher = in_dispatcher(core.clone());
        let mut reviewer = dispatcher.clone();
        reviewer["agent_type"] = json!("chronos:reviewer");
        transcript(&r.own, &[30_000, 149_999]);
        let baseline = [core.clone(), reviewer.clone(), dispatcher.clone()]
            .map(|e| hook::handle(&e, Some(&r.home), false));
        assert!(
            !refusal(&r, &dispatcher).is_some_and(|reason| reason.contains("handoff")),
            "below limit {name}"
        );
        transcript(&r.own, &[30_000, 160_000]);
        assert_eq!(
            hook::handle(&core, Some(&r.home), false),
            baseline[0],
            "core {name}"
        );
        assert_eq!(
            hook::handle(&reviewer, Some(&r.home), false),
            baseline[1],
            "reviewer {name}"
        );
        transcript(&r.own, &[75_000, 160_000]);
        assert_eq!(
            hook::handle(&dispatcher, Some(&r.home), false),
            baseline[2],
            "near-limit start {name}"
        );
        fs::write(&r.own, "not a transcript\n").unwrap();
        assert_eq!(
            hook::handle(&dispatcher, Some(&r.home), false),
            baseline[2],
            "unreadable {name}"
        );
        fs::remove_file(&r.own).unwrap();
        assert_eq!(
            hook::handle(&dispatcher, Some(&r.home), false),
            baseline[2],
            "missing {name}"
        );
    }

    let previous = std::env::var_os(context::HANDOFF_ENV);
    std::env::set_var(context::HANDOFF_ENV, "0");
    transcript(&r.own, &[30_000, 160_000]);
    let disabled: Vec<_> = cases()
        .into_iter()
        .map(|(name, input, _)| {
            let event = in_dispatcher(tool(&r, name, input));
            (event.clone(), hook::handle(&event, Some(&r.home), false))
        })
        .collect();
    if let Some(value) = previous {
        std::env::set_var(context::HANDOFF_ENV, value);
    } else {
        std::env::remove_var(context::HANDOFF_ENV);
    }
    transcript(&r.own, &[30_000, 149_999]);
    for (event, result) in disabled {
        assert_eq!(
            result,
            hook::handle(&event, Some(&r.home), false),
            "disabled {event}"
        );
    }
}

#[test]
fn lock_allows_only_the_four_orch_actions() {
    for action in context::LOCKED_ALLOWED {
        assert!(context::lock_allows(
            ORCH,
            &json!({"args":[action, "extra"]})
        ));
        assert!(!context::lock_allows("orch", &json!({"args":[action]})));
        assert!(!context::lock_allows("other", &json!({"args":[action]})));
    }
    for input in [
        json!({}),
        json!({"args":null}),
        json!({"args":"handoff"}),
        json!({"args":[]}),
        json!({"args":[1]}),
        json!({"args":["unit", "add"]}),
        json!({"args":["HANDOFF"]}),
    ] {
        assert!(!context::lock_allows(ORCH, &input), "{input}");
    }
}

#[test]
fn pre_tool_use_routes_every_tool() {
    let hooks: Value = serde_json::from_str(
        axiom::engine::FILES
            .iter()
            .find(|(path, _)| *path == "hooks/hooks.json")
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "*");
}

#[test]
fn newly_routed_tools_do_not_fail_closed_on_an_unreadable_goal() {
    let r = run();
    fs::write(r.home.join("goals/s.json"), "not json").unwrap();
    for name in ["Grep", "Glob", "WebFetch", ORCH, "unknown_tool"] {
        let event = in_dispatcher(tool(&r, name, json!({})));
        assert_eq!(hook::handle(&event, Some(&r.home), false), None, "{name}");
    }
}
