mod common;
use axiom::{
    goal, hook,
    orch::{self, events, lifecycle::scheduler, Evidence},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn module_with_unit(root: &Path, map: &str, track: &str) -> PathBuf {
    let store = root.join("orchestrate/demo");
    fs::create_dir_all(store.join("reports")).unwrap();
    let brief = store.join("u.brief");
    fs::write(&brief, "GOAL x\nSCOPE x\nVERIFY echo ok\nRISK trivial\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\t{track}\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(store.join("modules.md"), map).unwrap();
    store
}

fn write_rows(store: &Path, rows: &[(&str, &str)]) {
    let mut text = String::from("kind\tcommand\texit_code\tcommit\tts\n");
    for (kind, commit) in rows {
        let command = match *kind {
            "run" => "terra",
            "review" => "chronos:reviewer",
            other => other,
        };
        text.push_str(&format!("{kind}\t{command}\t0\t{commit}\t2026-01-01T00:00:00Z\n"));
    }
    fs::write(store.join("reports/u.evidence.tsv"), text).unwrap();
}

fn gate(rows: &[(&str, &str)]) -> bool {
    let temp = common::temp();
    let store = module_with_unit(
        temp.path(),
        "## m\nPURPOSE x\nPATHS x\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        "m",
    );
    for (kind, commit) in rows {
        orch::append_evidence(
            &store,
            "u",
            &Evidence {
                kind: kind.to_string(),
                command: match *kind {
                    "run" => "terra",
                    "review" => "chronos:reviewer",
                    other => other,
                }
                .into(),
                exit_code: 0,
                commit: commit.to_string(),
                ts: "2026-01-01T00:00:00Z".into(),
            },
        )
        .unwrap();
    }
    scheduler::module_verified(&store, "m").unwrap()
}

#[test]
fn module_gate_counts_accept_and_review_on_the_latest_snapshot_in_either_order() {
    let accept_then_review = [
        ("run", "tree:1"),
        ("verify", "tree:1"),
        ("accept", "tree:1"),
        ("review", "tree:1"),
    ];
    let review_then_accept = [
        ("run", "tree:1"),
        ("verify", "tree:1"),
        ("review", "tree:1"),
        ("accept", "tree:1"),
    ];
    let accept_rerun_after_review = [
        ("run", "tree:1"),
        ("verify", "tree:1"),
        ("accept", "tree:1"),
        ("review", "tree:1"),
        ("accept", "tree:1"),
        ("accept", "tree:1"),
    ];
    assert!(gate(&accept_then_review));
    assert!(gate(&review_then_accept));
    assert!(gate(&accept_rerun_after_review));
}

#[test]
fn module_gate_ignores_a_review_of_an_older_snapshot() {
    let review_before_an_accept_that_saw_changed_files = [
        ("run", "tree:1"),
        ("verify", "tree:1"),
        ("accept", "tree:1"),
        ("review", "tree:1"),
        ("accept", "tree:2"),
    ];
    assert!(!gate(&review_before_an_accept_that_saw_changed_files));
    let review_of_files_before_the_last_run = [
        ("run", "tree:1"),
        ("verify", "tree:1"),
        ("accept", "tree:1"),
        ("review", "tree:1"),
        ("run", "tree:2"),
        ("verify", "tree:2"),
        ("accept", "tree:2"),
    ];
    assert!(!gate(&review_of_files_before_the_last_run));
}

const DEPENDENT_MAP: &str = "## a\nPURPOSE x\nPATHS a/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## b\nPURPOSE x\nPATHS b/\nDEPENDS a\nCONTRACT x\nACCEPT test `echo ok`\n";
const A_VERIFIED: [(&str, &str); 4] = [
    ("run", "tree:1"),
    ("verify", "tree:1"),
    ("accept", "tree:1"),
    ("review", "tree:1"),
];
const A_REOPENED: [(&str, &str); 5] = [
    ("run", "tree:1"),
    ("verify", "tree:1"),
    ("accept", "tree:1"),
    ("review", "tree:1"),
    ("run", "tree:2"),
];
const A_VERIFIED_AGAIN: [(&str, &str); 8] = [
    ("run", "tree:1"),
    ("verify", "tree:1"),
    ("accept", "tree:1"),
    ("review", "tree:1"),
    ("run", "tree:2"),
    ("verify", "tree:2"),
    ("accept", "tree:2"),
    ("review", "tree:2"),
];

#[test]
fn module_released_while_waiting_is_auto_claimed_once_its_depends_are_verified() {
    let temp = common::temp();
    let store = module_with_unit(temp.path(), DEPENDENT_MAP, "a");
    write_rows(&store, &A_VERIFIED);
    scheduler::claim(&store, "b").unwrap();
    write_rows(&store, &A_REOPENED);
    scheduler::release(&store, "b", "stale claim").unwrap();

    let waiting = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert!(!waiting.start.contains(&"b".to_string()), "{waiting:?}");
    assert!(waiting.stuck.is_empty(), "{waiting:?}");
    assert!(
        waiting.waiting.iter().any(|w| w.module == "b" && w.on == ["a"]),
        "{waiting:?}"
    );

    write_rows(&store, &A_VERIFIED_AGAIN);
    let claimed = scheduler::next(&store, Some(waiting.cursor), Duration::ZERO).unwrap();
    assert_eq!(claimed.start, vec!["b"], "{claimed:?}");
    assert!(claimed.stuck.is_empty(), "{claimed:?}");
    assert_eq!(scheduler::claimed_modules(&store).unwrap(), vec!["b"]);
}

#[test]
fn module_released_while_ready_stays_stuck_for_the_core() {
    let temp = common::temp();
    let store = module_with_unit(temp.path(), DEPENDENT_MAP, "a");
    write_rows(&store, &A_VERIFIED);
    scheduler::claim(&store, "b").unwrap();
    scheduler::release(&store, "b", "dispatcher stopped without module_done").unwrap();
    for cursor in [None, Some(0)] {
        let reply = scheduler::next(&store, cursor, Duration::ZERO).unwrap();
        assert!(reply.start.is_empty(), "{reply:?}");
        assert_eq!(reply.stuck.len(), 1, "{reply:?}");
        assert_eq!(reply.stuck[0].module, "b");
        assert_eq!(reply.stuck[0].reason, "released");
        assert_eq!(reply.stuck[0].claim, "schedule claim b");
    }
    assert!(scheduler::claimed_modules(&store).unwrap().is_empty());
}

#[test]
fn release_without_a_ready_flag_from_an_older_engine_is_auto_claimed() {
    let temp = common::temp();
    let store = module_with_unit(temp.path(), DEPENDENT_MAP, "a");
    write_rows(&store, &A_VERIFIED);
    events::append_event(&store, events::MODULE_VERIFIED, Some("a"), None, json!({})).unwrap();
    for kind in [events::MODULE_CLAIMED, events::MODULE_RELEASED] {
        events::append_event(&store, kind, Some("b"), None, json!({"reason":"stale"})).unwrap();
    }
    let reply = scheduler::next(&store, None, Duration::ZERO).unwrap();
    assert_eq!(reply.start, vec!["b"], "{reply:?}");
    assert!(reply.stuck.is_empty(), "{reply:?}");
}

const STRING_FLAG: &str = "run_in_background must be the JSON boolean true (no quotes); the string runs the agent in the foreground. Send the call again with the boolean.";
const NON_BOOLEAN_FLAG: &str = "run_in_background must be the JSON boolean true or false. Send the call again with the boolean.";

#[test]
fn hook_denies_a_string_background_flag_and_passes_the_boolean_to_the_claim_check() {
    let temp = common::temp();
    let home = temp.path();
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let goal = goal::open(home, "s", "test", Some(repo.to_str().unwrap())).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
    let store = orch::store_for(home, goal.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## fast\nPURPOSE x\nPATHS fast/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## slow\nPURPOSE x\nPATHS slow/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    scheduler::claim(&store, "fast").unwrap();
    let event = |tool: &str, subagent: &str, prompt: &str, background: Value| {
        json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":tool,"tool_input":{"subagent_type":subagent,"prompt":prompt,"run_in_background":background}})
    };
    let reason = |event: &Value, home: Option<&Path>| {
        hook::handle(event, home, false)
            .filter(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny")
            .map(|v| v["hookSpecificOutput"]["permissionDecisionReason"].clone())
    };
    for tool in ["Agent", "Task"] {
        for flag in [json!("true"), json!("True"), json!("false")] {
            assert_eq!(
                reason(&event(tool, "chronos:dispatcher", "Build fast", flag.clone()), Some(home)),
                Some(json!(STRING_FLAG)),
                "{tool} {flag}"
            );
        }
        assert_eq!(
            reason(&event(tool, "general-purpose", "x", json!("true")), None),
            Some(json!(STRING_FLAG)),
            "{tool}"
        );
        for flag in [json!(1), json!(null), json!({}), json!([true])] {
            assert_eq!(
                reason(&event(tool, "chronos:dispatcher", "Build fast", flag.clone()), Some(home)),
                Some(json!(NON_BOOLEAN_FLAG)),
                "{tool} {flag}"
            );
        }
        assert_eq!(
            reason(&event(tool, "chronos:dispatcher", "Build fast", json!(true)), Some(home)),
            None,
            "{tool}"
        );
        let unclaimed = reason(&event(tool, "chronos:dispatcher", "Build slow", json!(true)), Some(home));
        assert!(
            unclaimed.as_ref().and_then(Value::as_str).is_some_and(|r| r.contains("in the foreground") && r != STRING_FLAG),
            "{tool} {unclaimed:?}"
        );
    }
}
