use crate::common;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn command(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake("fake_codex")).unwrap(),
        )
        .env_remove("ORCH_STORE");
    c
}
#[test]
fn requests_reach_dispatch_without_claude() {
    let t = common::temp();
    let marker = t.path().join("claude-record");
    for args in [
        vec!["wish", "--astra", "--no-architect", "add", "--pending"],
        vec!["research", "question"],
        vec!["wish", "--creative"],
        vec!["wish", "--resume", "missing"],
        vec!["wish", "--cancel", "missing"],
        vec![
            "wish",
            "--decide",
            "missing",
            "--research-digest",
            "abc",
            "--verdict",
            "pass",
            "--reason",
            "reviewed",
        ],
    ] {
        let o = command(t.path())
            .args(args)
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .env("FAKE_CLAUDE_RECORD", &marker)
            .output()
            .unwrap();
        // U3 may either deliver a boundary or return a typed failure; both must emit the replay key first.
        let line = String::from_utf8(o.stdout).unwrap();
        let first: Value = serde_json::from_str(line.lines().next().unwrap()).unwrap();
        assert!(!first["request_id"].as_str().unwrap().is_empty());
        assert!(!marker.exists());
    }
}
#[test]
fn usage_and_recovery_are_fail_closed() {
    let t = common::temp();
    for args in [
        vec!["wish"],
        vec!["wish", "--model", "opus", "x"],
        vec!["wish", "--resume", "id", "text"],
        vec!["wish", "--max-turns", "7", "x"],
        vec!["wish", "--decide", "id", "--verdict", "pass"],
    ] {
        assert_eq!(
            command(t.path()).args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
    let f = t.path().join("recovery.json");
    std::fs::write(
        &f,
        r#"{"retry_after_repair":{"attempt_id":"a","reason":"retry"}}"#,
    )
    .unwrap();
    assert_eq!(
        command(t.path())
            .args(["wish", "--resume", "id", "--recovery-file"])
            .arg(&f)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let artifact = t.path().join("report");
    std::fs::write(&artifact, "evidence").unwrap();
    std::fs::write(&f, serde_json::to_vec(&json!({"accept_terminal":{"attempt_id":"a","report":{"path":artifact,"digest":"wrong"},"reason":"reviewed"}})).unwrap()).unwrap();
    let o = command(t.path())
        .args(["wish", "--resume", "id", "--recovery-file"])
        .arg(&f)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("digest mismatch"));
    assert!(o.stdout.is_empty());
}
#[test]
fn bridge_protocol_and_version() {
    let t = common::temp();
    let o = command(t.path()).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8(o.stdout).unwrap().trim(),
        format!("axiom-chronos {}", env!("CARGO_PKG_VERSION"))
    );
    let mut child = command(t.path())
        .arg("bridge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}})
    )
    .unwrap();
    let o = child.wait_with_output().unwrap();
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["id"], 1);
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
