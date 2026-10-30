use axiom::orch::*;
use std::{fs, path::Path, process::Command};

fn temp() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "orch-evidence-{}-{}",
        std::process::id(),
        axiom::base::unique()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}
fn row(kind: &str, command: &str, code: i32, commit: &str, ts: &str) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: code,
        commit: commit.into(),
        ts: ts.into(),
    }
}
fn setup(p: &Path) {
    fs::create_dir_all(p.join("reports")).unwrap();
    fs::write(
        p.join("brief.txt"),
        "GOAL x\nSCOPE file.txt\nVERIFY echo ok\n",
    )
    .unwrap();
    fs::write(
        p.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            p.join("brief.txt").display()
        ),
    )
    .unwrap();
    fs::write(p.join("modules.md"), "## m\nPURPOSE x\nPATHS file.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\nFULL TEST `echo ok`\n").unwrap();
}
#[test]
fn state_table() {
    let run = row("run", "gpt-worker", 0, "", "1");
    let verify = row("verify", "test", 0, "", "2");
    let review = row("review", "claude-review", 0, "a", "3");
    assert_eq!(derive_state(&[], false), "pending");
    assert_eq!(derive_state(&[row("run", "x", 1, "", "")], false), "failed");
    assert_eq!(
        derive_state(&[run.clone(), row("scope", "", 1, "", "")], false),
        "failed"
    );
    assert_eq!(
        derive_state(&[run.clone(), row("verify", "", 1, "", "")], false),
        "failed"
    );
    assert_eq!(derive_state(&[run.clone(), verify.clone()], true), "built");
    assert_eq!(
        derive_state(
            &[run.clone(), verify.clone(), row("accept", "", 1, "", "")],
            true
        ),
        "accept-failed"
    );
    assert_eq!(
        derive_state(&[run.clone(), verify.clone(), review.clone()], false),
        "verified"
    );
    assert_eq!(
        derive_state(
            &[
                run.clone(),
                verify.clone(),
                row("review", "gpt-review", 0, "a", "")
            ],
            false
        ),
        "built"
    );
    assert_eq!(
        derive_state(
            &[
                run.clone(),
                verify.clone(),
                row("review", "claude-review", 1, "a", ""),
                review.clone()
            ],
            false
        ),
        "review-failed"
    );
    assert_eq!(
        derive_state(
            &[
                run,
                verify,
                row("review", "claude-review", 1, "old", ""),
                review
            ],
            false
        ),
        "verified"
    );
}
#[test]
fn snapshot_tracks_worktree() {
    let p = temp();
    assert!(snapshot(&p, &["file.txt".into()]).is_empty());
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&p)
        .status()
        .unwrap()
        .success());
    fs::write(p.join("file.txt"), "one").unwrap();
    let a = snapshot(&p, &["file.txt".into()]);
    assert!(!a.is_empty());
    assert_eq!(a, snapshot(&p, &["file.txt".into()]));
    fs::write(p.join("file.txt"), "two").unwrap();
    assert_ne!(a, snapshot(&p, &["file.txt".into()]));
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn records_and_gates() {
    let p = temp();
    setup(&p);
    append_evidence(&p, "u", &row("run", "gpt-worker", 0, "", "2000-01-01")).unwrap();
    append_evidence(&p, "u", &row("verify", "test", 0, "", "2000-01-02")).unwrap();
    assert_eq!(
        record_review(&p, &["u".into()], "gpt-review", true, &p)
            .unwrap_err()
            .kind,
        "same_family"
    );
    assert_eq!(record_accept(&p, "m", &p).unwrap().exit_code, 0);
    assert_eq!(evidence(&p, "u").unwrap().last().unwrap().kind, "accept");
    assert_eq!(record_full(&p, &p, Some("exit 7")).unwrap().exit_code, 7);
    assert_eq!(record_full(&p, &p, None).unwrap().exit_code, 0);
    assert_eq!(done_gate(&p, false).unwrap_err().kind, "not_done");
    assert_eq!(astra_gate(&p).unwrap_err().kind, "astra_gate");
    record_stage(&p, "astra", "gpt-6-astra", true).unwrap();
    record_stage(&p, "second_review", "claude-review", false).unwrap();
    fs::create_dir_all(p.join("astra")).unwrap();
    for f in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
        fs::write(p.join("astra").join(f), "").unwrap();
    }
    assert_eq!(
        astra_gate(&p).unwrap_err().message,
        "empty astra/review.json"
    );
    fs::write(p.join("astra/review.json"), "{} ").unwrap();
    assert_eq!(astra_gate(&p).unwrap_err().message, "empty astra/ruling.md");
    fs::write(p.join("astra/ruling.md"), "ruling").unwrap();
    assert!(astra_gate(&p).is_ok());
    set_unit_state(&p, "u", "verified").unwrap();
    append_evidence(&p, "_full", &row("full", "test", 0, "", "1999")).unwrap();
    assert_eq!(done_gate(&p, false).unwrap_err().kind, "not_done");
    append_evidence(&p, "_full", &row("full", "test", 0, "", "9999")).unwrap();
    append_evidence(&p, "_stages", &row("rewrite", "x", 1, "", "9999")).unwrap();
    assert!(done_gate(&p, false).is_ok());
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn review_identity_empty_when_brief_unreadable() {
    let p = temp();
    setup(&p);
    append_evidence(&p, "u", &row("run", "gpt-worker", 0, "", "1")).unwrap();
    append_evidence(&p, "u", &row("verify", "test", 0, "verified-tree", "2")).unwrap();
    fs::remove_file(p.join("brief.txt")).unwrap();
    record_review(&p, &["u".into()], "claude-review", true, &p).unwrap();
    assert_eq!(evidence(&p, "u").unwrap().last().unwrap().commit, "");
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn accept_checks_all_briefs_before_command() {
    let p = temp();
    setup(&p);
    let marker = p.join("marker");
    fs::write(p.join("modules.md"), "## m\nPURPOSE x\nPATHS file.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo executed > marker`\n").unwrap();
    let mut units = fs::read_to_string(p.join("units.tsv")).unwrap();
    units.push_str("v\tm\tpending\t\t\t\t\n");
    fs::write(p.join("units.tsv"), units).unwrap();
    assert_eq!(record_accept(&p, "m", &p).err().unwrap().kind, "no_brief");
    assert!(!marker.exists());
    assert!(evidence(&p, "u").unwrap().is_empty());
    fs::remove_dir_all(p).unwrap();
}

#[test]
fn identities_are_current_git_trees() {
    let root = temp();
    let repo = root.join("repo");
    let store = root.join("store");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&store).unwrap();
    setup(&store);
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("file.txt"), "one").unwrap();
    append_evidence(&store, "u", &row("run", "gpt-worker", 0, "", "1")).unwrap();
    let verify = snapshot(&repo, &["file.txt".into()]);
    append_evidence(&store, "u", &row("verify", "test", 0, &verify, "2")).unwrap();
    record_accept(&store, "m", &repo).unwrap();
    record_full(&store, &repo, Some("echo ok")).unwrap();
    let full_path = store.join("reports/_full.evidence.tsv");
    let first_full = fs::read_to_string(&full_path)
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .split('\t')
        .nth(3)
        .unwrap()
        .to_string();
    fs::write(repo.join("file.txt"), "two").unwrap();
    record_review(&store, &["u".into()], "claude-review", true, &repo).unwrap();
    record_full(&store, &repo, Some("echo ok")).unwrap();
    let rows = evidence(&store, "u").unwrap();
    let review = &rows.last().unwrap().commit;
    assert_ne!(&verify, review);
    let full = fs::read_to_string(full_path).unwrap();
    let last_full = full.lines().last().unwrap().split('\t').nth(3).unwrap();
    assert_ne!(first_full, last_full);
    for identity in [
        &verify,
        &rows.iter().find(|r| r.kind == "accept").unwrap().commit,
        review,
        &first_full,
        &last_full.to_string(),
    ] {
        let hash = identity.strip_prefix("tree:").unwrap();
        assert_eq!(hash.len(), 40);
        let output = Command::new("git")
            .args(["cat-file", "-t", hash])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "tree");
    }
    fs::remove_dir_all(root).unwrap();
}
