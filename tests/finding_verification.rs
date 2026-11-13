mod common;
use axiom::{
    base,
    codex::AppServer,
    engine::{
        finding_verification::{self as fv, Status},
        policy,
    },
    orch::{self, Evidence},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _temp: tempfile::TempDir,
    store: PathBuf,
    script: PathBuf,
    log: PathBuf,
    server: AppServer,
}
impl Fixture {
    fn new(reply: Value) -> Self {
        let temp = common::temp();
        let store = temp.path().join("store");
        fs::create_dir_all(store.join("reports")).unwrap();
        let script = temp.path().join("script.json");
        let log = temp.path().join("codex.jsonl");
        fs::write(
            &script,
            json!([{"match":"AXIOM-ENGINE verifier finding","reply":reply}]).to_string(),
        )
        .unwrap();
        let mut cmd = common::fake("fake_codex");
        cmd.push(format!("--fake-script={}", script.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        let server = AppServer::new(cmd).with_slots_home(temp.path().join("slots"));
        Self {
            _temp: temp,
            store,
            script,
            log,
            server,
        }
    }
    fn add(&self, kind: &str, command: &str, exit_code: i32, tree: &str) {
        orch::append_evidence(&self.store, "u1", &row(kind, command, exit_code, tree)).unwrap();
    }
    fn review(&self, tree: &str, reviewer: &str, findings: Value) {
        self.add("review", reviewer, 1, tree);
        fs::write(
            orch::review_verdict_path(&self.store, "u1", reviewer),
            json!({"unit":"u1","tree":tree,"verdict":"fail","findings":findings}).to_string(),
        )
        .unwrap();
    }
    fn check(&self) -> Vec<fv::Check> {
        fv::verify_review(&self.server, &self.store, "u1", &self.store).unwrap()
    }
    fn gate(&self) -> base::Res<()> {
        fv::fix_gate(&self.server, &self.store, "u1", &self.store)
    }
    fn turns(&self) -> Vec<Value> {
        lines(&self.log)
            .into_iter()
            .filter(|v| v["method"] == "turn/start")
            .collect()
    }
}
fn lines(p: &Path) -> Vec<Value> {
    fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn row(kind: &str, command: &str, exit_code: i32, tree: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code,
        commit: tree.into(),
        ts: base::now_iso(),
    }
}
fn findings(severity: &str) -> Value {
    json!([{"severity":severity,"location":"src/a:1","check":"contract","evidence":"evidence","fix":"fix"}])
}
fn reply(status: &str) -> Value {
    json!({"findings":[{"finding":1,"status":status,"reason":"grounded reason"}]})
}

#[test]
fn rejected_major_closes_review_without_waiver() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("rejected"));
    f.add("run", "gpt-6-terra", 0, "tree:a");
    f.add("verify", "tests", 0, "tree:a");
    f.review("tree:a", "chronos:reviewer", findings("major"));
    assert_eq!(f.gate().unwrap_err().kind, "findings_rejected");
    let rows = orch::evidence(&f.store, "u1").unwrap();
    assert_eq!(orch::derive_state(&rows, false), "verified");
    assert!(rows.iter().any(|r| r.kind == "checked"
        && r.exit_code == 0
        && r.command == "1 rejected grounded reason"));
    assert!(!rows.iter().any(|r| r.kind == "waive"));
    assert_eq!(orch::rejections(&rows), vec![(1, "grounded reason".into())]);
    let events = orch::events::read_events(&f.store, 0).unwrap();
    let event = events
        .iter()
        .find(|e| e.kind == "finding_verified")
        .unwrap();
    assert_eq!(event.unit.as_deref(), Some("u1"));
    assert_eq!(event.data["status"], "rejected");
    let turns = f.turns();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["params"]["effort"], "low");
    assert_eq!(fv::rejection_rate(&f.store).unwrap(), (1, 1));
}

#[test]
fn confirmed_and_non_failing_findings() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("confirmed"));
    f.review("a", "chronos:reviewer", findings("blocker"));
    assert!(f.gate().is_ok());
    assert_eq!(f.check()[0].status, Status::Confirmed);
    assert_eq!(f.turns().len(), 1);
    let f = Fixture::new(reply("rejected"));
    f.review("a", "chronos:reviewer", json!([{"severity":"minor","check":"contract"},{"severity":"major","check":"untested","needs":"manual"}]));
    assert!(f.gate().is_ok());
    assert!(f.check().is_empty());
    assert!(f.turns().is_empty());
}

#[test]
fn reuses_same_review_and_rechecks_new_tree() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("rejected"));
    f.review("a", "chronos:reviewer", findings("major"));
    assert_eq!(f.check()[0].review_id, "u1#1");
    f.check();
    assert_eq!(f.turns().len(), 1);
    f.review("b", "chronos:reviewer", findings("major"));
    let check = f.check();
    assert_eq!(check[0].review_id, "u1#2");
    assert_eq!(check[0].tree, "b");
    assert_eq!(f.turns().len(), 2);
    assert_eq!(fv::checks(&f.store, "u1").unwrap().len(), 2);
}

#[test]
fn errors_missing_reply_and_budget_are_inconclusive() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(json!({"findings":[]}));
    f.review("a", "chronos:reviewer", findings("major"));
    let c = f.check();
    assert_eq!(c[0].status, Status::Inconclusive);
    assert!(c[0].failed);
    assert_eq!(fv::rejection_rate(&f.store).unwrap(), (0, 0));
    policy::set(&f.store, "budget.verifier", "0").unwrap();
    assert!(f.gate().is_ok());
    let c = fv::checks(&f.store, "u1").unwrap();
    assert!(c[0].failed && c[0].reason.contains("budget_exceeded"));
    assert_eq!(f.turns().len(), 1);
    assert_eq!(fv::rejection_rate(&f.store).unwrap(), (0, 0));
}

static CLAUDE_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn claude_calls_by_severity() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("confirmed"));
    let log = f.store.join("member.jsonl");
    let mut cmd = common::fake("fake_member");
    cmd.push(format!("--fake-script={}", f.script.display()));
    cmd.push(format!("--fake-log={}", log.display()));
    let previous = std::env::var_os("AXIOM_CLAUDE_CMD");
    std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&cmd).unwrap());
    f.review("a", "engine-astra:gpt-6-astra", findings("major"));
    let check = f.check();
    match previous {
        Some(ref v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
    assert_eq!(check[0].status, Status::Confirmed);
    assert_eq!(check[0].verifier_model, "sonnet");
    assert_eq!(lines(&log).len(), 1);
    assert!(lines(&log)[0]["prompt"]
        .as_str()
        .unwrap()
        .starts_with("AXIOM-ENGINE verifier finding\n"));
    assert!(f.turns().is_empty());

    // A single review batches the two serious findings into one verifier request.
    let mut mixed = findings("blocker");
    mixed.as_array_mut().unwrap().push(findings("major")[0].clone());
    fs::write(&f.script, json!([{"match":"AXIOM-ENGINE verifier finding","reply":{"findings":[{"finding":1,"status":"confirmed","reason":"blocker confirmed"},{"finding":2,"status":"rejected","reason":"major rejected"}]}}]).to_string()).unwrap();
    std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&cmd).unwrap());
    f.review("b", "engine-astra:gpt-6-astra", mixed);
    let checks = f.check();
    match previous {
        Some(ref v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
        None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
    }
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0].severity, "blocker");
    assert_eq!(checks[1].severity, "major");
    assert_eq!(checks[1].status, Status::Rejected);
    let calls = lines(&log);
    assert_eq!(calls.len(), 2); // one initial major request, then one two-finding batch
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert_eq!(prompt.matches("\nFinding ").count(), 2);

    for severity in ["blocker", "major", "minor", "pass"] {
        let f = Fixture::new(reply("confirmed"));
        let log = f.store.join("member.jsonl");
        let mut cmd = common::fake("fake_member");
        cmd.push(format!("--fake-script={}", f.script.display()));
        cmd.push(format!("--fake-log={}", log.display()));
        let previous = std::env::var_os("AXIOM_CLAUDE_CMD");
        std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&cmd).unwrap());
        f.review("a", "engine-astra:gpt-6-astra", if severity == "pass" { json!([]) } else { findings(severity) });
        if severity == "pass" {
            fs::write(orch::review_verdict_path(&f.store, "u1", "engine-astra:gpt-6-astra"), json!({"unit":"u1","tree":"a","verdict":"pass","findings":[]}).to_string()).unwrap();
        }
        let checks = f.check();
        match previous {
            Some(v) => std::env::set_var("AXIOM_CLAUDE_CMD", v),
            None => std::env::remove_var("AXIOM_CLAUDE_CMD"),
        }
        let expected = usize::from(matches!(severity, "blocker" | "major"));
        assert_eq!(lines(&log).len(), expected);
        assert_eq!(checks.len(), expected);
        assert!(f.turns().is_empty());
    }
}

#[test]
fn minor_findings_get_no_check() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    for status in ["rejected", "confirmed"] {
        let f = Fixture::new(json!({"findings":[{"finding":2,"status":status,"reason":"grounded reason"}]}));
        let mut mixed = findings("minor");
        mixed.as_array_mut().unwrap().push(findings("major")[0].clone());
        f.review("a", "chronos:reviewer", mixed);
        let checks = f.check();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].finding, 2);
        assert_eq!(checks[0].severity, "major");
        assert_eq!(fv::checks(&f.store, "u1").unwrap(), checks);
        assert_eq!(fv::rejection_rate(&f.store).unwrap(), (u64::from(status == "rejected"), 1));
        if status == "rejected" { assert_eq!(f.gate().unwrap_err().kind, "findings_rejected"); }
        else { assert!(f.gate().is_ok()); }
        assert_eq!(f.turns().len(), 1);
    }
    let f = Fixture::new(reply("confirmed"));
    f.review("a", "chronos:reviewer", findings("minor"));
    assert!(f.check().is_empty());
    assert!(f.gate().is_ok());
    assert!(f.turns().is_empty());
}

#[test]
fn parsing_paths_and_failed_checked_control() {
    let p = Path::new("store");
    assert_eq!(
        orch::review_verdict_path(p, "u1", "chronos:reviewer"),
        p.join("reports/u1.verdict.json")
    );
    assert_eq!(
        orch::review_verdict_path(p, "u1", "chronos:pm-qa"),
        p.join("reports/u1.pm-qa.verdict.json")
    );
    assert_eq!(
        orch::review_verdict_path(p, "u1", "engine-astra:x"),
        p.join("reports/u1.astra.verdict.json")
    );
    let mut rows = vec![
        row("run", "gpt-6-terra", 0, "a"),
        row("verify", "x", 0, "a"),
        row("review", "chronos:reviewer", 1, "a"),
        row("checked", "1 rejected reason", 1, "a"),
    ];
    assert_eq!(orch::derive_state(&rows, false), "review-failed");
    rows.extend([
        row("checked", "0 rejected bad", 0, "a"),
        row("checked", "2 rejected ", 0, "a"),
        row("checked", "3 rejected old", 0, "b"),
        row("checked", "4 waived wrong", 0, "a"),
        row("checked", "02 rejected valid", 1, "a"),
    ]);
    assert_eq!(
        orch::rejections(&rows),
        vec![(1, "reason".into()), (2, "valid".into())]
    );
    rows.push(row("review", "chronos:reviewer", 1, "a"));
    assert!(orch::rejections(&rows).is_empty());
}

#[test]
fn call_error_is_failed_and_does_not_reject() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("rejected"));
    f.review("a", "chronos:reviewer", findings("major"));
    let broken = AppServer::new(vec!["axiom-nonexistent-finding-verifier".into()])
        .with_slots_home(f.store.join("bad-slots"));
    let checks = fv::verify_review(&broken, &f.store, "u1", &f.store).unwrap();
    assert_eq!(checks[0].status, Status::Inconclusive);
    assert!(checks[0].failed);
    assert_eq!(fv::rejection_rate(&f.store).unwrap(), (0, 0));
    assert!(fv::fix_gate(&broken, &f.store, "u1", &f.store).is_ok());
    assert!(!orch::evidence(&f.store, "u1")
        .unwrap()
        .iter()
        .any(|r| r.kind == "checked" || r.kind == "waive"));
}

#[test]
fn multiple_reviews_do_not_close_one_another() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("rejected"));
    f.add("run", "gpt-6-terra", 0, "a");
    f.add("verify", "test", 0, "a");
    f.review("a", "chronos:reviewer", findings("major"));
    f.review(
        "a",
        "chronos:pm-qa",
        json!([{"severity":"major"}, {"severity":"major"}]),
    );
    let checks = f.check();
    assert_eq!(checks.len(), 3);
    assert_eq!(checks[2].status, Status::Inconclusive);
    assert!(checks[2].failed);
    let rows = orch::evidence(&f.store, "u1").unwrap();
    assert_eq!(orch::derive_state(&rows, false), "review-failed");
    assert!(rows
        .iter()
        .filter(|r| r.kind == "checked")
        .all(|r| r.exit_code == 1));
    assert!(f.gate().is_ok());
}

#[test]
fn waived_finding_does_not_block_all_rejected_gate() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("rejected"));
    f.review(
        "a",
        "chronos:reviewer",
        json!([{"severity":"major"},{"severity":"major"}]),
    );
    f.add("waive", "2 accepted", 1, "a");
    assert_eq!(f.gate().unwrap_err().kind, "findings_rejected");
    assert!(orch::evidence(&f.store, "u1")
        .unwrap()
        .iter()
        .any(|r| r.kind == "checked" && r.exit_code == 0));
}

#[test]
fn absent_review_and_stale_verdict_do_not_call_verifier() {
    let _lock = CLAUDE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(reply("confirmed"));
    assert!(f.gate().is_ok());
    f.review("a", "chronos:reviewer", findings("major"));
    fs::write(
        orch::review_verdict_path(&f.store, "u1", "chronos:reviewer"),
        json!({"unit":"u1","tree":"other","verdict":"fail","findings":findings("major")})
            .to_string(),
    )
    .unwrap();
    assert!(f.check().is_empty());
    assert!(f.gate().is_ok());
    assert!(f.turns().is_empty());
}
