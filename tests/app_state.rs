use axiom::app::{self, Poller, Snapshot, Stage, Status, Wish};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
const ORCH: &str = "mcp__plugin_chronos_bridge__orch";
const EVIDENCE: &str = "kind\tcommand\texit_code\tcommit\tts\n";
const UNITS: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
const MAP: &str = "# Map\n\n## m1\nPURPOSE    core\nPATHS      src/a/\nDEPENDS    none\nCONTRACT   a\nACCEPT     tests/a.py `pytest tests/a.py`\n\n## m2\nPURPOSE    second\nPATHS      src/b/\nDEPENDS    m1\nCONTRACT   b\nACCEPT     tests/b.py `pytest tests/b.py`\n\nFULL TEST  `pytest`\n";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn goal(home: &Path, session: &str, age_secs: u64, mut value: Value) {
    let path = home.join("goals").join(format!("{session}.json"));
    let fields = value.as_object_mut().unwrap();
    fields.entry("amendments").or_insert(json!([]));
    write(&path, &serde_json::to_string_pretty(&value).unwrap());
    let when = UNIX_EPOCH + Duration::from_secs(1_800_000_000 - age_secs);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}
fn store(home: &Path, project: &str) -> PathBuf {
    let store = home.join("orchestrate").join(project);
    fs::create_dir_all(store.join("reports")).unwrap();
    store
}
fn lines(events: &[Value]) -> String {
    events.iter().map(|e| format!("{e}\n")).collect()
}
fn init(session: &str) -> Value {
    json!({"type":"system","subtype":"init","session_id":session,"cwd":"E:\\proj","model":"claude-opus-5-5"})
}
fn assistant(agent: Option<&str>, parent: Option<&str>, context: u64, blocks: Value) -> Value {
    let mut e = json!({"type":"assistant","message":{"usage":{"input_tokens":10,"cache_creation_input_tokens":context / 2,"cache_read_input_tokens":context - context / 2 - 10,"output_tokens":7},"content":blocks}});
    if let Some(a) = agent {
        e["agent_id"] = json!(a);
    }
    if let Some(p) = parent {
        e["parent_tool_use_id"] = json!(p);
    }
    e
}
fn agent_call(id: &str, kind: &str, description: &str) -> Value {
    json!([{"type":"tool_use","id":id,"name":"Agent","input":{"subagent_type":kind,"description":description}}])
}
fn result_for(id: &str, content: Value) -> Value {
    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":content}]}})
}
fn result_event(cost: f64) -> Value {
    json!({"type":"result","subtype":"success","is_error":false,"total_cost_usd":cost,"duration_ms":5000,"num_turns":3,"modelUsage":{"claude-opus-5-5":{"costUSD":1.0,"contextWindow":1000000,"inputTokens":98},"claude-haiku-5-5":{"costUSD":0.25}}})
}
fn wish<'a>(snap: &'a Snapshot, session: &str) -> &'a Wish {
    snap.wishes
        .iter()
        .find(|w| w.session == session)
        .unwrap_or_else(|| panic!("no wish {session}: {:?}", snap.warnings))
}
fn stage<'a>(wish: &'a Wish, name: &str) -> &'a Stage {
    wish.stages.iter().find(|s| s.name == name).unwrap()
}
fn statuses(wish: &Wish) -> Vec<(&str, Status)> {
    wish.stages.iter().map(|s| (s.name, s.status)).collect()
}

#[test]
fn empty_home_is_empty() {
    let d = tempfile::tempdir().unwrap();
    let snap = app::snapshot(d.path());
    assert!(snap.wishes.is_empty());
    assert!(snap.live.is_none());
    assert!(snap.needs_you.is_empty());
    assert_eq!(snap.warnings, Vec::<String>::new());
    let missing = app::snapshot(&d.path().join("nowhere"));
    assert!(missing.wishes.is_empty() && missing.warnings.is_empty());
    assert_eq!(app::STAGES.len(), 7);
}

#[test]
fn live_wish_mid_build() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(
        home,
        "s1",
        0,
        json!({"wish":"build it","cwd":"E:\\proj","status":"active","project":"demo","mode":"change","mode_locked":true,"dispatched":true}),
    );
    goal(
        home,
        "s0",
        3600,
        json!({"wish":"older","status":"done","project":"other","note":"fine"}),
    );
    let s = store(home, "demo");
    write(&s.join("modules.md"), MAP);
    let brief = s.join("briefs").join("a1.md");
    write(&brief, "GOAL a\nSCOPE src/a/x.py\nVERIFY pytest\n");
    write(&s.join("briefs").join("b1.md"), "GOAL b\nSCOPE src/b/\nVERIFY pytest\n");
    write(
        &s.join("units.tsv"),
        &format!(
            "{UNITS}a1\tm1\tverified\t\t\t\t{}\na2\tm1\tbuilt\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n",
            brief.display()
        ),
    );
    write(
        &s.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\nb1\t1-abc\tsrc/b/\t2026-11-09T10:00:00.000Z\n",
    );
    write(
        &s.join("reports/a1.evidence.tsv"),
        &format!("{EVIDENCE}run\tgpt-6.1-terra\t0\t\t2026-11-09T09:00:00.000Z\nscope\t\t0\t\t2026-11-09T09:00:01.000Z\nverify\tpytest\t0\ttree:x\t2026-11-09T09:00:02.000Z\naccept\tpytest tests/a.py\t0\ttree:x\t2026-11-09T09:00:03.000Z\nreview\tchronos:reviewer\t0\ttree:x\t2026-11-09T09:00:04.000Z\n"),
    );
    write(
        &s.join("reports/a2.evidence.tsv"),
        &format!("{EVIDENCE}run\tgpt-6.1-terra\t0\t\t2026-11-09T09:10:00.000Z\nscope\t\t0\t\t2026-11-09T09:10:01.000Z\nverify\tpytest\t0\ttree:y\t2026-11-09T09:10:02.000Z\n"),
    );
    write(&s.join("reports/a1.md"), "done\nBRIDGE VERIFY exit=0");
    write(&s.join("reports/a1.json"), "{}");
    write(&s.join("reports/a1.verify.txt"), "ok");
    write(
        &s.join("reports/_stages.evidence.tsv"),
        &format!("{EVIDENCE}astra\tgpt-6-astra\t0\t\t2026-11-09T08:00:00.000Z\n"),
    );
    write(&s.join("astra/plan.json"), "{}");
    write(&s.join("astra/review.json"), r#"{"verdict":"pass","findings":[]}"#);
    write(&s.join("astra/ruling.md"), "upheld nothing");
    write(&s.join("decisions.md"), "- use pytest: it is installed\n\n- keep m2 small\n");
    let codex_report = json!({"status":"completed","model":"gpt-6-astra","usage":{"inputTokens":1000,"cachedInputTokens":400,"outputTokens":200}}).to_string();
    write(
        &home.join("logs/20261109-100000-wish.jsonl"),
        &lines(&[
            init("s1"),
            assistant(None, None, 6000, agent_call("a1", "chronos:astra", "plan")),
            json!({"type":"system","subtype":"task_started","task_id":"ag1","tool_use_id":"a1","subagent_type":"chronos:astra","description":"plan"}),
            assistant(Some("ag1"), Some("a1"), 20000, json!([{"type":"tool_use","id":"c1","name":CODEX,"input":{"role":"astra","brief":"GOAL plan it\nSCOPE x"}}])),
            result_for("c1", json!(codex_report)),
            result_for("a1", json!("planned")),
            result_event(1.25),
            assistant(None, None, 42000, agent_call("a2", "chronos:dispatcher", "build m1")),
            assistant(Some("ag2"), Some("a2"), 65000, json!([{"type":"tool_use","id":"c2","name":CODEX,"input":{"role":"terra","project":"demo","unit":"b1","cwd":"E:\\proj"}}])),
        ]),
    );
    let snap = app::snapshot(home);
    assert_eq!(snap.warnings, Vec::<String>::new());
    assert_eq!(snap.live.as_deref(), Some("s1"));
    assert_eq!(
        snap.wishes.iter().map(|w| w.session.as_str()).collect::<Vec<_>>(),
        ["s1", "s0"]
    );
    let w = wish(&snap, "s1");
    assert_eq!(w.goal.project.as_deref(), Some("demo"));
    assert_eq!(w.store.as_deref(), Some(s.as_path()));
    assert_eq!(
        statuses(w),
        [
            ("astra", Status::Done),
            ("critic", Status::Done),
            ("architect", Status::Done),
            ("build", Status::Running),
            ("full_test", Status::NotStarted),
            ("memory", Status::NotStarted),
            ("report", Status::NotStarted),
        ]
    );
    assert_eq!(stage(w, "astra").detail.as_deref(), Some("gpt-6-astra"));
    assert_eq!(stage(w, "critic").detail.as_deref(), Some("pass"));
    assert_eq!(stage(w, "architect").detail.as_deref(), Some("2 modules"));
    assert_eq!(
        stage(w, "build").detail.as_deref(),
        Some("1/3 verified, 0 failed, 1 built, 1 running, 0 pending")
    );
    assert!(w.astra.plan && w.astra.review && w.astra.ruling && !w.astra.plan_final);
    assert_eq!(w.counts.total, 3);
    assert_eq!(w.counts.verified, 1);
    assert_eq!(w.counts.unreviewed, 1);
    assert_eq!(w.counts.in_progress, 1);
    assert_eq!(w.counts.failed + w.counts.pending, 0);
    assert_eq!(w.decisions, ["- use pytest: it is installed", "- keep m2 small"]);
    assert_eq!(w.modules.len(), 2);
    let m1 = &w.modules[0];
    assert_eq!((m1.id.as_str(), m1.state.as_str(), m1.accepted), ("m1", "built", Some(true)));
    assert_eq!((m1.counts.total, m1.counts.verified, m1.counts.unreviewed), (2, 1, 1));
    let m2 = &w.modules[1];
    assert_eq!((m2.state.as_str(), m2.accepted, m2.depends.as_slice()), ("running", None, &["m1".to_string()][..]));
    assert_eq!(m2.counts.in_progress, 1);
    let a1 = &w.units[0];
    assert_eq!((a1.id.as_str(), a1.state.as_str(), a1.derived.as_str(), a1.attempts), ("a1", "verified", "verified", 1));
    assert_eq!(a1.brief.as_deref(), Some(brief.as_path()));
    assert_eq!(a1.report.as_deref(), Some(s.join("reports/a1.md").as_path()));
    assert_eq!(a1.report_json.as_deref(), Some(s.join("reports/a1.json").as_path()));
    assert_eq!(a1.verify_output.as_deref(), Some(s.join("reports/a1.verify.txt").as_path()));
    let review = a1.review.as_ref().unwrap();
    assert_eq!((review.command.as_str(), review.exit_code), ("chronos:reviewer", 0));
    assert_eq!(a1.evidence.len(), 5);
    let a2 = &w.units[1];
    assert_eq!((a2.derived.as_str(), a2.review.is_none(), a2.report.is_none()), ("built", true, true));
    let b1 = &w.units[2];
    assert_eq!(b1.claim.as_ref().map(|c| c.attempt.as_str()), Some("1-abc"));
    assert_eq!(b1.claim.as_ref().unwrap().scope, ["src/b/"]);
    assert_eq!(b1.brief.as_deref(), Some(s.join("briefs/b1.md").as_path()));
    assert_eq!((b1.attempts, b1.derived.as_str()), (0, "pending"));
    assert_eq!(w.runs.len(), 1);
    let run = &w.runs[0];
    assert_eq!((run.skill.as_str(), run.started.as_deref()), ("wish", Some("20261109-100000")));
    assert_eq!(run.cwd.as_deref(), Some("E:\\proj"));
    let summary = run.summary.as_ref().unwrap();
    assert_eq!(summary.total_cost_usd, Some(1.25));
    assert_eq!(summary.num_turns, Some(3));
    assert_eq!(summary.models["claude-opus-5-5"].cost_usd, Some(1.0));
    assert_eq!(summary.models["claude-opus-5-5"].context_window, Some(1000000));
    assert_eq!(summary.models["claude-haiku-5-5"].input, None);
    let scan = run.scan.as_ref().unwrap();
    assert!(scan.complete);
    assert_eq!((scan.events, scan.malformed), (9, 0));
    let astra = &scan.codex["gpt-6-astra"];
    assert_eq!((astra.calls, astra.input, astra.cached, astra.output), (1, 1000, 400, 200));
    assert_eq!(scan.finished["chronos:astra"], 1);
    let open: Vec<_> = scan.open.iter().map(|c| (c.tool.as_str(), c.kind.as_deref(), c.detail.as_deref())).collect();
    assert_eq!(
        open,
        [("Agent", Some("chronos:dispatcher"), Some("build m1")), ("codex_run", Some("terra"), Some("b1"))]
    );
    let agents: Vec<_> = scan.agents.iter().map(|a| (a.id.as_str(), a.kind.as_deref(), a.peak_context, a.open)).collect();
    assert_eq!(
        agents,
        [
            ("main", None, Some(42000), false),
            ("ag1", Some("chronos:astra"), Some(20000), false),
            ("ag2", Some("chronos:dispatcher"), Some(65000), true),
        ]
    );
    assert_eq!(scan.agents[0].messages, 2);
    assert_eq!(scan.agents[1].description.as_deref(), Some("plan"));
    assert_eq!(stage(w, "memory").detail.as_deref(), Some("0 entries"));
    assert!(w.memory.is_empty());
    assert_eq!(w.last_activity, w.modified);
    let old = wish(&snap, "s0");
    assert!(old.runs.is_empty() && old.units.is_empty() && old.store.is_some());
    assert_eq!(stage(old, "report").status, Status::Done);
    assert_eq!(stage(old, "build").status, Status::Skipped);
    assert!(snap.needs_you.is_empty());
    let json = serde_json::to_value(&snap).unwrap();
    assert_eq!(json["wishes"][0]["stages"][3]["status"], "running");
    assert_eq!(json["wishes"][0]["stages"][4]["status"], "not_started");
}

#[test]
fn blocked_waiting_and_interjections_need_the_user() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "newest", 0, json!({"wish":"w1","status":"waiting","project":"p1","note":"batch runs; progress.json; done.flag"}));
    goal(home, "middle", 60, json!({"wish":"w2","status":"blocked","project":"p2","note":"needs a token","reason":"interrupted"}));
    goal(home, "oldest", 120, json!({"wish":"w3","status":"active","project":"p3","interjection":{"prompt":"stop please","prior_status":"active","prior_note":null}}));
    goal(home, "quiet", 180, json!({"wish":"w4","status":"done","note":"ok"}));
    let snap = app::snapshot(home);
    assert_eq!(snap.warnings, Vec::<String>::new());
    assert_eq!(
        snap.wishes.iter().map(|w| w.session.as_str()).collect::<Vec<_>>(),
        ["newest", "middle", "oldest", "quiet"]
    );
    assert_eq!(snap.live.as_deref(), Some("oldest"));
    assert_eq!(wish(&snap, "newest").modified.as_deref(), Some("2027-01-15T08:00:00.000Z"));
    assert_eq!(wish(&snap, "middle").modified.as_deref(), Some("2027-01-15T07:59:00.000Z"));
    let need: Vec<_> = snap
        .needs_you
        .iter()
        .map(|a| (a.session.as_str(), a.kind.as_str(), a.note.as_str(), a.project.as_deref()))
        .collect();
    assert_eq!(
        need,
        [
            ("newest", "waiting", "batch runs; progress.json; done.flag", Some("p1")),
            ("middle", "blocked", "needs a token", Some("p2")),
            ("oldest", "interjection", "stop please", Some("p3")),
        ]
    );
    assert_eq!(stage(wish(&snap, "newest"), "report").status, Status::Done);
    assert_eq!(stage(wish(&snap, "newest"), "report").detail.as_deref(), Some("waiting"));
    let interrupted = stage(wish(&snap, "middle"), "report");
    assert_eq!((interrupted.status, interrupted.detail.as_deref()), (Status::Failed, Some("interrupted")));
    let quiet = wish(&snap, "quiet");
    assert!(quiet.store.is_none());
    assert_eq!(stage(quiet, "report").status, Status::Done);
    assert_eq!(stage(quiet, "memory").status, Status::NotStarted);
    assert_eq!(stage(quiet, "memory").detail, None);
}

#[test]
fn memory_entries_match_the_run_window_or_the_wish() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "s1", 0, json!({"wish":"[astra] build it","status":"done","project":"demo","note":"ok"}));
    goal(home, "s2", 60, json!({"wish":"other wish","status":"done","project":"demo","note":"ok"}));
    store(home, "demo");
    write(&home.join("logs/20261109-100000-wish.jsonl"), &lines(&[init("s1"), result_event(0.1)]));
    axiom::memory::Memory::open(home).unwrap();
    let db = rusqlite::Connection::open(home.join("memory/memory.db")).unwrap();
    let insert = |unit: &str, wish: &str, created: f64| {
        db.execute(
            "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('demo', ?1, ?2, 's', 'b', '', ?3, '[]', 'E:/proj', ?4)",
            rusqlite::params![unit, format!("title {unit}"), wish, created],
        )
        .unwrap();
    };
    insert("early", "something else", 1_762_000_000.0);
    insert("inside", "build it, carried out", 1_794_500_000.0);
    insert("late", "unrelated", 1_800_100_000.0);
    insert("by-text", "the user said: build it", 1_700_000_000.0);
    let snap = app::snapshot(home);
    assert_eq!(snap.warnings, Vec::<String>::new());
    let w = wish(&snap, "s1");
    let run = &w.runs[0];
    let utc = 1_794_218_400i64;
    assert!(run.started_unix.is_some_and(|s| (s - utc).abs() <= 14 * 3600), "{:?}", run.started_unix);
    assert!(run.modified_unix.is_some());
    assert_eq!(w.last_activity.as_deref(), Some("2027-01-15T08:00:00.000Z"));
    let units: Vec<_> = w.memory.iter().map(|m| (m.id, m.unit.as_str())).collect();
    assert_eq!(units, [(2, "inside"), (4, "by-text")]);
    assert_eq!(w.memory[0].title, "title inside");
    assert_eq!(w.memory[0].created, "2026-11-12T16:13:20.000Z");
    assert_eq!(stage(w, "memory").status, Status::Done);
    assert_eq!(stage(w, "memory").detail.as_deref(), Some("2 entries"));
    let other = wish(&snap, "s2");
    assert!(other.memory.is_empty());
    assert_eq!(stage(other, "memory").status, Status::NotStarted);
    assert_eq!(stage(other, "memory").detail.as_deref(), Some("0 entries"));
}

#[test]
fn truncated_lines_do_not_fail_the_snapshot() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "s1", 0, json!({"wish":"x","status":"active","project":"demo"}));
    write(&home.join("goals/bad.json"), "{\"wish\": \"x\", \"status\": \"act");
    let s = store(home, "demo");
    write(&s.join("units.tsv"), &format!("{UNITS}u1\tbuild\tverified\t\t\t\t\nu2\tbuild\tpen"));
    write(
        &s.join("reports/u1.evidence.tsv"),
        &format!("{EVIDENCE}run\tgpt-6.1-terra\t0\t\t2026-11-09T09:00:00.000Z\nverify\tpytest\t0\ttree:"),
    );
    write(&s.join("claims.tsv"), "unit\tattempt\tscope\tstarted\nu1\t1-x");
    write(&s.join("reports/_full.evidence.tsv"), &format!("{EVIDENCE}full\tpytest\tzero\t\t2026"));
    let mut log = lines(&[
        init("s1"),
        assistant(None, None, 100, agent_call("a1", "chronos:dispatcher", "go")),
        assistant(Some("ag1"), Some("a1"), 200, json!([{"type":"tool_use","id":"o1","name":ORCH,"input":{"project":"demo","args":["fulltest","--repo","E:\\proj"]}}])),
    ]);
    log.push_str("not json at all\n");
    log.push_str("{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1");
    write(&home.join("logs/20261109-100000-wish.jsonl"), &log);
    let snap = app::snapshot(home);
    let w = wish(&snap, "s1");
    assert_eq!(w.units.len(), 1);
    assert_eq!((w.units[0].id.as_str(), w.units[0].attempts, w.units[0].claim.is_none()), ("u1", 1, true));
    assert_eq!(w.counts.verified, 1);
    assert_eq!(stage(w, "full_test").status, Status::Running);
    assert_eq!(stage(w, "build").status, Status::Running);
    let scan = w.runs[0].scan.as_ref().unwrap();
    assert!(scan.complete);
    assert_eq!((scan.events, scan.malformed), (3, 1));
    let open: Vec<_> = scan.open.iter().map(|c| (c.tool.as_str(), c.kind.as_deref(), c.detail.as_deref())).collect();
    assert_eq!(
        open,
        [("Agent", Some("chronos:dispatcher"), Some("go")), ("orch", Some("orch:fulltest"), Some("fulltest --repo E:\\proj"))]
    );
    let text = snap.warnings.join("\n");
    for expected in [
        "goals/bad.json: ",
        "demo/units.tsv: 1 malformed row(s) skipped",
        "demo/reports/u1.evidence.tsv: 1 malformed row(s) skipped",
        "demo/claims.tsv: 1 malformed row(s) skipped",
        "demo/reports/_full.evidence.tsv: 1 row(s) with an invalid exit code skipped",
        "logs/20261109-100000-wish.jsonl: 1 malformed line(s) skipped",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
    assert_eq!(snap.warnings.len(), 6, "{text}");
    assert_eq!(snap.wishes.len(), 1);
}

#[test]
fn old_stores_with_second_review_and_rewrite_rows_still_read() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "r4", 0, json!({"wish":"[astra] round 4","status":"done","project":"r4","note":"ok"}));
    goal(home, "r3", 60, json!({"wish":"[astra] round 3","status":"blocked","project":"r3","note":"stuck"}));
    let r4 = store(home, "r4");
    write(
        &r4.join("reports/_stages.evidence.tsv"),
        &format!("{EVIDENCE}astra\tgpt-6-astra\t0\t\t2026-11-02T11:32:28.608Z\nastra\tgpt-6-astra\t0\t\t2026-11-02T11:59:19.912Z\nsecond_review\tgemini-4-argon-high\t1\t\t2026-11-02T12:00:15.583Z\nrewrite\tgemini-4-argon-medium\t0\t\t2026-11-02T15:40:13.088Z\n"),
    );
    write(&r4.join("units.tsv"), &format!("{UNITS}memory\tbuild\tverified\t\t\t\t\n"));
    write(
        &r4.join("reports/_full.evidence.tsv"),
        &format!("{EVIDENCE}full\tcargo test\t0\ttree:z\t2026-11-02T16:00:00.000Z\n"),
    );
    let r3 = store(home, "r3");
    write(
        &r3.join("reports/_stages.evidence.tsv"),
        &format!("{EVIDENCE}astra\tgpt-6-astra\t0\t\t2026-10-30T03:34:38.761Z\nsecond_review\tgemini-4-argon-high\t1\t\t2026-10-30T03:35:19.376Z\n"),
    );
    let snap = app::snapshot(home);
    assert_eq!(snap.warnings, Vec::<String>::new());
    let w4 = wish(&snap, "r4");
    assert_eq!(
        statuses(w4),
        [
            ("astra", Status::Done),
            ("critic", Status::Done),
            ("architect", Status::Skipped),
            ("build", Status::Done),
            ("full_test", Status::Done),
            ("memory", Status::NotStarted),
            ("report", Status::Done),
        ]
    );
    assert_eq!(stage(w4, "critic").detail.as_deref(), Some("rewrite gemini-4-argon-medium"));
    assert_eq!(stage(w4, "architect").detail.as_deref(), Some("no module map"));
    assert_eq!(stage(w4, "full_test").detail.as_deref(), Some("exit 0 cargo test"));
    let w3 = wish(&snap, "r3");
    let critic = stage(w3, "critic");
    assert_eq!((critic.status, critic.detail.as_deref()), (Status::Failed, Some("second_review gemini-4-argon-high")));
    assert_eq!(stage(w3, "build").status, Status::NotStarted);
}

#[test]
fn stage_failures_are_derived_from_disk() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "s1", 0, json!({"wish":"x","status":"blocked","project":"demo","note":"units failed"}));
    goal(home, "s2", 60, json!({"wish":"[no-architect] y","status":"active","project":"tiny"}));
    let s = store(home, "demo");
    write(&s.join("modules.md"), "## m1\nPURPOSE x\n");
    write(
        &s.join("reports/_stages.evidence.tsv"),
        &format!("{EVIDENCE}astra\tgpt-6-astra\t1\t\t2026-11-09T08:00:00.000Z\n"),
    );
    write(&s.join("units.tsv"), &format!("{UNITS}u1\tm1\treview-failed\t\t\t\t\nu2\tm1\tverified\t\t\t\t\n"));
    write(
        &s.join("reports/u2.evidence.tsv"),
        &format!("{EVIDENCE}run\tgpt-6.1-terra\t0\t\t2026-11-09T09:00:00.000Z\nverify\tpytest\t0\ttree:q\t2026-11-09T09:30:00.000Z\n"),
    );
    write(
        &s.join("reports/_full.evidence.tsv"),
        &format!("{EVIDENCE}full\tpytest\t0\ttree:z\t2026-11-09T09:00:00.000Z\nfull\tpytest\t1\ttree:z\t2026-11-09T09:20:00.000Z\n"),
    );
    store(home, "tiny");
    let snap = app::snapshot(home);
    let w = wish(&snap, "s1");
    assert_eq!(
        statuses(w),
        [
            ("astra", Status::Failed),
            ("critic", Status::NotStarted),
            ("architect", Status::Failed),
            ("build", Status::Failed),
            ("full_test", Status::Failed),
            ("memory", Status::NotStarted),
            ("report", Status::Done),
        ]
    );
    assert_eq!(stage(w, "architect").detail.as_deref(), Some("module m1: missing PATHS"));
    assert_eq!(stage(w, "full_test").detail.as_deref(), Some("exit 1 pytest; stale: units changed since"));
    assert_eq!(stage(w, "build").detail.as_deref(), Some("1/2 verified, 1 failed, 0 built, 0 running, 0 pending"));
    assert_eq!(w.counts.failed, 1);
    assert!(w.modules.is_empty());
    assert_eq!(snap.warnings, ["demo/modules.md: module m1: missing PATHS"]);
    let tiny = wish(&snap, "s2");
    assert_eq!(stage(tiny, "architect").status, Status::Skipped);
    assert_eq!(stage(tiny, "architect").detail.as_deref(), Some("[no-architect]"));
    assert_eq!(stage(tiny, "build").status, Status::NotStarted);
}

#[test]
fn poller_scans_followed_logs_in_bounded_steps() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "s1", 0, json!({"wish":"x","status":"active","project":"demo"}));
    let log = home.join("logs/20261109-100000-wish.jsonl");
    let events = [
        init("s1"),
        assistant(None, None, 5000, agent_call("a1", "chronos:memory-keeper", "keep")),
        json!({"type":"system","subtype":"task_started","task_id":"k1","tool_use_id":"a1","subagent_type":"chronos:memory-keeper","description":"keep"}),
        assistant(Some("k1"), Some("a1"), 9000, json!([])),
        result_event(0.5),
    ];
    write(&log, &lines(&events));
    let size = fs::metadata(&log).unwrap().len();
    let mut poller = Poller::with_budget(home, 64);
    let first = poller.poll();
    let run = &wish(&first, "s1").runs[0];
    let scan = run.scan.as_ref().unwrap();
    assert!(!scan.complete);
    assert_eq!(scan.scanned, 64);
    assert!(run.summary.is_none());
    let mut polls = 1;
    let snap = loop {
        let snap = poller.poll();
        polls += 1;
        let run = &wish(&snap, "s1").runs[0];
        if run.scan.as_ref().unwrap().complete {
            break snap;
        }
        assert!(polls < 1000, "scan never completed");
    };
    assert!(polls >= (size / 64) as usize);
    let w = wish(&snap, "s1");
    let run = &w.runs[0];
    let scan = run.scan.as_ref().unwrap();
    assert_eq!((scan.scanned, scan.events, scan.malformed), (size, 5, 0));
    assert_eq!(run.summary.as_ref().unwrap().total_cost_usd, Some(0.5));
    assert_eq!(scan.open.len(), 1);
    assert_eq!(stage(w, "memory").status, Status::Running);
    assert_eq!(stage(w, "report").status, Status::Running);
    assert_eq!(scan.agents[1].kind.as_deref(), Some("chronos:memory-keeper"));
    assert!(scan.agents[1].open);
    let mut text = fs::read_to_string(&log).unwrap();
    text.push_str(&format!("{}\n", result_for("a1", json!("recorded 2"))));
    fs::write(&log, text).unwrap();
    let mut polls = 0;
    let snap = loop {
        let snap = poller.poll();
        polls += 1;
        if wish(&snap, "s1").runs[0].scan.as_ref().unwrap().complete {
            break snap;
        }
        assert!(polls < 1000);
    };
    let w = wish(&snap, "s1");
    let scan = w.runs[0].scan.as_ref().unwrap();
    assert!(scan.open.is_empty());
    assert_eq!(scan.finished["chronos:memory-keeper"], 1);
    assert!(!scan.agents[1].open);
    assert_eq!(stage(w, "memory").status, Status::Done);
    assert_eq!(snap.warnings, Vec::<String>::new());
}

#[test]
fn finished_wishes_read_only_the_log_tail() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path();
    goal(home, "s1", 0, json!({"wish":"x","status":"done","project":"demo","note":"ok"}));
    let mut text = lines(&[init("s1"), assistant(None, None, 5000, json!([]))]);
    for _ in 0..3000 {
        text.push_str(&format!("{}\n", json!({"type":"system","subtype":"task_progress","detail":"x".repeat(400)})));
    }
    text.push_str(&format!("{}\n", result_event(2.5)));
    text.push_str(&format!("{}\n", json!({"type":"system","subtype":"task_summary"})));
    write(&home.join("logs/20261109-100000-wish.jsonl"), &text);
    write(&home.join("logs/20261109-110000-wish-1.jsonl"), &lines(&[init("s1"), result_event(0.75)]));
    write(&home.join("logs/20261109-120000-research.jsonl"), &lines(&[json!({"type":"system","subtype":"init","session_id":"other"})]));
    write(&home.join("logs/notes.txt"), "ignored");
    let snap = app::snapshot(home);
    assert_eq!(snap.warnings, Vec::<String>::new());
    let w = wish(&snap, "s1");
    assert_eq!(w.runs.len(), 2);
    assert!(w.runs.iter().all(|r| r.scan.is_none()));
    assert_eq!(w.runs[0].summary.as_ref().unwrap().total_cost_usd, Some(2.5));
    assert_eq!(w.runs[1].summary.as_ref().unwrap().total_cost_usd, Some(0.75));
    assert_eq!((w.runs[1].skill.as_str(), w.runs[1].started.as_deref()), ("wish", Some("20261109-110000")));
    assert!(w.runs[0].bytes > 1_000_000);
    assert!(w.runs[0].modified.as_deref().is_some_and(|m| m.ends_with('Z') && m.len() == 24));
    let mut poller = Poller::new(home);
    poller.follow("s1");
    let followed = poller.poll();
    let run = &wish(&followed, "s1").runs[0];
    let scan = run.scan.as_ref().unwrap();
    assert!(scan.complete);
    assert_eq!(scan.events, 3004);
    assert_eq!(scan.agents[0].peak_context, Some(5000));
}
