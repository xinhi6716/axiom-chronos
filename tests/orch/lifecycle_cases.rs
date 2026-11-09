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
fn attempt(store: &Path, unit: &str) -> String {
    fs::read_to_string(store.join("claims.tsv"))
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|cols| cols[0] == unit)
        .unwrap()[1]
        .to_string()
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
    abort_unit(&store, "a", &attempt(&store, "a"));
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
    abort_unit(&store_a, "a", &attempt(&store_a, "a"));
    abort_unit(&store_b, "a", &attempt(&store_b, "a"));
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
    let finished = finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &report,
    )
    .unwrap();
    assert_eq!(finished.state, "built");
    assert_eq!(finished.verify_exit, Some(0));
    assert_eq!(finished.verify_tail.trim(), "ok");
    assert_eq!(
        fs::read_to_string(store.join("reports/a.md")).unwrap(),
        format!(
            "done\nBRIDGE VERIFY exit=0 output={}\n",
            store.join("reports").join("a.verify.txt").display()
        )
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
    let finished = finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &report,
    )
    .unwrap();
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
    abort_unit(&store, "a", &attempt(&store, "a"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

fn sol_refusal(store: &Path, repo: &Path) -> Option<String> {
    match begin_unit(store, "a", "sol", repo, false) {
        Ok(_) => {
            abort_unit(store, "a", &attempt(store, "a"));
            None
        }
        Err(e) => Some(e.kind),
    }
}
fn external(store: &Path, repo: &Path, flags: &[&str]) -> Output {
    let mut args = vec![
        "fulltest".to_string(),
        "--repo".to_string(),
        repo.to_string_lossy().into_owned(),
    ];
    args.extend(flags.iter().map(|s| s.to_string()));
    run(store, &args, false, false)
}

#[test]
fn ci_failure_opens_sol_like_a_local_failure() {
    let (store, repo) = setup();
    let refused = Some("sol_not_allowed".to_string());
    assert_eq!(
        sol_refusal(&store, &repo),
        refused,
        "no failure recorded yet"
    );
    record_full(&store, &repo, Some("echo ok")).unwrap();
    assert_eq!(sol_refusal(&store, &repo), refused, "only a passing run");

    let log = store.parent().unwrap().join("ci.log");
    fs::write(&log, "running tests\nerror: linux job failed\n").unwrap();
    let log = log.to_string_lossy().into_owned();
    let out = external(
        &store,
        &repo,
        &["--external", "ci-run-42", "--exit", "101", "--log", &log],
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "");
    assert!(
        out.stdout
            .starts_with("fulltest\texit=101\nrunning tests\nerror: linux job failed"),
        "{}",
        out.stdout
    );
    assert_eq!(
        sol_refusal(&store, &repo),
        None,
        "recorded failure opens sol"
    );

    record_full(&store, &repo, Some("echo ok")).unwrap();
    assert_eq!(sol_refusal(&store, &repo), refused, "later pass closes sol");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn external_failure_without_real_evidence_records_nothing() {
    let (store, repo) = setup();
    let root = store.parent().unwrap().to_path_buf();
    let text = |p: &Path| p.to_string_lossy().into_owned();
    fs::write(root.join("ci.log"), "failing job").unwrap();
    fs::write(root.join("blank.log"), " \r\n\t\0\n").unwrap();
    let log = text(&root.join("ci.log"));
    let blank = text(&root.join("blank.log"));
    let absent = text(&root.join("absent.log"));
    let directory = text(&root);
    let good = ["--external", "ci-1", "--exit", "1", "--log", log.as_str()];
    let cases: [(Vec<&str>, &str); 12] = [
        (
            vec!["--external", "", "--exit", "1", "--log", &log],
            "label is empty",
        ),
        (
            vec!["--external", "   ", "--exit", "1", "--log", &log],
            "label is empty",
        ),
        (
            vec!["--external", "ci-1", "--exit", "0", "--log", &log],
            "failures only",
        ),
        (
            vec!["--external", "ci-1", "--exit", "soon", "--log", &log],
            "--exit must be an integer",
        ),
        (vec!["--external", "ci-1", "--log", &log], "missing --exit"),
        (vec!["--external", "ci-1", "--exit", "1"], "missing --log"),
        (
            vec!["--external", "--exit", "1", "--log", &log],
            "--external requires a value",
        ),
        (
            vec!["--external", "ci-1", "--exit", "1", "--log", &absent],
            "cannot read",
        ),
        (
            vec!["--external", "ci-1", "--exit", "1", "--log", &directory],
            "cannot read",
        ),
        (
            vec!["--external", "ci-1", "--exit", "1", "--log", &blank],
            "log is empty",
        ),
        (
            [good.as_slice(), &["--command", "echo x > marker"]].concat(),
            "cannot be combined",
        ),
        (
            vec!["--exit", "1", "--log", &log, "--command", "echo x > marker"],
            "require --external",
        ),
    ];
    let full = store.join("reports/_full.evidence.tsv");
    for (flags, message) in cases {
        let out = external(&store, &repo, &flags);
        assert_eq!(out.exit_code, 1, "{flags:?}");
        assert_eq!(out.stdout, "", "{flags:?}");
        assert!(out.stderr.contains(message), "{flags:?}: {}", out.stderr);
        assert!(!full.exists(), "{flags:?}");
        assert!(!repo.join("marker").exists(), "{flags:?}");
        assert_eq!(
            sol_refusal(&store, &repo).as_deref(),
            Some("sol_not_allowed"),
            "{flags:?}"
        );
    }

    let out = external(&store, &root.join("absent-repo"), &good);
    assert!(out.stderr.contains("not a directory"), "{}", out.stderr);
    assert!(!full.exists());

    let unborn = root.join("unborn");
    let out = external(&unborn, &repo, &good);
    assert!(out.stderr.contains("run orch init"), "{}", out.stderr);
    assert!(!unborn.exists(), "a mistyped store must not be created");

    assert_eq!(external(&store, &repo, &good).exit_code, 1);
    assert!(
        full.exists(),
        "the same flags record once the input is valid"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn old_full_evidence_still_reads_beside_external_rows() {
    let (store, repo) = setup();
    let path = store.join("reports/_full.evidence.tsv");
    let header = "kind\tcommand\texit_code\tcommit\tts";
    let refused = Some("sol_not_allowed".to_string());
    fs::write(&path, format!("{header}\r\nfull\told\t0\t\t2000\r\n")).unwrap();
    assert_eq!(sol_refusal(&store, &repo), refused);
    fs::write(&path, format!("{header}\r\nfull\told\t3\t\t2000\r\n")).unwrap();
    assert_eq!(sol_refusal(&store, &repo), None);
    fs::write(&path, format!("{header}\r\nfull\told\t0\t\t2000\r\n")).unwrap();

    let log = store.parent().unwrap().join("ci.log");
    fs::write(&log, "failing job").unwrap();
    record_external_full(&store, &repo, "ci-1", 1, &log).unwrap();
    assert_eq!(sol_refusal(&store, &repo), None);
    let saved = fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = saved.lines().collect();
    assert_eq!(lines.len(), 3, "{saved}");
    assert_eq!(lines[0], header);
    assert_eq!(lines[1], "full\told\t0\t\t2000");
    assert_eq!(lines[2].split('\t').count(), header.split('\t').count());
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn dispatcher_ci_call_uses_the_real_flags() {
    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "agents/dispatcher.md")
        .unwrap()
        .1;
    let start = text
        .find("[\"fulltest\", \"--repo\", <dir>, \"--external\"")
        .unwrap();
    let call = &text[start..start + text[start..].find(']').unwrap()];
    let flags: Vec<_> = call.split('"').filter(|s| s.starts_with("--")).collect();
    assert_eq!(flags, ["--repo", "--external", "--exit", "--log"]);

    let (store, repo) = setup();
    let log = store.parent().unwrap().join("ci.log");
    fs::write(&log, "failing job").unwrap();
    let repo_arg = repo.to_string_lossy().into_owned();
    let log_arg = log.to_string_lossy().into_owned();
    let values = [repo_arg.as_str(), "ci-1", "1", log_arg.as_str()];
    let mut args = vec!["fulltest".to_string()];
    for (flag, value) in flags.iter().zip(values) {
        args.extend([flag.to_string(), value.to_string()]);
    }
    let out = run(&store, &args, false, false);
    assert!(
        out.stdout.starts_with("fulltest\texit=1\n"),
        "{}",
        out.stderr
    );
    assert_eq!(sol_refusal(&store, &repo), None);
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
                    &attempt(&store, id),
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
    let not_ready = |unit: &str| {
        let err = begin_unit(&store, unit, "worker", &repo, false)
            .err()
            .unwrap();
        assert_eq!(err.kind, "not_ready");
        err.message
    };
    assert_eq!(
        not_ready("b"),
        "b waits on a (pending; next run); ready now: a"
    );
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    assert_eq!(
        not_ready("b"),
        "b waits on a (pending; next run); none ready"
    );
    finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    assert_eq!(
        not_ready("b"),
        "b waits on a (built; next accept); none ready"
    );
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(
        not_ready("b"),
        "b waits on a (built; next review); none ready"
    );
    record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap();
    assert_eq!(
        derive_state(&evidence(&store, "a").unwrap(), true),
        "verified"
    );
    assert!(matches!(
        begin_unit(&store, "b", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "b", &attempt(&store, "b"));
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
        &attempt(&store, "a"),
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
        r#"{"unit":"a","verdict":"pass","findings":[{"severity":"major"}]}"#,
    )
    .unwrap();
    let review = call(vec![
        "review".into(),
        "a".into(),
        "--reviewer".into(),
        "chronos:reviewer".into(),
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
    assert!(output.stdout.contains("review\tchronos:reviewer\t1\t"));
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
        &attempt(&store, "a"),
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
    let out = call(
        &[
            "review",
            "a",
            "--reviewer",
            "chronos:reviewer",
            "--verdict-file",
            path,
            "--repo",
            repo.to_str().unwrap(),
        ],
        false,
    );
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains(&format!(
            "verdict file {path} does not exist; save the reviewer's verdict there"
        )),
        "{}",
        out.stderr
    );
    for (data, msg) in [("{", "error:"), ("{}", "missing verdict")] {
        fs::write(&verdict, data).unwrap();
        let out = call(
            &[
                "review",
                "a",
                "--reviewer",
                "chronos:reviewer",
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
    fs::write(
        &verdict,
        r#"{"unit":"missing","verdict":"pass","findings":[]}"#,
    )
    .unwrap();
    assert_eq!(
        call(
            &[
                "review",
                "missing",
                "--reviewer",
                "chronos:reviewer",
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
fn verify_command_refreshes_a_stale_unit() {
    let (store, repo) = setup();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let repo_arg = repo.to_string_lossy().into_owned();
    let verify = |json| {
        run(
            &store,
            &["verify".into(), "a".into(), "--repo".into(), repo_arg.clone()],
            json,
            false,
        )
    };
    let early = verify(false);
    assert_eq!(early.exit_code, 1);
    assert!(
        early.stderr.contains("unit a has no completed worker run to verify"),
        "{}",
        early.stderr
    );
    assert!(evidence(&store, "a").unwrap().is_empty());
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    record_accept(&store, "m", &repo).unwrap();
    fs::write(repo.join("a.txt"), "changed by a neighbour").unwrap();
    let stale = record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap_err();
    assert_eq!(stale.kind, "stale_review");
    assert!(
        stale.message.contains("run orch verify <unit> --repo <dir>, then review again"),
        "{}",
        stale.message
    );
    let out = verify(false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(out.stdout.starts_with("a\texit=0\tbuilt\taccept\nok"), "{}", out.stdout);
    assert_eq!(
        fs::read_to_string(store.join("reports/a.verify.txt")).unwrap().trim(),
        "ok"
    );
    record_accept(&store, "m", &repo).unwrap();
    let rows = record_review(&store, &["a".into()], "claude-review", true, &repo).unwrap();
    assert_eq!(rows, vec![("a".to_string(), "verified".to_string())]);
    fs::write(store.join("a.brief"), "GOAL test\nSCOPE a.txt\nVERIFY exit 3\n").unwrap();
    let out = verify(true);
    assert_eq!(out.exit_code, 1);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(v["exit_code"], 3);
    assert_eq!(v["state"], "failed");
    assert_eq!(v["next"], "ready");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn temp_dir_is_not_scope_and_is_removed() {
    let (store, repo) = setup();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    let temp = repo.join(TEMP_DIR);
    fs::create_dir_all(temp.join("a/pytest-0")).unwrap();
    fs::write(temp.join("a/pytest-0/case.txt"), "scratch").unwrap();
    fs::create_dir_all(temp.join("other")).unwrap();
    fs::write(temp.join("other/keep.txt"), "another unit's scratch").unwrap();
    let finished = finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[{"path":".axiom-tmp/a/pytest-0/case.txt"}]}),
    )
    .unwrap();
    assert_eq!(finished.state, "built");
    assert!(finished.outside_scope.is_empty(), "{:?}", finished.outside_scope);
    assert!(!temp.join("a").exists());
    assert!(temp.join("other/keep.txt").exists());
    fs::remove_dir_all(&temp).unwrap();
    fs::write(repo.join("a.txt"), "changed").unwrap();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    fs::create_dir_all(repo.join(".axiom-tmp-a")).unwrap();
    fs::write(repo.join(".axiom-tmp-a/case.txt"), "scratch").unwrap();
    fs::create_dir_all(temp.join("a")).unwrap();
    fs::write(temp.join("a/case.txt"), "scratch").unwrap();
    let finished = finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    assert_eq!(finished.state, "failed");
    assert_eq!(finished.outside_scope, vec![".axiom-tmp-a/case.txt"]);
    assert!(!temp.exists());
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
        &attempt(&store, "a"),
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
            ("g", "wait a (pending; next run), b (accept-failed; next fix), c (review-failed; next fix), d (built; next accept), e (built; next review), f (failed; next fix)")
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

#[test]
fn review_is_bound_to_unit_and_reviewer() {
    let (store, repo) = setup();
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed"}),
    )
    .unwrap();
    let verdict = store.join("verdict.json");
    let call = |units: &[&str], reviewer: &str, json: bool| {
        let mut args = vec!["review".to_string()];
        args.extend(units.iter().map(|s| s.to_string()));
        args.extend([
            "--repo".into(),
            repo.to_string_lossy().into_owned(),
            "--reviewer".into(),
            reviewer.into(),
            "--verdict-file".into(),
            verdict.to_string_lossy().into_owned(),
        ]);
        run(&store, &args, json, false)
    };
    let before = evidence(&store, "a").unwrap().len();
    fs::write(
        &verdict,
        r#"{"unit":"proc","verdict":"pass","findings":[]}"#,
    )
    .unwrap();
    let out = call(&["a"], "chronos:reviewer", false);
    assert!(out.stderr.contains("review is for unit proc, not a"));
    assert_eq!(evidence(&store, "a").unwrap().len(), before);
    for reviewer in ["gemini-4-argon-high", "claude-review"] {
        assert!(call(&["a"], reviewer, false)
            .stderr
            .contains("reviewer must be chronos:reviewer"));
    }
    assert!(call(&["a", "b"], "chronos:reviewer", false)
        .stderr
        .contains("review takes one unit per verdict file"));
    fs::write(&verdict, r#"{"verdict":"pass","findings":[]}"#).unwrap();
    assert!(call(&["a"], "chronos:reviewer", false)
        .stderr
        .contains("missing unit"));
    assert_eq!(evidence(&store, "a").unwrap().len(), before);
    fs::write(&verdict, r#"{"unit":"a","verdict":"pass","findings":[]}"#).unwrap();
    let out = call(&["a"], "chronos:reviewer", false);
    assert_eq!(out.stdout, "a\tverified\n(none ready)\n");
    let out = call(&["a"], "chronos:reviewer", true);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let value: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["states"], json!([["a", "verified"]]));
    assert_eq!(value["ready"], json!([]));
    assert_eq!(value["waiting"], json!([]));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn tracks_dependencies_and_auto_accept() {
    let (store, repo) = setup();
    assert!(
        matches!(accept_if_ready(&store, "a", &repo).unwrap(), AutoAccept::Skipped { reason, .. } if reason == "not_module")
    );
    add_unit(&store, "fix", "m", "fix.txt");
    add_unit(&store, "dependent", "n", "dependent.txt");
    add_unit(&store, "unknown", "other", "unknown.txt");
    fs::write(store.join("modules.md"), "## m\nPURPOSE x\nPATHS a.txt fix.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## z\nPURPOSE x\nPATHS z.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## n\nPURPOSE x\nPATHS dependent.txt\nDEPENDS m z\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let err = begin_unit(&store, "unknown", "worker", &repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "unknown_track");
    assert!(waiting_units(&store)
        .unwrap()
        .iter()
        .any(|w| w.unit == "unknown" && w.next == "unknown_track"));
    let err = begin_unit(&store, "dependent", "worker", &repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "not_ready");
    assert_eq!(
        err.message,
        "dependent waits on a (pending; next run), fix (pending; next run), z (no units registered); ready now: a, fix"
    );
    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    finish_unit(
        &store,
        "a",
        &attempt(&store, "a"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed"}),
    )
    .unwrap();
    assert!(
        matches!(accept_if_ready(&store, "a", &repo).unwrap(), AutoAccept::Skipped { reason, detail } if reason == "units_pending" && detail == "fix=pending")
    );
    assert!(matches!(
        begin_unit(&store, "fix", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    assert!(
        matches!(accept_if_ready(&store, "a", &repo).unwrap(), AutoAccept::Skipped { reason, detail } if reason == "claimed" && detail == "fix")
    );
    finish_unit(
        &store,
        "fix",
        &attempt(&store, "fix"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed"}),
    )
    .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = ["a", "fix"]
        .into_iter()
        .map(|id| {
            let store = store.clone();
            let repo = repo.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                accept_if_ready(&store, id, &repo).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, AutoAccept::Ran { exit_code: 0, .. }))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(
                |r| matches!(r, AutoAccept::Skipped { reason, .. } if reason == "already_accepted")
            )
            .count(),
        1
    );
    assert!(
        matches!(accept_if_ready(&store, "a", &repo).unwrap(), AutoAccept::Skipped { reason, .. } if reason == "already_accepted")
    );
    for id in ["a", "fix"] {
        assert!(evidence(&store, id)
            .unwrap()
            .iter()
            .any(|r| r.kind == "accept"));
    }
    record_full(&store, &repo, Some("exit 1")).unwrap();
    assert!(matches!(
        begin_unit(&store, "unknown", "sol", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "unknown", &attempt(&store, "unknown"));
    fs::remove_file(store.join("modules.md")).unwrap();
    assert!(matches!(
        begin_unit(&store, "unknown", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "unknown", &attempt(&store, "unknown"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
