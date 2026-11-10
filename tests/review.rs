mod common;

use axiom::{
    base::{self, Family},
    review::{self, Config, Outcome, Review},
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::Duration,
};

const RANGE: &str = "HEAD~1..HEAD";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
/// A repository whose last commit adds `files`.
fn repo(files: &[(&str, String)]) -> tempfile::TempDir {
    let temp = common::temp();
    let dir = temp.path().join("repo");
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    fs::write(dir.join("README"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    for (path, text) in files {
        let at = dir.join(path);
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        fs::write(at, text).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "change"]);
    temp
}
fn repo_dir(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("repo")
}
fn source(extra: &str) -> Vec<(&'static str, String)> {
    vec![("src/a.py", format!("print('one')\n# {extra}\n"))]
}

struct Case {
    temp: tempfile::TempDir,
    log: PathBuf,
}
impl Case {
    fn new() -> Self {
        let temp = common::temp();
        let log = temp.path().join("agy.jsonl");
        Self { temp, log }
    }
    fn agy(&self, scenario: &str) -> Vec<String> {
        let mut argv = common::fake("fake_member");
        argv.push(format!("--fake-scenario={scenario}"));
        argv.push(format!("--fake-log={}", self.log.display()));
        argv
    }
    fn config(&self, codex: bool, agy: Option<&str>) -> Config {
        let missing = || Err(base::Failure::new("not_installed", "not on PATH"));
        Config {
            codex: if codex {
                Ok(common::fake_codex(&[]))
            } else {
                missing()
            },
            agy: agy.map_or_else(missing, |s| Ok(self.agy(s))),
            codex_timeout: Duration::from_secs(60),
            agy_timeout: Duration::from_secs(60),
            schema_dir: self.temp.path().join("schemas"),
        }
    }
    /// Prompts agy was started with.
    fn agy_prompts(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["prompt"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
}
fn run(config: &Config, temp: &tempfile::TempDir) -> axiom::base::Res<Outcome> {
    review::run(config, &repo_dir(temp), RANGE, &AtomicBool::new(false))
}
fn reviewed(outcome: Outcome) -> Review {
    match outcome {
        Outcome::Reviewed(r) => r,
        Outcome::Skipped(why) => panic!("expected a review, got skipped: {why}"),
    }
}
fn skipped(outcome: Outcome) -> String {
    match outcome {
        Outcome::Skipped(why) => why,
        Outcome::Reviewed(r) => panic!("expected skipped, got a review by {}", r.reviewer),
    }
}

#[test]
fn codex_reviews_and_agy_is_not_used() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    let r = reviewed(run(&case.config(true, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "codex");
    assert_eq!(r.family, "openai");
    assert_eq!(r.verdict, "pass");
    assert!(r.pass());
    assert_eq!(r.fallback, None);
    assert!(case.agy_prompts().is_empty(), "agy must not run");
}

#[test]
fn codex_out_of_quota_falls_back_to_agy() {
    let case = Case::new();
    let temp = repo(&source("LIMIT"));
    let r = reviewed(run(&case.config(true, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "agy");
    assert_eq!(r.verdict, "pass");
    assert!(r.fallback.as_deref().unwrap().contains("usageLimitExceeded"));
    assert_eq!(case.agy_prompts().len(), 1);
}

#[test]
fn codex_auth_failure_falls_back_to_agy() {
    let case = Case::new();
    let temp = repo(&source("AUTH"));
    let r = reviewed(run(&case.config(true, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "agy");
    assert!(r.fallback.as_deref().unwrap().contains("AuthError"));
}

#[test]
fn codex_absent_falls_back_to_agy() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    let r = reviewed(run(&case.config(false, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "agy");
    assert_eq!(r.fallback.as_deref(), Some("codex is not installed"));
}

#[test]
fn agy_review_is_recorded_as_google() {
    let case = Case::new();
    let temp = repo(&source("BLOCKME"));
    let r = reviewed(run(&case.config(false, Some("")), &temp).unwrap());
    assert!(r.model.starts_with("gemini-"), "{}", r.model);
    assert_eq!(base::family_of(&r.model), Family::Google);
    assert_eq!(r.family, "google");
    let json = Outcome::Reviewed(r).to_json();
    assert_eq!(json["family"], "google");
    assert_eq!(json["verdict"], "revise");
    assert_eq!(json["pass"], false);
    assert_eq!(json["findings"][0]["severity"], "major");
}

#[test]
fn a_major_finding_fails_a_pass_verdict() {
    let r = Review {
        reviewer: "agy",
        model: "gemini-4-argon-high".into(),
        family: "google",
        verdict: "pass".into(),
        findings: vec![serde_json::json!({"severity":"Blocker"})],
        tokens: 0,
        files: vec![],
        omitted: vec![],
        fallback: None,
    };
    assert!(!r.pass());
}

#[test]
fn neither_reviewer_available_is_skipped() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    let why = skipped(run(&case.config(false, None), &temp).unwrap());
    assert!(why.contains("codex is not installed") && why.contains("agy is not installed"));
    // Codex out of quota and no agy is the same outcome.
    let temp = repo(&source("LIMIT"));
    let why = skipped(run(&case.config(true, None), &temp).unwrap());
    assert!(why.contains("agy is not installed"), "{why}");
}

#[test]
fn agy_timeout_is_skipped() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    let mut config = case.config(false, Some("hang"));
    config.agy_timeout = Duration::from_secs(1);
    let why = skipped(run(&config, &temp).unwrap());
    assert!(why.contains("agy timed out"), "{why}");
}

#[test]
fn agy_quota_and_bad_output_are_skipped() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    for (scenario, expect) in [
        ("garbage", "malformed output"),
        ("wrong-shape", "malformed output"),
        ("quota", "out of quota"),
        ("crash", "failed"),
    ] {
        let why = skipped(run(&case.config(false, Some(scenario)), &temp).unwrap());
        assert!(why.contains(&format!("agy {expect}")), "{scenario}: {why}");
    }
}

#[test]
fn another_codex_failure_is_an_error_and_never_reaches_agy() {
    let case = Case::new();
    let temp = repo(&source("BADMODEL"));
    let err = run(&case.config(true, Some("")), &temp).unwrap_err();
    assert_eq!(err.kind, "review_failed", "{err:?}");
    assert!(case.agy_prompts().is_empty());
}

#[test]
fn the_prompt_is_bounded_and_holds_only_repository_text() {
    let case = Case::new();
    let outside = case.temp.path().join("outside-secret.txt");
    fs::write(&outside, "OUTSIDE-SECRET").unwrap();
    let big: String = (0..400)
        .map(|n| format!("line {n} {}\n", "x".repeat(500)))
        .collect();
    let temp = repo(&[
        ("src/small.py", "LIMIT\nprint(1)\n".to_owned()),
        ("src/big.py", big),
        ("docs/ünï.md", "héllo\n".to_owned()),
    ]);
    let r = reviewed(run(&case.config(true, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "agy");
    assert_eq!(r.omitted, vec!["src/big.py".to_string()]);
    assert_eq!(r.files.len(), 3);
    let prompt = &case.agy_prompts()[0];
    assert!(prompt.encode_utf16().count() <= axiom::council::AGY_PROMPT_LIMIT);
    for path in ["src/small.py", "src/big.py", "docs/ünï.md"] {
        assert!(prompt.contains(path), "{path} is not listed");
    }
    assert!(prompt.contains("src/big.py  +400 -0  (read this file directly)"));
    assert!(!prompt.contains("line 399"));
    assert!(prompt.contains("print(1)"));
    let root = repo_dir(&temp).to_string_lossy().into_owned();
    for outside in [
        root.as_str(),
        "OUTSIDE-SECRET",
        case.temp.path().to_str().unwrap(),
    ] {
        assert!(!prompt.contains(outside), "prompt leaks {outside}");
    }
}

#[test]
fn many_files_with_long_paths_still_fit_the_agy_command_line() {
    let case = Case::new();
    // 250 changed files: the listing alone is far larger than the command-line limit.
    let files: Vec<(String, String)> = (0..250)
        .map(|n| (format!("pkg/{}{n:03}.txt", "p".repeat(110)), format!("content {n}
")))
        .collect();
    let files: Vec<(&str, String)> = files.iter().map(|(p, t)| (p.as_str(), t.clone())).collect();
    let temp = repo(&files);
    let r = reviewed(run(&case.config(false, Some("")), &temp).unwrap());
    assert_eq!(r.reviewer, "agy", "the backup must still run");
    assert_eq!(r.files.len(), 250);
    let prompt = &case.agy_prompts()[0];
    let units = prompt.encode_utf16().count();
    assert!(units <= axiom::council::AGY_PROMPT_LIMIT, "prompt is {units} UTF-16 units");
    assert!(prompt.contains("## Diffs"), "the fixed text is still there");
    assert!(prompt.contains("more changed files"), "the cut listing says how many were left out");
}

#[test]
fn a_bad_range_is_refused() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    let config = case.config(true, Some(""));
    let stop = AtomicBool::new(false);
    for range in ["--output=x", "", "HEAD..HEAD", "no-such-ref..HEAD"] {
        let err = review::run(&config, &repo_dir(&temp), range, &stop).unwrap_err();
        assert!(
            matches!(err.kind.as_str(), "invalid_argument" | "git_failed"),
            "{range}: {err:?}"
        );
    }
    assert!(case.agy_prompts().is_empty());
}

fn axiom(case: &Case, temp: &tempfile::TempDir, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
    command
        .arg("review")
        .args(args)
        .arg("--project")
        .arg(repo_dir(temp))
        .env("AXIOM_HOME", case.temp.path())
        .env("AXIOM_DATA_HOME", case.temp.path());
    command
}

#[test]
fn command_exits_zero_when_skipped_and_prints_the_reason() {
    let case = Case::new();
    let temp = repo(&source("plain"));
    // PATH holds git alone, so neither codex nor agy can be found.
    let git_dir = Path::new(&base::on_path("git").expect("git on PATH"))
        .parent()
        .unwrap()
        .to_path_buf();
    let output = {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
        command
            .args(["review", RANGE, "--project"])
            .arg(repo_dir(&temp))
            .env("PATH", &git_dir)
            .env_remove("AXIOM_CODEX_CMD")
            .env_remove("AXIOM_AGY_CMD")
            .env("AXIOM_HOME", case.temp.path());
        command.output().unwrap()
    };
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.starts_with("skipped: "), "{text}");
    assert!(text.contains("agy is not installed"));
}

#[test]
fn command_reports_the_agy_verdict_as_json() {
    let case = Case::new();
    let temp = repo(&source("LIMIT BLOCKME"));
    let json = |argv: Vec<String>| serde_json::to_string(&argv).unwrap();
    let output = axiom(&case, &temp, &[RANGE, "--json"])
        .env("AXIOM_CODEX_CMD", json(common::fake_codex(&[])))
        .env("AXIOM_AGY_CMD", json(case.agy("")))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["status"], "ok");
    assert_eq!(v["reviewer"], "agy");
    assert_eq!(v["family"], "google");
    assert_eq!(v["verdict"], "revise");
    assert!(v["findings"][0]["severity"].is_string());
    // Codex succeeding needs no agy.
    let temp = repo(&source("plain"));
    let output = axiom(&case, &temp, &[RANGE, "--json"])
        .env("AXIOM_CODEX_CMD", json(common::fake_codex(&[])))
        .env("AXIOM_AGY_CMD", json(case.agy("crash")))
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["reviewer"], "codex");
}

#[test]
fn command_without_a_range_is_a_usage_error() {
    let case = Case::new();
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .arg("review")
        .env("AXIOM_HOME", case.temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn doctor_lists_agy_as_optional() {
    let case = Case::new();
    let fake = serde_json::to_string(&common::fake("fake_claude")).unwrap();
    let codex =
        serde_json::to_string(&common::fake_codex(&[&common::tested_codex_flag()])).unwrap();
    let doctor = |agy: Option<Vec<String>>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
        command
            .arg("doctor")
            .env("AXIOM_HOME", case.temp.path())
            .env("AXIOM_DATA_HOME", case.temp.path())
            .env("AXIOM_CLAUDE_CMD", &fake)
            .env("AXIOM_CODEX_CMD", &codex)
            .env("AXIOM_WSL_CMD", &fake)
            .env(
                "AXIOM_PWSH_CMD",
                serde_json::to_string(&vec!["pwsh"]).unwrap(),
            );
        match agy {
            Some(argv) => command.env("AXIOM_AGY_CMD", serde_json::to_string(&argv).unwrap()),
            None => command.env_remove("AXIOM_AGY_CMD"),
        };
        command.output().unwrap()
    };
    let output = doctor(Some(case.agy("")));
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.lines()
            .any(|l| l.starts_with("ok   agy") && l.contains("optional")),
        "{text}"
    );
    // A broken agy setting only warns: agy is never required.
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .arg("doctor")
        .env("AXIOM_HOME", case.temp.path())
        .env("AXIOM_DATA_HOME", case.temp.path())
        .env("AXIOM_CLAUDE_CMD", &fake)
        .env("AXIOM_CODEX_CMD", &codex)
        .env("AXIOM_WSL_CMD", &fake)
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec!["pwsh"]).unwrap(),
        )
        .env("AXIOM_AGY_CMD", "not json")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|l| l.starts_with("warn agy")));
    let _ = doctor(None);
}
