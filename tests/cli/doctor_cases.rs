use crate::common;
use std::process::Command;

fn doctor(home: &std::path::Path) -> Command {
    let fake = serde_json::to_string(&common::fake("fake_codex_session")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
    command
        .arg("doctor")
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env("AXIOM_CLAUDE_CMD", &fake)
        .env("AXIOM_CODEX_CMD", &fake)
        .env("AXIOM_AGY_CMD", &fake)
        .env("AXIOM_WSL_CMD", &fake)
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec!["pwsh"]).unwrap(),
        );
    command
}

#[test]
fn all_present_and_missing_codex() {
    let temp = common::temp();
    let output = doctor(temp.path()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("trigram"));
    assert!(!text.contains("claude"));
    assert!(text.lines().any(|line| line.starts_with("ok   codex")));
    let output = doctor(temp.path())
        .env("AXIOM_CODEX_CMD", "")
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ERR  codex"));
}

#[test]
fn optional_and_required_failures() {
    let temp = common::temp();
    let missing = serde_json::to_string(&vec![temp
        .path()
        .join("missing")
        .to_string_lossy()
        .to_string()])
    .unwrap();
    let output = doctor(temp.path())
        .env("AXIOM_AGY_CMD", &missing)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("agy"));
    let output = doctor(temp.path())
        .env("AXIOM_CLAUDE_CMD", &missing)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("claude"));
    let file = temp.path().join("file");
    std::fs::write(&file, "x").unwrap();
    let output = doctor(&file).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ERR  state"));
}

#[cfg(windows)]
#[test]
fn store_pwsh_is_required_failure() {
    let temp = common::temp();
    let output = doctor(temp.path())
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec![
                r"C:\Users\u\AppData\Local\Microsoft\WindowsApps\pwsh.exe",
            ])
            .unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("ERR  pwsh"));
    assert!(text.contains("--installer-type wix"));
}
