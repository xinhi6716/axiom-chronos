use crate::common;
use std::process::Command;
#[test]
fn doctor_reports_paths_and_unverified_capabilities() {
    let t = common::temp();
    let fake = serde_json::to_string(&common::fake("fake_claude")).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .arg("doctor")
        .env("AXIOM_HOME", t.path())
        .env("AXIOM_DATA_HOME", t.path())
        .env("AXIOM_CODEX_CMD", fake)
        .env("AXIOM_CLAUDE_CMD", "missing")
        .env("AXIOM_AGY_CMD", "missing")
        .output()
        .unwrap();
    assert!(o.status.success());
    let s = String::from_utf8(o.stdout).unwrap();
    for part in ["codex", "data", "plugin", "binary", "not verified"] {
        assert!(s.contains(part), "{s}");
    }
    assert!(!s.contains("claude"));
    assert!(!s.contains("agy"));
    let o = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .arg("doctor")
        .env("AXIOM_HOME", t.path())
        .env("AXIOM_CODEX_CMD", "")
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
}
