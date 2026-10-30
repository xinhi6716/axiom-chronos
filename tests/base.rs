mod common;
use axiom::base::{self, proc, schema, Family};
use serde_json::json;
use std::{path::Path, time::Duration};
#[test]
fn primitives() {
    for s in ["ml_rram", "a"] {
        assert!(base::require_slug("name", s).is_ok());
    }
    for s in ["ML_RRAM", "_x", "../x", "a/b", ""] {
        assert!(base::require_slug("name", s).is_err());
    }
    assert_eq!(
        base::slug_for_dir(Path::new("Chronos Rust")),
        "chronos-rust"
    );
    assert_eq!(
        base::slug_for_dir(Path::new("TaiwanStockRadar")),
        "taiwanstockradar"
    );
    let d = common::temp();
    let p = d.path().join("file");
    base::atomic_write(&p, "中文".as_bytes()).unwrap();
    base::atomic_write(&p, b"second").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second");
    assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    assert!(
        regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$")
            .unwrap()
            .is_match(&base::now_iso())
    );
    assert_ne!(base::unique(), base::unique());
    for (m, f) in [
        ("gpt-6.1-terra", Family::OpenAi),
        ("gemini-4-argon-high", Family::Google),
        ("chronos:reviewer", Family::Anthropic),
        ("sonnet", Family::Anthropic),
    ] {
        assert_eq!(base::family_of(m), f);
    }
}
#[test]
fn resolution() {
    let key = "AXIOM_BASE_TEST_COMMAND";
    std::env::set_var(key, r#"["python","-I"]"#);
    assert_eq!(
        base::resolve_command(key, "none").unwrap(),
        vec!["python", "-I"]
    );
    std::env::set_var(key, "no");
    assert_eq!(
        base::resolve_command(key, "none").unwrap_err().kind,
        "config"
    );
    std::env::remove_var(key);
    assert_eq!(
        base::resolve_command(key, "axiom_nonexistent_test_executable")
            .unwrap_err()
            .kind,
        "not_installed"
    );
}
#[cfg(windows)]
#[test]
fn resolution_prefers_pathext() {
    const MARKER: &str = "AXIOM_BASE_RESOLUTION_CHILD";
    if std::env::var_os(MARKER).is_some() {
        let resolved = base::resolve_command("AXIOM_BASE_UNUSED_COMMAND", "codex").unwrap();
        assert!(resolved[0].to_ascii_lowercase().ends_with("codex.cmd"));
        assert!(base::resolve_command("AXIOM_BASE_UNUSED_COMMAND", "codex.cmd").is_ok());
        return;
    }
    let dir = common::temp();
    std::fs::write(dir.path().join("codex"), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.path().join("codex.cmd"), "@exit /b 0\r\n").unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "resolution_prefers_pathext"])
        .env(MARKER, "1")
        .env("PATH", dir.path())
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env_remove("AXIOM_BASE_UNUSED_COMMAND")
        .status()
        .unwrap();
    assert!(status.success());
}
#[test]
fn shell_and_schema() {
    let d = common::temp();
    let run = proc::run_shell("echo hello", d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 0);
    assert!(run.output.contains("hello"));
    let cmd = format!(
        "\"{}\" -c \"import sys; sys.stdout.buffer.write(bytes([228,184,173,230,150,135]))\"",
        common::python()
    );
    let run = proc::run_shell(&cmd, d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 0, "{}", run.output);
    assert_eq!(run.output, "中文");
    let run = proc::run_shell("exit 3", d.path(), Duration::from_secs(3)).unwrap();
    assert_eq!(run.exit_code, 3);
    assert!(!run.timed_out);
    let cmd = if cfg!(windows) {
        "ping -n 5 127.0.0.1 >nul"
    } else {
        "sleep 5"
    };
    let start = std::time::Instant::now();
    let run = proc::run_shell(cmd, d.path(), Duration::from_millis(100)).unwrap();
    assert!(run.timed_out);
    assert_eq!(run.exit_code, -1);
    assert!(start.elapsed() < Duration::from_secs(5));
    let s = json!({"type":"object","required":["changes"],"additionalProperties":false,"properties":{"changes":{"type":"array","items":{"type":"object","required":["area"],"additionalProperties":false,"properties":{"area":{"enum":["code","tests"]}}}}}});
    assert!(schema::validate(&s, &json!({"changes":[{"area":"code"}]})).is_ok());
    for v in [
        json!({"changes":[{"area":"bad"}]}),
        json!({}),
        json!({"changes":[],"extra":1}),
    ] {
        assert!(schema::validate(&s, &v).is_err());
    }
}
#[test]
fn process_tree() {
    if let Ok(mode) = std::env::var("AXIOM_TEST_CANCEL_TREE") {
        if mode == "sleep" {
            std::thread::sleep(Duration::from_secs(30));
        } else {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_tree", "--nocapture"])
                .env("AXIOM_TEST_CANCEL_TREE", "sleep")
                .spawn()
                .unwrap();
            std::fs::write(mode, format!("{} {}", std::process::id(), child.id())).unwrap();
        }
        return;
    }
    let dir = common::temp();
    let pidfile = dir.path().join("pid");
    let script="import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); open(sys.argv[1],'w').write(str(p.pid)); time.sleep(60)";
    let args = vec![
        common::python(),
        "-I".into(),
        "-c".into(),
        script.into(),
        pidfile.to_string_lossy().into_owned(),
    ];
    let mut child = proc::command(&args).spawn().unwrap();
    let start = std::time::Instant::now();
    let grandchild: u32 = loop {
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|s| s.parse().ok())
        {
            break pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "pid file did not contain a PID"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let parent = child.id();
    proc::kill_tree(&mut child);
    let start = std::time::Instant::now();
    while proc::pid_alive(grandchild) && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!proc::pid_alive(parent));
    assert!(!proc::pid_alive(grandchild));
}

#[test]
fn cancelled_scope_blocks_late_spawn_without_affecting_standalone_or_nested_scope() {
    let dir = common::temp();
    let cancelled = proc::Cancellation::default();
    let independent = proc::Cancellation::default();
    cancelled.cancel();
    cancelled.scope(|| {
        let r = proc::run_shell(
            "echo late > must-not-exist",
            dir.path(),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_ne!(r.exit_code, 0);
        assert!(!dir.path().join("must-not-exist").exists());
        independent.scope(|| {
            assert_eq!(
                proc::run_shell("echo independent", dir.path(), Duration::from_secs(2))
                    .unwrap()
                    .exit_code,
                0
            )
        });
        assert_ne!(
            proc::run_shell("echo late", dir.path(), Duration::from_secs(2))
                .unwrap()
                .exit_code,
            0
        );
    });
    assert_eq!(
        proc::run_shell("echo standalone", dir.path(), Duration::from_secs(2))
            .unwrap()
            .exit_code,
        0
    );
}

#[test]
fn cancellation_kills_descendants_even_after_verifier_shell_exits() {
    let dir = common::temp();
    let pids = dir.path().join("pids");
    let executable = std::env::current_exe().unwrap();
    #[cfg(windows)]
    let command = format!(
        "set \"AXIOM_TEST_CANCEL_TREE={}\" && \"{}\" --exact process_tree --nocapture",
        pids.display(),
        executable.display()
    );
    #[cfg(unix)]
    let command = format!(
        "AXIOM_TEST_CANCEL_TREE='{}' '{}' --exact process_tree --nocapture",
        pids.display(),
        executable.display()
    );
    let cancellation = proc::Cancellation::default();
    let worker = cancellation.clone();
    let cwd = dir.path().to_owned();
    let job = std::thread::spawn(move || {
        worker.scope(|| proc::run_shell(&command, &cwd, Duration::from_secs(5)).unwrap())
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let ids = loop {
        if let Some(ids) = std::fs::read_to_string(&pids)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids| ids.len() == 2)
        {
            break ids;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    while proc::pid_alive(ids[0]) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !proc::pid_alive(ids[0]),
        "intermediate verifier has not exited"
    );
    assert!(proc::pid_alive(ids[1]), "descendant must still be running");
    let cancelled_at = std::time::Instant::now();
    cancellation.cancel();
    let result = job.join().unwrap();
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(5),
        "cancellation waited for natural descendant exit"
    );
    assert_ne!(
        result.exit_code, 0,
        "cancelled pipe draining must not report success"
    );
    assert!(
        !proc::pid_alive(ids[1]),
        "descendant survived cancellation after parent exit"
    );
}

#[test]
fn research_schemas() {
    let audit: serde_json::Value = serde_json::from_str(r#"{"type": "object", "additionalProperties": false, "required": ["summary", "coverage", "changes", "out_of_scope"], "properties": {"summary": {"type": "string"}, "coverage": {"type": "object", "additionalProperties": false, "required": ["read", "not_read"], "properties": {"read": {"type": "array", "items": {"type": "string"}}, "not_read": {"type": "array", "items": {"type": "string"}}}}, "changes": {"type": "array", "items": {"type": "object", "additionalProperties": false, "required": ["id", "area", "location", "evidence", "problem", "change", "verify"], "properties": {"id": {"type": "string"}, "area": {"type": "string", "enum": ["design", "feature", "algorithm", "correctness", "performance", "tests"]}, "location": {"type": "string"}, "evidence": {"type": "string"}, "problem": {"type": "string"}, "change": {"type": "string"}, "verify": {"type": "string"}}}}, "out_of_scope": {"type": "array", "items": {"type": "string"}}}}"#).unwrap();
    let review: serde_json::Value = serde_json::from_str(r#"{"type": "object", "required": ["verdict", "findings"], "properties": {"verdict": {"type": "string", "enum": ["pass", "revise", "reject"]}, "findings": {"type": "array", "items": {"type": "object", "required": ["check", "item", "reason"], "properties": {"check": {"type": "string", "enum": ["out_of_scope", "overclaim", "unfalsifiable", "complexity"]}, "item": {"type": "string"}, "reason": {"type": "string"}}}}}}"#).unwrap();
    let valid = json!({"summary":"ok","coverage":{"read":[],"not_read":[]},"changes":[{"id":"1","area":"tests","location":"x","evidence":"x","problem":"x","change":"x","verify":"x"}],"out_of_scope":[]});
    assert!(schema::validate(&audit, &valid).is_ok());
    let mut wrong = valid.clone();
    wrong["changes"][0]["area"] = json!("wrong");
    assert!(schema::validate(&audit, &wrong)
        .unwrap_err()
        .contains("$.changes[0].area"));
    assert!(schema::validate(&audit, &json!({})).is_err());
    let mut extra = valid;
    extra["extra"] = json!(1);
    assert!(schema::validate(&audit, &extra).is_err());
    assert!(schema::validate(&review, &json!({"verdict":"pass","findings":[]})).is_ok());
    assert!(schema::validate(&review, &json!({"verdict":"bad","findings":[]})).is_err());
}
