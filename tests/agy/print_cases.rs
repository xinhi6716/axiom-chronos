use crate::common;
use axiom::agy;
use serde_json::json;

fn run(
    prompt: &str,
    cwd: Option<&std::path::Path>,
    schema: Option<&serde_json::Value>,
    timeout: u64,
) -> serde_json::Value {
    agy::run(
        &common::fake("fake_agy"),
        prompt,
        cwd,
        "gemini-4-argon-medium",
        schema,
        timeout,
        &agy::Children::new(),
    )
    .unwrap()
}
#[test]
fn results_and_validation() {
    let dir = common::temp();
    let cwd = Some(dir.path());
    let empty = run("AGY_EMPTY", cwd, None, 20);
    assert_eq!(empty["error"]["kind"], "empty_response");
    let error = run("AGY_ERROR", cwd, None, 20);
    assert_eq!(
        error["error"],
        json!({"kind":"agy_error","message":"model refused"})
    );
    let silent = run("AGY_SILENT", cwd, None, 20);
    assert_eq!(silent["status"], "bridge_error");
    assert_eq!(silent["error"]["kind"], "agy_no_result");
    assert!(silent["error"]["message"]
        .as_str()
        .unwrap()
        .contains("agy exited 3"));
    assert!(silent["error"]["message"]
        .as_str()
        .unwrap()
        .contains("crashed before result"));
    let schema =
        json!({"type":"object", "required":["answer"], "properties":{"answer":{"type":"string"}}});
    let result = run("解釋這個模組", cwd, Some(&schema), 20);
    assert_eq!(result["status"], "completed");
    assert_eq!(
        result["structured_output"],
        json!({"answer":"解釋這個模組", "schema_keys":["answer"]})
    );
    let bad = run("AGY_BADSHAPE", cwd, Some(&schema), 20);
    assert_eq!(bad["error"]["kind"], "invalid_structured_output");
    assert!(bad["error"]["message"]
        .as_str()
        .unwrap()
        .contains("$.answer: required"));
    assert_eq!(
        run("AGY_EMPTY", cwd, Some(&schema), 20)["error"]["kind"],
        "empty_response"
    );
    assert_eq!(
        agy::run(
            &common::fake("fake_agy"),
            " ",
            cwd,
            "m",
            None,
            1,
            &agy::Children::new()
        )
        .unwrap_err()
        .kind,
        "invalid_argument"
    );
}
#[test]
fn empty_directory_and_timeout() {
    let dir = common::temp();
    std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
    assert_eq!(run("AGY_CWD", None, None, 20)["response"], "[]");
    assert_eq!(
        run("AGY_CWD", Some(dir.path()), None, 20)["response"],
        "[\"secret.txt\"]"
    );
    assert_eq!(
        run("AGY_HANG", Some(dir.path()), None, 1)["status"],
        "timeout"
    );
}

fn pid_command(path: &std::path::Path) -> Vec<String> {
    vec![common::python(), "-I".into(), "-c".into(),
        "import os,runpy,sys; os.environ['FAKE_AGY_PID']=sys.argv[1]; sys.argv=sys.argv[2:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(),
        path.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_agy.py").to_string_lossy().into_owned()]
}
#[test]
fn timeout_and_cancel_kill_child() {
    let dir = common::temp();
    let pidfile = dir.path().join("pid");
    let result = agy::run(
        &pid_command(&pidfile),
        "AGY_HANG",
        Some(dir.path()),
        "m",
        None,
        2,
        &agy::Children::new(),
    )
    .unwrap();
    assert_eq!(result["status"], "timeout");
    let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().parse().unwrap();
    assert!(!axiom::base::proc::pid_alive(pid));
    std::fs::remove_file(&pidfile).unwrap();
    let command = pid_command(&pidfile);
    let cwd = dir.path().to_owned();
    let children = std::sync::Arc::new(agy::Children::new());
    let worker_children = children.clone();
    let thread = std::thread::spawn(move || {
        agy::run(
            &command,
            "AGY_HANG",
            Some(&cwd),
            "m",
            None,
            30,
            &worker_children,
        )
        .unwrap()
    });
    let start = std::time::Instant::now();
    let pid: u32 = loop {
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|s| s.parse().ok())
        {
            break pid;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let start = std::time::Instant::now();
    children.cancel();
    let result = thread.join().unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(result["status"], "bridge_error");
    assert_eq!(result["error"]["kind"], "agy_no_result");
    assert!(!axiom::base::proc::pid_alive(pid));
    assert_eq!(children.len(), 0);
}
