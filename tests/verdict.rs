use axiom::orch::*;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn row(kind: &str, command: &str, commit: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: 0,
        commit: commit.into(),
        ts: "1".into(),
    }
}

fn setup(git: bool) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let store = root.path().join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    if git {
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    fs::write(repo.join("file.txt"), "one").unwrap();
    let brief = store.join("brief.txt");
    fs::write(&brief, "GOAL x\nSCOPE file.txt\nVERIFY echo ok\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nunit-fresh\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    append_evidence(&store, "unit-fresh", &row("run", "gpt-worker", "")).unwrap();
    let verified = snapshot(&repo, &["file.txt".into()]);
    append_evidence(&store, "unit-fresh", &row("verify", "echo ok", &verified)).unwrap();
    (root, repo, store)
}

fn state(store: &Path) -> String {
    derive_state(&evidence(store, "unit-fresh").unwrap(), false)
}

#[test]
fn review_requires_the_verified_tree_to_remain_unchanged() {
    let (_root, repo, store) = setup(true);
    fs::write(repo.join("file.txt"), "changed during review").unwrap();
    let before = evidence(&store, "unit-fresh").unwrap().len();
    let err =
        record_review(&store, &["unit-fresh".into()], "claude-review", true, &repo).unwrap_err();
    assert_eq!(err.kind, "stale_review");
    assert!(err.message.contains("unit-fresh"));
    assert_eq!(evidence(&store, "unit-fresh").unwrap().len(), before);
    assert_eq!(state(&store), "built");
    fs::write(repo.join("file.txt"), "another edit").unwrap();
    assert_eq!(
        record_review(&store, &["unit-fresh".into()], "claude-review", true, &repo)
            .unwrap_err()
            .kind,
        "stale_review"
    );
    assert_eq!(state(&store), "built");
}

#[test]
fn unchanged_tree_is_verified_but_non_git_is_not() {
    let (_root, repo, store) = setup(true);
    record_review(&store, &["unit-fresh".into()], "claude-review", true, &repo).unwrap();
    assert_eq!(state(&store), "verified");

    let (_root, repo, store) = setup(false);
    assert_eq!(
        record_review(&store, &["unit-fresh".into()], "claude-review", true, &repo)
            .unwrap_err()
            .kind,
        "stale_review"
    );
    assert_eq!(state(&store), "built");
    append_evidence(&store, "unit-fresh", &row("review", "claude-review", "")).unwrap();
    assert_eq!(state(&store), "built");
}

#[test]
fn stale_batch_appends_no_reviews_and_names_all_stale_units() {
    let (_root, repo, store) = setup(true);
    let brief = store.join("brief.txt");
    let mut units = fs::read_to_string(store.join("units.tsv")).unwrap();
    for id in ["unit-stale-a", "unit-stale-b"] {
        units.push_str(&format!("{id}\tm\tpending\t\t\t\t{}\n", brief.display()));
        append_evidence(&store, id, &row("run", "gpt-worker", "")).unwrap();
        append_evidence(&store, id, &row("verify", "echo ok", "old")).unwrap();
    }
    fs::write(store.join("units.tsv"), units).unwrap();
    let err = record_review(
        &store,
        &[
            "unit-fresh".into(),
            "unit-stale-a".into(),
            "unit-stale-b".into(),
        ],
        "claude-review",
        true,
        &repo,
    )
    .unwrap_err();
    assert_eq!(err.kind, "stale_review");
    assert!(err.message.contains("unit-stale-a"));
    assert!(err.message.contains("unit-stale-b"));
    assert!(!err.message.contains("unit-fresh"));
    for id in ["unit-fresh", "unit-stale-a", "unit-stale-b"] {
        assert_eq!(evidence(&store, id).unwrap().len(), 2);
    }
}

#[test]
fn legacy_matching_hashes_and_mismatches() {
    let run = row("run", "gpt-worker", "");
    let verify = row("verify", "test", "abc");
    assert_eq!(
        derive_state(
            &[
                run.clone(),
                verify.clone(),
                row("review", "claude-review", "abc")
            ],
            false
        ),
        "verified"
    );
    assert_eq!(
        derive_state(
            &[run, verify, row("review", "claude-review", "other")],
            false
        ),
        "built"
    );
}

#[test]
fn failed_review_with_mismatched_or_empty_commit_remains_failed() {
    for commit in ["other", ""] {
        let mut review = row("review", "claude-review", commit);
        review.exit_code = 1;
        assert_eq!(
            derive_state(
                &[
                    row("run", "gpt-worker", ""),
                    row("verify", "test", "abc"),
                    review
                ],
                false
            ),
            "review-failed"
        );
    }
}

#[test]
fn empty_latest_verify_commit_is_stale_in_git_repo() {
    let (_root, repo, store) = setup(true);
    append_evidence(&store, "unit-fresh", &row("verify", "test", "")).unwrap();
    let before = evidence(&store, "unit-fresh").unwrap().len();
    let err = record_review(&store, &["unit-fresh".into()], "critic", true, &repo).unwrap_err();
    assert_eq!(err.kind, "stale_review");
    assert!(err.message.contains("unit-fresh"));
    assert_eq!(evidence(&store, "unit-fresh").unwrap().len(), before);
}

#[test]
fn review_requires_latest_verify_not_an_earlier_matching_tree() {
    let (_root, repo, store) = setup(true);
    fs::write(repo.join("file.txt"), "second tree").unwrap();
    let second = snapshot(&repo, &["file.txt".into()]);
    assert_ne!(second, evidence(&store, "unit-fresh").unwrap()[1].commit);
    append_evidence(&store, "unit-fresh", &row("verify", "test", &second)).unwrap();
    fs::write(repo.join("file.txt"), "one").unwrap();
    let before = evidence(&store, "unit-fresh").unwrap().len();
    let err = record_review(&store, &["unit-fresh".into()], "critic", true, &repo).unwrap_err();
    assert_eq!(err.kind, "stale_review");
    assert!(err.message.contains("unit-fresh"));
    assert_eq!(evidence(&store, "unit-fresh").unwrap().len(), before);
    assert_eq!(state(&store), "built");
}

#[test]
fn later_edit_cannot_make_a_failed_review_verified() {
    let (_root, repo, store) = setup(true);
    record_review(&store, &["unit-fresh".into()], "critic", false, &repo).unwrap();
    assert_eq!(state(&store), "review-failed");
    fs::write(repo.join("file.txt"), "later edit").unwrap();
    assert_eq!(
        record_review(&store, &["unit-fresh".into()], "reviewer", true, &repo)
            .unwrap_err()
            .kind,
        "stale_review"
    );
    assert_eq!(state(&store), "review-failed");
    assert_eq!(evidence(&store, "unit-fresh").unwrap().len(), 3);
}

#[test]
fn fallback_reviewer_names_work_on_unchanged_tree() {
    for reviewer in ["critic", "reviewer"] {
        let (_root, repo, store) = setup(true);
        record_review(&store, &["unit-fresh".into()], reviewer, true, &repo).unwrap();
        assert_eq!(state(&store), "verified");
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let store = temp.path().join("store");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("a.txt"), "before\n").unwrap();
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        fs::write(
            store.join("a.brief"),
            "GOAL x\nSCOPE a.txt\nVERIFY echo ok\n",
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
                store.join("a.brief").display()
            ),
        )
        .unwrap();
        fs::write(
            store.join("ledger.tsv"),
            "pr\tsha\tverdict\tevidence\tverifier\tts\n",
        )
        .unwrap();
        let Begin::Run { attempt_id, .. } = begin_unit(&store, "a", "terra", &repo, false).unwrap()
        else {
            panic!()
        };
        fs::write(repo.join("a.txt"), "after\n").unwrap();
        finish_unit(
            &store,
            "a",
            &attempt_id,
            &repo,
            "terra",
            "gpt-6.1-terra",
            &json!({"status":"completed","file_changes":[{"path":"a.txt"}]}),
        )
        .unwrap();
        record_accept(&store, "m", &repo).unwrap();
        Self {
            _temp: temp,
            repo,
            store,
        }
    }
    fn call(&self, args: &[&str], json: bool) -> Output {
        run(
            &self.store,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            json,
            false,
        )
    }
    fn tree(&self) -> String {
        evidence(&self.store, "a")
            .unwrap()
            .into_iter()
            .rfind(|r| r.kind == "verify")
            .unwrap()
            .commit
    }
    fn verdict(&self, findings: Value) {
        fs::create_dir_all(self.store.join("reports")).unwrap();
        fs::write(
            verdict_path(&self.store, "a"),
            json!({"unit":"a","tree":self.tree(),"verdict":"pass","findings":findings}).to_string(),
        )
        .unwrap();
    }
    fn review(&self, json: bool) -> Output {
        self.call(
            &[
                "review",
                "a",
                "--repo",
                self.repo.to_str().unwrap(),
                "--reviewer",
                "chronos:reviewer",
            ],
            json,
        )
    }
    fn pack(&self) {
        review_pack(&self.store, "a", &self.repo).unwrap();
    }
    fn waive(&self, number: &str, reason: &str, json: bool) -> Output {
        self.call(&["waive", "a", number, "--reason", reason], json)
    }
}

fn finding(check: &str, severity: &str, needs: Option<&str>) -> Value {
    let mut v = json!({"check":check,"severity":severity,"location":"a.txt:1","evidence":"proof"});
    if let Some(needs) = needs {
        v["needs"] = json!(needs);
    }
    v
}

#[test]
fn verify_returns_pack_with_verify_tree_in_text_and_json() {
    let f = Fixture::new();
    for json in [false, true] {
        let out = f.call(&["verify", "a", "--repo", f.repo.to_str().unwrap()], json);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        let path = if json {
            let value: Value = serde_json::from_str(&out.stdout).unwrap();
            value["pack"].as_str().unwrap().to_owned()
        } else {
            out.stdout
                .lines()
                .next()
                .unwrap()
                .split_once("\tpack=")
                .unwrap()
                .1
                .to_owned()
        };
        assert!(Path::new(&path).is_file());
        assert!(fs::read_to_string(path).unwrap().contains(&f.tree()));
    }
}

#[test]
fn verify_pack_error_does_not_change_exit_and_failed_verify_has_no_pack() {
    let f = Fixture::new();
    fs::remove_file(pack_path(&f.store, "a")).unwrap();
    fs::create_dir_all(pack_path(&f.store, "a")).unwrap();
    for json in [false, true] {
        let out = f.call(&["verify", "a", "--repo", f.repo.to_str().unwrap()], json);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        if json {
            let value: Value = serde_json::from_str(&out.stdout).unwrap();
            assert!(value["pack"].is_null());
            assert!(value["pack_error"].as_str().is_some());
        } else {
            assert!(!out.stdout.lines().next().unwrap().contains("\tpack="));
        }
    }
    fs::write(
        f.store.join("a.brief"),
        "GOAL x\nSCOPE a.txt\nVERIFY exit 3\n",
    )
    .unwrap();
    let out = f.call(&["verify", "a", "--repo", f.repo.to_str().unwrap()], true);
    assert_eq!(out.exit_code, 1);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert!(value["pack"].is_null());
    assert!(value.get("pack_error").is_none());
}

#[test]
fn explicit_verdict_is_saved_and_minor_only_output_is_unchanged() {
    let f = Fixture::new();
    f.pack();
    let source = f.store.join("external.json");
    let data = json!({"unit":"a", "tree":f.tree(), "verdict":"pass", "findings":[finding("style", "minor", None)]}).to_string();
    fs::write(&source, &data).unwrap();
    let out = f.call(
        &[
            "review",
            "a",
            "--repo",
            f.repo.to_str().unwrap(),
            "--reviewer",
            "chronos:reviewer",
            "--verdict-file",
            source.to_str().unwrap(),
        ],
        true,
    );
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["findings"], json!([]));
    assert_eq!(
        fs::read_to_string(verdict_path(&f.store, "a")).unwrap(),
        data
    );
    assert_eq!(f.review(false).stdout, "a\tverified\n(none ready)\n");
}

#[test]
fn default_verdict_and_findings_are_reported_and_truncated() {
    let f = Fixture::new();
    f.pack();
    let long = "證".repeat(260);
    f.verdict(json!([{ "check":"correctness", "severity":"major", "location":"a:1", "evidence":long }, finding("style", "minor", None)]));
    let out = f.review(false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(
        out.stdout.starts_with("a\treview-failed\n"),
        "{}",
        out.stdout
    );
    let line = out.stdout.lines().find(|l| l.starts_with("  #1 ")).unwrap();
    assert_eq!(line.chars().count(), 2 + 240 + 1);
    assert!(line.ends_with('…'));
    assert!(out.stdout.contains("blocker and major findings:"));
    assert!(!out.stdout.contains("#2"));
    let out = f.review(true);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["findings"].as_array().unwrap().len(), 1);
    assert_eq!(v["findings"][0]["severity"], "major");
    assert_eq!(
        v["findings"][0]["evidence"].as_str().unwrap(),
        format!("{}…", "證".repeat(240))
    );
}

#[test]
fn needs_only_exempts_untested_findings() {
    for (findings, expected) in [
        (
            json!([
                finding("untested", "major", Some("smoke")),
                finding("untested", "blocker", Some("manual"))
            ]),
            0,
        ),
        (json!([finding("untested", "major", None)]), 1),
        (json!([finding("correctness", "major", Some("smoke"))]), 1),
        (
            json!([
                finding("untested", "major", Some("manual")),
                finding("correctness", "blocker", None)
            ]),
            1,
        ),
    ] {
        let f = Fixture::new();
        f.pack();
        f.verdict(findings);
        let out = f.review(false);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
        assert_eq!(
            evidence(&f.store, "a")
                .unwrap()
                .iter()
                .rfind(|r| r.kind == "review")
                .unwrap()
                .exit_code,
            expected
        );
    }
    let f = Fixture::new();
    f.pack();
    f.verdict(json!([finding("untested", "major", Some("other"))]));
    let out = f.review(false);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("finding 1") && out.stderr.contains("other"),
        "{}",
        out.stderr
    );
}

#[test]
fn waive_requires_a_review_and_verdict_and_refuses_bad_inputs() {
    let f = Fixture::new();
    assert_eq!(
        f.call(&["waive", "missing", "1", "--reason", "x"], false)
            .exit_code,
        2
    );
    assert_eq!(f.waive("1", "x", false).exit_code, 1);
    f.pack();
    f.verdict(json!([
        finding("bug", "major", None),
        finding("style", "minor", None)
    ]));
    assert_eq!(f.review(false).exit_code, 0);
    for n in ["0", "2", "3", "bad"] {
        assert_eq!(f.waive(n, "x", false).exit_code, 1, "{n}");
    }
    for reason in ["", "   "] {
        assert_eq!(f.waive("1", reason, false).exit_code, 1);
    }
    assert_eq!(f.call(&["waive", "a", "1", "--reason"], false).exit_code, 1);
    let path = verdict_path(&f.store, "a");
    let data = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(f.waive("1", "x", false).exit_code, 1);
    fs::write(&path, "{").unwrap();
    assert_eq!(f.waive("1", "x", false).exit_code, 1);
    fs::write(path, data).unwrap();
    assert_eq!(f.waive("1", "x", false).exit_code, 0);
    assert_eq!(f.waive("1", "x", false).exit_code, 1);
}

#[test]
fn partial_and_full_waivers_update_state() {
    let f = Fixture::new();
    f.pack();
    f.verdict(json!([
        finding("bug", "blocker", None),
        finding("bug", "major", None),
        finding("untested", "major", Some("smoke"))
    ]));
    assert_eq!(f.review(false).exit_code, 0);
    let out = f.waive("1", "accepted", false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "a\twaived #1\treview-failed\tremaining=1\n");
    let rows = evidence(&f.store, "a").unwrap();
    assert_eq!(rows.last().unwrap().exit_code, 1);
    assert_eq!(
        rows.last().unwrap().commit,
        rows.iter().rfind(|r| r.kind == "review").unwrap().commit
    );
    let out = f.waive("2", "accepted too", true);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let v: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        v,
        json!({"unit":"a","number":2,"state":"verified","remaining":0})
    );
    assert_eq!(
        evidence(&f.store, "a").unwrap().last().unwrap().exit_code,
        0
    );
}

#[test]
fn default_missing_and_unknown_unit_are_reported() {
    let f = Fixture::new();
    let out = f.review(false);
    assert_eq!(out.exit_code, 1);
    assert!(out
        .stderr
        .contains(&verdict_path(&f.store, "a").display().to_string()));
    let out = f.call(
        &[
            "review",
            "missing",
            "--repo",
            f.repo.to_str().unwrap(),
            "--reviewer",
            "chronos:reviewer",
        ],
        false,
    );
    assert_eq!(out.exit_code, 2);
}
