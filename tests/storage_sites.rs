mod common;
use axiom::{
    bridge::{call, Bridge, Config},
    council, orch, setup,
    storage::{self, Class, Ledger},
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

fn git(repo: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap()
        .success());
}
fn init(home: &Path) -> std::path::PathBuf {
    let store = home.join("orchestrate/demo/w-1234567890123-abcdef");
    let out = orch::run(&store, &["init".into()], true, false);
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    store
}
fn entry(home: &Path, path: &Path, class: Class) {
    let e = Ledger::open(home).unwrap().get(path).unwrap().unwrap();
    assert_eq!(e.class, class);
    assert_eq!(e.owner.project, "demo");
    assert_eq!(e.owner.wish.as_deref(), Some("w-1234567890123-abcdef"));
}
#[test]
fn init_evidence_archive_and_scratch_cleanup() {
    let t = common::temp();
    let home = t.path();
    let store = init(home);
    entry(home, &store, Class::UnitStore);
    entry(home, &store.join("reports"), Class::Evidence);
    entry(home, &store.join("receipts"), Class::Evidence);
    let row = orch::Evidence {
        kind: "run".into(),
        command: "worker".into(),
        exit_code: 0,
        commit: "".into(),
        ts: "now".into(),
    };
    orch::append_evidence(&store, "u", &row).unwrap();
    let file = store.join("reports/u.evidence.tsv");
    storage::archive::compress(&file).unwrap();
    assert_eq!(orch::evidence(&store, "u").unwrap().len(), 1);
    orch::append_evidence(&store, "u", &row).unwrap();
    assert_eq!(orch::evidence(&store, "u").unwrap().len(), 2);
    assert!(!storage::archive::archived_path(&file).exists());
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    let run = orch::record_full(&store, &repo, Some("echo checked")).unwrap();
    assert_eq!(run.exit_code, 0);
    let audit = fs::read_to_string(storage::retention::audit_log(home)).unwrap();
    let record: Value = audit
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["reason"] == "unit_ended")
        .unwrap();
    assert_eq!(record["outcome"], "done");
    let scratch = std::path::PathBuf::from(record["path"].as_str().unwrap());
    assert!(!scratch.exists());
    assert!(!Ledger::open(home)
        .unwrap()
        .entries()
        .unwrap()
        .iter()
        .any(|e| e.path == storage::ledger::normalize(&scratch)));
}
#[test]
fn setup_preflight_pauses_and_retries() {
    if std::env::var_os("AXIOM_STORAGE_SITES_CHILD").is_none() {
        let t = common::temp();
        let disk = t.path().join("disk.json");
        fs::write(&disk, r#"{"free_bytes":1}"#).unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "setup_preflight_pauses_and_retries",
                "--nocapture"
            ])
            .env("AXIOM_STORAGE_SITES_CHILD", "1")
            .env("AXIOM_FAKE_DISK", &disk)
            .env("AXIOM_HOME", t.path())
            .env("AXIOM_DATA_HOME", t.path())
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = std::path::PathBuf::from(std::env::var_os("AXIOM_HOME").unwrap());
    let store = init(&home);
    let repo = home.join("repo");
    fs::create_dir_all(repo.join(".axiom")).unwrap();
    git(&repo, &["init", "-q"]);
    let marker = home.join("spawned");
    fs::write(repo.join(".axiom/setup.json"), serde_json::to_vec(&json!({"version":1,"steps":[{"id":"custom","argv":[common::python(),"-c",format!("open({:?},'w').write('yes')",marker.to_string_lossy())],"cwd":".","plan_independent":true,"cache_dirs":["cache"]},{"id":"plain","argv":[common::python(),"-c","pass"],"cwd":".","plan_independent":true}]})).unwrap()).unwrap();
    let states = setup::start(&store, &repo).unwrap();
    assert!(states
        .iter()
        .all(|s| s.state == "paused" && s.reason.as_ref().unwrap().starts_with("disk_space: ")));
    assert!(states.iter().any(|s| s.id == "plain"
        && s.reason
            .as_deref()
            .unwrap_or("")
            .contains("setup_step needs")));
    assert!(states.iter().any(|s| s.id == "custom"
        && s.reason
            .as_deref()
            .unwrap_or("")
            .contains("cache_build needs")));
    assert!(!marker.exists());
    assert!(fs::read_to_string(store.join("events.jsonl"))
        .unwrap()
        .contains("disk_preflight_paused"));
    fs::write(
        std::env::var_os("AXIOM_FAKE_DISK").unwrap(),
        r#"{"free_bytes":1000000000000}"#,
    )
    .unwrap();
    setup::start(&store, &repo).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.exists());
    entry(&home, &repo.join("cache"), Class::BuildCache);
}
#[test]
fn unit_worker_cargo_env_and_worktree() {
    for cargo in [false, true] {
        let t = common::temp();
        let home = t.path();
        let repo = home.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("file.txt"), "x").unwrap();
        if cargo {
            fs::write(
                repo.join("Cargo.toml"),
                "[package]\nname='fixture'\nversion='0.1.0'\n",
            )
            .unwrap();
        }
        git(&repo, &["init", "-q"]);
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-qm",
                "init",
            ],
        );
        let work = home.join("work");
        git(
            &repo,
            &["worktree", "add", "-qb", "worker", work.to_str().unwrap()],
        );
        let store = home.join("orchestrate/demo");
        let out = orch::run(&store, &["init".into()], true, false);
        assert_eq!(out.exit_code, 0);
        let brief = store.join("brief.txt");
        fs::write(
            &brief,
            format!("GOAL build\nRISK trivial\nSCOPE file.txt src/mod.py\nVERIFY \"{}\" -c \"import os; print(os.getenv('CARGO_TARGET_DIR', 'missing'))\"\n", common::python()),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let log = home.join("log");
        let wrapper = home.join("wrapper.py");
        fs::write(&wrapper,"import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        let b = Bridge::new(Config {
            data_home: Some(home.into()),
            codex: Ok(vec![
                common::python(),
                wrapper.to_string_lossy().into(),
                log.to_string_lossy().into(),
                common::repo_root()
                    .join("tests/fakes/fake_codex.py")
                    .to_string_lossy()
                    .into(),
            ]),
            wsl: None,
            on_windows: false,
        });
        let result = call(
            &b,
            "codex_run",
            &json!({"project":"demo","unit":"u","role":"terra","cwd":work}),
        );
        assert_eq!(result["structuredContent"]["state"], "built", "{result}");
        let verify = fs::read_to_string(store.join("reports/u.verify.txt")).unwrap();
        assert_eq!(verify.trim() != "missing", cargo);
        let audit = fs::read_to_string(storage::retention::audit_log(home)).unwrap();
        let records: Vec<Value> = audit
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for part in ["verify"] {
            let record = records
                .iter()
                .find(|v| {
                    v["reason"] == "unit_ended"
                        && v["outcome"] == "done"
                        && Path::new(v["path"].as_str().unwrap())
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .contains(part)
                })
                .unwrap();
            let scratch = Path::new(record["path"].as_str().unwrap());
            assert!(!scratch.exists());
            assert!(!Ledger::open(home)
                .unwrap()
                .entries()
                .unwrap()
                .iter()
                .any(|e| e.path == storage::ledger::normalize(scratch)));
        }
        let attempt = records
            .iter()
            .find(|v| {
                v["reason"] == "unit_ended"
                    && v["outcome"] == "done"
                    && Path::new(v["path"].as_str().unwrap()).parent()
                        == Some(store.join("scratch/u").as_path())
            })
            .unwrap();
        let attempt_path = Path::new(attempt["path"].as_str().unwrap());
        assert!(!attempt_path.exists());
        assert!(!Ledger::open(home)
            .unwrap()
            .entries()
            .unwrap()
            .iter()
            .any(|e| e.path == storage::ledger::normalize(attempt_path)));
        let starts: Vec<Value> = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|v| v["method"] == "thread/start")
            .collect();
        let cfg = &starts[0]["params"]["config"];
        let env = &cfg["shell_environment_policy"]["set"];
        let roots = cfg["sandbox_workspace_write"]["writable_roots"]
            .as_array()
            .unwrap();
        assert_eq!(env["CARGO_TARGET_DIR"].is_string(), cargo);
        assert_eq!(roots.len(), if cargo { 2 } else { 1 });
        if cargo {
            assert_eq!(roots[1], env["CARGO_TARGET_DIR"]);
        }
        assert_eq!(
            Ledger::open(home)
                .unwrap()
                .get(&work)
                .unwrap()
                .unwrap()
                .class,
            Class::Worktree
        );
    }
}
#[test]
fn council_directory_is_registered() {
    let t = common::temp();
    let home = t.path();
    let store = init(home);
    fs::remove_file(store.join("units.tsv")).unwrap();
    let plan_dir = store.join("astra");
    fs::create_dir_all(&plan_dir).unwrap();
    let plan = plan_dir.join("plan.json");
    let schema = plan_dir.join("schema.json");
    fs::write(&plan, r#"{"changes":[]}"#).unwrap();
    fs::write(
        &schema,
        r#"{"type":"object","properties":{"changes":{"type":"array"}}}"#,
    )
    .unwrap();
    let missing = || Err(axiom::base::Failure::new("not_installed", "none"));
    let setup = council::Setup::new(missing(), missing());
    let req = council::Request {
        brief: "fixture".into(),
        cwd: home.into(),
        plan_path: plan,
        schema_path: schema,
    };
    let _ = council::run(
        &setup,
        &req,
        &|_| council::CrossReply {
            status: council::Status::Failed,
            output: None,
            raw: None,
            tokens: 0,
            model: None,
            detail: None,
        },
        &AtomicBool::new(false),
    );
    entry(home, &plan_dir.join("council"), Class::ResearchCache);
}

#[test]
fn setup_cargo_env_lease_and_cancel() {
    if std::env::var_os("AXIOM_SITES_SETUP_CHILD").is_none() {
        let t = common::temp();
        let disk = t.path().join("disk.json");
        fs::write(&disk, r#"{"free_bytes":1000000000000}"#).unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "setup_cargo_env_lease_and_cancel", "--nocapture"])
            .env("AXIOM_SITES_SETUP_CHILD", "1")
            .env("AXIOM_FAKE_DISK", disk)
            .env("AXIOM_HOME", t.path())
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = std::path::PathBuf::from(std::env::var_os("AXIOM_HOME").unwrap());
    let store = init(&home);
    let repo = home.join("repo");
    fs::create_dir_all(repo.join(".axiom")).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    git(&repo, &["init", "-q"]);
    let marker = home.join("target.txt");
    let script = home.join("step.py");
    fs::write(&script, "import os,sys,time\nopen(sys.argv[1], 'w').write(os.environ['CARGO_TARGET_DIR'])\ntime.sleep(2)\n").unwrap();
    fs::write(
        repo.join(".axiom/setup.json"),
        serde_json::to_vec(&json!({"version":1,"steps":[
            {"id":"cargo-fetch","argv":[common::python(),"-c","pass"],"plan_independent":true},
            {"id":"hold","argv":[common::python(),script,marker],"plan_independent":true}
        ]}))
        .unwrap(),
    )
    .unwrap();
    let states = setup::start(&store, &repo).unwrap();
    let state = states.iter().find(|s| s.id == "hold").unwrap();
    assert_eq!(state.state, "running");
    let deadline = Instant::now() + Duration::from_secs(15);
    while fs::read_to_string(&marker).unwrap_or_default().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let target = fs::read_to_string(marker).unwrap();
    assert!(Path::new(&target).is_dir());
    assert!(storage::ledger::normalize(Path::new(&target))
        .starts_with(storage::ledger::normalize(&home.join("build-cache"))));
    let ledger = Ledger::open(&home).unwrap();
    assert_eq!(
        ledger.get(&state.scratch).unwrap().unwrap().class,
        Class::Scratch
    );
    assert!(ledger.in_use(&state.scratch).unwrap());
    // Simulate an already-exited PID while the supervisor still holds its lease.
    let state_path = store.join("setup/hold.json");
    let mut persisted: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    persisted["pid"] = Value::Null;
    fs::write(state_path, serde_json::to_vec(&persisted).unwrap()).unwrap();
    setup::cancel(&store, Some(&["hold".into()]), "test").unwrap();
    assert!(!state.scratch.exists());
    assert!(!ledger
        .entries()
        .unwrap()
        .iter()
        .any(|e| e.path == storage::ledger::normalize(&state.scratch)));
    let audit = fs::read_to_string(storage::retention::audit_log(&home)).unwrap();
    let records: Vec<Value> = audit
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(records
        .iter()
        .any(|v| v["reason"] == "cancelled" && v["outcome"] == "cancelled"));
    assert!(records
        .iter()
        .any(|v| v["reason"] == "cancelled" && v["outcome"] == "done"));
}

#[test]
fn verify_and_accept_cargo_env_and_cleanup() {
    let t = common::temp();
    let home = t.path();
    let store = init(home);
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    git(&repo, &["init", "-q"]);
    let command = format!(
        "\"{}\" -c \"import os; print(os.environ['CARGO_TARGET_DIR'])\"",
        common::python()
    );
    let brief = store.join("brief.txt");
    fs::write(
        &brief,
        format!("GOAL x\nSCOPE Cargo.toml\nVERIFY {command}\n"),
    )
    .unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS Cargo.toml\nDEPENDS none\nCONTRACT x\nACCEPT test `{command}`\n")).unwrap();
    orch::append_evidence(
        &store,
        "u",
        &orch::Evidence {
            kind: "run".into(),
            command: "worker".into(),
            exit_code: 0,
            commit: "".into(),
            ts: "now".into(),
        },
    )
    .unwrap();
    for run in [
        orch::record_verify(&store, "u", &repo).unwrap(),
        orch::record_accept(&store, "m", &repo).unwrap(),
    ] {
        assert_eq!(run.exit_code, 0, "{}", run.output);
        assert!(Path::new(run.output.trim()).is_dir());
    }
    let audit = fs::read_to_string(storage::retention::audit_log(home)).unwrap();
    let records: Vec<Value> = audit
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        records
            .iter()
            .filter(|v| v["reason"] == "unit_ended" && v["outcome"] == "done")
            .count(),
        2
    );
    for record in records {
        let scratch = Path::new(record["path"].as_str().unwrap());
        assert!(!scratch.exists());
        assert!(!Ledger::open(home)
            .unwrap()
            .entries()
            .unwrap()
            .iter()
            .any(|e| e.path == storage::ledger::normalize(scratch)));
    }
}

#[test]
fn astra_scratch_is_removed() {
    let t = common::temp();
    let home = t.path();
    let store = home.join("orchestrate/demo");
    assert_eq!(
        orch::run(&store, &["init".into()], true, false).exit_code,
        0
    );
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[])),
        wsl: None,
        on_windows: false,
    });
    let result = call(
        &bridge,
        "codex_run",
        &json!({"project":"demo", "role":"astra", "cwd":home, "sandbox":"read-only", "brief":"test"}),
    );
    assert_eq!(
        result["structuredContent"]["status"], "completed",
        "{result}"
    );
    let audit = fs::read_to_string(storage::retention::audit_log(home)).unwrap();
    let record: Value = audit
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["reason"] == "unit_ended" && v["outcome"] == "done")
        .unwrap();
    let scratch = Path::new(record["path"].as_str().unwrap());
    assert!(!scratch.exists());
    assert!(!Ledger::open(home)
        .unwrap()
        .entries()
        .unwrap()
        .iter()
        .any(|e| e.path == storage::ledger::normalize(scratch)));
}

#[test]
fn council_without_store_uses_axiom_home() {
    if std::env::var_os("AXIOM_SITES_COUNCIL_CHILD").is_none() {
        let t = common::temp();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "council_without_store_uses_axiom_home",
                "--nocapture"
            ])
            .env("AXIOM_SITES_COUNCIL_CHILD", "1")
            .env("AXIOM_HOME", t.path().join("home"))
            .env("AXIOM_SITES_PLAN", t.path().join("other/plan"))
            .status()
            .unwrap()
            .success());
        return;
    }
    let home = std::path::PathBuf::from(std::env::var_os("AXIOM_HOME").unwrap());
    let dir = std::path::PathBuf::from(std::env::var_os("AXIOM_SITES_PLAN").unwrap());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("plan.json"), r#"{"changes":[]}"#).unwrap();
    fs::write(
        dir.join("schema.json"),
        r#"{"type":"object","properties":{"changes":{"type":"array"}}}"#,
    )
    .unwrap();
    let missing = || Err(axiom::base::Failure::new("not_installed", "none"));
    let req = council::Request {
        brief: "fixture".into(),
        cwd: dir.clone(),
        plan_path: dir.join("plan.json"),
        schema_path: dir.join("schema.json"),
    };
    let result = council::run(
        &council::Setup::new(missing(), missing()),
        &req,
        &|_| council::CrossReply {
            status: council::Status::Failed,
            output: None,
            raw: None,
            tokens: 0,
            model: None,
            detail: None,
        },
        &AtomicBool::new(false),
    );
    assert_eq!(
        Ledger::open(&home)
            .unwrap()
            .get(&dir.join("council"))
            .unwrap()
            .unwrap_or_else(|| panic!(
                "council result: {result:?}, storage errors: {:?}",
                fs::read_to_string(home.join("storage/errors.log"))
            ))
            .class,
        Class::ResearchCache
    );
    assert!(!dir.parent().unwrap().join("storage").exists());
}

#[test]
fn failed_unit_worker_removes_scratch() {
    let t = common::temp();
    let home = t.path();
    let repo = home.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("file.txt"), "x").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "file.txt"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-qm",
            "init",
        ],
    );
    let store = home.join("orchestrate/demo");
    assert_eq!(
        orch::run(&store, &["init".into()], true, false).exit_code,
        0
    );
    let brief = store.join("brief.txt");
    fs::write(&brief, "GOAL build\nRISK trivial\nSCOPE file.txt\nVERIFY echo ok\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let bridge = Bridge::new(Config {
        data_home: Some(home.into()),
        codex: Ok(common::fake_codex(&[
            "--fake-break=turn/start:123:worker_failure",
        ])),
        wsl: None,
        on_windows: false,
    });
    let result = call(
        &bridge,
        "codex_run",
        &json!({"project":"demo","unit":"u","role":"terra","cwd":repo}),
    );
    assert!(result.to_string().contains("worker_failure"), "{result}");
    let audit = fs::read_to_string(storage::retention::audit_log(home)).unwrap();
    let record: Value = audit
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["reason"] == "unit_ended" && v["outcome"] == "done")
        .unwrap();
    let scratch = Path::new(record["path"].as_str().unwrap());
    assert!(!scratch.exists());
    assert!(!Ledger::open(home)
        .unwrap()
        .entries()
        .unwrap()
        .iter()
        .any(|e| e.path == storage::ledger::normalize(scratch)));
}
