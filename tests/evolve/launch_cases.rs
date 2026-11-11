use crate::common;
use axiom::{
    evolve::{self, digest::MAX_PROBLEMS, launch, Inputs, StartOutcome},
    session::{Clock, Config},
};
use std::{
    fs,
    io::{BufRead, Cursor},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SESSION: &str = "abcdef12-1234-1234-1234-123456789abc";

fn inputs(root: &Path) -> Inputs {
    Inputs {
        home: root.join("home"),
        projects: root.join("projects"),
        now_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
        since_days: 7,
    }
}
fn make_config(home: &Path) -> Config {
    Config {
        claude: Ok(common::fake("fake_claude")),
        home: home.into(),
        bin: home.join("axiom"),
        clock: Clock::system(),
    }
}
fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Evolve Test",
            "-c",
            "user.email=evolve@example.test",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
fn repo(root: &Path) -> PathBuf {
    let dir = root.join("repo");
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init"]);
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"axiom-chronos\"\n",
    )
    .unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-m", "fixture"]);
    dir
}
fn blocked(home: &Path, session: &str) {
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(
        home.join("goals").join(format!("{session}.json")),
        r#"{"wish":"test","status":"blocked","note":"need input"}"#,
    )
    .unwrap();
}
fn date(ms: i64) -> String {
    let id = evolve::history::new_id(ms);
    format!("{}-{}-{}", &id[0..4], &id[4..6], &id[6..8])
}
fn run(inputs: &Inputs, repo: Option<&Path>, config: &Config) -> axiom::base::Res<StartOutcome> {
    evolve::start(inputs, repo, config, empty(), &mut Vec::new())
}

#[test]
fn read_only_and_invalid_arguments() {
    let temp = common::temp();
    let mut inputs = inputs(temp.path());
    let mut config = make_config(&inputs.home);
    let preview = evolve::preview(&inputs).unwrap();
    assert_eq!(preview.text, evolve::digest::render(&preview.digest));
    assert!(preview.text.contains("Estimate:"));
    assert!(evolve::list_runs(&inputs.home).unwrap().is_empty());
    assert!(!inputs.home.join("evolve").exists());
    for days in [0, 366] {
        inputs.since_days = days;
        assert_eq!(
            evolve::preview(&inputs).unwrap_err().kind,
            "invalid_argument"
        );
        assert_eq!(
            run(&inputs, None, &config).unwrap_err().kind,
            "invalid_argument"
        );
    }
    inputs.since_days = 7;
    config.home = temp.path().join("other");
    assert_eq!(
        run(&inputs, None, &config).unwrap_err().kind,
        "invalid_argument"
    );
    assert!(!inputs.home.exists());
}

#[test]
fn fake_launcher_covers_start_lifecycle() {
    // Environment is scoped to this one test; no other evolve tests alter these variables.
    let keys = [
        "FAKE_CLAUDE_RECORD",
        "FAKE_CLAUDE_SCENARIO",
        "FAKE_CLAUDE_SESSION",
    ];
    struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                if let Some(v) = v {
                    std::env::set_var(k, v)
                } else {
                    std::env::remove_var(k)
                }
            }
        }
    }
    let _restore = Restore(keys.into_iter().map(|k| (k, std::env::var_os(k))).collect());
    let temp = common::temp();
    let record_file = temp.path().join("fake-record.json");
    std::env::set_var("FAKE_CLAUDE_RECORD", &record_file);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "ok");
    std::env::set_var("FAKE_CLAUDE_SESSION", SESSION);
    let dir = repo(temp.path());
    let mut inputs = inputs(temp.path());
    let config = make_config(&inputs.home);

    assert!(matches!(
        run(&inputs, Some(&dir), &config).unwrap(),
        StartOutcome::NoOp { .. }
    ));
    assert!(!record_file.exists());
    assert!(!inputs.home.join("evolve").exists());
    assert!(!git(
        &dir,
        &[
            "branch",
            "--list",
            &format!("evolve/{}", date(inputs.now_ms))
        ]
    )
    .contains("evolve/"));

    blocked(&inputs.home, SESSION);
    let reports = inputs.home.join("orchestrate/project/reports");
    fs::create_dir_all(&reports).unwrap();
    let findings: Vec<_> = (0..12)
        .map(|i| serde_json::json!({"severity": format!("category-{i:02}")}))
        .collect();
    fs::write(
        reports.join("unit.verdict.json"),
        serde_json::json!({"findings": findings}).to_string(),
    )
    .unwrap();
    inputs.now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        + 1000;
    let preview = evolve::preview(&inputs).unwrap();
    assert!(preview.digest.problems.len() > MAX_PROBLEMS);
    fs::write(dir.join("dirty"), "dirty").unwrap();
    assert_eq!(
        run(&inputs, Some(&dir), &config).unwrap_err().kind,
        "dirty_tree"
    );
    assert!(!record_file.exists());
    fs::remove_file(dir.join("dirty")).unwrap();

    let (record, code) = match run(&inputs, Some(&dir), &config).unwrap() {
        StartOutcome::Ran { record, exit_code } => (record, exit_code),
        _ => panic!("expected run"),
    };
    assert_eq!(code, 0);
    assert_eq!(record.goal_status.as_deref(), Some("blocked"));
    assert_eq!(record.state, "ended");
    assert_eq!(record.session.as_deref(), Some(SESSION));
    assert_eq!(
        evolve::list_runs(&inputs.home).unwrap(),
        vec![record.clone()]
    );
    assert_eq!(
        git(&dir, &["branch", "--show-current"]),
        format!("evolve/{}", date(inputs.now_ms))
    );
    let fake: serde_json::Value = serde_json::from_slice(&fs::read(&record_file).unwrap()).unwrap();
    assert_eq!(
        fs::canonicalize(fake["cwd"].as_str().unwrap()).unwrap(),
        fs::canonicalize(&dir).unwrap()
    );
    let text = fake["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.starts_with("/chronos:wish"));
    assert!(text.contains(&evolve::digest::render(&preview.digest)));
    assert!(text.contains(launch::EVIDENCE_RULE));
    assert!(text.contains(launch::NO_MERGE_PUSH));
    assert!(text.contains(
        &fs::canonicalize(&dir)
            .unwrap()
            .to_string_lossy()
            .to_string()
    ));
    let ids: Vec<_> = preview
        .digest
        .problems
        .iter()
        .take(MAX_PROBLEMS)
        .map(|p| p.id.clone())
        .collect();
    assert_eq!(record.problems, ids);
    assert!(ids.iter().all(|id| text.contains(id)));
    assert_eq!(record.metrics.len(), preview.digest.problems.len());
    for (metric, problem) in record.metrics.iter().zip(&preview.digest.problems) {
        assert_eq!(
            (&metric.problem, metric.value, metric.samples),
            (&problem.id, problem.metric.value, problem.metric.samples)
        );
    }
    assert!(!text.contains(&preview.digest.problems[MAX_PROBLEMS].id));

    inputs.now_ms += 86_400_000;
    std::env::set_var(
        "FAKE_CLAUDE_SESSION",
        "bbbbbbbb-1234-1234-1234-123456789abc",
    );
    let second_preview = evolve::preview(&inputs).unwrap();
    assert!(second_preview
        .digest
        .comparisons
        .iter()
        .any(|c| c.earlier_run == record.id && c.problem == ids[0]));
    let second = run(&inputs, None, &config).unwrap();
    match second {
        StartOutcome::Ran { record, .. } => assert_eq!(record.goal_status, None),
        _ => panic!("expected second run"),
    }
    let fake: serde_json::Value = serde_json::from_slice(&fs::read(&record_file).unwrap()).unwrap();
    let text = fake["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains(&record.id));

    inputs.now_ms += 86_400_000;
    let mut missing = make_config(&inputs.home);
    missing.claude = Ok(vec![temp
        .path()
        .join("nonexistent-launcher")
        .to_string_lossy()
        .into_owned()]);
    let error = run(&inputs, None, &missing).unwrap_err();
    assert_eq!(error.kind, "launch_failed");
    let failed = evolve::list_runs(&inputs.home)
        .unwrap()
        .into_iter()
        .find(|r| r.state == "launch-failed")
        .unwrap();
    assert!(failed.error.as_deref().is_some_and(|e| !e.is_empty()));
    assert!(error.message.contains(failed.error.as_deref().unwrap()));
}
