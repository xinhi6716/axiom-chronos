mod common;
use axiom::{
    base::Failure,
    modes::{
        plaza::router::Router,
        providers::{Ask, Member, Provider},
        runners::{MemberCommands, ModelProvider, ModelRouter},
        Mode,
    },
};
use std::fs;
fn commands(script: &std::path::Path) -> MemberCommands {
    let cmd = Ok(vec![
        common::python(),
        "-I".into(),
        script.to_string_lossy().into_owned(),
    ]);
    MemberCommands {
        claude: cmd.clone(),
        agy: cmd.clone(),
        codex: cmd,
    }
}
fn ask(member: Member) -> Ask {
    Ask {
        member,
        mode: Mode::Chat,
        session: "test".into(),
        prompt: "hello".into(),
        context: "context".into(),
        tools: vec![],
    }
}
fn fake(dir: &std::path::Path, result: &str) -> std::path::PathBuf {
    let path = dir.join("fake.py");
    fs::write(
        &path,
        format!("import json\nprint(json.dumps({}))\n", result),
    )
    .unwrap();
    path
}
#[test]
fn configured_model_called() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'text':'hello','tool_requests':[]}), 'usage':{'input_tokens':10,'output_tokens':5}}"#,
    );
    let r = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Opus))
        .unwrap();
    assert_eq!(r.text, "hello");
    assert_eq!(r.tokens, Some(15));
}
#[test]
fn routing() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'picks':[{'member':'opus','reason':'expert'}]})}"#,
    );
    let r = ModelRouter::with_commands(d.path().into(), commands(&p))
        .route("hello", "")
        .unwrap();
    assert_eq!(r.picks[0].member, Member::Opus);
}
#[test]
fn missing_model() {
    let d = tempfile::tempdir().unwrap();
    let missing = Err(Failure::new("not_installed", "missing"));
    let c = MemberCommands {
        claude: missing.clone(),
        agy: missing.clone(),
        codex: missing,
    };
    let e = ModelProvider::with_commands(d.path().into(), c)
        .ask(&ask(Member::Opus))
        .unwrap_err();
    assert_eq!(e.kind, "model_missing");
    assert!(e.message.contains("opus"));
}
#[test]
fn malformed_reply() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(d.path(), r#"{'type':'result','result':'not json'}"#);
    let e = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Opus))
        .unwrap_err();
    assert_eq!(e.kind, "malformed_reply");
}
#[test]
fn exhausted_pool() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(d.path(), r#"{}"#);
    axiom::usage::pools::mark_exhausted(
        d.path(),
        axiom::usage::pools::Pool::Argon,
        i64::MAX,
        "test",
    )
    .unwrap();
    let e = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Argon))
        .unwrap_err();
    assert_eq!(e.kind, "quota_exhausted");
}
#[test]
fn argon_charged() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'response':{'text':'argon answer','tool_requests':[]}, 'usage':{'total_tokens':42}}"#,
    );
    let r = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Argon))
        .unwrap();
    assert_eq!(r.text, "argon answer");
    assert_eq!(r.tokens, Some(42));
    let status = axiom::usage::pools::status(d.path(), 0).unwrap();
    assert_eq!(
        status
            .iter()
            .find(|s| s.pool == axiom::usage::pools::Pool::Argon)
            .unwrap()
            .used_tokens,
        Some(42)
    );
}
#[test]
fn provider_quota_marks_argon_pool() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fake.py");
    fs::write(
        &p,
        "import sys\nprint('quota exhausted', file=sys.stderr)\nsys.exit(1)\n",
    )
    .unwrap();
    let e = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Argon))
        .unwrap_err();
    assert_eq!(e.kind, "quota_exhausted");
    assert!(axiom::usage::pools::status(d.path(), 0)
        .unwrap()
        .iter()
        .find(|s| s.pool == axiom::usage::pools::Pool::Argon)
        .unwrap()
        .exhausted_until_ms
        .is_some());
}
#[test]
fn rejects_ungranted_or_excessive_tools() {
    for requests in [
        r#"[{'tool':'write_file','args':{}}]"#,
        r#"[{'tool':'read_file','args':{}}]*4"#,
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = fake(d.path(), &format!("{{'type':'result','result':json.dumps({{'text':'hello','tool_requests':{requests}}})}}"));
        let mut a = ask(Member::Fable);
        a.tools = vec!["read_file".into(), "write_file".into()];
        assert_eq!(
            ModelProvider::with_commands(d.path().into(), commands(&p))
                .ask(&a)
                .unwrap_err()
                .kind,
            "malformed_reply"
        );
    }
}
#[test]
fn budget_prevents_call() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'text':'hello','tool_requests':[]})}"#,
    );
    axiom::engine::policy::set(&d.path().join("modes"), "budget.core", "0").unwrap();
    assert_eq!(
        ModelProvider::with_commands(d.path().into(), commands(&p))
            .ask(&ask(Member::Opus))
            .unwrap_err()
            .kind,
        "budget_exceeded"
    );
}
#[test]
fn astra_uses_codex_appserver_read_only() {
    let d = tempfile::tempdir().unwrap();
    let fake = d.path().join("codex.py");
    fs::copy(common::repo_root().join("tests/fakes/fake_codex.py"), &fake).unwrap();
    let script = d.path().join("script.json");
    let log = d.path().join("log.jsonl");
    fs::write(&script, serde_json::json!([{"match":"hello", "reply":serde_json::json!({"text":"astra answer","tool_requests":[]}).to_string()}]).to_string()).unwrap();
    let mut cmds = commands(&fake);
    cmds.codex = Ok(vec![
        common::python(),
        "-I".into(),
        fake.to_string_lossy().into_owned(),
        format!("--fake-script={}", script.display()),
        format!("--fake-log={}", log.display()),
    ]);
    let r = ModelProvider::with_commands(d.path().into(), cmds)
        .ask(&ask(Member::Astra))
        .unwrap();
    assert_eq!(r.text, "astra answer");
    let events: Vec<serde_json::Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let start = events
        .iter()
        .find(|e| e["method"] == "thread/start")
        .unwrap();
    assert_eq!(start["params"]["sandbox"], "read-only");
    assert_eq!(start["params"]["model"], "gpt-6-astra");
}
#[test]
fn routing_rejects_duplicate_members() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'picks':[{'member':'opus','reason':'one'},{'member':'opus','reason':'two'}]})}"#,
    );
    assert_eq!(
        ModelRouter::with_commands(d.path().into(), commands(&p))
            .route("hi", "")
            .unwrap_err()
            .kind,
        "malformed_reply"
    );
}
#[test]
fn bounds_context_and_disables_native_claude_tools() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fake.py");
    let log = d.path().join("input.json");
    fs::write(&p, format!("import json,sys\nwith open({}, 'w', encoding='utf-8') as f: json.dump({{'argv':sys.argv,'prompt':sys.stdin.read()}},f)\nprint(json.dumps({{'type':'result','result':json.dumps({{'text':'ok','tool_requests':[]}})}}))\n", serde_json::to_string(&log.to_string_lossy().to_string()).unwrap())).unwrap();
    let mut a = ask(Member::Fable);
    a.context = "x".repeat(50_000);
    a.tools = vec!["write_file".into(), "read_file".into()];
    ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&a)
        .unwrap();
    let input: serde_json::Value = serde_json::from_slice(&fs::read(log).unwrap()).unwrap();
    let prompt = input["prompt"].as_str().unwrap();
    let context = prompt
        .split("\nContext:\n")
        .nth(1)
        .unwrap()
        .split("\nUser:\n")
        .next()
        .unwrap();
    assert_eq!(context.len(), 24_000);
    let argv = input["argv"].as_array().unwrap();
    let index = argv.iter().position(|v| v == "--tools").unwrap();
    assert_eq!(argv[index + 1], "");
    assert!(!prompt.contains("write_file"));
}
#[test]
fn project_role_budget_is_checked() {
    let d = tempfile::tempdir().unwrap();
    let project = d.path().join("my-project");
    fs::create_dir(&project).unwrap();
    let session = axiom::modes::runs::create_session(d.path(), Some(&project), &[]).unwrap();
    let store = axiom::orch::store_for(d.path(), "my-project").unwrap();
    axiom::engine::policy::set(&store, "budget.core", "0").unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'text':'hello','tool_requests':[]})}"#,
    );
    let mut a = ask(Member::Opus);
    a.session = session.id;
    assert_eq!(
        ModelProvider::with_commands(d.path().into(), commands(&p))
            .ask(&a)
            .unwrap_err()
            .kind,
        "budget_exceeded"
    );
}
#[test]
fn fable_reports_cost_and_runtime_command_is_used() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'text':'fable','tool_requests':[]}), 'usage':{'input_tokens':100,'output_tokens':50}}"#,
    );
    let r = ModelProvider::with_commands(d.path().into(), commands(&p))
        .ask(&ask(Member::Fable))
        .unwrap();
    assert_eq!(r.tokens, Some(150));
    assert!(r.cost_usd.unwrap() > 0.0);
    let command = vec!["runtime-codex".to_string(), "arg".to_string()];
    axiom::codex::slots::save_runtime(
        d.path(),
        &axiom::codex::slots::Runtime {
            codex_max_parallel: 2,
            codex_command: command.clone(),
        },
    )
    .unwrap();
    assert_eq!(MemberCommands::from_env(d.path()).codex.unwrap(), command);
}
#[test]
fn provider_reset_time_is_preserved() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fake.py");
    fs::write(&p, "import json,sys\nprint(json.dumps({'error':'quota exhausted','reset_at_ms':4102444800000}))\nsys.exit(1)\n").unwrap();
    assert_eq!(
        ModelProvider::with_commands(d.path().into(), commands(&p))
            .ask(&ask(Member::Argon))
            .unwrap_err()
            .kind,
        "quota_exhausted"
    );
    let s = axiom::usage::pools::status(d.path(), 0).unwrap();
    assert_eq!(
        s.iter()
            .find(|s| s.pool == axiom::usage::pools::Pool::Argon)
            .unwrap()
            .exhausted_until_ms,
        Some(4102444800000)
    );
}
#[test]
fn missing_executable_identifies_member() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("absent");
    let e = ModelProvider::with_commands(
        d.path().into(),
        MemberCommands {
            claude: Ok(vec![p.to_string_lossy().into_owned()]),
            agy: Ok(vec![p.to_string_lossy().into_owned()]),
            codex: Ok(vec![p.to_string_lossy().into_owned()]),
        },
    )
    .ask(&ask(Member::Fable))
    .unwrap_err();
    assert_eq!(e.kind, "model_missing");
    assert!(e.message.contains("fable"));
}
#[test]
fn malformed_but_metered_reply_is_charged() {
    let d = tempfile::tempdir().unwrap();
    let p = fake(
        d.path(),
        r#"{'type':'result','result':json.dumps({'wrong':'shape'}), 'usage':{'input_tokens':100,'output_tokens':50}}"#,
    );
    assert_eq!(
        ModelProvider::with_commands(d.path().into(), commands(&p))
            .ask(&ask(Member::Fable))
            .unwrap_err()
            .kind,
        "malformed_reply"
    );
    let spent = axiom::engine::budgets::spent(
        &d.path().join("modes"),
        axiom::engine::budgets::Role::CouncilMember,
    )
    .unwrap();
    assert!(spent.usd > 0.0);
}
