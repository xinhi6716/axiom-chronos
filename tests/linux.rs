mod common;
use axiom::linux;
use std::fs;

#[test]
fn runs_and_reports_exit_codes() {
    let dir = common::temp();
    fs::write(
        dir.path().join("probe.py"),
        "import os, sys\nprint(os.getcwd())\nsys.exit(int(sys.argv[1]))\n",
    )
    .unwrap();
    let path = dir.path().to_str().unwrap();
    let capture = dir.path().join(".fake_wsl_capture_cd");
    fs::write(&capture, "").unwrap();
    for code in [0, 3] {
        let result = linux::run(
            Some(common::fake("fake_wsl")),
            false,
            path,
            &["probe.py".into(), code.to_string()],
            10,
        )
        .unwrap();
        assert_eq!(result["exit_code"], code);
        assert_eq!(
            result["status"],
            if code == 0 { "completed" } else { "failed" }
        );
        assert_eq!(fs::read_to_string(&capture).unwrap(), path);
        if code != 0 {
            assert_eq!(result["error"]["kind"], "linux_command_failed");
        }
    }
}

#[test]
fn unavailable_and_invalid_arguments() {
    let dir = common::temp();
    let path = dir.path().to_str().unwrap();
    for (windows, reason) in [
        (true, "WSL is not installed"),
        (false, "this host is not Windows; run the command locally"),
    ] {
        let result = linux::run(None, windows, path, &["x".into()], 10).unwrap();
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["reason"], reason);
        assert!(result["error"].is_null());
    }
    assert_eq!(
        linux::run(None, true, path, &[], 10).unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        linux::run(None, true, "not-absolute", &["x".into()], 10)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let args = vec![
        "-c".into(),
        "import sys; sys.stderr.write('exit 127'); sys.exit(127)".into(),
    ];
    let result = linux::run(Some(common::fake("fake_wsl")), false, path, &args, 10).unwrap();
    assert_eq!(result["status"], "unavailable");
    assert_eq!(result["output_tail"], "exit 127");
    assert!(result["error"].is_null());
}

#[test]
fn missing_venv_is_unavailable() {
    let dir = common::temp();
    fs::write(dir.path().join("FAKE_WSL_NO_VENV"), "").unwrap();
    let result = linux::run(
        Some(common::fake("fake_wsl")),
        false,
        dir.path().to_str().unwrap(),
        &["-V".into()],
        10,
    )
    .unwrap();
    assert_eq!(result["status"], "unavailable");
    assert!(result["output_tail"]
        .as_str()
        .unwrap()
        .contains("sh: 1: exec: /home/u/.axiom/linux-venv/bin/python: not found"));
    assert!(result.get("exit_code").is_none());
    assert!(result["error"].is_null());
}

#[test]
fn timeout_is_reported() {
    let dir = common::temp();
    let args = ["-c".into(), "import time; time.sleep(30)".into()];
    let err = linux::run(
        Some(common::fake("fake_wsl")),
        false,
        dir.path().to_str().unwrap(),
        &args,
        1,
    )
    .unwrap_err();
    assert_eq!(err.kind, "timeout");
}

#[test]
fn output_tail_is_truncated_and_nuls_removed() {
    let dir = common::temp();
    let args = [
        "-c".into(),
        "import sys; sys.stdout.write('discard' + 'a' * 3999 + '\\0' + 'z')".into(),
    ];
    let result = linux::run(
        Some(common::fake("fake_wsl")),
        false,
        dir.path().to_str().unwrap(),
        &args,
        10,
    )
    .unwrap();
    assert_eq!(result["output_tail"], format!("{}z", "a".repeat(3999)));
}
