use axiom::{goal, hook, hook::context, orch};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";

fn section(id: &str) -> String {
    format!("## {id}\nPURPOSE purpose\nPATHS src/{id}/\nDEPENDS none\nCONTRACT\n  contract\nACCEPT tests/{id}.rs, `cargo test`\n")
}

struct Run {
    _dir: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    store: PathBuf,
    main: PathBuf,
    own: PathBuf,
}

/// A change goal for project `demo` with modules m1 and m2, and a dispatcher `d1` whose
/// transcript lives where Claude Code keeps subagent transcripts.
fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let project = dir.path().join("demo");
    fs::create_dir_all(&project).unwrap();
    goal::open(&home, "s", "wish", Some(project.to_str().unwrap())).unwrap();
    goal::set_mode(&home, "s", "change").unwrap();
    let store = orch::store_for(&home, "demo").unwrap();
    assert_eq!(
        orch::run(&store, &["init".to_string()], false, false).exit_code,
        0
    );
    fs::write(
        store.join("modules.md"),
        format!("{}\n{}", section("m1"), section("m2")),
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na1\tm1\tfailed\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n",
    )
    .unwrap();
    let main = dir.path().join("projects").join("sess.jsonl");
    let own = dir.path().join("projects/sess/subagents/agent-d1.jsonl");
    fs::create_dir_all(own.parent().unwrap()).unwrap();
    fs::write(&main, assistant(900_000)).unwrap();
    Run {
        _dir: dir,
        home,
        project,
        store,
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
fn codex(r: &Run) -> Value {
    in_dispatcher(tool(
        r,
        CODEX,
        json!({"project":"demo","unit":"a1","role":"terra","cwd":r.project}),
    ))
}
fn reviewer(r: &Run) -> Value {
    in_dispatcher(tool(
        r,
        "Agent",
        json!({"subagent_type":"chronos:reviewer","prompt":"review a1"}),
    ))
}
fn call(r: &Run, e: &Value) -> Option<Value> {
    hook::handle(e, Some(&r.home), false)
}
fn refusal(r: &Run, e: &Value) -> Option<String> {
    call(r, e).and_then(|v| {
        (v["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
            v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .to_string()
        })
    })
}
fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}
fn handoff(r: &Run, module: &str, next: &str) {
    let out = orch::run(
        &r.store,
        &args(&[
            "handoff",
            "--module",
            module,
            "--repo",
            r.project.to_str().unwrap(),
            "--next",
            next,
        ]),
        false,
        false,
    );
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
}

#[test]
fn a_dispatcher_past_the_threshold_is_refused_with_the_handoff_instruction() {
    let r = run();
    transcript(&r.own, &[30_000, 120_000, 210_000]);
    let reason = refusal(&r, &codex(&r)).expect("codex_run past 150k is refused");
    assert!(
        reason.contains("210k tokens, past the 150k handoff limit"),
        "{reason}"
    );
    assert!(
        reason.contains(r#"["handoff", "--module", "m1", "--repo""#),
        "{reason}"
    );
    assert!(reason.contains("then return your report"), "{reason}");
    assert!(refusal(&r, &reviewer(&r)).is_some_and(|why| why.contains("handoff")));
    let read = in_dispatcher(tool(
        &r,
        "Read",
        json!({"file_path": r.store.join("units.tsv")}),
    ));
    assert!(refusal(&r, &read).is_some_and(|why| why.contains("handoff")));
    let threshold = |limit| context::handoff_refusal(&codex(&r), limit, None);
    assert!(
        threshold(300_000).is_none(),
        "the threshold is configurable"
    );
    assert!(threshold(100_000).is_some());
}

#[test]
fn no_refusal_below_the_threshold_for_the_core_or_for_other_agents() {
    let r = run();
    transcript(&r.own, &[30_000, 149_000]);
    assert_eq!(refusal(&r, &codex(&r)), None);
    assert_eq!(refusal(&r, &reviewer(&r)), None);

    transcript(&r.own, &[30_000, 400_000]);
    let core = tool(
        &r,
        "Agent",
        json!({"subagent_type":"chronos:dispatcher","prompt":"module m2"}),
    );
    assert_eq!(
        refusal(&r, &core),
        None,
        "the core is never refused for its context"
    );

    let mut astra = codex(&r);
    astra["agent_type"] = json!("chronos:astra");
    astra["tool_input"] =
        json!({"project":"demo","role":"astra","brief":"b","cwd":r.project,"sandbox":"read-only"});
    assert_eq!(refusal(&r, &astra), None, "only dispatchers hand off");

    transcript(&r.own, &[150_000, 400_000]);
    assert_eq!(
        refusal(&r, &codex(&r)),
        None,
        "a dispatcher that started near the limit would hand off forever"
    );
}

#[test]
fn a_malformed_or_missing_transcript_never_blocks_work() {
    let r = run();
    let garbage: &[&[u8]] = &[
        b"",
        b"not json at all\n{\"type\":\"assistant\"\n",
        b"{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":\"many\"}}}\n",
        b"{\"type\":\"assistant\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":900000,\"cache_read",
        b"{\"type\":\"assistant\",\"message\":{\"model\":\"<synthetic>\",\"usage\":{\"input_tokens\":900000}}}\n",
        &[0xff, 0xfe, 0x00, b'\n', 0x80, 0x81],
    ];
    for bytes in garbage {
        fs::write(&r.own, bytes).unwrap();
        assert_eq!(refusal(&r, &codex(&r)), None);
    }
    fs::remove_file(&r.own).unwrap();
    assert_eq!(refusal(&r, &codex(&r)), None, "missing subagent transcript");
    let mut no_path = codex(&r);
    no_path.as_object_mut().unwrap().remove("transcript_path");
    assert_eq!(refusal(&r, &no_path), None);
    let mut dir_path = codex(&r);
    dir_path["transcript_path"] = json!(r.project);
    assert_eq!(refusal(&r, &dir_path), None);
}

#[test]
fn a_large_transcript_is_read_by_its_tail_only() {
    let r = run();
    let mut file = fs::File::create(&r.own).unwrap();
    file.write_all(assistant(900_000).as_bytes()).unwrap();
    let filler = format!(
        "{}\n",
        json!({"type":"user","message":{"content":"x".repeat(4000)}})
    );
    let mut written = 0u64;
    while written <= context::TAIL_BYTES + 64 * 1024 {
        file.write_all(filler.as_bytes()).unwrap();
        written += filler.len() as u64;
    }
    drop(file);
    assert_eq!(
        context::last_context(&r.own),
        None,
        "the only assistant line sits before the tail and must not be read"
    );
    assert_eq!(refusal(&r, &codex(&r)), None);
    let (start, tail) = context::read_tail(&r.own, context::TAIL_BYTES).unwrap();
    assert!(start > 0);
    assert!(tail.len() as u64 <= context::TAIL_BYTES);
    let first = tail.split(|b| *b == b'\n').next().unwrap();
    assert!(
        serde_json::from_slice::<Value>(first).is_ok(),
        "the tail starts at a line boundary"
    );

    fs::OpenOptions::new()
        .append(true)
        .open(&r.own)
        .unwrap()
        .write_all(assistant(230_000).as_bytes())
        .unwrap();
    assert_eq!(context::last_context(&r.own), Some(230_000));
}

#[test]
fn a_handoff_is_written_and_read_back_intact() {
    let r = run();
    let repo = r.project.to_str().unwrap();
    let fields = [
        (
            "--next",
            "dispatch fix unit a1-fix with the VERIFY output of a1",
        ),
        ("--in-flight", "a1 failed VERIFY; b1 untouched"),
        ("--stuck", "a1: the parser rejects CRLF briefs (換行)"),
        (
            "--decisions",
            "kept the old flag name; logged in decisions.md",
        ),
    ];
    let mut call = args(&["handoff", "--module", "m1", "--repo", repo]);
    for (flag, value) in fields {
        call.extend(args(&[flag, value]));
    }
    let out = orch::run(&r.store, &call, true, false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let note = orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap();
    assert_eq!(note.module.as_deref(), Some("m1"));
    assert_eq!(note.repo.as_deref(), Some(repo));
    assert_eq!(note.next, fields[0].1);
    assert_eq!(note.in_flight, fields[1].1);
    assert_eq!(note.stuck, fields[2].1);
    assert_eq!(note.decisions, fields[3].1);
    assert!(!note.taken);

    let resume = orch::run(&r.store, &args(&["resume", "--module", "m1"]), false, false);
    assert_eq!(resume.exit_code, 0, "{}", resume.stderr);
    for (_, value) in fields {
        assert!(
            resume.stdout.contains(value),
            "{value} missing from {}",
            resume.stdout
        );
    }
    assert!(
        resume.stdout.contains("DIGEST\nunits: 1 (failed 1)"),
        "{}",
        resume.stdout
    );
    assert!(
        !resume.stdout.contains("b1  pending"),
        "another module's units stay out"
    );
    let report = orch::digest(&r.store, &r.project, Some("m1")).unwrap();
    assert!(report.contains("handoff pending (m1): next: dispatch fix unit a1-fix"));

    for (bad, why) in [
        (
            args(&["handoff", "--module", "m1", "--next", &"n".repeat(401)]),
            "the limit is 400",
        ),
        (
            args(&[
                "handoff",
                "--module",
                "m1",
                "--next",
                "n",
                "--stuck",
                &"s".repeat(601),
            ]),
            "the limit is 600",
        ),
        (
            args(&["handoff", "--module", "m9", "--next", "n"]),
            "not in the module map",
        ),
        (args(&["handoff", "--module", "m1"]), "--next is required"),
        (
            args(&["handoff", "--module", "m1", "--next", "n", "--notes", "x"]),
            "unknown handoff option",
        ),
        (
            args(&["handoff", "--module", "m1", "--next", "n", "--next", "m"]),
            "given twice",
        ),
    ] {
        let out = orch::run(&r.store, &bad, false, false);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains(why), "{}", out.stderr);
    }
    assert_eq!(
        orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap(),
        note
    );

    fs::write(r.store.join("decisions.md"), "d".repeat(40_000)).unwrap();
    handoff(&r, "m2", "start b1");
    let whole = orch::resume_text(&r.store, Some(&r.project), None).unwrap();
    assert!(whole.chars().count() <= orch::MAX_RESUME, "{}", whole.len());
    assert!(whole.contains("next: start b1") && whole.contains("cut; run orch report"));
}

#[test]
fn a_fresh_dispatcher_prompt_carries_the_digest_and_the_handoff() {
    let r = run();
    handoff(&r, "m1", "dispatch fix unit a1-fix");
    let start = |prompt: &str| {
        tool(
            &r,
            "Agent",
            json!({"subagent_type":"chronos:dispatcher","description":"m1","prompt":prompt}),
        )
    };
    assert!(
        call(&r, &start("Carry out module m12 only.")).is_none(),
        "m12 is not m1"
    );
    assert!(call(&r, &start("Carry out module m2.")).is_none());

    let out = call(&r, &start("Carry out module m1. Project dir given.")).unwrap();
    let hso = &out["hookSpecificOutput"];
    assert_eq!(hso["permissionDecision"], "allow");
    let updated = &hso["updatedInput"];
    assert_eq!(updated["subagent_type"], "chronos:dispatcher");
    assert_eq!(updated["description"], "m1");
    let prompt = updated["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("Carry out module m1. Project dir given."));
    assert!(prompt.contains("HANDOFF for module m1"));
    assert!(prompt.contains("next: dispatch fix unit a1-fix"));
    assert!(prompt.contains("DIGEST\nunits: 1 (failed 1)"));
    assert!(prompt.contains("a1  failed"));

    dispatcher_runs(&r, prompt);
    assert!(
        orch::read_handoff(&r.store, Some("m1"))
            .unwrap()
            .unwrap()
            .taken
    );
    assert!(
        call(&r, &start("Carry out module m1.")).is_none(),
        "a handoff is given to one fresh dispatcher"
    );
}

fn start_with(r: &Run, description: &str, prompt: &str) -> Value {
    tool(
        r,
        "Agent",
        json!({"subagent_type":"chronos:dispatcher","description":description,"prompt":prompt}),
    )
}
fn given_prompt(out: &Value) -> String {
    out["hookSpecificOutput"]["updatedInput"]["prompt"]
        .as_str()
        .unwrap()
        .to_string()
}
fn pending_modules(r: &Run) -> Vec<String> {
    orch::pending_handoffs(&r.store)
        .unwrap()
        .into_iter()
        .filter_map(|h| h.module)
        .collect()
}
/// The dispatcher `d1` runs: its transcript opens with the prompt it was started with, then it reads the store.
fn dispatcher_runs(r: &Run, prompt: &str) {
    let opening = json!({"type":"user","message":{"role":"user","content":prompt}});
    fs::write(&r.own, format!("{opening}\n{}", assistant(20_000))).unwrap();
    let read = in_dispatcher(tool(
        r,
        "Read",
        json!({"file_path": r.store.join("units.tsv")}),
    ));
    assert_eq!(refusal(r, &read), None);
}

#[test]
fn only_the_module_a_dispatcher_carries_out_gets_its_handoff() {
    let r = run();
    handoff(&r, "m1", "fix a1");
    handoff(&r, "m2", "start b1");
    let out = call(&r, &start_with(&r, "Dispatch m2", "處理 m2，不要修改 m1")).unwrap();
    let prompt = given_prompt(&out);
    assert!(prompt.contains("HANDOFF for module m2"), "{prompt}");
    assert!(
        !prompt.contains("HANDOFF for module m1"),
        "m1 is named only as a module to leave alone: {prompt}"
    );
    assert!(pending_modules(&r).contains(&"m1".to_string()));
    dispatcher_runs(&r, &prompt);
    assert_eq!(pending_modules(&r), ["m1"], "only m2's note was delivered");

    let why = refusal(&r, &start_with(&r, "Dispatcher", "Carry out m1, after m2."))
        .expect("two named modules and a description naming neither leave the target unclear");
    assert!(why.contains("m1, m2") && why.contains("description"), "{why}");
    let out = call(&r, &start_with(&r, "Dispatch m1", "Carry out m1, after m2.")).unwrap();
    assert!(given_prompt(&out).contains("HANDOFF for module m1"));
    assert!(
        call(&r, &start_with(&r, "Dispatch m2", "Carry out module m2 (m1 is done).")).is_none(),
        "m2's note is taken and m1's goes to no m2 dispatcher"
    );
}

#[test]
fn a_handoff_stays_pending_until_a_dispatcher_runs_with_it() {
    let r = run();
    handoff(&r, "m1", "dispatch fix unit a1-fix");
    let start = start_dispatcher(&r, "m1");
    let first = given_prompt(&call(&r, &start).unwrap());
    assert!(first.contains("next: dispatch fix unit a1-fix"));
    // The launch failed or was cancelled: no dispatcher ever ran with the note.
    assert_eq!(pending_modules(&r), ["m1"]);
    assert!(orch::digest(&r.store, &r.project, Some("m1"))
        .unwrap()
        .contains("handoff pending (m1)"));
    goal::set_status(&r.home, "s", "blocked", Some("needs a key"))
        .expect("a failed offer allows blocked without consuming the note");
    goal::reopen(&r.home, "s").unwrap();
    // A dispatcher started without the note consumes nothing.
    dispatcher_runs(&r, "Carry out module m1.");
    assert_eq!(pending_modules(&r), ["m1"]);

    let retry = given_prompt(&call(&r, &start).expect("the retry gets the note again"));
    assert!(retry.contains("next: dispatch fix unit a1-fix"));
    dispatcher_runs(&r, &retry);
    assert!(pending_modules(&r).is_empty());
    assert!(call(&r, &start).is_none(), "a delivered note is given once");
    goal::set_status(&r.home, "s", "blocked", Some("needs a key")).unwrap();
}

#[test]
fn the_goal_cannot_close_blocked_while_a_handoff_waits() {
    let r = run();
    handoff(&r, "m2", "start b1");
    let refused = goal::set_status(&r.home, "s", "blocked", Some("needs a key")).unwrap_err();
    assert_eq!(refused.kind, "handoff_pending");
    assert!(refused.message.contains("names module m2"));
    orch::take_handoff(&r.store, Some("m2")).unwrap();
    goal::set_status(&r.home, "s", "blocked", Some("needs a key")).unwrap();
}

#[test]
fn whole_wish_handoff_reaches_a_module_dispatcher_and_is_taken() {
    for prompt in [
        "Module: m1. Consume the whole-wish handoff.",
        "Wish mentions m1 and m2. Consume the whole-wish handoff.",
    ] {
        let r = run();
        orch::record_handoff(&r.store, &args(&["handoff", "--next", "check acceptance"])).unwrap();
        let out = call(&r, &start_with(&r, "Consume handoff", prompt))
            .expect("whole-wish note must be offered even when modules are named");
        let prompt = given_prompt(&out);
        assert!(prompt.contains("HANDOFF for the whole wish"), "{prompt}");
        assert!(!orch::read_handoff(&r.store, None).unwrap().unwrap().taken);
        dispatcher_runs(&r, &prompt);
        assert!(orch::pending_handoffs(&r.store).unwrap().is_empty());
    }
}

#[test]
fn blocked_is_legal_after_an_offer_but_records_every_pending_handoff() {
    let r = run();
    handoff(&r, "m1", "fix a1");
    handoff(&r, "m2", "start b1");
    call(&r, &start_with(&r, "Dispatch m1", "Module m1")).unwrap();
    assert_eq!(
        goal::set_status(&r.home, "s", "blocked", Some("launch failed"))
            .unwrap_err()
            .kind,
        "handoff_pending",
        "m2 has never been offered"
    );
    call(&r, &start_with(&r, "Dispatch m2", "Module m2")).unwrap();
    goal::set_status(&r.home, "s", "blocked", Some("launch failed"))
        .expect("offered but untaken handoffs must allow blocked");
    let note = goal::load(&r.home, "s").unwrap().unwrap().note.unwrap();
    assert!(
        note.contains("launch failed") && note.contains("m1.json") && note.contains("m2.json"),
        "{note}"
    );
    assert_eq!(pending_modules(&r), ["m1", "m2"]);
}

#[test]
fn replacement_handoffs_need_their_own_offer() {
    let r = run();
    handoff(&r, "m1", "fix a1");
    call(&r, &start_with(&r, "Dispatch m1", "Module m1")).unwrap();
    handoff(&r, "m1", "fix a1 again");
    let why = goal::set_status(&r.home, "s", "blocked", Some("launch failed")).unwrap_err();
    assert_eq!(why.kind, "handoff_pending");
    assert!(why.message.contains("m1.json"), "{}", why.message);
}

#[test]
fn completed_work_is_not_a_handoff() {
    for next in [
        "none",
        "None: wish complete, fulltest passed",
        "wish complete",
        "Nothing left to do",
        "No remaining work",
    ] {
        let r = run();
        let err =
            orch::record_handoff(&r.store, &args(&["handoff", "--next", next])).expect_err(next);
        assert!(err.message.contains("finish normally"), "{}", err.message);
        assert!(orch::pending_handoffs(&r.store).unwrap().is_empty());
    }
    let r = run();
    orch::record_handoff(
        &r.store,
        &args(&[
            "handoff",
            "--next",
            "verify the wish complete message after fixing a1",
        ]),
    )
    .unwrap();
}

#[test]
fn stop_names_the_pending_note_and_the_way_to_take_it() {
    let r = run();
    orch::record_handoff(&r.store, &args(&["handoff", "--next", "check acceptance"])).unwrap();
    let out = call(&r, &json!({"hook_event_name":"Stop", "session_id":"s"})).unwrap();
    let text = out["reason"].as_str().unwrap();
    assert!(
        text.contains("wish.json")
            && text.contains("fresh chronos:dispatcher")
            && text.contains("taken"),
        "{text}"
    );
    assert!(!text.contains("or close it"), "{text}");
}

#[test]
fn stop_names_the_actual_completion_gate_without_a_handoff() {
    let r = run();
    let out = call(&r, &json!({"hook_event_name":"Stop", "session_id":"s"})).unwrap();
    let text = out["reason"].as_str().unwrap();
    assert!(text.contains("a1=failed") && text.contains("b1=pending"), "{text}");
}

#[test]
fn compaction_brings_back_the_work_state() {
    let r = run();
    handoff(&r, "m1", "dispatch fix unit a1-fix");
    let session = |source: &str| json!({"hook_event_name":"SessionStart","session_id":"s","source":source,"transcript_path":r.main});
    assert!(call(&r, &session("startup")).is_none());
    let out = call(&r, &session("compact")).unwrap();
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let text = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("Chronos goal s is active"));
    assert!(text.contains("next: dispatch fix unit a1-fix"));
    assert!(text.contains("units: 2"));

    fs::create_dir_all(r.store.join("agents")).unwrap();
    fs::write(r.store.join("agents").join("d1"), "m1").unwrap();
    transcript(&r.own, &[30_000, 160_000]);
    let read = in_dispatcher(tool(
        &r,
        "Read",
        json!({"file_path": r.store.join("units.tsv")}),
    ));
    assert!(refusal(&r, &read).is_some_and(|why| why.contains("handoff")));

    let boundary = |uuid: &str| {
        format!(
            "{}\n",
            json!({"type":"system","subtype":"compact_boundary","uuid":uuid,"compactMetadata":{"trigger":"auto","preTokens":167189}})
        )
    };
    let mut lines = fs::read_to_string(&r.own).unwrap();
    lines.push_str(&boundary("c1"));
    lines.push_str(&assistant(20_000));
    fs::write(&r.own, &lines).unwrap();
    let grep = in_dispatcher(tool(&r, "Grep", json!({})));
    assert!(call(&r, &grep).is_none(), "newly routed tools do not consume compaction context");
    let out = call(&r, &read).unwrap();
    let text = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(out["hookSpecificOutput"]
        .get("permissionDecision")
        .is_none());
    assert!(text.contains("HANDOFF for module m1"));
    assert!(text.contains("DIGEST\nunits: 1 (failed 1)"));
    assert!(call(&r, &read).is_none(), "once per compaction");

    lines.push_str(&boundary("c2"));
    fs::write(&r.own, &lines).unwrap();
    assert!(call(&r, &read).is_some());
}

fn start_dispatcher(r: &Run, module: &str) -> Value {
    tool(
        r,
        "Agent",
        json!({"subagent_type":"chronos:dispatcher","description":module,"prompt":format!("Carry out module {module}.")}),
    )
}
fn handoff_stuck(r: &Run, module: &str, stuck: &str) {
    let out = orch::run(
        &r.store,
        &args(&["handoff", "--module", module, "--next", "go on", "--stuck", stuck]),
        false,
        false,
    );
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
}
fn units(r: &Run, rows: &str) {
    fs::write(
        r.store.join("units.tsv"),
        format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n{rows}"),
    )
    .unwrap();
}
fn stalls(n: &str) {
    std::env::set_var(orch::STALL_ENV, n);
}

// The limit lives in a process-wide variable, so these tests run in one function each under a lock.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn three_handoffs_without_progress_stop_the_module() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    stalls("3");
    let r = run();
    for round in 0..4 {
        handoff_stuck(&r, "m1", "a1 keeps failing");
        let n = orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled;
        assert_eq!(n, round, "the first handoff has nothing to compare with");
        if round < 3 {
            // not yet stopped: the fresh dispatcher starts and takes the note
            let out = call(&r, &start_dispatcher(&r, "m1")).expect("allowed with the note");
            assert_eq!(out["hookSpecificOutput"]["permissionDecision"], "allow");
        }
    }
    let why = refusal(&r, &start_dispatcher(&r, "m1")).expect("fourth dispatcher refused");
    assert!(why.contains("a1 keeps failing"), "{why}");
    assert!(why.contains("a1 failed"), "{why}");
    assert!(why.contains("blocked"), "{why}");
    // the core can now close the goal as blocked despite the pending note
    goal::set_status(&r.home, "s", "blocked", Some("stalled")).unwrap();
}

#[test]
fn progress_resets_the_count() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    stalls("3");
    let r = run();
    for _ in 0..3 {
        handoff_stuck(&r, "m1", "stuck");
    }
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 2);
    // a state change
    units(&r, "a1\tm1\tverified\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n");
    handoff_stuck(&r, "m1", "stuck");
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 0);
    handoff_stuck(&r, "m1", "stuck");
    handoff_stuck(&r, "m1", "stuck");
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 2);
    // a newly registered unit
    units(&r, "a1\tm1\tverified\t\t\t\t\na2\tm1\tpending\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n");
    handoff_stuck(&r, "m1", "stuck");
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 0);
    // progress made after the last handoff also lifts a stop
    for _ in 0..4 {
        handoff_stuck(&r, "m1", "stuck");
    }
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_some());
    units(&r, "a1\tm1\tverified\t\t\t\t\na2\tm1\tverified\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n");
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_none());
}

#[test]
fn the_limit_is_configurable() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let r = run();
    stalls("1");
    handoff_stuck(&r, "m1", "s");
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_none());
    handoff_stuck(&r, "m1", "s");
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_some());
    stalls("5");
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_none());
    stalls("0");
    for _ in 0..6 {
        handoff_stuck(&r, "m1", "s");
    }
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_none(), "0 turns the guard off");
    stalls("3");
}

#[test]
fn other_modules_are_unaffected() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    stalls("2");
    let r = run();
    for _ in 0..3 {
        handoff_stuck(&r, "m1", "s");
    }
    handoff_stuck(&r, "m2", "s");
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_some());
    assert!(refusal(&r, &start_dispatcher(&r, "m2")).is_none());
    assert_eq!(orch::read_handoff(&r.store, Some("m2")).unwrap().unwrap().stalled, 0);
    // m1's progress is not m2's
    units(&r, "a1\tm1\tverified\t\t\t\t\nb1\tm2\tpending\t\t\t\t\n");
    handoff_stuck(&r, "m2", "s");
    assert_eq!(orch::read_handoff(&r.store, Some("m2")).unwrap().unwrap().stalled, 1);
    stalls("3");
}

#[test]
fn old_handoffs_without_the_counter_keep_working() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    stalls("1");
    let r = run();
    let dir = r.store.join("handoffs/module");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("m1.json"),
        r#"{"module":"m1","ts":"2026-01-01T00:00:00Z","next":"old step","in_flight":"","stuck":"","decisions":"","taken":false}"#,
    )
    .unwrap();
    let old = orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap();
    assert_eq!((old.stalled, old.snapshot), (0, None));
    assert!(refusal(&r, &start_dispatcher(&r, "m1")).is_none());
    // the next handoff has nothing comparable, so it starts at 0 rather than being counted
    handoff_stuck(&r, "m1", "s");
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 0);
    handoff_stuck(&r, "m1", "s");
    assert_eq!(orch::read_handoff(&r.store, Some("m1")).unwrap().unwrap().stalled, 1);
    stalls("3");
}
