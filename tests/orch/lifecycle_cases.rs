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
        json!({"unit":"a","tree":verify_commit,"verdict":"pass","findings":[{"severity":"major"}]})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        call(vec![
            "review-pack".into(),
            "a".into(),
            "--repo".into(),
            repo.to_string_lossy().into_owned(),
        ])
        .exit_code,
        0
    );
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
    fs::create_dir_all(store.join("astra")).unwrap();
    fs::write(store.join("astra/plan-final.md"), "### P1 One
### P2 Two
").unwrap();
    let refused = call(&["map"], false);
    assert_ne!(refused.exit_code, 0);
    assert!(refused.stderr.contains("plan item P1"), "{}", refused.stderr);
    let map = fs::read_to_string(store.join("modules.md")).unwrap();
    fs::write(store.join("modules.md"), format!("{map}SERVES P1
OUT OF SCOPE P2 later
")).unwrap();
    assert_eq!(call(&["map"], false).exit_code, 0);
    fs::write(store.join("modules.md"), map).unwrap();
    fs::remove_dir_all(store.join("astra")).unwrap();
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
        r#"{"unit":"missing","tree":"tree:0","verdict":"pass","findings":[]}"#,
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
    assert!(out.stdout.starts_with("a\texit=0\tbuilt\taccept\tpack="), "{}", out.stdout);
    assert!(out.stdout.contains("\nok"), "{}", out.stdout);
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
        r#"{"unit":"proc","tree":"tree:0","verdict":"pass","findings":[]}"#,
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
    assert!(call(&["a"], "chronos:reviewer", false)
        .stderr
        .contains("missing tree"));
    let tree = evidence(&store, "a")
        .unwrap()
        .into_iter()
        .rfind(|r| r.kind == "verify")
        .unwrap()
        .commit;
    fs::write(
        &verdict,
        json!({"unit":"a","tree":tree,"verdict":"pass","findings":[]}).to_string(),
    )
    .unwrap();
    fs::remove_file(pack_path(&store, "a")).unwrap();
    assert!(call(&["a"], "chronos:reviewer", false)
        .stderr
        .contains("run orch review-pack first"));
    assert_eq!(evidence(&store, "a").unwrap().len(), before);
    review_pack(&store, "a", &repo).unwrap();
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

fn build(store: &Path, repo: &Path, unit: &str) {
    assert!(matches!(
        begin_unit(store, unit, "worker", repo, false).unwrap(),
        Begin::Run { .. }
    ));
    let finished = finish_unit(
        store,
        unit,
        &attempt(store, unit),
        repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    assert_eq!(finished.state, "built");
}
fn column(store: &Path, unit: &str) -> String {
    read_units(store)
        .unwrap()
        .into_iter()
        .find(|u| u.id == unit)
        .unwrap()
        .state
}
fn pass_review(store: &Path, repo: &Path, unit: &str) -> Vec<(String, String)> {
    record_review(store, &[unit.into()], "claude-review", true, repo).unwrap()
}
fn review_rows(store: &Path, unit: &str) -> usize {
    evidence(store, unit)
        .unwrap()
        .iter()
        .filter(|r| r.kind == "review")
        .count()
}
fn module(store: &Path, paths: &str, accept: &str) {
    fs::write(
        store.join("modules.md"),
        format!(
            "## m\nPURPOSE x\nPATHS {paths}\nDEPENDS none\nCONTRACT x\nACCEPT test `{accept}`\n"
        ),
    )
    .unwrap();
}
fn cli(store: &Path, args: &[&str]) -> Output {
    run(
        store,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        false,
        false,
    )
}
fn supersede(store: &Path, unit: &str, by: &str) -> Output {
    cli(store, &["unit", "supersede", unit, "--by", by])
}

#[test]
fn a_new_accept_keeps_the_review_of_unchanged_files() {
    let (store, repo) = setup();
    fs::write(repo.join("b.txt"), "b").unwrap();
    add_unit(&store, "b", "m", "b.txt");
    module(&store, "a.txt b.txt", "echo ok");
    build(&store, &repo, "a");
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(
        pass_review(&store, &repo, "a"),
        vec![("a".to_string(), "verified".to_string())]
    );
    build(&store, &repo, "b");
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(
        column(&store, "a"),
        "verified",
        "nothing a reviewed changed"
    );
    assert_eq!(column(&store, "b"), "built");
    assert_eq!(review_rows(&store, "a"), 1);
    assert_eq!(next_step_for(&store, "a").unwrap(), "verified");
    fs::write(repo.join("a.txt"), "really changed").unwrap();
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(column(&store, "a"), "built", "a reviewed file changed");
    assert_eq!(derive_state(&evidence(&store, "a").unwrap(), true), "built");
    assert_eq!(
        record_review(&store, &["a".into()], "claude-review", true, &repo)
            .unwrap_err()
            .kind,
        "stale_review"
    );
    let repo_arg = repo.to_string_lossy().into_owned();
    assert_eq!(
        cli(&store, &["verify", "a", "--repo", &repo_arg]).exit_code,
        0
    );
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(column(&store, "a"), "built", "the new files are unreviewed");
    assert_eq!(
        pass_review(&store, &repo, "a"),
        vec![("a".to_string(), "verified".to_string())]
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_failed_accept_blocks_a_unit_without_consuming_its_review() {
    let (store, repo) = setup();
    module(&store, "a.txt", "echo ok");
    build(&store, &repo, "a");
    record_accept(&store, "m", &repo).unwrap();
    pass_review(&store, &repo, "a");
    assert_eq!(column(&store, "a"), "verified");
    module(&store, "a.txt", "exit 3");
    assert_eq!(record_accept(&store, "m", &repo).unwrap().exit_code, 3);
    assert_eq!(column(&store, "a"), "accept-failed");
    assert_eq!(next_step_for(&store, "a").unwrap(), "fix");
    module(&store, "a.txt", "echo ok");
    assert_eq!(record_accept(&store, "m", &repo).unwrap().exit_code, 0);
    assert_eq!(
        column(&store, "a"),
        "verified",
        "same files, ACCEPT green again"
    );
    assert_eq!(review_rows(&store, "a"), 1);
    module(&store, "a.txt", "exit 3");
    fs::write(repo.join("a.txt"), "changed while red").unwrap();
    assert_eq!(record_accept(&store, "m", &repo).unwrap().exit_code, 3);
    assert_eq!(column(&store, "a"), "accept-failed");
    module(&store, "a.txt", "echo ok");
    assert_eq!(record_accept(&store, "m", &repo).unwrap().exit_code, 0);
    assert_eq!(
        column(&store, "a"),
        "built",
        "the review was of other files"
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_failed_review_is_not_erased_by_another_accept() {
    let (store, repo) = setup();
    module(&store, "a.txt", "echo ok");
    build(&store, &repo, "a");
    record_accept(&store, "m", &repo).unwrap();
    record_review(&store, &["a".into()], "claude-review", false, &repo).unwrap();
    assert_eq!(column(&store, "a"), "review-failed");
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(column(&store, "a"), "review-failed");
    assert_eq!(next_step_for(&store, "a").unwrap(), "fix");
    fs::write(repo.join("a.txt"), "fixed").unwrap();
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(column(&store, "a"), "built");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

fn fail_run(store: &Path, repo: &Path, unit: &str) {
    assert!(matches!(
        begin_unit(store, unit, "worker", repo, false).unwrap(),
        Begin::Run { .. }
    ));
    let finished = finish_unit(
        store,
        unit,
        &attempt(store, unit),
        repo,
        "worker",
        "gpt-worker",
        &json!({"status":"failed"}),
    )
    .unwrap();
    assert_eq!(finished.state, "failed");
}
fn not_ready(store: &Path, repo: &Path, unit: &str) -> String {
    let err = begin_unit(store, unit, "worker", repo, false)
        .err()
        .unwrap();
    assert_eq!(err.kind, "not_ready");
    err.message
}
const TWO_MODULES: &str = "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## n\nPURPOSE x\nPATHS c.txt\nDEPENDS m\nCONTRACT x\nACCEPT test `echo ok`\n";
fn replacement_setup() -> (std::path::PathBuf, std::path::PathBuf) {
    let (store, repo) = setup();
    fs::write(repo.join("c.txt"), "c").unwrap();
    add_unit(&store, "fix", "m", "a.txt");
    add_unit(&store, "c", "n", "c.txt");
    fs::write(store.join("modules.md"), TWO_MODULES).unwrap();
    (store, repo)
}
fn future_full_test(store: &Path) {
    append_evidence(
        store,
        "_full",
        &Evidence {
            kind: "full".into(),
            command: "test".into(),
            exit_code: 0,
            commit: String::new(),
            ts: "9999".into(),
        },
    )
    .unwrap();
}

#[test]
fn a_superseded_unit_keeps_its_history_and_stands_on_its_replacement() {
    let (store, repo) = replacement_setup();
    fail_run(&store, &repo, "a");
    let before = evidence(&store, "a").unwrap().len();
    assert!(before > 0);
    let out = supersede(&store, "a", "fix");
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    let brief = store.join("a.brief");
    assert_eq!(
        out.stdout,
        format!("a\tm\tsuperseded:fix\t\t\t\t{}\nready\n", brief.display())
    );
    assert_eq!(column(&store, "a"), "superseded:fix");
    let units = fs::read_to_string(store.join("units.tsv")).unwrap();
    assert_eq!(
        units.lines().next(),
        Some("id\ttrack\tstate\tbranch\tpr\tsha\tbrief")
    );
    assert_eq!(evidence(&store, "a").unwrap().len(), before);
    assert!(cli(&store, &["evidence", "a"])
        .stdout
        .contains("run\tgpt-worker\t1"));
    assert_eq!(ready_units(&store).unwrap(), vec!["fix"]);
    assert!(waiting_units(&store).unwrap().iter().all(|w| w.unit != "a"));
    assert_eq!(
        not_ready(&store, &repo, "c"),
        "c waits on fix (pending; next run); ready now: fix"
    );
    build(&store, &repo, "fix");
    assert!(
        matches!(accept_if_ready(&store, "fix", &repo).unwrap(),
            AutoAccept::Ran { exit_code: 0, states, .. } if states == vec![("fix".to_string(), "built".to_string())]),
        "a superseded unit must not hold ACCEPT back"
    );
    assert_eq!(
        evidence(&store, "a").unwrap().len(),
        before,
        "no accept row"
    );
    assert_eq!(column(&store, "a"), "superseded:fix");
    assert_eq!(
        not_ready(&store, &repo, "c"),
        "c waits on fix (built; next review); none ready"
    );
    let err = done_gate(&store, false).unwrap_err();
    assert_eq!(err.kind, "not_done");
    assert!(err.message.contains("a=superseded:fix"), "{}", err.message);
    pass_review(&store, &repo, "fix");
    assert_eq!(column(&store, "fix"), "verified");
    assert!(matches!(
        begin_unit(&store, "c", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    finish_unit(
        &store,
        "c",
        &attempt(&store, "c"),
        &repo,
        "worker",
        "gpt-worker",
        &json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    record_accept(&store, "n", &repo).unwrap();
    pass_review(&store, &repo, "c");
    future_full_test(&store);
    done_gate(&store, false).unwrap();
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_superseded_unit_blocks_done_until_its_replacement_is_verified() {
    let (store, repo) = replacement_setup();
    let units = fs::read_to_string(store.join("units.tsv")).unwrap();
    fs::write(
        store.join("units.tsv"),
        units
            .lines()
            .filter(|l| !l.starts_with("c\t"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    fail_run(&store, &repo, "a");
    assert_eq!(supersede(&store, "a", "fix").exit_code, 0);
    future_full_test(&store);
    let blocked = done_gate(&store, false).unwrap_err();
    assert_eq!(blocked.kind, "not_done");
    assert!(
        blocked.message.contains("a=superseded:fix"),
        "{}",
        blocked.message
    );
    assert!(
        blocked.message.contains("fix=pending"),
        "{}",
        blocked.message
    );
    set_unit_state(&store, "fix", "built").unwrap();
    let blocked = done_gate(&store, false).unwrap_err();
    assert!(
        blocked.message.contains("a=superseded:fix"),
        "{}",
        blocked.message
    );
    set_unit_state(&store, "fix", "verified").unwrap();
    done_gate(&store, false).unwrap();
    let rows = fs::read_to_string(store.join("units.tsv")).unwrap();
    fs::write(
        store.join("units.tsv"),
        rows.lines()
            .filter(|l| !l.starts_with("fix\t"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let blocked = done_gate(&store, false).unwrap_err();
    assert_eq!(
        blocked.message, "a=superseded:fix",
        "a lost replacement is not a pass"
    );
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn replacement_chains_resolve_to_their_last_unit() {
    let (store, repo) = replacement_setup();
    add_unit(&store, "fix2", "m", "a.txt");
    fail_run(&store, &repo, "a");
    assert_eq!(supersede(&store, "a", "fix").exit_code, 0);
    assert_eq!(supersede(&store, "fix", "fix2").exit_code, 0);
    assert_eq!(
        not_ready(&store, &repo, "c"),
        "c waits on fix2 (pending; next run); ready now: fix2"
    );
    build(&store, &repo, "fix2");
    record_accept(&store, "m", &repo).unwrap();
    pass_review(&store, &repo, "fix2");
    assert!(matches!(
        begin_unit(&store, "c", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit(&store, "c", &attempt(&store, "c"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn hand_edited_replacement_chains_fail_explicitly() {
    let (store, repo) = replacement_setup();
    let path = store.join("units.tsv");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        original.replace("a\tm\tpending", "a\tm\tsuperseded:gone"),
    )
    .unwrap();
    assert!(
        not_ready(&store, &repo, "c").contains("a (superseded:gone; replacement chain is broken)")
    );
    assert_eq!(done_gate(&store, false).unwrap_err().kind, "not_done");
    fs::write(
        &path,
        original
            .replace("a\tm\tpending", "a\tm\tsuperseded:fix")
            .replace("fix\tm\tpending", "fix\tm\tsuperseded:a"),
    )
    .unwrap();
    assert!(not_ready(&store, &repo, "c").contains("replacement chain is broken"));
    assert_eq!(done_gate(&store, false).unwrap_err().kind, "not_done");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn supersede_rejects_nonsense() {
    let (store, repo) = setup();
    add_unit(&store, "b", "m", "a.txt");
    add_unit(&store, "c", "m", "a.txt");
    add_unit(&store, "other", "n", "a.txt");
    add_unit(&store, "apart", "m", "z.txt");
    let path = store.join("units.tsv");
    let mut rows = fs::read_to_string(&path).unwrap();
    rows.push_str("blank\tm\tpending\t\t\t\t\n");
    fs::write(&path, rows).unwrap();
    let rejected = |unit: &str, by: &str, code: i32, phrase: &str| {
        let before = fs::read(store.join("units.tsv")).unwrap();
        let out = supersede(&store, unit, by);
        assert_eq!(out.exit_code, code, "{unit} by {by}: {}", out.stderr);
        assert!(
            out.stderr.contains(phrase),
            "{unit} by {by}: {}",
            out.stderr
        );
        assert_eq!(fs::read(store.join("units.tsv")).unwrap(), before);
    };
    rejected("a", "missing", 2, "unit missing not found");
    rejected("missing", "a", 2, "unit missing not found");
    rejected("a", "a", 1, "cannot supersede itself");
    rejected("a", "other", 1, "belongs to n");
    rejected("a", "apart", 1, "does not cover a.txt");
    rejected("a", "blank", 1, "has no brief");
    assert!(cli(&store, &["unit", "supersede", "a"])
        .stderr
        .contains("--by is required"));

    assert!(matches!(begin(&store, &repo), Begin::Run { .. }));
    rejected("a", "b", 1, "has a running claim");
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
    pass_review(&store, &repo, "a");
    assert_eq!(column(&store, "a"), "verified");
    rejected("a", "b", 1, "is verified");
    set_unit_state(&store, "a", "built").unwrap();
    rejected("a", "b", 1, "is verified");
    set_unit_state(&store, "b", "verified").unwrap();
    rejected("b", "c", 1, "is verified");
    set_unit_state(&store, "b", "pending").unwrap();

    assert_eq!(supersede(&store, "b", "c").exit_code, 0);
    rejected("b", "a", 1, "already superseded by c");
    rejected("c", "b", 1, "would form a cycle");
    rejected("apart", "b", 1, "name c instead");
    set_unit_state(&store, "c", "pending").unwrap();
    assert_eq!(column(&store, "b"), "superseded:c");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_cycle_through_a_longer_chain_is_rejected() {
    let (store, _repo) = setup();
    add_unit(&store, "b", "m", "a.txt");
    add_unit(&store, "c", "m", "a.txt");
    assert_eq!(supersede(&store, "a", "b").exit_code, 0);
    assert_eq!(supersede(&store, "b", "c").exit_code, 0);
    let out = supersede(&store, "c", "a");
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("would form a cycle"), "{}", out.stderr);
    assert_eq!(column(&store, "c"), "pending");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_replacement_must_cover_every_file_it_takes_over() {
    let (store, _repo) = setup();
    add_unit(&store, "wide", "m", "a.txt b.txt");
    add_unit(&store, "narrow", "m", "a.txt");
    add_unit(&store, "dir", "m", "src/");
    add_unit(&store, "file", "m", "src/x.rs");
    let refused = |unit: &str, by: &str, missing: &str| {
        let before = fs::read(store.join("units.tsv")).unwrap();
        let out = supersede(&store, unit, by);
        assert_eq!(out.exit_code, 1, "{unit} by {by}: {}", out.stdout);
        assert!(
            out.stderr.contains(&format!("does not cover {missing}")),
            "{unit} by {by}: {}",
            out.stderr
        );
        assert_eq!(fs::read(store.join("units.tsv")).unwrap(), before);
    };
    refused("wide", "narrow", "b.txt");
    refused("dir", "file", "src/");
    assert_eq!(supersede(&store, "file", "dir").exit_code, 0);
    assert_eq!(supersede(&store, "narrow", "wide").exit_code, 0);
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_replacement_covers_the_files_of_the_whole_chain_it_takes_over() {
    let (store, _repo) = setup();
    add_unit(&store, "wide", "m", "a.txt b.txt");
    add_unit(&store, "fix", "m", "a.txt b.txt");
    add_unit(&store, "narrow", "m", "a.txt");
    assert_eq!(supersede(&store, "wide", "fix").exit_code, 0);
    fs::write(
        store.join("fix.brief"),
        "GOAL test\nSCOPE a.txt\nVERIFY echo ok\n",
    )
    .unwrap();
    let out = supersede(&store, "fix", "narrow");
    assert_eq!(out.exit_code, 1, "{}", out.stdout);
    assert!(out.stderr.contains("does not cover b.txt"), "{}", out.stderr);
    assert_eq!(column(&store, "fix"), "pending");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn superseded_is_terminal() {
    let (store, repo) = setup();
    add_unit(&store, "fix", "m", "a.txt");
    module(&store, "a.txt", "echo ok");
    build(&store, &repo, "a");
    assert_eq!(supersede(&store, "a", "fix").exit_code, 0);
    let refused = begin_unit(&store, "a", "worker", &repo, false)
        .err()
        .unwrap();
    assert_eq!(refused.kind, "superseded");
    assert!(
        refused.message.contains("superseded by fix"),
        "{}",
        refused.message
    );
    assert_eq!(
        record_verify(&store, "a", &repo).err().unwrap().kind,
        "superseded"
    );
    assert_eq!(
        record_review(&store, &["a".into()], "claude-review", true, &repo)
            .unwrap_err()
            .kind,
        "superseded"
    );
    assert_eq!(
        evidence(&store, "a")
            .unwrap()
            .iter()
            .filter(|r| r.kind == "review")
            .count(),
        0
    );
    let before = evidence(&store, "a").unwrap().len();
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(evidence(&store, "a").unwrap().len(), before);
    set_unit_state(&store, "a", "built").unwrap();
    assert_eq!(
        column(&store, "a"),
        "superseded:fix",
        "a late writer cannot reopen it"
    );
    let out = cli(&store, &["unit", "set", "a", "--state", "pending"]);
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("superseded by fix"), "{}", out.stderr);
    for state in ["superseded", "superseded:fix"] {
        let out = cli(&store, &["unit", "set", "fix", "--state", state]);
        assert_eq!(out.exit_code, 1, "{state}");
        assert!(out.stderr.contains("orch unit supersede"), "{}", out.stderr);
    }
    assert_eq!(column(&store, "fix"), "pending");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn a_rebuilt_unit_can_be_superseded_once_its_files_really_changed() {
    let (store, repo) = setup();
    add_unit(&store, "later", "m", "a.txt");
    module(&store, "a.txt", "echo ok");
    build(&store, &repo, "a");
    record_accept(&store, "m", &repo).unwrap();
    pass_review(&store, &repo, "a");
    fs::write(repo.join("a.txt"), "rebuilt by a later unit").unwrap();
    let out = supersede(&store, "a", "later");
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("is verified"), "{}", out.stderr);
    assert!(out.stderr.contains("orch accept"), "{}", out.stderr);
    build(&store, &repo, "later");
    record_accept(&store, "m", &repo).unwrap();
    assert_eq!(column(&store, "a"), "built");
    assert_eq!(supersede(&store, "a", "later").exit_code, 0);
    pass_review(&store, &repo, "later");
    future_full_test(&store);
    done_gate(&store, false).unwrap();
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn superseded_units_are_counted_under_one_name() {
    let (store, _repo) = setup();
    assert_eq!(cli(&store, &["init"]).exit_code, 0);
    add_unit(&store, "b", "m", "a.txt");
    add_unit(&store, "c", "m", "a.txt");
    assert_eq!(supersede(&store, "a", "c").exit_code, 0);
    assert_eq!(supersede(&store, "b", "c").exit_code, 0);
    assert_eq!(
        cli(&store, &["unit", "counts"]).stdout,
        "pending=1, superseded=2\n"
    );
    let listed = cli(&store, &["unit", "list", "--state", "superseded"]).stdout;
    assert!(listed.contains("a\tm\tsuperseded:c"), "{listed}");
    assert!(listed.contains("b\tm\tsuperseded:c"), "{listed}");
    assert!(cli(&store, &["unit", "list", "--state", "superseded:c"])
        .stdout
        .contains("a\tm\tsuperseded:c"));
    assert!(cli(&store, &["status"])
        .stdout
        .contains("states=pending=1, superseded=2"));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn map_fails_when_the_plan_cannot_be_read() {
    let (store, repo) = setup();
    fs::write(
        store.join("modules.md"),
        "## m\nPURPOSE x\nPATHS a.txt\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
    )
    .unwrap();
    let map = || run(&store, &["map".to_string()], false, false);
    let report = || {
        let args = ["report", "--repo", repo.to_str().unwrap()];
        run(&store, &args.map(String::from), false, false)
    };
    assert_eq!(map().exit_code, 0);
    assert_eq!(report().exit_code, 0);
    fs::create_dir_all(store.join("astra")).unwrap();
    fs::write(store.join("astra/plan-final.md"), b"### C1 \xff\xfe broken\n").unwrap();
    for out in [map(), report()] {
        assert_ne!(out.exit_code, 0, "{}", out.stdout);
        assert!(out.stderr.contains("plan-final.md"), "{}", out.stderr);
    }
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
