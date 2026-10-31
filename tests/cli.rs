#[path = "cli/command_cases.rs"]
mod command_cases;
mod common;
#[path = "cli/doctor_cases.rs"]
mod doctor_cases;
#[test]
fn caller_cli_bootstrap_and_single_bridge_call() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let temp = common::temp();
    let binary = env!("CARGO_BIN_EXE_axiom");
    let cmd = serde_json::to_string(&common::fake("fake_codex")).unwrap();
    let begin = Command::new(binary)
        .args([
            "wish",
            "--project",
            temp.path().to_str().unwrap(),
            "inspect",
        ])
        .env("AXIOM_HOME", temp.path())
        .env("AXIOM_DATA_HOME", temp.path())
        .env("AXIOM_CODEX_CMD", &cmd)
        .output()
        .unwrap();
    assert!(
        begin.status.success(),
        "{}",
        String::from_utf8_lossy(&begin.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&begin.stdout).unwrap();
    assert!(value["session"].is_string());
    assert!(value["workflow"].as_str().unwrap().contains("inspect"));
    let mut call = Command::new(binary)
        .args(["bridge", "call", "-"])
        .env("AXIOM_HOME", temp.path())
        .env("AXIOM_DATA_HOME", temp.path())
        .env("AXIOM_CODEX_CMD", cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    call.stdin
        .take()
        .unwrap()
        .write_all(br#"{"name":"memory_search","arguments":{"query":"nothing"}}"#)
        .unwrap();
    let result = call.wait_with_output().unwrap();
    assert!(result.status.success());
    let response: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(response["structuredContent"]["status"], "completed");
}
