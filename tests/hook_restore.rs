use axiom::{goal, hook, hook::context, orch};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

static ENV: Mutex<()> = Mutex::new(());
const MAP: &str = "## m1\nPURPOSE one\nPATHS src/m1/\nDEPENDS none\nCONTRACT\n  keep 原樣  spaces\n  next line\nACCEPT tests/m1.rs, `cargo test`\n\n## m2\nPURPOSE two\nPATHS src/m2/\nDEPENDS none\nCONTRACT\n  private second contract\nACCEPT tests/m2.rs, `cargo test`\n";

struct Vars(Option<OsString>, Option<OsString>);
impl Vars {
    fn defaults() -> Self {
        let saved = Self(
            std::env::var_os(context::HANDOFF_ENV),
            std::env::var_os(context::WARN_ENV),
        );
        std::env::remove_var(context::HANDOFF_ENV);
        std::env::remove_var(context::WARN_ENV);
        saved
    }
}
impl Drop for Vars {
    fn drop(&mut self) {
        for (key, value) in [
            (context::HANDOFF_ENV, &self.0),
            (context::WARN_ENV, &self.1),
        ] {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}
struct Run {
    _dir: tempfile::TempDir,
    home: PathBuf,
    store: PathBuf,
    main: PathBuf,
    own: PathBuf,
}
fn run(wish: &str) -> Run {
    let root = Path::new(".axiom-tmp/hook-wiring-1");
    fs::create_dir_all(root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let home = dir.path().join("home");
    let project = dir.path().join("demo");
    fs::create_dir_all(&project).unwrap();
    goal::open(&home, "s", wish, Some(project.to_str().unwrap())).unwrap();
    goal::set_mode(&home, "s", "change").unwrap();
    let store = orch::store_for(&home, "demo").unwrap();
    let init = orch::run(&store, &["init".into()], false, false);
    assert_eq!(init.exit_code, 0, "{}", init.stderr);
    fs::write(store.join("modules.md"), MAP).unwrap();
    fs::create_dir_all(store.join("agents")).unwrap();
    fs::write(store.join("agents/d1"), "m1").unwrap();
    let main = dir.path().join("projects/sess.jsonl");
    let own = dir.path().join("projects/sess/subagents/agent-d1.jsonl");
    fs::create_dir_all(own.parent().unwrap()).unwrap();
    fs::write(&main, "").unwrap();
    fs::write(&own, assistant(30_000)).unwrap();
    Run {
        _dir: dir,
        home,
        store,
        main,
        own,
    }
}
fn assistant(n: u64) -> String {
    format!(
        "{}\n",
        json!({"type":"assistant","message":{"model":"claude","usage":{"input_tokens":10,"cache_creation_input_tokens":90,"cache_read_input_tokens":n-100,"output_tokens":7}}})
    )
}
fn boundary(id: usize) -> String {
    format!(
        "{}\n",
        json!({"type":"system","subtype":"compact_boundary","uuid":format!("c{id}")})
    )
}
fn event(r: &Run, tool: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":tool,"tool_input":input,"transcript_path":r.main,"agent_id":"d1","agent_type":"chronos:dispatcher"})
}
fn call(r: &Run, e: &Value) -> Option<Value> {
    hook::handle(e, Some(&r.home), false)
}
fn text(v: &Value) -> &str {
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
}
fn assert_anchor(r: &Run, value: &Value) {
    let s = text(value);
    let rules = orch::dispatcher_rules();
    let contract = orch::contract_block(MAP, "m1").unwrap();
    assert!(s.contains(&format!(
        "BINDING RULES, verbatim from agents/dispatcher.md:\n{rules}\n"
    )));
    assert!(s.contains(&format!(
        "CONTRACT of module m1, verbatim from {}:\n{contract}\n",
        r.store.join("modules.md").display()
    )));
    assert!(!s.contains("private second contract"));
    assert!(value["hookSpecificOutput"]
        .get("permissionDecision")
        .is_none());
}

#[test]
fn boundaries_restore_verbatim_each_time_and_session_start_marks_seen() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let r = run("wish");
    let read = event(&r, "Read", json!({"file_path":"missing"}));
    let mut lines = assistant(30_000);
    let mut first = String::new();
    for n in 1..=5 {
        lines.push_str(&boundary(n));
        fs::write(&r.own, &lines).unwrap();
        let out = call(&r, &read).unwrap();
        assert_anchor(&r, &out);
        if n == 1 {
            first = text(&out).to_owned();
            let anchor = orch::anchor_text(
                &r.store,
                &orch::Anchor {
                    rules: true,
                    wish: None,
                    wish_file: None,
                    module: Some("m1"),
                },
            );
            let resume = orch::resume_text(&r.store, Some(&r._dir.path().join("demo")), Some("m1"))
                .unwrap();
            assert_eq!(first, format!("Your context was compacted. Your binding rules and your module's contract follow verbatim; they bind you as before.\n{anchor}Your work state, from the engine:\n{resume}"));
        }
        if n == 5 {
            assert_eq!(first, text(&out));
        }
        assert!(call(&r, &read).is_none());
    }
    lines.push_str(&boundary(6));
    fs::write(&r.own, &lines).unwrap();
    let session = json!({"hook_event_name":"SessionStart","session_id":"s","source":"compact","transcript_path":r.main,"agent_id":"d1","agent_type":"chronos:dispatcher"});
    let out = call(&r, &session).unwrap();
    assert_anchor(&r, &out);
    assert_eq!(
        text(&out)
            .matches("Your work state, from the engine")
            .count(),
        0
    );
    assert_eq!(text(&out).matches("work state, from the engine").count(), 1);
    assert_eq!(text(&out).matches(&orch::dispatcher_rules()).count(), 1);
    assert_eq!(
        text(&out)
            .matches(&orch::contract_block(MAP, "m1").unwrap())
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(r.store.join("agents/d1.compact")).unwrap(),
        "c6"
    );
    assert!(call(&r, &read).is_none());
    assert!(!text(&call(&r, &session).unwrap()).contains("BINDING RULES"));
    let mut outside = session.clone();
    outside["agent_type"] = json!("chronos:reviewer");
    lines.push_str(&boundary(7));
    fs::write(&r.own, &lines).unwrap();
    assert!(!text(&call(&r, &outside).unwrap()).contains("BINDING RULES"));
    assert_anchor(&r, &call(&r, &read).unwrap());
}

#[test]
fn seen_boundary_with_invalid_handoff_returns_none() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let r = run("wish");
    let read = event(&r, "Read", json!({"file_path":"missing"}));
    fs::write(&r.own, format!("{}{}", assistant(30_000), boundary(1))).unwrap();
    assert_anchor(&r, &call(&r, &read).unwrap());

    let handoff = r.store.join("handoffs/module/m1.json");
    fs::create_dir_all(handoff.parent().unwrap()).unwrap();
    fs::write(&handoff, b"invalid handoff").unwrap();
    assert!(orch::resume_text(&r.store, Some(&r._dir.path().join("demo")), Some("m1")).is_err());
    assert!(call(&r, &read).is_none());
    assert_eq!(
        fs::read_to_string(r.store.join("agents/d1.compact")).unwrap(),
        "c1"
    );
}

#[test]
fn missing_project_store_does_not_add_an_anchor() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let r = run("wish");
    fs::remove_dir_all(&r.store).unwrap();
    fs::write(&r.own, format!("{}{}", assistant(30_000), boundary(1))).unwrap();
    let session = json!({"hook_event_name":"SessionStart","session_id":"s","source":"compact","transcript_path":r.main,"agent_id":"d1","agent_type":"chronos:dispatcher"});
    let out = call(&r, &session).unwrap();
    assert!(!text(&out).contains("BINDING RULES"));
    assert!(!r.store.exists());
    let read = event(&r, "Read", json!({"file_path":"missing"}));
    assert!(call(&r, &read).is_none());
    assert!(!r.store.exists());
}

fn handoff(r: &Run, module: Option<&str>) {
    let mut args = vec!["handoff".to_string(), "--next".into(), "continue".into()];
    if let Some(m) = module {
        args.extend(["--module".into(), m.into()]);
    }
    let out = orch::run(&r.store, &args, false, false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
}
fn start(r: &Run, prompt: &str) -> String {
    let e = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Agent","tool_input":{"subagent_type":"chronos:dispatcher","description":"m1","prompt":prompt}});
    call(r, &e).unwrap()["hookSpecificOutput"]["updatedInput"]["prompt"]
        .as_str()
        .unwrap()
        .to_owned()
}
#[test]
fn successor_gets_goal_wish_and_target_contract_once() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let wish = "完成 中文\n  保留內部  空白";
    let r = run(wish);
    handoff(&r, Some("m1"));
    let prompt = start(&r, "Carry out m1.");
    let contract = orch::contract_block(MAP, "m1").unwrap();
    assert!(prompt.contains(&format!("WISH, verbatim:\n{wish}\n")));
    assert_eq!(prompt.matches(&contract).count(), 1);
    assert!(!prompt.contains("private second contract"));
    fs::write(
        &r.own,
        format!(
            "{}\n{}",
            json!({"type":"user","message":{"content":prompt}}),
            assistant(30_000)
        ),
    )
    .unwrap();
    assert!(call(&r, &event(&r, "Read", json!({"file_path":"missing"}))).is_none());
    assert!(!r.store.join("agents/d1.compact").exists());
}
#[test]
fn whole_wish_and_oversized_wish() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let wish = format!("{}尾", "x".repeat(4096));
    let r = run(&wish);
    handoff(&r, None);
    let prompt = start(&r, "Carry out the whole wish.");
    assert!(prompt.contains("INCOMPLETE: wish is cut at 4096 of 4099 bytes"));
    assert!(prompt.contains(&r.home.join("goals").join("s.json").display().to_string()));
    assert!(!prompt.contains("CONTRACT of module"));
}

#[test]
fn warnings_cover_gated_calls_and_never_override_denials() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _vars = Vars::defaults();
    let r = run("wish");
    let read = event(&r, "Read", json!({"file_path":"missing"}));
    for n in [119_999, 120_000, 149_999, 150_000] {
        fs::write(&r.own, format!("{}{}", assistant(30_000), assistant(n))).unwrap();
        for tool in ["Read", "Grep", "Glob"] {
            let mut e = read.clone();
            e["tool_name"] = json!(tool);
            let out = call(&r, &e);
            match n {
                119_999 => assert!(out.is_none(), "{tool}"),
                150_000 => assert_eq!(
                    out.unwrap()["hookSpecificOutput"]["permissionDecision"],
                    "deny"
                ),
                _ => {
                    let expected = context::handoff_warning(&e, 150_000, 80).unwrap();
                    assert_eq!(text(&out.unwrap()), expected);
                    assert_eq!(text(&call(&r, &e).unwrap()), expected);
                }
            }
        }
    }
    fs::write(
        &r.own,
        format!("{}{}{}", assistant(30_000), boundary(1), assistant(120_000)),
    )
    .unwrap();
    let out = call(&r, &read).unwrap();
    assert_anchor(&r, &out);
    assert!(text(&out).ends_with(&context::handoff_warning(&read, 150_000, 80).unwrap()));
    let forbidden = event(
        &r,
        "Write",
        json!({"file_path":r.home.join("goals/s.json")}),
    );
    assert_eq!(
        call(&r, &forbidden).unwrap()["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    std::env::set_var(context::WARN_ENV, "90");
    assert!(call(&r, &read).is_none());
    fs::write(
        &r.own,
        format!("{}{}", assistant(30_000), assistant(135_000)),
    )
    .unwrap();
    assert!(call(&r, &read).is_some());
    std::env::set_var(context::WARN_ENV, "0");
    assert!(call(&r, &read).is_none());
    std::env::set_var(context::WARN_ENV, "invalid");
    assert!(call(&r, &read).is_some());
    std::env::set_var(context::HANDOFF_ENV, "0");
    assert!(call(&r, &read).is_none());
    std::env::remove_var(context::HANDOFF_ENV);
    let mut reviewer = read.clone();
    reviewer["agent_type"] = json!("chronos:reviewer");
    assert!(call(&r, &reviewer).is_none());
}
