use axiom::orch::*;
use serde_json::json;
use std::{fs, path::Path, process::Command};

fn setup() -> (std::path::PathBuf, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "orch-life-{}-{}",
        std::process::id(),
        axiom::base::unique()
    ));
    let repo = root.join("repo");
    let store = root.join("store");
    fs::create_dir_all(&repo).unwrap();
    for d in ["reports", "inbox"] {
        fs::create_dir_all(store.join(d)).unwrap();
    }
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    fs::write(repo.join("a.txt"), "a").unwrap();
    let brief = store.join("a.brief");
    fs::write(&brief, "GOAL test\nSCOPE a.txt\nVERIFY echo ok\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    (store, repo)
}
fn begin(store: &Path, repo: &Path) -> Begin {
    begin_unit(store, "a", "worker", repo, false).unwrap()
}
#[test]
fn changed_contract_invalidates_reuse_and_done_gate() {
    let (store, repo) = setup();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap();
    let future = |kind: &str| Evidence {
        kind: kind.into(),
        command: "test".into(),
        exit_code: 0,
        commit: String::new(),
        ts: "9999".into(),
    };
    append_evidence(&store, "_full", &future("full")).unwrap();
    append_evidence(&store, "_stages", &future("rewrite")).unwrap();
    assert!(done_gate(&store, true).is_ok());
    fs::write(
        store.join("a.brief"),
        "GOAL test\nSCOPE a.txt\nVERIFY echo changed\n",
    )
    .unwrap();
    assert_eq!(done_gate(&store, true).unwrap_err().kind, "not_done");
    assert!(matches!(
        begin(&store, &repo),
        Begin::Run { attempt: 2, .. }
    ));
    abort_unit(&store, "a");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn already_dirty_outside_file_changes_are_detected() {
    let (store, repo) = setup();
    fs::write(repo.join("outside.txt"), "before").unwrap();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    fs::write(repo.join("outside.txt"), "after").unwrap();
    let finished = finish_unit(
        &store,
        "a",
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    assert_eq!(finished.outside_scope, ["outside.txt"]);
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn dirty_content_and_deletions_are_distinct() {
    for case in ["same-length", "deleted", "restored-empty"] {
        let (store, repo) = setup();
        let path = repo.join("outside.txt");
        fs::write(
            &path,
            if case == "restored-empty" {
                ""
            } else {
                "before"
            },
        )
        .unwrap();
        assert!(Command::new("git")
            .args(["add", "--", "outside.txt"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        if case == "restored-empty" {
            fs::remove_file(&path).unwrap();
        }
        begin(&store, &repo);
        match case {
            "deleted" => fs::remove_file(&path).unwrap(),
            "restored-empty" => fs::write(&path, "").unwrap(),
            _ => fs::write(&path, "after!").unwrap(),
        }
        let result = finish_unit(
            &store,
            "a",
            &repo,
            "worker",
            "gpt-worker",
            &json!({"status":"completed","file_changes":[]}),
        )
        .unwrap();
        assert_eq!(result.outside_scope, ["outside.txt"], "{case}");
        assert_eq!(result.state, "failed");
        assert_eq!(result.verify_exit, None);
        fs::remove_dir_all(store.parent().unwrap()).unwrap();
    }
}
#[test]
fn dirty_read_errors_fail_closed_and_release_claim() {
    let (store, repo) = setup();
    let path = repo.join("outside.txt");
    fs::write(&path, "tracked").unwrap();
    assert!(Command::new("git")
        .args(["add", "--", "outside.txt"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    begin(&store, &repo);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        finish_unit(
            &store,
            "a",
            &repo,
            "worker",
            "gpt-worker",
            &json!({"status":"completed","file_changes":[]})
        )
        .err()
        .unwrap()
        .kind,
        "io"
    );
    assert!(evidence(&store, "a")
        .unwrap()
        .iter()
        .all(|r| r.kind != "verify"));
    // An unreadable dirty path also prevents acquiring a new claim.
    assert_eq!(
        begin_unit(&store, "a", "worker", &repo, false)
            .err()
            .unwrap()
            .kind,
        "io"
    );
    fs::remove_dir(&path).unwrap();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    abort_unit(&store, "a");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn concurrent_scopes_survive_either_completion_order() {
    for first in ["a", "b"] {
        let (store, repo) = setup();
        fs::write(repo.join("b.txt"), "b").unwrap();
        let brief = store.join("b.brief");
        fs::write(&brief, "GOAL peer\nSCOPE b.txt\nVERIFY echo ok\n").unwrap();
        let mut units = fs::read_to_string(store.join("units.tsv")).unwrap();
        units.push_str(&format!("b\tn\tpending\t\t\t\t{}\n", brief.display()));
        fs::write(store.join("units.tsv"), units).unwrap();
        begin(&store, &repo);
        begin_unit(&store, "b", "worker", &repo, false).unwrap();
        fs::write(repo.join("a.txt"), "changed a").unwrap();
        fs::write(repo.join("b.txt"), "changed b").unwrap();
        let second = if first == "a" { "b" } else { "a" };
        for id in [first, second] {
            let result = finish_unit(
                &store,
                id,
                &repo,
                "worker",
                "gpt-worker",
                &json!({"status":"completed","file_changes":[{"path":format!("{id}.txt")}]}),
            )
            .unwrap();
            assert!(
                result.outside_scope.is_empty(),
                "{first} first, {id}: {:?}",
                result.outside_scope
            );
            assert_eq!(result.verify_exit, Some(0));
        }
        fs::remove_dir_all(store.parent().unwrap()).unwrap();
    }
}
#[test]
fn peer_scope_never_excuses_explicit_outside_changes() {
    for peer_finished in [false, true] {
        let (store, repo) = setup();
        let brief = store.join("b.brief");
        fs::write(&brief, "GOAL peer\nSCOPE b.txt\nVERIFY echo ok\n").unwrap();
        fs::write(repo.join("b.txt"), "b").unwrap();
        let mut units = fs::read_to_string(store.join("units.tsv")).unwrap();
        units.push_str(&format!("b\tn\tpending\t\t\t\t{}\n", brief.display()));
        fs::write(store.join("units.tsv"), units).unwrap();
        begin(&store, &repo);
        begin_unit(&store, "b", "worker", &repo, false).unwrap();
        if peer_finished {
            finish_unit(
                &store,
                "b",
                &repo,
                "worker",
                "gpt-worker",
                &json!({"status":"completed","file_changes":[]}),
            )
            .unwrap();
        }
        let result = finish_unit(
            &store,
            "a",
            &repo,
            "worker",
            "gpt-worker",
            &json!({"status":"completed","file_changes":[{"path":repo.join("b.txt")}]}),
        )
        .unwrap();
        assert_eq!(result.outside_scope, ["b.txt"]);
        assert_eq!(result.state, "failed");
        abort_unit(&store, "b");
        fs::remove_dir_all(store.parent().unwrap()).unwrap();
    }
}
#[test]
fn inactive_or_earlier_peer_scopes_do_not_excuse_changes() {
    for peer_ran in [false, true] {
        let (store, repo) = setup();
        let brief = store.join("b.brief");
        fs::write(&brief, "GOAL peer\nSCOPE b.txt\nVERIFY echo ok\n").unwrap();
        fs::write(repo.join("b.txt"), "b").unwrap();
        let mut units = fs::read_to_string(store.join("units.tsv")).unwrap();
        units.push_str(&format!("b\tn\tpending\t\t\t\t{}\n", brief.display()));
        fs::write(store.join("units.tsv"), units).unwrap();
        if peer_ran {
            begin_unit(&store, "b", "worker", &repo, false).unwrap();
            finish_unit(
                &store,
                "b",
                &repo,
                "worker",
                "gpt-worker",
                &json!({"status":"completed","file_changes":[]}),
            )
            .unwrap();
        }
        begin(&store, &repo);
        fs::write(repo.join("b.txt"), "outside change").unwrap();
        let result = finish_unit(
            &store,
            "a",
            &repo,
            "worker",
            "gpt-worker",
            &json!({"status":"completed","file_changes":[]}),
        )
        .unwrap();
        assert_eq!(result.outside_scope, ["b.txt"]);
        fs::remove_dir_all(store.parent().unwrap()).unwrap();
    }
}
#[test]
fn dependency_module_without_units_blocks() {
    let (store, repo) = setup();
    fs::write(store.join("modules.md"), "## n\nPURPOSE x\nPATHS b.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## m\nPURPOSE x\nPATHS a.txt\nDEPENDS n\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let err = begin_unit(&store, "a", "worker", &repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "not_ready");
    assert!(err.message.contains('n'));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn legacy_bare_verify_identity_is_read_but_not_reused() {
    let (store, repo) = setup();
    let identity = snapshot(&repo, &["a.txt".into()]);
    let hash = identity.strip_prefix("tree:").unwrap();
    assert_eq!(hash.len(), 40);
    fs::write(
        store.join("reports/a.evidence.tsv"),
        format!("kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-worker\t0\t\t1\nverify\ttest\t0\t{hash}\t2\nreview\tclaude-review\t0\t{hash}\t3\n"),
    ).unwrap();
    let rows = evidence(&store, "a").unwrap();
    assert_eq!(rows[1].commit, hash);
    assert_eq!(derive_state(&rows, false), "verified");
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    abort_unit(&store, "a");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn review_requires_repo() {
    let (store, _) = setup();
    let verdict = store.join("verdict.json");
    fs::write(&verdict, r#"{"verdict":"pass","findings":[]}"#).unwrap();
    let result = run(
        &store,
        &[
            "review".into(),
            "a".into(),
            "--reviewer".into(),
            "claude".into(),
            "--verdict-file".into(),
            verdict.to_string_lossy().into_owned(),
        ],
        false,
        false,
    );
    assert_eq!(result.exit_code, 1);
    assert!(result.stderr.contains("error: missing --repo"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn projects_do_not_share_running_units() {
    let (store_a, repo_a) = setup();
    let (store_b, repo_b) = setup();
    assert!(matches!(begin(&store_a, &repo_a), Begin::Run { .. }));
    // Same unit id and scope in another project: neither busy nor excused by the first claim.
    assert!(matches!(begin(&store_b, &repo_b), Begin::Run { .. }));
    abort_unit(&store_a, "a");
    abort_unit(&store_b, "a");
    fs::write(
        store_b.join("a.brief"),
        "GOAL other project\nSCOPE foreign.txt\nVERIFY echo ok\n",
    )
    .unwrap();
    fs::write(repo_b.join("foreign.txt"), "foreign").unwrap();
    fs::write(repo_a.join("foreign.txt"), "before").unwrap();
    begin(&store_a, &repo_a);
    begin(&store_b, &repo_b);
    fs::write(repo_a.join("foreign.txt"), "after").unwrap();
    let result = finish_unit(
        &store_a,
        "a",
        &repo_a,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    assert_eq!(result.outside_scope, ["foreign.txt"]);
    abort_unit(&store_b, "a");
    for store in [store_a, store_b] {
        fs::remove_dir_all(store.parent().unwrap()).unwrap();
    }
}
#[test]
fn lifecycle_reuse_scope_and_limit() {
    let (store, repo) = setup();
    assert_eq!(ready_units(&store).unwrap(), vec!["a"]);
    assert!(matches!(
        begin(&store, &repo),
        Begin::Run { attempt: 1, .. }
    ));
    assert_eq!(
        begin_unit(&store, "a", "worker", &repo, false)
            .err()
            .unwrap()
            .kind,
        "busy"
    );
    let report = json!({"status":"completed","final_message":"done","file_changes":[]});
    let finished = finish_unit(&store, "a", &repo, "worker", "gpt-worker", &report).unwrap();
    assert_eq!(finished.state, "built");
    assert_eq!(finished.verify_exit, Some(0));
    assert_eq!(finished.verify_tail.trim(), "ok");
    assert_eq!(
        fs::read_to_string(store.join("reports/a.md")).unwrap(),
        "done"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(store.join("reports/a.json")).unwrap()
        )
        .unwrap(),
        report
    );
    let pointer = fs::read_dir(store.join("inbox"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert!(fs::read_to_string(pointer.path())
        .unwrap()
        .contains(&format!(
            "worker\ta\tcompleted\t{}",
            store.join("reports").join("a.md").display()
        )));
    record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap();
    assert!(matches!(begin(&store, &repo), Begin::Reused));
    fs::write(repo.join("a.txt"), "changed").unwrap();
    assert!(matches!(
        begin(&store, &repo),
        Begin::Run { attempt: 2, .. }
    ));
    fs::write(repo.join("outside.txt"), "outside").unwrap();
    let report = json!({"status":"completed","file_changes":[{"path":"other.txt"}]});
    let finished = finish_unit(&store, "a", &repo, "worker", "gpt-worker", &report).unwrap();
    assert_eq!(finished.state, "failed");
    assert_eq!(finished.outside_scope, vec!["other.txt", "outside.txt"]);
    assert_eq!(
        evidence(&store, "a")
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.kind == "scope")
            .unwrap()
            .command,
        "other.txt outside.txt"
    );
    assert_eq!(
        begin_unit(&store, "a", "worker", &repo, false)
            .err()
            .unwrap()
            .kind,
        "fix_limit"
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn sol_and_extra_commands() {
    let (store, repo) = setup();
    assert_eq!(
        begin_unit(&store, "a", "sol", &repo, false)
            .err()
            .unwrap()
            .kind,
        "sol_not_allowed"
    );
    assert_eq!(
        begin_unit(&store, "a", "worker", &repo, true)
            .err()
            .unwrap()
            .kind,
        "astra_gate"
    );
    let ready = run(&store, &["ready".into()], false, false);
    assert_eq!(ready.exit_code, 0);
    assert_eq!(ready.stderr, "");
    assert_eq!(ready.stdout, "a\n");
    let fail = run(
        &store,
        &[
            "fulltest".into(),
            "--repo".into(),
            repo.to_string_lossy().into(),
            "--command".into(),
            "exit 7".into(),
        ],
        false,
        false,
    );
    assert_eq!(fail.exit_code, 1);
    assert!(fail.stdout.contains("fulltest\texit=7"));
    assert_eq!(fail.stderr, "");
    assert!(matches!(
        begin_unit(&store, "a", "sol", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "a");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

fn add_unit(store: &Path, id: &str, track: &str, scope: &str) {
    let brief = store.join(format!("{id}.brief"));
    fs::write(
        &brief,
        format!("GOAL test\nSCOPE {scope}\nVERIFY echo ok\n"),
    )
    .unwrap();
    let path = store.join("units.tsv");
    let mut units = fs::read_to_string(&path).unwrap();
    units.push_str(&format!(
        "{id}\t{track}\tpending\t\t\t\t{}\n",
        brief.display()
    ));
    fs::write(path, units).unwrap();
}

#[test]
fn concurrent_workers_and_overlapping_scopes() {
    let (store, repo) = setup();
    add_unit(&store, "b", "m", "b.txt");
    add_unit(&store, "c", "m", "a.txt");
    assert_eq!(ready_units(&store).unwrap(), vec!["a", "b"]);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let store = store.clone();
            let repo = repo.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                assert!(matches!(
                    begin_unit(&store, id, "worker", &repo, false).unwrap(),
                    Begin::Run { .. }
                ));
                barrier.wait();
                if id == "a" {
                    assert_eq!(
                        begin_unit(&store, "c", "worker", &repo, false)
                            .err()
                            .unwrap()
                            .kind,
                        "busy"
                    );
                }
                barrier.wait();
                let result = finish_unit(
                    &store,
                    id,
                    &repo,
                    "worker",
                    "gpt-worker",
                    &json!({"status":"completed","file_changes":[]}),
                )
                .unwrap();
                assert_eq!(result.state, "built");
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn dependencies_require_verified_unit() {
    let (store, repo) = setup();
    add_unit(&store, "b", "n", "b.txt");
    fs::write(store.join("modules.md"), "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## n\nPURPOSE x\nPATHS b.txt\nDEPENDS m\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let err = begin_unit(&store, "b", "worker", &repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "not_ready");
    assert!(err.message.contains('a'));
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    record_accept(&store, "m", &repo).unwrap();
    record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap();
    assert_eq!(
        derive_state(&evidence(&store, "a").unwrap(), true),
        "verified"
    );
    assert!(matches!(
        begin_unit(&store, "b", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "b");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn extra_commands_preserve_output_and_failures() {
    let (store, repo) = setup();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `exit 7`\n",
    )
    .unwrap();
    let repo_arg = repo.to_string_lossy().into_owned();
    let call = |args: Vec<String>| run(&store, &args, false, false);
    let accept = call(vec![
        "accept".into(),
        "m".into(),
        "--repo".into(),
        repo_arg.clone(),
    ]);
    assert_eq!(accept.exit_code, 1);
    assert!(accept.stdout.contains("m\texit=7"));
    assert_eq!(accept.stderr, "");
    let full = call(vec![
        "fulltest".into(),
        "--repo".into(),
        repo_arg,
        "--command".into(),
        "echo full-ok".into(),
    ]);
    assert_eq!(full.exit_code, 0);
    assert!(full.stdout.contains("fulltest\texit=0\nfull-ok"));
    assert_eq!(full.stderr, "");
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    assert_eq!(record_accept(&store, "m", &repo).unwrap().exit_code, 0);
    let verify_commit = evidence(&store, "a")
        .unwrap()
        .iter()
        .find(|r| r.kind == "verify")
        .unwrap()
        .commit
        .clone();
    let verdict = store.join("verdict.json");
    fs::write(
        &verdict,
        r#"{"verdict":"pass","findings":[{"severity":"major"}]}"#,
    )
    .unwrap();
    let review = call(vec![
        "review".into(),
        "a".into(),
        "--reviewer".into(),
        "claude-review".into(),
        "--verdict-file".into(),
        verdict.to_string_lossy().into_owned(),
        "--repo".into(),
        repo.to_string_lossy().into_owned(),
    ]);
    assert_eq!(review.exit_code, 0);
    assert!(review.stdout.contains("a\treview-failed"));
    assert_eq!(review.stderr, "");
    let rows = evidence(&store, "a").unwrap();
    let row = rows.iter().find(|r| r.kind == "review").unwrap();
    assert_eq!(row.exit_code, 1);
    assert_eq!(row.commit, verify_commit);
    let output = call(vec!["evidence".into(), "a".into()]);
    assert_eq!(output.exit_code, 0);
    assert!(output.stdout.contains("review\tclaude-review\t1\t"));
    assert_eq!(output.stderr, "");
    let error = call(vec!["accept".into(), "m".into()]);
    assert_eq!(error.exit_code, 1);
    assert_eq!(error.stdout, "");
    assert!(error.stderr.contains("missing --repo"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn non_git_and_reported_paths() {
    let (store, repo) = setup();
    let plain = store.parent().unwrap().join("plain");
    fs::create_dir(&plain).unwrap();
    assert!(matches!(begin(&store, &plain), Begin::Run { .. }));
    let result = finish_unit(
        &store,
        "a",
        &plain,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[{"path":"z.txt"},{"path":"b.txt"}]}),
    )
    .unwrap();
    assert_eq!(result.outside_scope, ["b.txt", "z.txt"]);
    assert_eq!(
        evidence(&store, "a").unwrap().last().unwrap().command,
        "b.txt z.txt"
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
    let _ = repo;
}

#[test]
fn map_and_review_errors() {
    let (store, repo) = setup();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let call = |args: &[&str], json| {
        run(
            &store,
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            json,
            false,
        )
    };
    assert_eq!(call(&["map"], false).stdout, "m\t\ta.txt\n");
    let map: serde_json::Value = serde_json::from_str(&call(&["map"], true).stdout).unwrap();
    assert_eq!(map["modules"][0]["id"], "m");
    assert_eq!(call(&["evidence", "missing"], false).exit_code, 2);
    let verdict = store.join("verdict.json");
    let path = verdict.to_str().unwrap();
    for (data, msg) in [("{", "error:"), ("{}", "missing verdict")] {
        fs::write(&verdict, data).unwrap();
        let out = call(
            &[
                "review",
                "a",
                "--reviewer",
                "claude",
                "--verdict-file",
                path,
                "--repo",
                repo.to_str().unwrap(),
            ],
            false,
        );
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains(msg));
    }
    fs::write(&verdict, r#"{"verdict":"pass","findings":[]}"#).unwrap();
    assert_eq!(
        call(
            &[
                "review",
                "missing",
                "--reviewer",
                "claude",
                "--verdict-file",
                path,
                "--repo",
                repo.to_str().unwrap()
            ],
            false
        )
        .exit_code,
        2
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn verify_tail_keeps_last_2000_characters() {
    let (store, repo) = setup();
    let long = format!("{}END", "x".repeat(2100));
    fs::write(
        store.join("a.brief"),
        format!("GOAL test\nSCOPE a.txt\nVERIFY echo {long}\n"),
    )
    .unwrap();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    let result = finish_unit(
        &store,
        "a",
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed"}),
    )
    .unwrap();
    assert_eq!(result.verify_tail.chars().count(), 2000);
    assert!(result.verify_tail.trim_end().ends_with("END"));
    assert!(result.verify_tail.starts_with('x'));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn ready_lists_waiting_units_with_next_step() {
    let (store, repo) = setup();
    for (id, track) in [
        ("b", "m"),
        ("c", "m"),
        ("d", "m"),
        ("e", "m"),
        ("f", "m"),
        ("g", "n"),
    ] {
        add_unit(&store, id, track, &format!("{id}.txt"));
    }
    fs::write(store.join("modules.md"), "## m\nPURPOSE x\nPATHS a.txt b.txt c.txt d.txt e.txt f.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## n\nPURPOSE x\nPATHS g.txt\nDEPENDS m\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let row = |kind: &str, code| Evidence {
        kind: kind.into(),
        command: if kind == "run" {
            "gpt".into()
        } else {
            "claude".into()
        },
        exit_code: code,
        commit: "old".into(),
        ts: "1".into(),
    };
    for id in ["b", "c", "d", "e"] {
        append_evidence(&store, id, &row("run", 0)).unwrap();
        append_evidence(&store, id, &row("verify", 0)).unwrap();
    }
    append_evidence(&store, "b", &row("accept", 1)).unwrap();
    append_evidence(&store, "c", &row("accept", 0)).unwrap();
    append_evidence(&store, "c", &row("review", 1)).unwrap();
    append_evidence(&store, "e", &row("accept", 0)).unwrap();
    for _ in 0..2 {
        append_evidence(&store, "f", &row("run", 1)).unwrap();
    }
    let waiting = waiting_units(&store).unwrap();
    let pairs: Vec<_> = waiting
        .iter()
        .map(|w| (w.unit.as_str(), w.next.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("b", "fix"),
            ("c", "fix"),
            ("d", "accept"),
            ("e", "review"),
            ("f", "blocked"),
            ("g", "wait a,b,c,d,e,f")
        ]
    );
    let compact = run(&store, &["ready".into()], false, false);
    assert!(compact.stdout.starts_with("a\nb\taccept-failed\tfix\n"));
    let json = run(&store, &["ready".into()], true, false);
    let parsed: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(parsed["ready"], json!(["a"]));
    assert_eq!(
        parsed["waiting"][2],
        json!({"unit":"d","state":"built","next":"accept"})
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
    let (store, _) = setup();
    assert_eq!(run(&store, &["ready".into()], false, false).stdout, "a\n");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
    let _ = repo;
}
