use axiom::orch::*;
use serde_json::json;
use std::{fs, path::Path, process::Command};
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
fn lifecycle_reuse_scope_and_limit() {
    let _guard = TEST_LOCK.lock().unwrap();
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
    record_review(&store, &["a".into()], "claude-review", true).unwrap();
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
    assert!(!running().contains(&"a".to_string()));
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}
#[test]
fn sol_and_extra_commands() {
    let _guard = TEST_LOCK.lock().unwrap();
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
    abort_unit("a");
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
    let _guard = TEST_LOCK.lock().unwrap();
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
                let worker = || {
                    let start = std::time::Instant::now();
                    barrier.wait();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    (start, std::time::Instant::now())
                };
                let times = worker();
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
                times
            })
        })
        .collect();
    let times: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(times[0].0 < times[1].1 && times[1].0 < times[0].1);
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn dependencies_require_verified_unit() {
    let _guard = TEST_LOCK.lock().unwrap();
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
    record_review(&store, &["a".into()], "claude-review", true).unwrap();
    assert_eq!(
        derive_state(&evidence(&store, "a").unwrap(), true),
        "verified"
    );
    assert!(matches!(
        begin_unit(&store, "b", "worker", &repo, false).unwrap(),
        Begin::Run { .. }
    ));
    abort_unit("b");
    fs::remove_dir_all(store.parent().unwrap()).unwrap();
}

#[test]
fn extra_commands_preserve_output_and_failures() {
    let _guard = TEST_LOCK.lock().unwrap();
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
    let _guard = TEST_LOCK.lock().unwrap();
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
    let _guard = TEST_LOCK.lock().unwrap();
    let (store, _) = setup();
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
                path
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
    let _guard = TEST_LOCK.lock().unwrap();
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
