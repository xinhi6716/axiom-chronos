use crate::common;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Instant,
};
fn command(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home).env("AXIOM_DATA_HOME", home);
    c.env_remove("ORCH_STORE");
    c
}
#[test]
fn wish_and_research() {
    let temp = common::temp();
    let record = temp.path().join("record.json");
    for (args, expected, turns) in [
        (
            vec!["wish", "--astra", "--no-architect", "add", "--pending"],
            "/chronos:wish [no-architect] [astra] add --pending",
            None,
        ),
        (
            vec!["research", "RRAM", "因果"],
            "/chronos:research RRAM 因果",
            Some("40"),
        ),
    ] {
        let o = command(temp.path())
            .args(args)
            .env(
                "AXIOM_CODEX_CMD",
                serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
            )
            .env("FAKE_CODEX_RECORD", &record)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let v: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        assert_eq!(
            v["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["method"] == "turn/start")
                .unwrap()["params"]["input"][0]["text"],
            expected
        );
        let root = v["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["method"] == "turn/start")
            .unwrap()["params"]["threadId"]
            .as_str()
            .unwrap();
        let metadata: Value = serde_json::from_slice(
            &std::fs::read(temp.path().join("sessions").join(format!("{root}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            metadata["max_turns"].as_u64(),
            turns.map(|s| s.parse::<u64>().unwrap())
        );
        assert_eq!(v["argv"].as_array().unwrap().last().unwrap(), "stdio://");
    }
    let o = command(temp.path())
        .args(["wish", "x"])
        .env("PATH", "")
        .env_remove("AXIOM_CODEX_CMD")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("codex is not on PATH"));
}
#[test]
fn wish_rejects_model_without_starting_session() {
    let t = common::temp();
    let record = t.path().join("record.json");
    let o = command(t.path())
        .args(["wish", "--model", "opus", "x"])
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
        )
        .env("FAKE_CODEX_RECORD", &record)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unrecognized option: --model"));
    assert!(!record.exists());
}
fn pipe(
    exe: &std::path::Path,
    home: &std::path::Path,
    sub: &str,
    input: &str,
) -> std::process::Output {
    let mut p = Command::new(exe)
        .arg(sub)
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    p.wait_with_output().unwrap()
}
#[test]
fn bridge_hook_and_unicode_path() {
    let t = common::temp();
    let dir = t.path().join("空 白");
    std::fs::create_dir(&dir).unwrap();
    let exe = dir.join(if cfg!(windows) { "axiom.exe" } else { "axiom" });
    std::fs::copy(env!("CARGO_BIN_EXE_axiom"), &exe).unwrap();
    assert_eq!(
        Command::new(&exe).arg("--version").output().unwrap().stdout,
        format!("axiom-chronos {} (beta)\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    );
    let o = pipe(&exe, t.path(), "bridge", &input);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(
        text.contains("protocolVersion") && text.contains("codex_run"),
        "{text}"
    );
    let o = pipe(
        &exe,
        t.path(),
        "hook",
        "{\"hook_event_name\":\"Stop\",\"session_id\":\"none\"}",
    );
    assert!(o.status.success());
    axiom::goal::open(t.path(), "s", "wish", None).unwrap();
    let o = pipe(
        &exe,
        t.path(),
        "hook",
        &json!({"hook_event_name":"Stop","session_id":"s"}).to_string(),
    );
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["decision"], "block");
    let o = pipe(&exe, t.path(), "hook", &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"mcp__plugin_chronos_bridge__codex_run","tool_input":{}}).to_string());
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
    let fake = serde_json::to_string(&common::fake("fake_codex_session")).unwrap();
    let o = Command::new(&exe)
        .arg("doctor")
        .env("AXIOM_HOME", t.path())
        .env("AXIOM_DATA_HOME", t.path())
        .env("AXIOM_CODEX_CMD", &fake)
        .env("AXIOM_AGY_CMD", &fake)
        .env("AXIOM_WSL_CMD", &fake)
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec!["pwsh"]).unwrap(),
        )
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let mut times = Vec::new();
    for _ in 0..20 {
        let now = Instant::now();
        assert!(pipe(&exe, t.path(), "hook", "{}").status.success());
        times.push(now.elapsed());
    }
    times.sort();
    println!("hook cold start median: {:?}", (times[9] + times[10]) / 2);
}
#[test]
fn orch_and_memory() {
    let t = common::temp();
    let store = t.path().join("store");
    for args in [
        vec!["init"],
        vec!["unit", "add", "u1", "--track", "core"],
        vec!["status"],
    ] {
        let o = command(t.path())
            .arg("orch")
            .arg("--store")
            .arg(&store)
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let o = command(t.path())
        .args(["orch", "--json", "status"])
        .env("ORCH_STORE", &store)
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(String::from_utf8_lossy(&o.stdout).contains("u1"));
    let o = command(t.path())
        .args(["orch", "unit", "get", "nope"])
        .env("ORCH_STORE", &store)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error:"));
    let o = command(t.path()).args(["orch", "status"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("orch: --store or ORCH_STORE is required"));
    axiom::memory::Memory::open(t.path()).unwrap();
    let db = rusqlite::Connection::open(t.path().join("memory/memory.db")).unwrap();
    db.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('demo', 'u1', 'searchable title', 'searchable summary', 'searchable body', '[]', 'wish', '[]', '', 0)", []).unwrap();
    let o = command(t.path())
        .args(["memory", "search", "searchable", "--project", "demo"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stdout).contains("searchable"));
    for args in [vec!["memory", "get", "1"], vec!["memory", "digest", "demo"]] {
        let o = command(t.path()).args(args).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let _: Value = serde_json::from_slice(&o.stdout).unwrap();
    }
    let o = command(t.path())
        .args([
            "memory",
            "search",
            "searchable",
            "--project",
            "other",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 0);
    db.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('demo', 'u2', 'searchable second', '', '', '', '', '[]', '', 1)", []).unwrap();
    let o = command(t.path())
        .args([
            "memory",
            "search",
            "searchable",
            "--project",
            "demo",
            "--limit",
            "1",
        ])
        .output()
        .unwrap();
    assert!(o.status.success());
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 1);
    let bad = t.path().join("file");
    std::fs::write(&bad, "x").unwrap();
    let o = command(&bad).args(["memory", "get", "1"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("axiom: "));
}
#[test]
fn wish_usage_and_overrides() {
    let t = common::temp();
    for (args, message) in [
        (
            vec!["wish"],
            "axiom: wish: say what you want, e.g. axiom wish add a --stats flag to cli.py",
        ),
        (
            vec!["wish", "--no-architect"],
            "axiom: wish: say what you want, e.g. axiom wish add a --stats flag to cli.py",
        ),
        (
            vec!["unknown"],
            "axiom: unknown command or arguments: unknown",
        ),
        (vec![], "axiom: missing command"),
    ] {
        let o = command(t.path()).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&o.stderr).trim_end(), message);
    }
    let o = command(t.path())
        .args(["wish", "--project", "does-not-exist", "x"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--project is not a directory"));
    let record = t.path().join("record.json");
    let o = command(t.path())
        .args(["wish", "--project"])
        .arg(t.path())
        .args(["--max-turns", "7", "text", "--model", "--max-turns"])
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
        )
        .env("FAKE_CODEX_RECORD", &record)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    assert_eq!(
        v["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["method"] == "turn/start")
            .unwrap()["params"]["input"][0]["text"],
        "/chronos:wish text --model --max-turns"
    );
    let core = v["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["method"] == "thread/start")
        .unwrap();
    assert_eq!(core["params"]["cwd"], t.path().to_string_lossy().as_ref());
}

#[test]
fn creative_and_resume_usage() {
    let t = common::temp();
    let record = t.path().join("record");
    for (args, expected) in [
        (vec!["wish", "--creative"], "/chronos:wish [creative]"),
        (
            vec!["wish", "--creative", "invent", "x"],
            "/chronos:wish [creative] invent x",
        ),
        (
            vec!["wish", "--creative", "--no-architect", "--astra", "x"],
            "/chronos:wish [creative] x",
        ),
    ] {
        let o = command(t.path())
            .args(args)
            .env(
                "AXIOM_CODEX_CMD",
                serde_json::to_string(&common::fake("fake_codex_session")).unwrap(),
            )
            .env("FAKE_CODEX_RECORD", &record)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(o.status.success());
        let v: Value = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
        assert_eq!(
            v["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["method"] == "turn/start")
                .unwrap()["params"]["input"][0]["text"],
            expected
        );
    }
    for args in [
        vec!["wish", "--resume", "id", "text"],
        vec!["wish", "text", "--resume", "id"],
        vec!["wish", "--resume", "id", "--project", "."],
        vec!["wish", "--creative", "--resume", "id"],
        vec!["wish", "--resume", "id", "--astra"],
        vec!["wish", "--resume", "id", "--no-architect"],
    ] {
        let o = command(t.path()).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&o.stderr).contains("axiom: --resume takes only a session id")
        );
    }
}
