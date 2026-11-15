use serde::Deserialize;
use serde_json::Value;
use std::{fs, path::Path, process::Command};

#[derive(Deserialize)]
struct Fixture {
    merges: Vec<Merge>,
    p6_only: Vec<String>,
}

#[derive(Deserialize)]
struct Merge {
    branch: String,
    out_file: String,
    exit: Value,
    exit_note: Option<String>,
    tip: String,
    merge_commit: String,
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn is_ancestor(commit: &str) -> bool {
    let status = Command::new("git")
        .current_dir(root())
        .args(["merge-base", "--is-ancestor", commit, "HEAD"])
        .output()
        .expect("run git merge-base");
    match status.status.code() {
        Some(0) => true,
        Some(1) => false,
        other => panic!("git merge-base failed for {commit}: {other:?}"),
    }
}

fn commit_exists(commit: &str) -> bool {
    Command::new("git")
        .current_dir(root())
        .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .output()
        .expect("run git cat-file")
        .status
        .success()
}

fn exit_ok(exit: &Value, note: Option<&str>) -> bool {
    exit.as_i64() == Some(0)
        || (exit.as_str() == Some("unknown")
            && note.is_some_and(|note| !note.trim().is_empty() && note.contains("written by hand")))
}

fn merge_commit_ok(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) && commit_exists(commit) && is_ancestor(commit)
}

fn prompt_size_ok(inputs: &[&[u8]]) -> bool {
    let size: usize = inputs.iter().map(|input| {
        input.len() - input.windows(2).filter(|pair| *pair == b"\r\n").count()
    }).sum();
    size <= 20813
}

#[test]
fn recorded_merges_and_exclusions() {
    let path = root().join("tests/fixtures/r11/merges.json");
    let fixture: Fixture = serde_json::from_slice(&fs::read(&path).expect("read merge fixture"))
        .expect("parse merge fixture");
    assert!(!fixture.merges.is_empty(), "no recorded merges");
    let p11 = fixture.merges.iter().find(|merge| merge.branch == "r9/p11").expect("missing r9/p11 merge");
    assert_eq!(p11.exit.as_str(), Some("unknown"), "r9/p11 exit must be unknown");
    for merge in &fixture.merges {
        assert!(is_ancestor(&merge.tip), "{} tip {} is not an ancestor", merge.branch, merge.tip);
        assert!(!merge.out_file.is_empty());
        assert!(merge_commit_ok(&merge.merge_commit), "{} merge commit {} is not an ancestor", merge.branch, merge.merge_commit);
        assert!(exit_ok(&merge.exit, merge.exit_note.as_deref()), "{} integration run failed", merge.branch);
    }
    for commit in &fixture.p6_only {
        // r9/p6 is a local-only branch: it is never merged and never pushed. A clone without
        // it (CI) cannot hold the commit, so the commit cannot be an ancestor either; where the
        // branch exists, the exclusion is checked against history as before.
        if !commit_exists(commit) {
            eprintln!("excluded p6 commit {commit} is not in this clone; it cannot be an ancestor");
            continue;
        }
        assert!(!is_ancestor(commit), "excluded p6 commit {commit} is an ancestor");
    }
}

#[test]
fn exit_negative_control() {
    assert!(!exit_ok(&serde_json::json!(1), None));
    assert!(!exit_ok(&serde_json::json!("unknown"), None));
    assert!(!exit_ok(&serde_json::json!("unknown"), Some("")));
    assert!(!exit_ok(&serde_json::json!("unknown"), Some("some other note")));
    assert!(!exit_ok(&Value::Null, None));
    assert!(exit_ok(&serde_json::json!(0), None));
    assert!(exit_ok(&serde_json::json!("unknown"), Some("written by hand")));
}

#[test]
fn unrelated_sha_negative_control() {
    let unrelated = "0000000000000000000000000000000000000000";
    assert!(!commit_exists(unrelated));
    assert!(!merge_commit_ok(unrelated));
    let output = Command::new("git").current_dir(root()).args(["rev-parse", "HEAD"]).output().expect("run git rev-parse");
    assert!(output.status.success());
    let head = String::from_utf8(output.stdout).expect("HEAD is UTF-8");
    assert!(is_ancestor(head.trim()));
}

#[test]
fn package_version_and_prompt_limit() {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let package = manifest.split("[package]").nth(1).unwrap().split('[').next().unwrap();
    let versions: Vec<_> = package.lines().filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "version").then(|| value.trim())
    }).collect();
    assert_eq!(versions, ["\"3.0.0-beta.1\""]);

    let prompts: Vec<_> = ["engine/agents/dispatcher.md", "engine/DISPATCH.md", "engine/skills/wish/SKILL.md"]
        .iter().map(|path| fs::read(root().join(path)).unwrap()).collect();
    assert!(prompt_size_ok(&prompts.iter().map(Vec::as_slice).collect::<Vec<_>>()));
}

#[test]
fn prompt_size_negative_control() {
    assert!(!prompt_size_ok(&[&vec![b'x'; 20814]]));
    assert!(prompt_size_ok(&[&vec![b'x'; 20812], b"\r\n"]));
}
