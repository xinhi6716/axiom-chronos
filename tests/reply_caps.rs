mod common;

use axiom::{
    bridge::{call, call_body, tools::bound_reply, Bridge, Config},
    orch::summary::{failing_lines, path_sample, test_counts, MAX_REPLY_BYTES},
};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

fn size(value: &Value) -> usize {
    serde_json::to_vec(&call_body(|| Ok(value.clone())))
        .unwrap()
        .len()
}

#[test]
fn check_commands_save_capture_and_summarize() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    fs::write(repo.path().join("a.txt"), "a").unwrap();
    fs::write(repo.path().join("emit.py"), "import sys\nprint('FIRST_MARKER')\nprint('x' * 120000)\nprint('error last failure')\nprint('LAST_MARKER')\nprint('test result: FAILED. 2 passed; 1 failed; 3 ignored;')\nsys.exit(1)\n").unwrap();
    let command = format!("{} emit.py", common::python());
    let store = home.path().join("orchestrate/demo");
    fs::create_dir_all(store.join("reports")).unwrap();
    let brief = store.join("a.brief");
    fs::write(
        &brief,
        format!("GOAL test\nSCOPE a.txt\nVERIFY {command}\n"),
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        format!(
            "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `{command}`\n"
        ),
    )
    .unwrap();
    fs::write(
        store.join("reports/a.evidence.tsv"),
        "kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-worker\t0\t\t1\n",
    )
    .unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.path().into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    });
    for (cmd, name) in [("verify", "a"), ("accept", "m")] {
        let args = json!({"project":"demo", "args":["--json",cmd,name,"--repo",repo.path()]});
        let result = call(&bridge, "orch", &args);
        assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_REPLY_BYTES);
        let payload = &result["structuredContent"];
        let summary: Value = serde_json::from_str(payload["stdout"].as_str().unwrap()).unwrap();
        assert!(summary.get("output").is_none());
        assert_eq!(summary["tests"], json!({"passed":2,"failed":1,"ignored":3}));
        assert_eq!(
            summary["failing"],
            json!([
                "error last failure",
                "test result: FAILED. 2 passed; 1 failed; 3 ignored;"
            ])
        );
        assert!(summary["changed"].is_null());
        let path = store.join("reports").join(format!("{name}.{cmd}.txt"));
        assert_eq!(Path::new(summary["output_file"].as_str().unwrap()), path);
        let capture = fs::read_to_string(path).unwrap();
        assert!(
            capture.len() > 100_000
                && capture.contains("FIRST_MARKER")
                && capture.contains("LAST_MARKER")
        );
        let text = call(
            &bridge,
            "orch",
            &json!({"project":"demo", "args":[cmd,name,"--repo",repo.path()]}),
        );
        let stdout = text["structuredContent"]["stdout"].as_str().unwrap();
        let lines: Vec<_> = stdout.lines().collect();
        if cmd == "verify" {
            assert!(lines[0].starts_with("a\texit=1\t"));
        } else {
            assert_eq!(lines[0], "m\texit=1");
        }
        assert_eq!(lines[1], "tests passed=2 failed=1 ignored=3");
        assert_eq!(lines[2], "error last failure");
        assert!(lines.last().unwrap().starts_with("output="));
        assert!(!stdout.contains("FIRST_MARKER"));
    }
    fs::write(
        repo.path().join("emit.py"),
        "print('unrecognized output')\n",
    )
    .unwrap();
    let unknown = call(
        &bridge,
        "orch",
        &json!({"project":"demo", "args":["--json","verify","a","--repo",repo.path()]}),
    );
    let stdout: Value =
        serde_json::from_str(unknown["structuredContent"]["stdout"].as_str().unwrap()).unwrap();
    assert!(stdout["tests"].is_null());
    let text = call(
        &bridge,
        "orch",
        &json!({"project":"demo", "args":["verify","a","--repo",repo.path()]}),
    );
    assert_eq!(
        text["structuredContent"]["stdout"]
            .as_str()
            .unwrap()
            .lines()
            .nth(1),
        Some("tests unknown")
    );
}

#[test]
fn summary_boundaries() {
    for n in [20, 21] {
        let output = (0..n).map(|i| format!("error {i}\n")).collect::<String>();
        assert_eq!(failing_lines(&output, 1).len(), n.min(20));
        let paths: Vec<_> = (0..n).map(|i| format!("path{i}")).collect();
        assert_eq!(path_sample(&paths)["count"], n);
        assert_eq!(
            path_sample(&paths)["sample"].as_array().unwrap().len(),
            n.min(20)
        );
    }
    for n in [2048, 2049] {
        let output = format!("error{}", "x".repeat(n - 5));
        let lines = failing_lines(&output, 1);
        assert_eq!(lines[0].len(), 2048);
        assert_eq!(lines[0].ends_with('…'), n == 2049);
    }
    assert_eq!(test_counts("unrecognized output"), None);
}

#[test]
fn bound_replies_at_boundary_and_for_large_payloads() {
    let small = json!({"status":"completed","unit":"u","text":"unchanged"});
    assert_eq!(bound_reply(small.clone()), small);
    let mut under = None;
    let mut over = None;
    for n in 3800..4200 {
        for extra in ["x", "\n"] {
            let p = json!({"text":"x".repeat(n),"extra":extra});
            match size(&p) {
                MAX_REPLY_BYTES => under = Some(p),
                n if n == MAX_REPLY_BYTES + 1 => over = Some(p),
                _ => {}
            }
        }
    }
    let under = under.unwrap();
    assert_eq!(bound_reply(under.clone()), under);
    let over = bound_reply(over.unwrap());
    assert_eq!(over["truncated"], true);
    assert!(size(&over) <= MAX_REPLY_BYTES);
    for bulky in [
        json!("x".repeat(1_000_000)),
        json!(vec!["x".repeat(1000); 1000]),
        json!(vec![json!({"path":"file", "change":"edit"}); 10000]),
        json!("\"\n界".repeat(40000)),
    ] {
        let payload = json!({"status":"failed", "error":{"kind":"failure", "message":"m".repeat(100000)}, "unit":"u", "state":"failed", "next":"fix", "exit_code":1, "path":"p", "pack":"p", "report":"r", "report_json":"rj", "verify_output":"v", "output_file":"o", "other_file":"f", "data":bulky});
        let result = bound_reply(payload.clone());
        assert!(size(&result) <= MAX_REPLY_BYTES);
        assert_eq!(result["truncated"], true);
        for key in [
            "status",
            "unit",
            "state",
            "next",
            "exit_code",
            "path",
            "pack",
            "report",
            "report_json",
            "verify_output",
            "output_file",
            "other_file",
        ] {
            assert_eq!(result[key], payload[key]);
        }
        assert_eq!(result["error"]["kind"], "failure");
    }
    let large_protected = bound_reply(
        json!({"status":"failed", "unit":"u".repeat(100_000), "output_file":"p".repeat(100_000)}),
    );
    assert!(size(&large_protected) <= MAX_REPLY_BYTES);
    assert!(large_protected.get("unit").is_some());
    assert!(large_protected.get("output_file").is_some());
    let stdout = json!({"exit_code":1,"tests":null,"failing":["\\\"".repeat(2048)],"output_file":"result.txt","changed":null});
    let result = bound_reply(
        json!({"status":"failed","stdout":stdout.to_string(),"error":{"kind":"orch_failed","message":"x".repeat(5000)}}),
    );
    assert!(size(&result) <= MAX_REPLY_BYTES);
    let parsed: Value = serde_json::from_str(result["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(parsed["output_file"], "result.txt");
    assert!(parsed.get("failing").is_some());
}

#[test]
fn unit_mode_omits_check_output_and_reports_accept_file() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = home.path().join("orchestrate/demo");
    fs::create_dir_all(store.join("inbox")).unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    fs::write(repo.path().join("a.txt"), "a").unwrap();
    fs::create_dir_all(repo.path().join("src")).unwrap();
    fs::write(repo.path().join("src/mod.py"), "fixture").unwrap();
    assert!(Command::new("git")
        .args(["add", "a.txt", "src/mod.py"])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture"
        ])
        .current_dir(repo.path())
        .status()
        .unwrap()
        .success());
    let brief = store.join("a.brief");
    fs::write(
        &brief,
        "GOAL test\nSCOPE src/mod.py\nVERIFY echo check-marker\n",
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(store.join("modules.md"), "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo accept-marker`\n").unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.path().into()),
        codex: Ok(common::fake("fake_codex")),
        wsl: None,
        on_windows: false,
    });
    let result = call(
        &bridge,
        "codex_run",
        &json!({"project":"demo","unit":"a","cwd":repo.path(),"role":"terra"}),
    );
    let body = &result["structuredContent"];
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["accept"]["ran"], true, "{body}");
    assert!(body["accept"].get("output").is_none());
    assert_eq!(
        Path::new(body["accept"]["output_file"].as_str().unwrap()),
        store.join("reports/m.accept.txt")
    );
    assert!(!result.to_string().contains("accept-marker"));
    assert!(fs::read_to_string(store.join("reports/m.accept.txt"))
        .unwrap()
        .contains("accept-marker"));
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_REPLY_BYTES);

    let brief = store.join("b.brief");
    fs::write(
        &brief,
        "GOAL test\nSCOPE src/mod.py\nVERIFY echo raw-verify-marker&& echo error verify failed&& exit 1\n",
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nb\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let failed = call(
        &bridge,
        "codex_run",
        &json!({"project":"demo","unit":"b","cwd":repo.path(),"role":"terra"}),
    );
    assert_eq!(
        failed["structuredContent"]["error"]["kind"], "verify_failed",
        "{failed}"
    );
    assert!(failed["structuredContent"].get("verify_tail").is_none());
    assert!(failed["structuredContent"].get("output").is_none());
    assert_eq!(failed["structuredContent"]["verify_exit"], 1);
    // Failure excerpts belong in the summary, but unrelated raw output does not.
    assert_eq!(
        failed["structuredContent"]["failing"],
        json!(["error verify failed"])
    );
    assert!(!failed.to_string().contains("raw-verify-marker"));
    assert_eq!(
        Path::new(failed["structuredContent"]["verify_output"].as_str().unwrap()),
        store.join("reports/b.verify.txt")
    );
    let capture = fs::read_to_string(store.join("reports/b.verify.txt")).unwrap();
    assert_eq!(
        capture,
        if cfg!(windows) {
            "raw-verify-marker\r\nerror verify failed\r\n"
        } else {
            "raw-verify-marker\nerror verify failed\n"
        }
    );
    assert!(serde_json::to_vec(&failed).unwrap().len() <= MAX_REPLY_BYTES);
}
