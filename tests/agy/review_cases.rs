use crate::common;
use axiom::agy;
use std::fs;

#[test]
fn review_verdict_and_failures() {
    let dir = common::temp();
    fs::write(dir.path().join("spec.tla"), "VARIABLE x\nNEEDS_FIX\n").unwrap();
    fs::write(dir.path().join("ok.py"), "def f():\n    return 1\n").unwrap();
    let root = dir.path().to_str().unwrap();
    let run = |file: &str, contract: &str| {
        agy::review(
            root,
            &[file.into()],
            contract,
            "model",
            20,
            &common::fake("fake_agy"),
            &agy::Children::new(),
        )
        .unwrap()
    };
    let failed = run("spec.tla", "contract");
    assert_eq!(failed["verdict"], "fail");
    assert_eq!(failed["findings"][0]["severity"], "blocker");
    assert_eq!(failed["bytes_reviewed"], 21);
    let pass = run("ok.py", "contract");
    assert_eq!(pass["verdict"], "pass");
    assert_eq!(pass["reviewer"], "model");
    assert_eq!(pass["usage"]["total_tokens"], 1);
    for marker in ["AGY_EMPTY", "AGY_BADSHAPE", "AGY_ERROR", "AGY_SILENT"] {
        let result = run("ok.py", marker);
        assert_ne!(result["status"], "completed");
        assert_eq!(result["error"]["fallback"], true);
    }
}

#[test]
fn paths_and_size_limits() {
    let dir = common::temp();
    fs::write(dir.path().join("ok"), "ok").unwrap();
    fs::write(dir.path().join("big"), vec![b'x'; agy::MAX_FILE_BYTES + 1]).unwrap();
    fs::write(
        dir.path().join("part"),
        vec![b'x'; agy::MAX_TOTAL_BYTES / 2 + 1],
    )
    .unwrap();
    let sibling = common::temp();
    fs::write(sibling.path().join("f"), "outside").unwrap();
    fs::create_dir(dir.path().join("subdir")).unwrap();
    let relative = format!(
        "../{}/f",
        sibling.path().file_name().unwrap().to_string_lossy()
    );
    let absolute = sibling.path().join("f").to_string_lossy().into_owned();
    let count = dir.path().join("count");
    let command = count_command(&count);
    let root = dir.path().to_str().unwrap();
    let review = |paths: &[String], contract: &str| {
        agy::review(
            root,
            paths,
            contract,
            "m",
            20,
            &command,
            &agy::Children::new(),
        )
    };
    for paths in [
        vec![],
        vec!["missing".into()],
        vec![relative],
        vec![absolute],
        vec!["subdir".into()],
    ] {
        assert_eq!(
            review(&paths, "contract").unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert_eq!(
        review(&["ok".into()], " ").unwrap_err().kind,
        "invalid_argument"
    );
    for paths in [vec!["big".into()], vec!["part".into(), "part".into()]] {
        let result = review(&paths, "contract").unwrap();
        assert_eq!(result["status"], "failed");
        assert!(!count.exists());
        assert_eq!(result["error"]["kind"], "too_large");
        assert_eq!(result["error"]["fallback"], true);
    }
}

#[test]
fn empty_response_calls_agy_once() {
    let dir = common::temp();
    fs::write(dir.path().join("ok"), "ok").unwrap();
    let count = dir.path().join("count");
    let command = count_command(&count);
    let result = agy::review(
        dir.path().to_str().unwrap(),
        &["ok".into()],
        "AGY_EMPTY",
        "m",
        20,
        &command,
        &agy::Children::new(),
    )
    .unwrap();
    assert_eq!(result["error"]["fallback"], true);
    assert_eq!(result["error"]["kind"], "empty_response");
    assert_eq!(fs::read_to_string(count).unwrap().lines().count(), 1);
}

fn count_command(count: &std::path::Path) -> Vec<String> {
    vec![common::python(), "-I".into(), "-c".into(), "import os,runpy,sys; os.environ['FAKE_AGY_COUNT']=sys.argv[1]; sys.argv=sys.argv[2:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(), count.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_agy.py").to_string_lossy().into_owned()]
}
#[test]
fn structured_output_cases() {
    use serde_json::json;
    let dir = common::temp();
    fs::write(dir.path().join("f"), "x").unwrap();
    let finding =
        json!({"check":"untested","severity":"major","location":"f","evidence":"x","fix":"y"});
    let cases = [
        (json!(null), None),
        (json!({}), None),
        (json!({"verdict":"fail","findings":[]}), Some("fail")),
        (json!({"verdict":"pass","findings":[]}), Some("pass")),
        (json!({"findings":null}), None),
        (json!({"findings":[]}), None),
        (json!({"verdict":"maybe","findings":[]}), None),
        (json!({"verdict":"pass","findings":[finding]}), Some("fail")),
    ];
    for (output, expected) in cases {
        let command = vec![common::python(), "-I".into(), "-c".into(), "import os,runpy,sys; os.environ['FAKE_AGY_STRUCTURED']=sys.argv[1]; sys.argv=sys.argv[2:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(), output.to_string(), common::repo_root().join("tests/fakes/fake_agy.py").to_string_lossy().into_owned()];
        let result = agy::review(
            dir.path().to_str().unwrap(),
            &["f".into()],
            "contract",
            "m",
            20,
            &command,
            &agy::Children::new(),
        )
        .unwrap();
        assert_eq!(result["verdict"].as_str(), expected, "{output}");
        if expected.is_none() {
            assert_eq!(result["error"]["kind"], "no_review_output");
            assert_eq!(result["error"]["fallback"], true);
        } else {
            assert_eq!(result["status"], "completed");
        }
    }
}
