use axiom::hook::context;
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

const LIMIT: u64 = 150_000;
static ENV: Mutex<()> = Mutex::new(());

struct RestoreEnv(Option<OsString>);

impl Drop for RestoreEnv {
    fn drop(&mut self) {
        match &self.0 {
            Some(value) => std::env::set_var(context::WARN_ENV, value),
            None => std::env::remove_var(context::WARN_ENV),
        }
    }
}

struct Run {
    _dir: tempfile::TempDir,
    own: PathBuf,
    event: Value,
}

fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("projects/sess.jsonl");
    let own = dir.path().join("projects/sess/subagents/agent-d1.jsonl");
    fs::create_dir_all(own.parent().unwrap()).unwrap();
    fs::write(&main, assistant(900_000)).unwrap();
    let event = json!({
        "hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Read",
        "tool_input": {}, "transcript_path": main,
        "agent_id": "d1", "agent_type": "chronos:dispatcher"
    });
    Run {
        _dir: dir,
        own,
        event,
    }
}

fn assistant(context: u64) -> String {
    format!(
        "{}\n",
        json!({"type":"assistant","message":{
            "id":format!("msg_{context}"), "model":"claude-sonnet-4-5",
            "usage":{"input_tokens":10,"cache_creation_input_tokens":90,
                     "cache_read_input_tokens":context - 100,"output_tokens":7}
        }})
    )
}

fn transcript(path: &Path, contexts: &[u64]) {
    fs::write(
        path,
        contexts.iter().map(|c| assistant(*c)).collect::<String>(),
    )
    .unwrap();
}

fn line(context: u64) -> String {
    format!(
        "Context {}k of 150k: reach a clean stopping point, then call orch handoff.",
        context / 1000
    )
}

#[test]
fn warning_boundaries_and_hard_lock() {
    let r = run();
    for (current, expected) in [
        (119_999, None),
        (120_000, Some(line(120_000))),
        (149_999, Some(line(149_999))),
        (150_000, None),
    ] {
        transcript(&r.own, &[10_000, current]);
        let warning = context::handoff_warning(&r.event, LIMIT, 80);
        assert_eq!(warning, expected, "context {current}");
        if let Some(warning) = warning {
            assert!(warning.len() <= 100 && !warning.contains('\n'));
        }
        assert_eq!(
            context::handoff_refusal(&r.event, LIMIT, None).is_some(),
            current >= LIMIT
        );
    }
    transcript(&r.own, &[10_000, 74_999]);
    assert_eq!(context::handoff_warning(&r.event, LIMIT, 50), None);
    transcript(&r.own, &[10_000, 75_000]);
    assert_eq!(
        context::handoff_warning(&r.event, LIMIT, 50),
        Some(line(75_000))
    );
}

#[test]
fn only_dispatchers_with_a_readable_transcript_and_safe_start_warn() {
    let r = run();
    transcript(&r.own, &[10_000, 120_000]);
    let mut reviewer = r.event.clone();
    reviewer["agent_type"] = json!("chronos:reviewer");
    let mut core = r.event.clone();
    core.as_object_mut().unwrap().remove("agent_id");
    core.as_object_mut().unwrap().remove("agent_type");
    assert_eq!(context::handoff_warning(&reviewer, LIMIT, 80), None);
    assert_eq!(context::handoff_warning(&core, LIMIT, 80), None);
    transcript(&r.own, &[80_000, 120_000]);
    assert_eq!(context::handoff_warning(&r.event, LIMIT, 80), None);
    transcript(&r.own, &[120_000]);
    assert_eq!(context::handoff_warning(&r.event, LIMIT, 80), None);
    fs::write(&r.own, "not a transcript\n").unwrap();
    assert_eq!(context::handoff_warning(&r.event, LIMIT, 80), None);
    fs::remove_file(&r.own).unwrap();
    assert_eq!(context::handoff_warning(&r.event, LIMIT, 80), None);
}

#[test]
fn warnings_are_repeatable_and_leave_the_transcript_unchanged() {
    let r = run();
    transcript(&r.own, &[10_000, 120_000]);
    let before = fs::read(&r.own).unwrap();
    for _ in 0..2 {
        assert_eq!(
            context::handoff_warning(&r.event, LIMIT, 80),
            Some(line(120_000))
        );
    }
    assert_eq!(fs::read(&r.own).unwrap(), before);
}

#[test]
fn percentage_configuration() {
    let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = RestoreEnv(std::env::var_os(context::WARN_ENV));
    std::env::remove_var(context::WARN_ENV);
    assert_eq!(context::warn_percent(), Some(80));
    for (raw, expected) in [
        ("", Some(80)),
        ("0", None),
        ("abc", Some(80)),
        ("100", Some(80)),
        ("90", Some(90)),
        ("1", Some(1)),
        ("99", Some(99)),
    ] {
        std::env::set_var(context::WARN_ENV, raw);
        assert_eq!(context::warn_percent(), expected, "{raw}");
    }
}

#[test]
fn invalid_percent_reports_one_line() {
    for raw in ["abc", "100"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "percent_stderr_child", "--nocapture"])
            .env("AXIOM_WARN_TEST_CHILD", "1")
            .env(context::WARN_ENV, raw)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "{}={raw} is not a percent from 0 to 99; using 80\n",
                context::WARN_ENV
            )
        );
    }
}

#[test]
fn percent_stderr_child() {
    if std::env::var_os("AXIOM_WARN_TEST_CHILD").is_some() {
        assert_eq!(context::warn_percent(), Some(80));
    }
}

#[test]
fn threshold_uses_integer_arithmetic_without_overflow() {
    assert_eq!(context::soft_threshold(LIMIT, 80), 120_000);
    assert_eq!(context::soft_threshold(LIMIT, 50), 75_000);
    assert_eq!(context::soft_threshold(LIMIT, 0), 0);
    assert_eq!(context::soft_threshold(101, 99), 99);
    assert_eq!(
        context::soft_threshold(u64::MAX, 99),
        ((u64::MAX as u128 * 99) / 100) as u64
    );
}
