use serde::Deserialize;
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
    exit: i32,
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
        .status()
        .expect("run git merge-base");
    match status.code() {
        Some(0) => true,
        Some(1) => false,
        other => panic!("git merge-base failed for {commit}: {other:?}"),
    }
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
    for merge in &fixture.merges {
        assert!(is_ancestor(&merge.tip), "{} tip {} is not an ancestor", merge.branch, merge.tip);
        assert!(!merge.out_file.is_empty() && !merge.merge_commit.is_empty());
        assert_eq!(merge.exit, 0, "{} integration run failed", merge.branch);
    }
    for commit in &fixture.p6_only {
        assert!(!is_ancestor(commit), "excluded p6 commit {commit} is an ancestor");
    }
}

#[test]
fn package_version_and_prompt_limit() {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let package = manifest.split("[package]").nth(1).unwrap().split('[').next().unwrap();
    let versions: Vec<_> = package.lines().filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "version").then(|| value.trim())
    }).collect();
    assert_eq!(versions, ["\"2.0.1\""]);

    let prompts: Vec<_> = ["engine/agents/dispatcher.md", "engine/DISPATCH.md", "engine/skills/wish/SKILL.md"]
        .iter().map(|path| fs::read(root().join(path)).unwrap()).collect();
    assert!(prompt_size_ok(&prompts.iter().map(Vec::as_slice).collect::<Vec<_>>()));
}

#[test]
fn prompt_size_negative_control() {
    assert!(!prompt_size_ok(&[&vec![b'x'; 20814]]));
    assert!(prompt_size_ok(&[&vec![b'x'; 20812], b"\r\n"]));
}
