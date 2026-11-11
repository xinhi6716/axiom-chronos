mod common;

use axiom::evolve::{self, Inputs};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = common::temp();
        let home = root.path().join("home");
        let data = root.path().join("data");
        let config = root.path().join("config");
        for dir in [&home, &data, &config] {
            fs::create_dir_all(dir).unwrap();
        }
        Self {
            root,
            home,
            data,
            config,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_axiom"));
        cmd.env("AXIOM_HOME", &self.home)
            .env("AXIOM_DATA_HOME", &self.data)
            .env("CLAUDE_CONFIG_DIR", &self.config)
            .env("AXIOM_CLAUDE_CMD", "axiom-nonexistent-claude-command")
            .current_dir(self.root.path());
        cmd
    }

    fn preview(&self, since_days: u32) -> String {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        evolve::preview(&Inputs {
            home: self.home.clone(),
            projects: self.config.join("projects"),
            now_ms,
            since_days,
        })
        .unwrap()
        .text
    }
}

fn tree(root: &Path) -> Vec<(PathBuf, Option<u64>)> {
    fn visit(root: &Path, dir: &Path, entries: &mut Vec<(PathBuf, Option<u64>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            entries.push((
                path.strip_prefix(root).unwrap().to_owned(),
                (!meta.is_dir()).then_some(meta.len()),
            ));
            if meta.is_dir() {
                visit(root, &path, entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort();
    entries
}

fn assert_error(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("axiom: "),
        "{output:?}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn dry_run_matches_preview_without_writes_and_ignores_repo() {
    let f = Fixture::new();
    fs::create_dir(f.home.join("goals")).unwrap();
    let before_tree = tree(&f.home);
    for (args, days) in [
        (vec!["evolve", "--dry-run"], evolve::DEFAULT_SINCE_DAYS),
        (
            vec!["evolve", "--since", "1", "--dry-run", "--repo", "missing"],
            1,
        ),
        (
            vec!["evolve", "--since", "365", "--dry-run"],
            evolve::MAX_SINCE_DAYS,
        ),
    ] {
        let before = f.preview(days);
        let output = f.command().args(args).output().unwrap();
        let after = f.preview(days);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let actual = String::from_utf8(output.stdout).unwrap();
        assert!(
            actual == before || actual == after,
            "unexpected preview: {actual:?}"
        );
        assert_eq!(tree(&f.home), before_tree);
        assert!(!f.home.join("evolve").exists());
    }
}

#[test]
fn invalid_arguments_fail_before_writing() {
    let f = Fixture::new();
    for args in [
        vec!["--since", "0"],
        vec!["--since", "abc"],
        vec!["--since", "366"],
        vec!["--since"],
        vec!["--unknown"],
        vec!["--repo"],
        vec!["--since", "--dry-run"],
        vec!["extra"],
    ] {
        let output = f.command().arg("evolve").args(&args).output().unwrap();
        assert_error(&output, 2);
        assert!(tree(&f.home).is_empty(), "arguments: {args:?}");
    }
}

#[test]
fn non_git_repo_fails_without_creating_runs() {
    let f = Fixture::new();
    let output = f
        .command()
        .args(["evolve", "--repo"])
        .arg(&f.data)
        .output()
        .unwrap();
    assert_error(&output, 1);
    assert!(!f.home.join("evolve/runs").exists());
}

#[test]
fn invalid_remembered_repo_is_usage_error() {
    let f = Fixture::new();
    fs::create_dir(f.home.join("evolve")).unwrap();
    fs::write(
        f.home.join("evolve/settings.json"),
        r#"{"repo":"relative"}"#,
    )
    .unwrap();
    let output = f.command().arg("evolve").output().unwrap();
    assert_error(&output, 2);
    assert!(!f.home.join("evolve/runs").exists());
}

#[test]
fn no_op_with_relative_repo_prints_reason_without_creating_branch() {
    let f = Fixture::new();
    let repo = f.root.path().join("repo");
    fs::create_dir(&repo).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"axiom-chronos\"\n",
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "Cargo.toml"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "initial",
        ],
    ] {
        assert!(Command::new("git")
            .current_dir(&repo)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    let output = f
        .command()
        .args(["evolve", "--repo", "repo"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "No recorded problems in the period.\n"
    );
    assert!(output.stderr.is_empty());
    assert!(!f.home.join("evolve").exists());
    let branches = Command::new("git")
        .current_dir(&repo)
        .args(["branch", "--list", "evolve/*"])
        .output()
        .unwrap();
    assert!(branches.status.success());
    assert!(branches.stdout.is_empty());
}

#[test]
fn run_propagates_fake_claude_exit_code() {
    let fake = serde_json::to_string(&common::fake("fake_claude")).unwrap();
    for (scenario, code) in [("ok", 0), ("crash", 3)] {
        let f = Fixture::new();
        let repo = f.root.path().join("repo");
        fs::create_dir(&repo).unwrap();
        fs::write(
            repo.join("Cargo.toml"),
            "[package]\nname = \"axiom-chronos\"\n",
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "Cargo.toml"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        ] {
            assert!(Command::new("git")
                .current_dir(&repo)
                .args(args)
                .status()
                .unwrap()
                .success());
        }

        let reports = f.home.join("orchestrate/project/reports");
        fs::create_dir_all(&reports).unwrap();
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let id = evolve::history::new_id(now_ms);
        let date = format!("{}-{}-{}", &id[..4], &id[4..6], &id[6..8]);
        fs::write(
            reports.join("unit.evidence.tsv"),
            format!(
                "kind\tcommand\texit_code\tcommit\tts\nverify\ttest\t1\tabc\t{date}T00:00:00Z\n"
            ),
        )
        .unwrap();

        let fake_record = f.root.path().join("fake-record.json");
        let output = f
            .command()
            .args(["evolve", "--repo", "repo"])
            .env("AXIOM_CLAUDE_CMD", &fake)
            .env("FAKE_CLAUDE_RECORD", &fake_record)
            .env("FAKE_CLAUDE_SCENARIO", scenario)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        let fake: serde_json::Value =
            serde_json::from_slice(&fs::read(&fake_record).unwrap()).unwrap();
        assert!(fake["messages"][0]["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("/chronos:wish"));
        let runs = evolve::list_runs(&f.home).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].state, "ended");
        assert_eq!(runs[0].exit_code, Some(code));
        assert!(runs[0].problems.iter().any(|p| p == "orch.verify-failed"));
    }
}

#[test]
fn unknown_command_still_fails() {
    let f = Fixture::new();
    assert_error(&f.command().arg("not-a-command").output().unwrap(), 2);
}
