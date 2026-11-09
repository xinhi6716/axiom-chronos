use crate::common;
use axiom::{
    bridge::{call, Bridge, Config},
    codex::TESTED_CODEX,
};
use serde_json::json;
use std::process::Command;

fn doctor(home: &std::path::Path) -> Command {
    let fake = serde_json::to_string(&common::fake("fake_claude")).unwrap();
    let codex =
        serde_json::to_string(&common::fake_codex(&[&common::tested_codex_flag()])).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
    command
        .arg("doctor")
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home)
        .env("AXIOM_CLAUDE_CMD", &fake)
        .env("AXIOM_CODEX_CMD", &codex)
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
    assert!(text.contains("claude"));
    assert!(text.lines().any(|line| line.starts_with("ok   claude")));
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
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!text.contains("agy"));
    assert!(!text.contains("Antigravity"));
    let output = doctor(temp.path())
        .env("AXIOM_CLAUDE_CMD", &missing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ERR  claude"));
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

#[test]
fn doctor_and_bridge_status_report_the_same_codex_result() {
    let tested = common::tested_codex_flag();
    let cases: [(&[&str], &str, &str, i32); 6] = [
        (&[&tested], "ok   codex", "ok", 0),
        (
            &["--fake-version=codex-cli 999.1.2"],
            "warn codex",
            "warn",
            0,
        ),
        (&["--fake-version=codex-cli 0.0.1"], "ERR  codex", "fail", 1),
        (&["--fake-version=codex-cli fake"], "ERR  codex", "fail", 1),
        (
            &["--fake-version=2.1.281 (Claude Code)"],
            "ERR  codex",
            "fail",
            1,
        ),
        (&[&tested, "--fake-version-exit=3"], "ERR  codex", "fail", 1),
    ];
    for (flags, mark, verdict, exit) in cases {
        let temp = common::temp();
        let command = common::fake_codex(flags);
        let output = doctor(temp.path())
            .env("AXIOM_CODEX_CMD", serde_json::to_string(&command).unwrap())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(output.status.code(), Some(exit), "{flags:?}: {text}");
        let line = text
            .lines()
            .find(|line| line.starts_with(mark))
            .unwrap_or_else(|| panic!("{flags:?}: no {mark} in {text}"));
        let bridge = Bridge::new(Config {
            data_home: Some(temp.path().into()),
            codex: Ok(command),
            wsl: None,
            on_windows: false,
        });
        let status = call(&bridge, "bridge_status", &json!({}));
        let check = &status["structuredContent"]["codex"]["version_check"];
        assert_eq!(check["verdict"], verdict, "{flags:?}: {status}");
        assert_eq!(check["minimum"], TESTED_CODEX.minimum.to_string());
        assert_eq!(check["last_tested"], TESTED_CODEX.last_tested.to_string());
        let message = check["message"].as_str().unwrap();
        assert!(line.contains(message), "{line} does not carry {message}");
    }
}

#[test]
fn codex_that_cannot_start_is_a_required_failure() {
    let temp = common::temp();
    let missing = serde_json::to_string(&vec![temp
        .path()
        .join("missing")
        .to_string_lossy()
        .to_string()])
    .unwrap();
    let output = doctor(temp.path())
        .env("AXIOM_CODEX_CMD", &missing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("ERR  codex"));
}

#[test]
fn newer_codex_is_a_clear_warning_and_older_codex_names_the_minimum() {
    let temp = common::temp();
    let last_tested = TESTED_CODEX.last_tested.to_string();
    let output = doctor(temp.path())
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake_codex(&["--fake-version=codex-cli 999.1.2"]))
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    let warning = text.lines().find(|l| l.starts_with("warn codex")).unwrap();
    assert!(warning.contains("999.1.2"), "{warning}");
    assert!(
        warning.contains(&format!("newer than {last_tested}")),
        "{warning}"
    );
    assert!(warning.contains("not blocked"), "{warning}");
    let output = doctor(temp.path())
        .env(
            "AXIOM_CODEX_CMD",
            serde_json::to_string(&common::fake_codex(&["--fake-version=codex-cli 0.0.1"]))
                .unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8_lossy(&output.stdout);
    let failure = text.lines().find(|l| l.starts_with("ERR  codex")).unwrap();
    assert!(
        failure.contains(&format!("older than {}", TESTED_CODEX.minimum)),
        "{failure}"
    );
}
