use axiom::evolve::{
    collect::{collect, dedup},
    Event, Inputs, Source,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
fn put(path: &Path, data: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, p: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        if !p.exists() {
            return;
        }
        let rel = p.strip_prefix(root).unwrap().to_path_buf();
        if p.is_dir() {
            out.insert(rel, None);
            for e in fs::read_dir(p).unwrap() {
                walk(root, &e.unwrap().path(), out);
            }
        } else {
            out.insert(rel, Some(fs::read(p).unwrap()));
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
fn inputs(root: &Path, now_ms: i64) -> Inputs {
    Inputs {
        home: root.join("home"),
        projects: root.join("projects"),
        now_ms,
        since_days: 7,
    }
}

fn event(key: &str, title: &str) -> Event {
    Event {
        problem: "test".into(),
        title: title.into(),
        count: 1,
        cost_usd: None,
        unpriced: 0,
        detail: String::new(),
        snapshot: false,
        source: Source {
            family: "test".into(),
            location: "fixture".into(),
            session: None,
            project: None,
            unit: None,
            ts_ms: None,
            key: key.into(),
        },
    }
}

#[test]
fn dedup_keeps_first_event_per_key_in_order() {
    let first = event("same", "first");
    let other = event("other", "other");
    let duplicate = event("same", "later");
    assert_eq!(
        dedup(vec![first.clone(), other.clone(), duplicate]),
        vec![first, other]
    );
}

#[test]
fn dedup_keeps_keys_differing_in_one_field() {
    // Length-prefixed source-key fields: family, location, suffix.
    let first = event("4:test2:p11:a", "first");
    let second = event("4:test2:p21:a", "second");
    assert_eq!(
        dedup(vec![first.clone(), second.clone()]),
        vec![first, second]
    );
}
#[test]
fn snapshots_failures_gaps_and_read_only_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut input = inputs(tmp.path(), now());
    let home = input.home.clone();
    put(
        &home.join("goals/a.json"),
        &json!({"wish":"x", "status":"blocked", "note":"need key", "reason":"interrupted"})
            .to_string(),
    );
    put(
        &home.join("goals/b.json"),
        &json!({"wish":"x", "status":"waiting", "note":"batch pending"}).to_string(),
    );
    put(&home.join("goals/bad.json"), "{");
    // The window is anchored to the current clock; evidence uses a date in that window.
    let ts = axiom::base::now_iso();
    let evidence = format!("kind\tcommand\texit_code\tcommit\tts\nrun\tbuild\t1\ta\t{ts}\nrun\tbuild\t0\ta\t{ts}\nreview\tmodel\t1\ta\t{ts}\nreview\tmodel\t0\ta\t{ts}\nverify\ttest\t1\ta\tbroken\nrun\told\t1\ta\t2000-01-01T00:00:00Z\nmalformed\nverify\ttest\t1\ta\t{ts}\naccept\tcheck\t1\ta\t{ts}\n");
    put(
        &home.join("orchestrate/p/reports/u.evidence.tsv"),
        &evidence,
    );
    put(
        &home.join("orchestrate/p/reports/u.verdict.json"),
        &json!({"findings":[{"severity":"MAJOR", "evidence":"bad"}]}).to_string(),
    );
    put(&home.join("orchestrate/p/handoffs/module/m.json"), &json!({"module":"m", "ts":ts, "next":"work", "taken":true, "stalled":3, "stuck":"no progress"}).to_string());
    put(
        &home.join("orchestrate/p/handoffs/wish.json"),
        &json!({"module":null, "ts":"invalid", "next":"x"}).to_string(),
    );
    input.now_ms = now();
    let before = tree(&home);
    let result = collect(&input).unwrap();
    assert_eq!(before, tree(&home));
    for kind in [
        "goal.blocked",
        "goal.waiting",
        "orch.run-failed",
        "orch.verify-failed",
        "orch.accept-failed",
        "orch.review-failed",
        "orch.re-review",
        "handoff.taken",
        "handoff.stall/m",
        "review.finding/major",
    ] {
        assert!(
            result.events.iter().any(|e| e.problem == kind),
            "missing {kind}: {result:?}"
        );
    }
    assert_eq!(
        result
            .events
            .iter()
            .filter(|e| e.problem == "orch.run-failed")
            .count(),
        1
    );
    assert_eq!(
        result
            .events
            .iter()
            .find(|e| e.problem == "handoff.stall/m")
            .unwrap()
            .count,
        3
    );
    assert!(result
        .events
        .iter()
        .find(|e| e.problem == "goal.blocked")
        .unwrap()
        .detail
        .contains("need key"));
    assert!(result
        .events
        .iter()
        .find(|e| e.problem == "goal.waiting")
        .unwrap()
        .detail
        .contains("batch pending"));
    assert!(result
        .events
        .iter()
        .filter(|e| e.source.family == "review" || e.source.family == "handoff")
        .all(|e| e.snapshot));
    assert!(result.gaps.iter().any(|g| g.family == "goal"));
    assert!(result
        .gaps
        .iter()
        .any(|g| g.family == "orch" && g.location.ends_with(":6")));
    assert!(result.gaps.iter().any(|g| g.family == "handoff"));
    assert_eq!(
        result
            .sources
            .iter()
            .find(|s| s.family == "hook")
            .unwrap()
            .state,
        "not logged"
    );
    let keys: std::collections::HashSet<_> = result.events.iter().map(|e| &e.source.key).collect();
    assert_eq!(keys.len(), result.events.len());
}
#[test]
fn hook_and_usage_respect_period_and_report_undated() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    let home = &input.home;
    put(
        &home.join("hook/refusals.jsonl"),
        &format!(
            "{}\n{}\n{}\n{{bad\n",
            json!({"ts":"2023-11-14T22:13:20Z", "session":"s", "reason":"denied"}),
            json!({"ts":"2000-01-01T00:00:00Z", "reason":"old"}),
            json!({"session":"s", "reason":"undated"})
        ),
    );
    put(
        &home.join("goals/s.json"),
        &json!({"cwd":"/fixture"}).to_string(),
    );
    let a = json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "message":{"id":"a", "model":"claude-opus-5-5", "content":[], "usage":{"input_tokens":600001,"output_tokens":1}}});
    let b = json!({"type":"assistant", "message":{"id":"b", "model":"unknown", "content":[], "usage":{"input_tokens":10,"output_tokens":1}}});
    put(
        &input.projects.join("p/s.jsonl"),
        &format!("{a}\n{a}\n{b}\n"),
    );
    let result = collect(&input).unwrap();
    let hook: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.problem == "hook.refusal")
        .collect();
    assert_eq!(hook.len(), 1);
    assert!(hook[0].source.location.ends_with(":1"));
    let usage: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.problem == "usage.context-high/main")
        .collect();
    assert_eq!(usage.len(), 1, "{result:?}");
    assert_eq!(usage[0].count, 1);
    assert!(axiom::usage::price::usd("claude-opus-5-5", &Default::default()).is_some());
    assert!(usage[0].cost_usd.is_some());
    assert!(result
        .gaps
        .iter()
        .any(|g| g.family == "usage" && g.reason.contains("undated")));
    assert!(result
        .gaps
        .iter()
        .any(|g| g.family == "hook" && g.reason.contains("timestamp")));
    assert!(result
        .gaps
        .iter()
        .any(|g| g.family == "hook" && g.location.ends_with(":4")));
}
#[test]
fn missing_home_does_not_create_anything() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), now());
    let out = collect(&input).unwrap();
    assert!(out.events.is_empty());
    assert!(out.gaps.is_empty());
    assert!(!input.home.exists());
    assert!(!input.projects.exists());
}
#[test]
fn cache_gap_large_read_and_inclusive_hook_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    put(
        &input.home.join("goals/s.json"),
        &json!({"cwd":"/fixture"}).to_string(),
    );
    let first = json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "message":{"id":"a", "model":"claude-opus-5-5", "content":[{"type":"tool_use", "id":"read1", "name":"Read", "input":{"file_path":"large.txt"}}], "usage":{"input_tokens":1,"output_tokens":1}}});
    let tool = json!({"type":"user", "timestamp":"2023-11-14T22:13:21Z", "message":{"content":[{"type":"tool_result", "tool_use_id":"read1", "content":"x".repeat(50_000)}]}});
    let second = json!({"type":"assistant", "timestamp":"2023-11-14T22:19:20Z", "message":{"id":"b", "model":"claude-opus-5-5", "content":[], "usage":{"input_tokens":1,"cache_creation_input_tokens":50000,"output_tokens":1}}});
    put(
        &input.projects.join("p/s.jsonl"),
        &format!("{first}\n{tool}\n{second}\n"),
    );
    let start = "2023-11-07T22:23:20Z";
    let end = "2023-11-14T22:23:20Z";
    put(
        &input.home.join("hook/refusals.jsonl"),
        &format!(
            "{}\n{}\n{}\n",
            json!({"ts":start,"session":"s","reason":"start"}),
            json!({"ts":end,"session":"s","reason":"end"}),
            json!({"ts":"2023-11-14T22:23:20.001Z","session":"s","reason":"late"})
        ),
    );
    let result = collect(&input).unwrap();
    assert!(axiom::usage::price::usd("claude-opus-5-5", &Default::default()).is_some());
    let cache = result
        .events
        .iter()
        .find(|e| e.problem == "usage.cache-gap/main")
        .unwrap();
    assert_eq!(cache.count, 1);
    assert!(cache.cost_usd.is_some());
    assert!(
        result
            .events
            .iter()
            .any(|e| e.problem == "usage.large-read/main" && e.detail.contains("large.txt")),
        "{result:?}"
    );
    assert_eq!(
        result
            .events
            .iter()
            .filter(|e| e.problem == "hook.refusal")
            .count(),
        2
    );
}
#[test]
fn log_source_uses_init_session_not_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    let init =
        json!({"type":"system", "subtype":"init", "session_id":"session-a", "cwd":"/tmp/example"});
    let call = json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "message":{"id":"a", "model":"unknown-model", "content":[], "usage":{"input_tokens":600001}}});
    let log = input.home.join("logs/unrelated-name.jsonl");
    put(&log, &format!("{init}\n{call}\n"));
    let result = collect(&input).unwrap();
    let event = result
        .events
        .iter()
        .find(|e| e.problem == "usage.context-high/main")
        .unwrap();
    assert_eq!(std::path::Path::new(&event.source.location), log.as_path());
    assert_eq!(event.cost_usd, None);
    assert_eq!(event.unpriced, 1);
}
#[test]
fn old_goal_mtime_is_excluded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut input = inputs(tmp.path(), now());
    let path = input.home.join("goals/old.json");
    put(
        &path,
        &json!({"wish":"x", "status":"blocked", "note":"old"}).to_string(),
    );
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(8 * 86_400))
        .unwrap();
    input.now_ms = now();
    let result = collect(&input).unwrap();
    assert!(!result.events.iter().any(|e| e.problem == "goal.blocked"));
}

#[test]
fn handoff_and_verdict_exclude_old_items_and_include_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let mut input = inputs(tmp.path(), now());
    let handoffs = input.home.join("orchestrate/p/handoffs/module");
    put(&handoffs.join("old.json"), &json!({"module":"old", "ts":"2000-01-01T00:00:00Z", "next":"old", "taken":false, "stalled":2}).to_string());
    put(
        &handoffs.join("new.json"),
        &json!({"module":"new", "ts":axiom::base::now_iso(), "next":"new", "taken":false})
            .to_string(),
    );
    put(&handoffs.join("stalled.json"), &json!({"module":"active", "ts":axiom::base::now_iso(), "next":"work", "taken":true, "stalled":3}).to_string());
    put(
        &input.home.join("orchestrate/p/reports/new.verdict.json"),
        &json!({"findings":[{"severity":"MAJOR", "evidence":"new"}]}).to_string(),
    );
    let verdict = input.home.join("orchestrate/p/reports/old.verdict.json");
    put(
        &verdict,
        &json!({"findings":[{"severity":"OLD", "evidence":"old"}]}).to_string(),
    );
    fs::File::options()
        .write(true)
        .open(verdict)
        .unwrap()
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(8 * 86_400))
        .unwrap();
    input.now_ms = now();
    let result = collect(&input).unwrap();
    let handoffs: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.source.family == "handoff")
        .collect();
    assert_eq!(handoffs.len(), 3, "{result:?}");
    assert!(handoffs
        .iter()
        .any(|e| e.problem == "handoff.pending" && e.snapshot));
    assert!(handoffs.iter().any(|e| e.problem == "handoff.taken"
        && e.source.unit.as_deref() == Some("active")
        && e.snapshot));
    assert!(handoffs
        .iter()
        .any(|e| e.problem == "handoff.stall/active" && e.count == 3 && e.snapshot));
    assert!(!handoffs.iter().any(|e| e.problem == "handoff.stall/old"));
    let review: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.source.family == "review")
        .collect();
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].problem, "review.finding/major");
    assert!(review[0].snapshot);
}

#[test]
fn non_ascii_agent_types_have_distinct_safe_problem_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    put(
        &input.home.join("goals/s.json"),
        &json!({"cwd":"/fixture"}).to_string(),
    );
    put(&input.projects.join("p/s.jsonl"), "");
    for (id, kind) in [("a", "研究"), ("b", "分析")] {
        let agent = input
            .projects
            .join(format!("p/s/subagents/agent-{id}.jsonl"));
        let call = json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "message":{"id":id, "model":"model", "content":[], "usage":{"input_tokens":600001}}});
        put(&agent, &format!("{call}\n"));
        put(
            &agent.with_extension("meta.json"),
            &json!({"agentType":kind}).to_string(),
        );
    }
    let result = collect(&input).unwrap();
    let ids: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.problem.starts_with("usage.context-high/"))
        .map(|e| e.problem.as_str())
        .collect();
    assert_eq!(ids.len(), 2, "{result:?}");
    assert_ne!(ids[0], ids[1]);
    for id in ids {
        let qualifier = id.strip_prefix("usage.context-high/").unwrap();
        assert!(qualifier
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert!(
            !qualifier.contains("2023")
                && !qualifier.contains("session")
                && !qualifier.contains('/')
                && !qualifier.contains('\\')
        );
    }
}

#[test]
fn distinct_usage_source_keys_do_not_collide() {
    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    let init = json!({"type":"system", "subtype":"init", "session_id":"s"});
    // Instance/model boundaries differed despite coinciding under colon joining.
    let call = |id, parent, model| json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "parent_tool_use_id":parent, "message":{"id":id, "model":model, "content":[], "usage":{"input_tokens":600001}}});
    put(
        &input.home.join("logs/s.jsonl"),
        &format!(
            "{init}\n{}\n{}\n",
            call("1", "a", "b:c"),
            call("2", "a:b", "c")
        ),
    );
    let result = collect(&input).unwrap();
    let warnings: Vec<_> = result
        .events
        .iter()
        .filter(|e| e.problem == "usage.context-high/unknown")
        .collect();
    assert_eq!(warnings.len(), 2, "{result:?}");
    assert_ne!(warnings[0].source.key, warnings[1].source.key);
}

#[test]
fn chronos_scope_counts_named_sessions_and_ignores_unrelated_records() {
    use axiom::{
        evolve,
        usage::{self, Locations},
    };
    use std::collections::BTreeSet;

    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    let locations = Locations {
        home: input.home.clone(),
        projects: input.projects.clone(),
    };
    assert!(usage::chronos_sessions(&input.home).unwrap().is_empty());
    assert!(usage::chronos_period(&locations, input.now_ms, 7)
        .unwrap()
        .history
        .is_empty());
    assert_eq!(
        usage::chronos_period(&locations, input.now_ms, 0)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let cwd = "/same/project";
    let dir = input.projects.join(usage::transcript_dir_name(cwd));
    let call = |id: &str, input_tokens: u64| {
        json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "cwd":cwd, "message":{"id":id,"model":"claude-opus-5-5","content":[],"usage":{"input_tokens":input_tokens,"output_tokens":1}}}).to_string()
    };
    put(
        &input.home.join("goals/goal-only.json"),
        &json!({"cwd":cwd}).to_string(),
    );
    // Identity must survive an old goal mtime.
    fs::File::options()
        .write(true)
        .open(input.home.join("goals/goal-only.json"))
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH)
        .unwrap();
    put(&dir.join("goal-only.jsonl"), &call("goal", 10));
    put(
        &dir.join("goal-only/subagents/agent-real.jsonl"),
        &call("sub", 20),
    );
    put(
        &input.home.join("goals/both.json"),
        &json!({"cwd":cwd}).to_string(),
    );
    put(&dir.join("both.jsonl"), &call("both", 30));
    for session in ["log-only", "both"] {
        let init = json!({"type":"system","subtype":"init","session_id":session,"cwd":cwd});
        put(
            &input
                .home
                .join(format!("logs/20231114-221320-wish-{session}.jsonl")),
            &format!("{init}\n{}\n", call(session, 40)),
        );
    }
    put(
        &input.home.join("hook/refusals.jsonl"),
        &format!(
            "{}\n",
            json!({"session":"goal-only","reason":"allowed","ts":"2023-11-14T22:13:20Z"})
        ),
    );
    let expected: BTreeSet<String> = ["both", "goal-only", "log-only"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(usage::chronos_sessions(&input.home).unwrap(), expected);
    let period = usage::chronos_period(&locations, input.now_ms, 7).unwrap();
    assert_eq!(period.history.len(), 3);
    assert_eq!(
        period
            .runs
            .iter()
            .map(|r| r.ledger.totals.calls)
            .sum::<u64>(),
        4
    );
    assert_eq!(
        period
            .runs
            .iter()
            .find(|r| r.session == "goal-only")
            .unwrap()
            .ledger
            .subagent_files,
        1
    );
    assert_eq!(
        period
            .runs
            .iter()
            .find(|r| r.session == "both")
            .unwrap()
            .ledger
            .totals
            .calls,
        1
    );
    let before_tree = tree(&input.home);
    let before = evolve::preview(&input).unwrap();
    assert_eq!(tree(&input.home), before_tree);
    assert_eq!(before.digest.estimate.samples, 2);
    assert!(before.digest.estimate.median_usd.is_some());
    assert_eq!(
        before.digest.cost.iter().map(|row| row.calls).sum::<u64>(),
        4
    );
    assert_eq!(
        before
            .digest
            .sources
            .iter()
            .find(|s| s.family == "usage")
            .unwrap()
            .samples,
        3
    );
    assert!(before
        .digest
        .problems
        .iter()
        .any(|p| p.id == "hook.refusal"));
    put(
        &dir.join("unrelated.jsonl"),
        &format!("{}\n{{bad\n", call("expensive", 900_000)),
    );
    put(
        &dir.join("unrelated/subagents/agent-expensive.jsonl"),
        &call("expensive-sub", 900_000),
    );
    let hook = input.home.join("hook/refusals.jsonl");
    let mut hook_text = fs::read_to_string(&hook).unwrap();
    hook_text.push_str(&format!(
        "{}\n{}\n",
        json!({"session":"unrelated","reason":"ignored","ts":"2023-11-14T22:13:20Z"}),
        json!({"reason":"no session"})
    ));
    put(&hook, &hook_text);
    let after_tree = tree(&input.home);
    let after = evolve::preview(&input).unwrap();
    assert_eq!(tree(&input.home), after_tree);
    assert_eq!(before, after);
    let collected = collect(&input).unwrap();
    assert_eq!(
        collected
            .events
            .iter()
            .filter(|e| e.problem == "hook.refusal")
            .count(),
        1
    );
    assert!(!collected.gaps.iter().any(|g| g.family == "hook"));
    assert_eq!(
        usage::period(&locations, input.now_ms, 7)
            .unwrap()
            .history
            .len(),
        4
    );
}

#[test]
fn chronos_identities_require_goal_files() {
    use axiom::usage::{self, Locations};

    let tmp = tempfile::tempdir().unwrap();
    let input = inputs(tmp.path(), 1_700_000_600_000);
    fs::create_dir_all(input.home.join("goals/not-a-goal.json")).unwrap();
    let call = json!({"type":"assistant", "timestamp":"2023-11-14T22:13:20Z", "message":{"id":"a", "model":"claude-opus-5-5", "content":[], "usage":{"input_tokens":600001,"output_tokens":1}}});
    put(
        &input.projects.join("p/not-a-goal.jsonl"),
        &format!("{call}\n"),
    );
    let locations = Locations {
        home: input.home,
        projects: input.projects,
    };
    assert!(usage::chronos_sessions(&locations.home).unwrap().is_empty());
    assert!(usage::chronos_period(&locations, input.now_ms, 7)
        .unwrap()
        .history
        .is_empty());
    assert_eq!(
        usage::period(&locations, input.now_ms, 7)
            .unwrap()
            .history
            .len(),
        1
    );
}
