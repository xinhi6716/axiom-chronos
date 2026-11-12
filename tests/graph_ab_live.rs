mod common;

use axiom::{
    graph::{
        self,
        measure::{self, AbReport, ClaudeUsage, RunUsage},
        GraphStore,
    },
    usage::{self, Locations},
};
use std::{collections::BTreeMap, error::Error, fs, path::Path, process::Command};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            return Err(format!("unsupported fixture entry: {}", entry.path().display()).into());
        }
    }
    Ok(())
}

fn session_usage(argv: &[String], prompt: &str, repo: &Path, home: &Path) -> Result<RunUsage> {
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .args(["--output-format", "json", prompt])
        .current_dir(repo)
        .output()?;
    let session = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|v| v["session_id"].as_str().map(str::to_owned));
    let mut locations = Locations::from_env();
    locations.home = home.to_path_buf();
    // Only accept a transcript in this fresh working directory.
    let project = locations
        .projects
        .join(usage::transcript_dir_name(&repo.to_string_lossy()));
    let Some(session) = session.filter(|s| project.join(format!("{s}.jsonl")).is_file()) else {
        return Ok(RunUsage::default());
    };
    let Ok(ledger) = usage::ledger(&locations, Some(&session)) else {
        return Ok(RunUsage::default());
    };
    Ok(convert_ledger(&ledger))
}

fn convert_ledger(ledger: &usage::Ledger) -> RunUsage {
    let t = &ledger.totals;
    let known = |i: usize, n| (t.calls > 0 && !t.missing[i]).then_some(n);
    let claude = ClaudeUsage {
        input: known(0, t.input),
        cache_read: known(1, t.cache_read),
        cache_write: known(2, t.cache_write),
        output: known(3, t.output),
    };
    let codex = if ledger.codex.is_empty() {
        None
    } else {
        let sum = |index: usize, get: fn(&usage::CodexTokens) -> u64| {
            if ledger.codex_runs_without_usage > 0
                || ledger.codex.iter().any(|r| r.tokens.missing[index])
            {
                None
            } else {
                ledger
                    .codex
                    .iter()
                    .try_fold(0u64, |n, r| n.checked_add(get(&r.tokens)))
            }
        };
        Some(ClaudeUsage {
            input: sum(0, |t| t.input),
            cache_write: None,
            cache_read: sum(1, |t| t.cached_input),
            output: sum(2, |t| t.output),
        })
    };
    RunUsage { claude, codex }
}

fn run_ab(claude_argv: Option<String>, out_dir: &Path) -> Result<AbReport> {
    let unset = claude_argv.is_none();
    let argv = claude_argv
        .map(|s| serde_json::from_str::<Vec<String>>(&s))
        .transpose()?;
    if argv
        .as_ref()
        .is_some_and(|a| a.is_empty() || a[0].is_empty())
    {
        return Err("AXIOM_AB_CLAUDE must contain a nonempty executable".into());
    }
    let fixtures = common::repo_root().join("tests/fixtures/graph/eval");
    let manifest = measure::load_manifest(&fixtures)?;
    let mut runs = BTreeMap::new();
    let mut payload = Vec::new();
    if let Some(argv) = argv {
        let temp = tempfile::tempdir()?;
        for scenario in &manifest.scenarios {
            let seed_dir = temp.path().join(&scenario.id);
            let seed_repo = seed_dir.join("repo");
            let seed_home = seed_dir.join("home");
            copy_dir(&fixtures.join(&scenario.repo), &seed_repo)?;
            let store = GraphStore::open(&seed_home)?;
            graph::code::sync(&store, &seed_repo, None, &Default::default())?;
            measure::seed_home(&seed_home, &seed_repo, &fixtures, Path::new(&scenario.seed))?;
            let a = fs::read_to_string(fixtures.join(&scenario.a_brief))?;
            let b = measure::graph_brief(&seed_home, &seed_repo, scenario)?;
            let pos = b.find("\n\n{").ok_or("graph brief has no JSON payload")? + 2;
            payload.push(measure::payload_check(scenario, &a, &b, b[pos..].len()));
            for (arm, prompt) in [("A", &a), ("B", &b)] {
                for i in 0..3 {
                    let run_dir = tempfile::tempdir()?;
                    let repo = run_dir.path().join("repo");
                    copy_dir(&fixtures.join(&scenario.repo), &repo)?;
                    let usage = session_usage(&argv, prompt, &repo, &run_dir.path().join("home"))?;
                    runs.insert((scenario.id.clone(), arm.to_owned(), i), usage);
                }
            }
        }
    }
    let mut report = measure::evaluate(&manifest, payload, &runs, None);
    if unset {
        report.reason = Some("AXIOM_AB_CLAUDE is unset.".into());
    }
    // Zero is the sum identity, not a measurement when every value is unknown.
    for arm in ["A", "B"] {
        let count = runs.keys().filter(|(_, a, _)| a == arm).count() as u64;
        for provider in ["claude", "codex"] {
            for name in ["input", "cache_write", "cache_read", "output"] {
                if report.classes[arm][provider]["unknown"][name].as_u64() == Some(count) {
                    report.classes[arm][provider][name] = serde_json::Value::Null;
                }
            }
        }
    }
    fs::create_dir_all(out_dir)?;
    fs::write(
        out_dir.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    fs::write(out_dir.join("report.md"), measure::report_markdown(&report))?;
    Ok(report)
}

#[test]
#[ignore = "starts real Claude sessions when AXIOM_AB_CLAUDE is set"]
fn live_graph_ab() -> Result<()> {
    run_ab(
        std::env::var("AXIOM_AB_CLAUDE").ok(),
        &common::repo_root().join("target/graph-ab"),
    )?;
    Ok(())
}

#[test]
fn unset_is_not_measured() -> Result<()> {
    let out = tempfile::tempdir()?;
    let report = run_ab(None, out.path())?;
    assert_eq!(report.status, "not_measured");
    assert!(report
        .reason
        .as_deref()
        .unwrap()
        .contains("AXIOM_AB_CLAUDE"));
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(out.path().join("report.json"))?)?;
    assert_eq!(json["status"], "not_measured");
    assert!(json["reason"].as_str().unwrap().contains("AXIOM_AB_CLAUDE"));
    assert!(fs::read_to_string(out.path().join("report.md"))?.contains("Not measured"));
    Ok(())
}

#[test]
fn malformed_argv_writes_no_report() -> Result<()> {
    for argv in ["not json", "{}", "[1]", "[]", "[\"\"]"] {
        let out = tempfile::tempdir()?;
        assert!(run_ab(Some(argv.into()), out.path()).is_err());
        assert!(!out.path().join("report.json").exists());
        assert!(!out.path().join("report.md").exists());
    }
    Ok(())
}

#[test]
fn failed_sessions_never_measure_zero_usage() -> Result<()> {
    let out = tempfile::tempdir()?;
    let argv = serde_json::to_string(&[
        common::python(),
        "-c".into(),
        "import sys; sys.exit(1)".into(),
    ])?;
    let report = run_ab(Some(argv), out.path())?;
    assert!(report.status == "not_measured" || report.status == "incomplete");
    for scenario in &report.scenarios {
        assert!(scenario["a_median"].is_null());
        assert!(scenario["b_median"].is_null());
    }
    for arm in ["A", "B"] {
        for provider in ["claude", "codex"] {
            for name in ["input", "cache_write", "cache_read", "output"] {
                assert!(report.classes[arm][provider][name].is_null());
                assert_eq!(report.classes[arm][provider]["unknown"][name], 36);
            }
        }
    }
    Ok(())
}

#[test]
fn transcript_usage_preserves_unreported_classes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let locations = Locations {
        home: dir.path().join("home"),
        projects: dir.path().join("projects"),
    };
    let project = locations.projects.join("project");
    fs::create_dir_all(&project)?;
    let session = "11111111-2222-3333-4444-555555555555";
    let call = serde_json::json!({
        "type": "assistant", "timestamp": "2026-01-01T00:00:00Z",
        "message": {
            "id": "message", "model": "claude-opus-5-5", "content": [],
            "usage": { "input_tokens": 12, "cache_read_input_tokens": 0, "output_tokens": 7 }
        }
    });
    fs::write(
        project.join(format!("{session}.jsonl")),
        format!("{call}\n"),
    )?;
    let ledger = usage::ledger(&locations, Some(session))?;
    let result = convert_ledger(&ledger);
    assert_eq!(result.claude.input, Some(12));
    assert_eq!(result.claude.cache_write, None);
    assert_eq!(result.claude.cache_read, Some(0));
    assert_eq!(result.claude.output, Some(7));
    assert!(result.codex.is_none());
    Ok(())
}
