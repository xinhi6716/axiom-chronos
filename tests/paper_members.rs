mod common;

use axiom::{
    base::Failure,
    council::{
        self,
        paper_member::{PaperArgon, PaperFable},
        Status, AGY_PROMPT_LIMIT, ARGON_MODEL, FABLE_MODEL,
    },
    research_program::investigation::{MemberCall, ResearchMember},
};
use serde_json::{json, Value};
use std::{fs, path::Path, sync::atomic::AtomicBool, time::Duration};

fn command(dir: &Path, scenario: &str) -> Vec<String> {
    let mut command = common::fake("fake_paper_member");
    command.push(format!("--fake-log={}", dir.join("calls.jsonl").display()));
    command.push(format!("--fake-scenario={scenario}"));
    command
}
fn call(dir: &Path, prompt: &str) -> MemberCall {
    MemberCall {
        member: "test".into(),
        stage: "test".into(),
        prompt: prompt.into(),
        readable: vec![],
        out_dir: dir.to_path_buf(),
        schema: json!({"type":"object","required":["ok","echo_len"],"properties":{"ok":{"type":"boolean"},"echo_len":{"type":"integer"}}}),
        timeout_s: 3,
    }
}
fn logs(dir: &Path) -> Vec<Value> {
    fs::read_to_string(dir.join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn run(
    member: &dyn ResearchMember,
    call: &MemberCall,
) -> axiom::research_program::investigation::MemberReply {
    member.call(call, &AtomicBool::new(false))
}

#[test]
fn fable_ok_and_cumulative_resume_and_cap() {
    let dir = common::temp();
    let path = dir.path();
    let ledger = path.join("fable-meter.json");
    let fable = PaperFable::new(Ok(command(path, "ok")), ledger.clone(), 10.0);
    assert_eq!(fable.name(), "fable");
    let request = call(path, "hello 世界");
    let first = run(&fable, &request);
    assert_eq!(first.status, Status::Ok, "{:?}", first.detail);
    assert!(first.usd.unwrap() > 0.0);
    assert!(first.tokens > 0);
    assert_eq!(first.model.as_deref(), Some(FABLE_MODEL));
    assert_eq!(first.provider, "fable");
    assert!(first.raw.is_some());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path.join("schema.json")).unwrap()).unwrap(),
        request.schema
    );
    assert_eq!(logs(path)[0]["cwd"], path.to_string_lossy().as_ref());
    let second = run(&fable, &request);
    assert_eq!(second.status, Status::Ok);
    let before: Value = serde_json::from_slice(&fs::read(&ledger).unwrap()).unwrap();
    assert!(
        (before["usd"].as_f64().unwrap() - first.usd.unwrap() - second.usd.unwrap()).abs() < 1e-9
    );
    let resumed = PaperFable::new(Ok(command(path, "ok")), ledger.clone(), 10.0);
    let third = run(&resumed, &request);
    assert_eq!(third.status, Status::Ok);
    let after: Value = serde_json::from_slice(&fs::read(&ledger).unwrap()).unwrap();
    assert!(
        (after["usd"].as_f64().unwrap() - before["usd"].as_f64().unwrap() - third.usd.unwrap())
            .abs()
            < 1e-9
    );
    let capped = PaperFable::new(Ok(command(path, "ok")), ledger, 0.000001);
    let count = logs(path).len();
    assert_eq!(run(&capped, &request).status, Status::BudgetCut);
    assert_eq!(logs(path).len(), count);
}

#[test]
fn fable_errors() {
    for (scenario, expected) in [
        ("missing-usage", Status::BudgetCut),
        ("wrong-model", Status::Failed),
        ("garbage", Status::Malformed),
        ("hang", Status::Timeout),
        ("quota", Status::Quota),
    ] {
        let dir = common::temp();
        let mut request = call(dir.path(), "hello");
        request.timeout_s = 1;
        let fable = PaperFable::new(
            Ok(command(dir.path(), scenario)),
            dir.path().join("ledger.json"),
            10.0,
        );
        let reply = run(&fable, &request);
        assert_eq!(reply.status, expected, "{scenario}: {:?}", reply.detail);
        if scenario == "missing-usage" {
            assert!(reply.detail.unwrap().contains("cost cannot be bounded"));
            assert!(reply.output.is_none());
        }
    }
}

#[test]
fn argon_short_and_long_transport() {
    let dir = common::temp();
    let argon = PaperArgon::new(Ok(command(dir.path(), "ok")));
    assert_eq!(argon.name(), "argon");
    let short = call(dir.path(), "short");
    let reply = run(&argon, &short);
    assert_eq!(reply.status, Status::Ok, "{:?}", reply.detail);
    assert_eq!(reply.model.as_deref(), Some(ARGON_MODEL));
    assert_eq!(reply.provider, "argon");
    assert_eq!(reply.usd, None);
    assert!(reply.tokens > 0 && reply.raw.is_some());
    let first = logs(dir.path());
    let args = first[0]["argv"].as_array().unwrap();
    assert!(args.iter().any(|a| a == "--print"));
    assert!(!args.iter().any(|a| a == "--input-format"));
    let prompt = format!("{}\n末尾😀", "漢".repeat(40_000));
    assert!(prompt.encode_utf16().count() > AGY_PROMPT_LIMIT);
    let reply = run(&argon, &call(dir.path(), &prompt));
    assert_eq!(reply.status, Status::Ok, "{:?}", reply.detail);
    assert_eq!(reply.output.unwrap()["echo_len"], prompt.chars().count());
    assert_eq!(reply.model.as_deref(), Some(ARGON_MODEL));
    assert_eq!(reply.usd, None);
    assert!(reply.tokens > 0);
    let all = logs(dir.path());
    assert_eq!(
        all[1]["prompt"].as_str().unwrap().as_bytes(),
        prompt.as_bytes()
    );
    let args = all[1]["argv"].as_array().unwrap();
    assert!(args
        .windows(2)
        .any(|a| a[0] == "--model" && a[1] == ARGON_MODEL));
    assert!(args
        .windows(2)
        .any(|a| a[0] == "--input-format" && a[1] == "stream-json"));
    assert_eq!(all[1]["cwd"], dir.path().to_string_lossy().as_ref());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(dir.path().join("schema.json")).unwrap())
            .unwrap(),
        short.schema
    );
}

#[test]
fn argon_errors_and_legacy_limit() {
    for (scenario, expected) in [
        ("garbage", Status::Malformed),
        ("quota", Status::Quota),
        ("hang", Status::Timeout),
    ] {
        let dir = common::temp();
        let mut request = call(dir.path(), &"漢".repeat(AGY_PROMPT_LIMIT + 1));
        request.timeout_s = 1;
        let reply = run(
            &PaperArgon::new(Ok(command(dir.path(), scenario))),
            &request,
        );
        assert_eq!(reply.status, expected, "{scenario}: {:?}", reply.detail);
    }
    let dir = common::temp();
    let request = call(dir.path(), "test");
    let missing = PaperArgon::new(Err(Failure::new("not_installed", "not installed")));
    assert_eq!(run(&missing, &request).status, Status::Missing);
    let prompt = "漢".repeat(AGY_PROMPT_LIMIT + 1);
    let old = council::ask_agy(
        &Ok(command(dir.path(), "ok")),
        &prompt,
        dir.path(),
        &request.schema,
        &dir.path().join("schema.json"),
        Duration::from_secs(1),
        &AtomicBool::new(false),
    );
    assert_eq!(old.status, Status::Failed);
    assert!(old.detail.unwrap().contains("too long for a command line"));
    assert!(logs(dir.path()).is_empty());
}
