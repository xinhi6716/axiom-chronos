use axiom::orch::{append_evidence, derive_state, evidence, record_review, snapshot, Evidence};
use std::{fs, path::Path, process::Command};

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
