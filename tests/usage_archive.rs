use axiom::{
    evolve::{self, Inputs},
    storage::archive,
    usage::{self, Locations, DAY_MS},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const NOW: i64 = 1_767_830_400_000;
const SESSION: &str = "session-a";

fn put(path: &Path, data: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}
fn call(id: &str, timestamp: &str, model: &str, input: u64) -> String {
    json!({"type":"assistant", "timestamp":timestamp, "message":{"id":id,"model":model,
        "usage":{"input_tokens":input,"output_tokens":2},"content":[]}})
    .to_string()
        + "\n"
}
fn locations(root: &Path) -> Locations {
    Locations {
        home: root.join("home"),
        projects: root.join("projects"),
    }
}
fn inputs(loc: &Locations) -> Inputs {
    Inputs {
        home: loc.home.clone(),
        projects: loc.projects.clone(),
        now_ms: NOW,
        since_days: 7,
    }
}
fn totals(loc: &Locations) -> (Value, Value, Value) {
    let ledger = usage::ledger(loc, Some(SESSION)).unwrap();
    let period = usage::period(loc, NOW, 7).unwrap();
    let preview = evolve::preview(&inputs(loc)).unwrap();
    (
        serde_json::to_value(&ledger).unwrap(),
        serde_json::to_value(&period).unwrap(),
        serde_json::to_value(&preview.digest.cost).unwrap(),
    )
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    fn visit(dir: &Path, out: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
        if !dir.exists() {
            return;
        }
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out);
            } else {
                out.insert(
                    path.clone(),
                    (
                        fs::read(&path).unwrap(),
                        fs::metadata(&path).unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, &mut result);
    result
}
fn setup(loc: &Locations) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let log = loc.home.join("logs/20260101-000000-wish.jsonl");
    put(
        &log,
        &format!(
            "{}\n",
            json!({"type":"system","subtype":"init","session_id":SESSION,"cwd":"E:\\project"})
        ),
    );
    let goal = loc.home.join(format!("goals/{SESSION}.json"));
    put(
        &goal,
        &json!({"wish":"test","cwd":"E:\\project","status":"done"}).to_string(),
    );
    let transcript = loc.projects.join(format!("E--project/{SESSION}.jsonl"));
    put(
        &transcript,
        &(call("outside", "2025-12-31T23:59:59.999Z", "claude-opus-5-5", 7)
            + &call("inside", "2026-01-01T00:00:00.001Z", "future-model", 13)),
    );
    let agent = loc
        .projects
        .join(format!("E--project/{SESSION}/subagents/agent-sub.jsonl"));
    put(
        &agent,
        &call("sub", "2026-01-02T00:00:00Z", "claude-opus-5-5", 23),
    );
    let meta = agent.with_extension("meta.json");
    put(
        &meta,
        &json!({"agentType":"researcher","description":"research task"}).to_string(),
    );
    (log, goal, transcript, agent, meta)
}

#[test]
fn archived_sources_match_plain_without_mutating_files() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let (log, goal, transcript, agent, meta) = setup(&loc);
    let before = totals(&loc);
    assert_eq!(before.0["totals"]["calls"], 3);
    assert_eq!(before.0["totals"]["usd"], Value::Null);
    assert_eq!(before.1["runs"][0]["ledger"]["totals"]["calls"], 2);
    assert_eq!(before.1["runs"][0]["first_ms"], NOW - 7 * DAY_MS + 1);
    assert!(before.0["by_agent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["agent_type"] == "researcher"));
    assert!(!before.2.as_array().unwrap().is_empty());
    fs::File::options()
        .write(true)
        .open(&transcript)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(86400 * 365))
        .unwrap();
    for path in [&log, &goal, &transcript, &agent, &meta] {
        archive::compress(path).unwrap();
    }
    let files = snapshot(temp.path());
    assert_eq!(before, totals(&loc));
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn plain_wins_over_different_archives_and_is_counted_once() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let (log, goal, transcript, agent, meta) = setup(&loc);
    let expected = totals(&loc);
    for path in [&log, &goal, &transcript, &agent, &meta] {
        let plain = fs::read(path).unwrap();
        archive::compress(path).unwrap();
        put(path, std::str::from_utf8(&plain).unwrap());
    }
    // Replace the archive with valid but different contents; the plain file must still win.
    let other = transcript.with_file_name("other.jsonl");
    put(
        &other,
        &call("other", "2026-01-02T00:00:00Z", "claude-opus-5-5", 999),
    );
    archive::compress(&other).unwrap();
    fs::copy(
        archive::archived_path(&other),
        archive::archived_path(&transcript),
    )
    .unwrap();
    fs::remove_file(archive::archived_path(&other)).unwrap();
    let files = snapshot(temp.path());
    assert_eq!(expected, totals(&loc));
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn corrupt_archive_does_not_hide_good_sessions_or_subagents() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let (_, _, _, agent, _) = setup(&loc);
    put(
        &archive::archived_path(&loc.projects.join("E--project/bad.jsonl")),
        "not gzip",
    );
    put(
        &archive::archived_path(&agent.with_file_name("agent-bad.jsonl")),
        "not gzip",
    );
    let files = snapshot(temp.path());
    let ledger = usage::ledger(&loc, Some(SESSION)).unwrap();
    assert_eq!(ledger.totals.calls, 3);
    assert_eq!(ledger.totals.usd, None);
    let period = usage::period(&loc, NOW, 7).unwrap();
    assert_eq!(
        period
            .runs
            .iter()
            .find(|r| r.session == SESSION)
            .unwrap()
            .ledger
            .totals
            .calls,
        2
    );
    assert!(period.skipped_lines > 0);
    let preview = evolve::preview(&inputs(&loc)).unwrap();
    assert!(!preview.digest.cost.is_empty());
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn archived_log_only_and_goal_only_sessions_remain_discoverable() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let log = loc.home.join("logs/20260101-000000-wish.jsonl");
    put(
        &log,
        &(format!(
            "{}\n",
            json!({"type":"system","subtype":"init","session_id":SESSION})
        ) + &call("log-call", "2026-01-02T00:00:00Z", "future-model", 10)),
    );
    let goal = loc.home.join("goals/goal-only.json");
    put(
        &goal,
        &json!({"wish":"test","cwd":"E:\\project","status":"done"}).to_string(),
    );
    let transcript = loc.projects.join("E--project/goal-only.jsonl");
    put(
        &transcript,
        &call("goal-call", "2026-01-02T00:00:00Z", "claude-opus-5-5", 20),
    );
    let before_log = serde_json::to_value(usage::ledger(&loc, Some(SESSION)).unwrap()).unwrap();
    let before_goal =
        serde_json::to_value(usage::ledger(&loc, Some("goal-only")).unwrap()).unwrap();
    let before_period = serde_json::to_value(usage::chronos_period(&loc, NOW, 7).unwrap()).unwrap();
    let before_cost =
        serde_json::to_value(evolve::preview(&inputs(&loc)).unwrap().digest.cost).unwrap();
    for path in [&log, &goal, &transcript] {
        archive::compress(path).unwrap();
    }
    let files = snapshot(temp.path());
    assert_eq!(
        before_log,
        serde_json::to_value(usage::ledger(&loc, Some(SESSION)).unwrap()).unwrap()
    );
    assert_eq!(
        before_goal,
        serde_json::to_value(usage::ledger(&loc, Some("goal-only")).unwrap()).unwrap()
    );
    assert_eq!(
        before_period,
        serde_json::to_value(usage::chronos_period(&loc, NOW, 7).unwrap()).unwrap()
    );
    assert_eq!(
        before_cost,
        serde_json::to_value(evolve::preview(&inputs(&loc)).unwrap().digest.cost).unwrap()
    );
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn evolve_collects_archived_handoffs_once() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let wish = loc.home.join("orchestrate/project/handoffs/wish.json");
    let module = loc
        .home
        .join("orchestrate/project/handoffs/module/unit.json");
    for path in [&wish, &module] {
        put(
            path,
            &json!({"module":"unit","ts":"2026-01-02T00:00:00Z","next":"continue"}).to_string(),
        );
    }
    let before = evolve::collect::collect(&inputs(&loc)).unwrap();
    assert_eq!(
        before
            .events
            .iter()
            .filter(|e| e.problem == "handoff.pending")
            .count(),
        2
    );
    for path in [&wish, &module] {
        archive::compress(path).unwrap();
    }
    let files = snapshot(temp.path());
    assert_eq!(before, evolve::collect::collect(&inputs(&loc)).unwrap());
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn archived_goal_and_default_run_keep_metadata_based_discovery() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    let goal = loc.home.join("goals/goal-only.json");
    put(
        &goal,
        &json!({"wish":"test", "status":"blocked", "note":"waiting"}).to_string(),
    );
    let when = SystemTime::UNIX_EPOCH + Duration::from_millis((NOW - 1000) as u64);
    fs::File::options()
        .write(true)
        .open(&goal)
        .unwrap()
        .set_modified(when)
        .unwrap();
    let transcript = loc.projects.join("project/goal-only.jsonl");
    put(
        &transcript,
        &call("one", "2026-01-02T00:00:00Z", "claude-opus-5-5", 1),
    );
    let collected = evolve::collect::collect(&inputs(&loc)).unwrap();
    assert!(collected.events.iter().any(|e| e.problem == "goal.blocked"));
    archive::compress(&goal).unwrap();
    fs::File::options()
        .write(true)
        .open(archive::archived_path(&goal))
        .unwrap()
        .set_modified(when)
        .unwrap();
    archive::compress(&transcript).unwrap();
    let files = snapshot(temp.path());
    assert_eq!(usage::ledger(&loc, None).unwrap().session, "goal-only");
    assert_eq!(collected, evolve::collect::collect(&inputs(&loc)).unwrap());
    assert_eq!(files, snapshot(temp.path()));
}

#[test]
fn latest_run_uses_archive_mtime_when_plain_files_are_absent() {
    let temp = tempfile::tempdir().unwrap();
    let loc = locations(temp.path());
    for (session, seconds) in [("a-new", 10), ("z-old", 100)] {
        let log = loc.home.join(format!("logs/{session}.jsonl"));
        put(
            &log,
            &format!(
                "{}\n",
                json!({"type":"system","subtype":"init","session_id":session})
            ),
        );
        archive::compress(&log).unwrap();
        fs::File::options()
            .write(true)
            .open(archive::archived_path(&log))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(seconds))
            .unwrap();
    }
    assert_eq!(usage::ledger(&loc, None).unwrap().session, "a-new");
}
