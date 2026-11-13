mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    orch,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command};

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    bridge: Bridge,
}

impl Fixture {
    fn new(units: &[(&str, &str)], accept: &str) -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        fs::write(store.join("modules.md"), format!("## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `{accept}`\n")).unwrap();
        let mut rows = String::from("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n");
        for (id, verify) in units {
            let brief = store.join(format!("{id}.brief"));
            fs::write(
                &brief,
                format!("GOAL build\nSCOPE src/mod.py\nVERIFY {verify}\n"),
            )
            .unwrap();
            rows.push_str(&format!("{id}\tm\tpending\t\t\t\t{}\n", brief.display()));
        }
        fs::write(store.join("units.tsv"), rows).unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(vec![common::python(), common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned(), format!("--fake-log={}", temp.path().join("codex.log").display())]);
        config.wsl = None;
        config.on_windows = false;
        let bridge = Bridge::new(config);
        Self {
            temp,
            repo,
            store,
            bridge,
        }
    }
    fn cycle(&self, unit: &str) -> Value {
        call(
            &self.bridge,
            "codex_run",
            &json!({"project":"demo","unit":unit,"role":"terra","cwd":self.repo,"cycle":true}),
        )["structuredContent"]
            .clone()
    }
}

#[test]
fn checkpoint_written_and_removed() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let orch::Begin::Run { attempt, attempt_id, .. } =
        orch::begin_unit(&f.store, "one", "terra", &f.repo, false).unwrap() else { panic!("expected claim") };
    let path = f.store.join("checkpoints/one.json");
    let checkpoint: Value = serde_json::from_slice(&fs::read(&path).expect("claim must write checkpoint")).unwrap();
    assert_eq!(checkpoint["unit"], "one");
    assert_eq!(checkpoint["attempt"], attempt);
    assert_eq!(checkpoint["attempt_id"], attempt_id);
    assert_eq!(checkpoint["stage"], "claimed");
    for key in ["base_tree", "brief_sha256", "routing", "pid", "updated"] {
        assert!(checkpoint.get(key).is_some(), "missing {key}");
    }
    orch::release_claim(&f.store, "one", &attempt_id).unwrap();
    assert!(!path.exists());
}

fn dead_claim(f: &Fixture) -> Value {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "claim_child", "--nocapture"])
        .env("RESUME_STORE", &f.store).env("RESUME_REPO", &f.repo).spawn().unwrap();
    let path = f.store.join("checkpoints/one.json");
    for _ in 0..200 {
        if path.exists() { break; }
        if child.try_wait().unwrap().is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let checkpoint = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    child.kill().ok(); child.wait().ok();
    checkpoint.expect("child claim must write checkpoint")
}

#[test]
fn claim_child() {
    let Ok(store) = std::env::var("RESUME_STORE") else { return };
    let repo = std::env::var("RESUME_REPO").unwrap();
    if let Ok(mode) = std::env::var("RESUME_USAGE_MODE") {
        usage_child(&store, &repo, &mode);
        return;
    }
    if let Ok(home) = std::env::var("RESUME_BRIDGE_HOME") {
        let log = std::path::Path::new(&home).join("codex.log");
        let wrapper = std::path::Path::new(&home).join("wrapper.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(PathBuf::from(&home));
        config.codex = Ok(vec![common::python(), wrapper.to_string_lossy().into_owned(), log.to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned()]);
        config.wsl = None; config.on_windows = false;
        let bridge = Bridge::new(config);
        call(&bridge, "codex_run", &json!({"project":"demo","unit":"one","role":"terra","cwd":repo,"cycle":true}));
    } else {
        let _claim = orch::begin_unit(std::path::Path::new(&store), "one", "terra", std::path::Path::new(&repo), false).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[test]
fn kill_mid_worker_resumes_same_attempt() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let gate = f.temp.path().join("gate");
    fs::write(f.store.join("one.brief"), format!("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_HOLD {}\n", gate.display())).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap()).args(["--exact", "claim_child", "--nocapture"])
        .env("RESUME_STORE", &f.store).env("RESUME_REPO", &f.repo).env("RESUME_BRIDGE_HOME", f.temp.path()).spawn().unwrap();
    let log = f.temp.path().join("codex.log");
    let mut started = false;
    for _ in 0..400 {
        if fs::read_to_string(&log).unwrap_or_default().contains("turn/start") { started = true; break; }
        if child.try_wait().unwrap().is_some() { break; }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).unwrap()).unwrap();
    child.kill().unwrap(); child.wait().unwrap(); fs::write(&gate, "release").unwrap();
    assert!(started, "worker did not start");
    let orch::Begin::Run { attempt, attempt_id, .. } = orch::begin_unit(&f.store, "one", "terra", &f.repo, false).expect("resume dead owner") else { panic!("expected run") };
    assert_eq!(cp["attempt"], attempt); assert_eq!(cp["attempt_id"], attempt_id);
    assert!(!fs::read_to_string(f.store.join("units.tsv")).unwrap().contains("superseded"));
    assert!(!orch::evidence(&f.store, "one").unwrap().iter().any(|r| r.kind == "run"));
    orch::release_claim(&f.store, "one", &attempt_id).unwrap();
}

#[test]
fn live_owner_is_already_running() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let orch::Begin::Run { attempt_id, .. } = orch::begin_unit(&f.store, "one", "terra", &f.repo, false).unwrap() else { panic!() };
    assert!(f.store.join("checkpoints/one.json").exists());
    assert_eq!(orch::begin_unit(&f.store, "one", "terra", &f.repo, false).err().unwrap().kind, "already_running");
    orch::release_claim(&f.store, "one", &attempt_id).unwrap();
}

#[test]
fn brief_changed_refused() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    dead_claim(&f);
    fs::write(f.store.join("one.brief"), "GOAL changed\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
    assert_eq!(orch::begin_unit(&f.store, "one", "terra", &f.repo, false).err().unwrap().kind, "brief_changed");
}

#[test]
fn concurrent_resume_single_owner() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    dead_claim(&f);
    let results = std::thread::scope(|s| (0..8).map(|_| s.spawn(|| orch::begin_unit(&f.store, "one", "terra", &f.repo, false))).collect::<Vec<_>>().into_iter().map(|t| t.join().unwrap()).collect::<Vec<_>>());
    let ids: Vec<_> = results.into_iter().filter_map(|r| match r { Ok(orch::Begin::Run { attempt_id, .. }) => Some(attempt_id), _ => None }).collect();
    assert_eq!(ids.len(), 1);
    orch::release_claim(&f.store, "one", &ids[0]).unwrap();
}


fn crash_cycle(f: &Fixture, point: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "claim_child", "--nocapture"])
        .env("RESUME_STORE", &f.store).env("RESUME_REPO", &f.repo)
        .env("RESUME_BRIDGE_HOME", f.temp.path())
        .env("AXIOM_TEST_CRASH_AT", point).status().unwrap();
    assert!(!status.success(), "crash point {point} was not reached");
}
fn turns(f: &Fixture) -> usize {
    fs::read_to_string(f.temp.path().join("codex.log")).unwrap_or_default()
        .lines().filter(|line| line.contains("turn/start")).count()
}
fn evidence_count(f: &Fixture, kind: &str) -> usize {
    orch::evidence(&f.store, "one").unwrap().iter().filter(|r| r.kind == kind).count()
}

#[test]
fn kill_after_worker() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    crash_cycle(&f, "after_worker");
    let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).unwrap()).unwrap();
    assert_eq!(cp["stage"], "worker_done");
    assert_eq!(cp["worker_result"]["status"], "completed");
    assert!(turns(&f) > 0);
    let before = turns(&f);
    let r = f.cycle("one");
    assert_eq!(r["attempt_id"], cp["attempt_id"], "{r}");
    assert_eq!(turns(&f), before);
    assert_eq!(orch::receipts::read_runs(&f.store).unwrap().len(), 1);
    assert_eq!(evidence_count(&f, "run"), 1);
    assert_eq!(orch::receipts::read_routing(&f.store).unwrap().len(), 1);
    let costs = orch::events::read_events(&f.store, 0).unwrap().into_iter()
        .filter(|e| e.data["call"] == "codex_run").count();
    assert_eq!(costs, 1);
    let evidence = orch::evidence(&f.store, "one").unwrap();
    let _ = f.cycle("one");
    assert_eq!(json!(orch::evidence(&f.store, "one").unwrap()), json!(evidence));
    assert_eq!(turns(&f), before);
    assert!(!f.store.join("checkpoints/one.json").exists());
}

#[test]
fn kill_after_verify_no_extra_worker() {
    for point in ["after_verify", "after_selfcheck"] {
        let f = Fixture::new(&[("one", "echo ok")], "echo ok");
        crash_cycle(&f, point);
        let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).expect("checkpoint must survive verification crash")).unwrap();
        assert_eq!(cp["stage"], if point == "after_verify" { "verified" } else { "selfchecked" });
        let before = turns(&f);
        let evidence = evidence_count(&f, "verify");
        assert_eq!(evidence, 1);
        let r = f.cycle("one");
        assert_eq!(r["status"], "completed", "{r}");
        assert_eq!(turns(&f), before);
        assert_eq!(evidence_count(&f, "verify"), evidence);
        assert_eq!(r["attempt_id"], cp["attempt_id"]);
        assert_eq!(evidence_count(&f, "run"), 1);
        assert_eq!(orch::receipts::read_runs(&f.store).unwrap().len(), 1);
        assert_eq!(orch::receipts::read_routing(&f.store).unwrap().len(), 1);
        assert!(!f.store.join("checkpoints/one.json").exists());
    }
}

#[test]
fn kill_after_review() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    axiom::engine::policy::set(&f.store, "reviewer", "astra-max").unwrap();
    fs::write(f.store.join("one.brief"), "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_REVIEW {\"verdict\":\"pass\",\"findings\":[]}\n").unwrap();
    crash_cycle(&f, "after_review");
    let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).expect("checkpoint must survive review crash")).unwrap();
    assert_eq!(cp["stage"], "reviewed");
    let before = turns(&f);
    let evidence = orch::evidence(&f.store, "one").unwrap();
    assert_eq!(evidence_count(&f, "review"), 1);
    let r = f.cycle("one");
    assert!(r["status"] == "completed" || r["error"]["kind"] == "already_verified", "{r}");
    assert_eq!(turns(&f), before);
    assert_eq!(json!(orch::evidence(&f.store, "one").unwrap()), json!(evidence));
    assert!(!f.store.join("checkpoints/one.json").exists());
}

#[test]
fn tree_change_invalidates_evidence() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo ok");
    assert_eq!(f.cycle("one")["state"], "built");
    let before = turns(&f);
    let _ = f.cycle("one");
    assert_eq!(turns(&f), before);
    fs::write(f.repo.join("src/mod.py"), "changed").unwrap();
    let _ = f.cycle("one");
    assert!(turns(&f) > before);
    assert_eq!(evidence_count(&f, "verify"), 2);
    let rows = orch::evidence(&f.store, "one").unwrap();
    let latest = rows.iter().rev().find(|r| r.kind == "verify").unwrap();
    assert_eq!(latest.commit, orch::snapshot(&f.repo, &["src/mod.py".into()]));
    let before = turns(&f);
    let resumed = f.cycle("one");
    assert_eq!(resumed["status"], "completed", "{resumed}");
    assert_eq!(turns(&f), before, "unchanged second build must not hit fix_limit or run again");
}

#[test]
fn resume_idempotent_for_verified() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo ok");
    assert_eq!(f.cycle("one")["state"], "built");
    let before = turns(&f);
    let _ = f.cycle("one");
    assert_eq!(turns(&f), before);
    assert_eq!(f.cycle("two")["state"], "built");
    orch::record_review(&f.store, &["one".into(), "two".into()], "reviewer", true, &f.repo).unwrap();
    let before = turns(&f);
    for unit in ["one", "two"] {
        let rows = orch::evidence(&f.store, unit).unwrap();
        assert_eq!(orch::derive_state(&rows, true), "verified");
        let r = f.cycle(unit);
        assert_eq!(r["error"]["kind"], "already_verified", "{r}");
        assert_eq!(json!(orch::evidence(&f.store, unit).unwrap()), json!(rows));
    }
    assert_eq!(turns(&f), before);
}

// Isolated caller processes keep provider flags out of concurrent tests.
fn usage_child(store: &str, repo: &str, mode: &str) {
    let home = PathBuf::from(std::env::var("RESUME_BRIDGE_HOME").unwrap());
    let _ = fs::remove_file(home.join("codex.log"));
    let wrapper = home.join("usage-wrapper.py");
    fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
    let mut config = Config::from_env();
    config.data_home = Some(home.clone());
    config.codex = Ok(vec![common::python(), wrapper.to_string_lossy().into_owned(), home.join("codex.log").to_string_lossy().into_owned(), common::repo_root().join("tests/fakes/fake_codex.py").to_string_lossy().into_owned()]);
    config.wsl = None;
    config.on_windows = false;
    let bridge = Bridge::new(config);
    let args = json!({"project":"demo","unit":"one","role":"terra","cwd":repo,"cycle":true});
    let result = if mode == "claude_limit_caller_cancel" {
        std::thread::scope(|s| {
            let worker = s.spawn(|| call(&bridge, "codex_run", &args)["structuredContent"].clone());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !fs::read_to_string(home.join("codex.log")).unwrap_or_default().contains("turn/start") {
                assert!(std::time::Instant::now() < deadline, "worker did not start");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // claude_quick uses a plain prompt; the CLI fake expects stream-json input.
            let adapter = home.join("claude-adapter.py");
            fs::write(&adapter, "import json,subprocess,sys\nprompt=sys.stdin.read()\nmessage={'type':'user','message':{'content':[{'type':'text','text':prompt}]}}\nsys.exit(subprocess.run(sys.argv[1:],input=json.dumps(message)+'\\n',text=True).returncode)\n").unwrap();
            std::env::set_var("AXIOM_CLAUDE_CMD", serde_json::to_string(&vec![
                common::python(), adapter.to_string_lossy().into_owned(),
                common::python(), "-I".into(),
                common::repo_root().join("tests/fakes/fake_claude.py").to_string_lossy().into_owned(),
            ]).unwrap());
            std::env::set_var("FAKE_CLAUDE_SCENARIO", "limit");
            std::env::set_var("FAKE_CLAUDE_RECORD", home.join("claude.json"));
            let claude = axiom::engine::calls::claude_quick(
                std::path::Path::new(store),
                &axiom::engine::calls::EngineCall {
                    role: axiom::engine::budgets::Role::Reviewer,
                    case: "resume-caller-cancel",
                    pack: "work".into(),
                    cwd: std::path::Path::new(repo),
                    schema: json!({"type":"object","required":["answer"]}),
                    timeout_s: 10,
                },
                "sonnet",
            );
            // This matrix row covers caller cancel-as-interruption, NOT automatic
            // Claude quota propagation into the bridge (there is no such path).
            // Join before asserting so a changed Claude signal cannot strand the worker.
            bridge.cancel();
            let result = worker.join().unwrap();
            let error = claude.expect_err("engine must observe the Claude limit");
            assert_eq!(error.kind, "claude_call");
            assert_eq!(error.message, "You've hit your usage limit");
            result
        })
    } else {
        call(&bridge, "codex_run", &args)["structuredContent"].clone()
    };
    assert_eq!(result["status"], "interrupted", "{result}");
    assert_eq!(result["unit"], "one");
    fs::write(home.join("interrupted.json"), serde_json::to_vec(&result).unwrap()).unwrap();
    assert!(!orch::evidence(std::path::Path::new(store), "one").unwrap().iter().any(|r| r.kind == "run"), "interruption must not count: {result}");
    let orch::Begin::Run { attempt, attempt_id, .. } = orch::begin_unit(std::path::Path::new(store), "one", "terra", std::path::Path::new(repo), false).expect("same live process must be able to reclaim its suspended attempt") else { panic!("expected resume") };
    assert_eq!(attempt, 1);
    assert_eq!(json!(attempt_id), result["attempt_id"]);
}

fn interrupted_attempt_resume(mode: &str) {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let gate = f.temp.path().join("usage.gate");
    if mode == "claude_limit_caller_cancel" {
        fs::write(f.store.join("one.brief"), format!("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_HOLD {}\n", gate.display())).unwrap();
    }
    let units_before = fs::read(f.store.join("units.tsv")).unwrap();
    let mut previous = None;
    // Matrix: Codex usage limit; Claude call error followed by explicit caller cancel.
    // Three interruptions must not consume the two-run budget or create fix rounds.
    for _ in 0..3 {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args(["--exact", "claim_child", "--nocapture"])
            .env("RESUME_STORE", &f.store).env("RESUME_REPO", &f.repo)
            .env("RESUME_BRIDGE_HOME", f.temp.path()).env("RESUME_USAGE_MODE", mode);
        if mode == "codex" { child.env("FAKE_USAGE_LIMIT", "1"); }
        assert!(child.status().unwrap().success(), "interruption must not count a run");
        assert_eq!(evidence_count(&f, "run"), 0);
        assert_eq!(evidence_count(&f, "verify"), 0);
        assert!(orch::receipts::read_runs(&f.store).unwrap().iter().all(|r| r.status == "interrupted"), "only non-counting interruption receipts are allowed");
        assert_eq!(fs::read(f.store.join("units.tsv")).unwrap(), units_before, "no supersede or fix units");
        let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).expect("interruption must preserve checkpoint")).unwrap();
        assert_eq!(cp["stage"], "claimed");
        assert_eq!(cp["unit"], "one");
        assert_eq!(cp["attempt"], 1);
        if let Some(id) = &previous { assert_eq!(&cp["attempt_id"], id); }
        previous = Some(cp["attempt_id"].clone());
    }
    fs::write(gate, "release").unwrap();
    let result = f.cycle("one");
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["unit"], "one");
    assert_eq!(result["attempt"], 1);
    assert_eq!(result["attempt_id"], previous.unwrap());
    assert_eq!(evidence_count(&f, "run"), 1);
    assert_eq!(evidence_count(&f, "verify"), 1);
    assert_eq!(orch::receipts::read_runs(&f.store).unwrap().len(), 1);
    assert_eq!(orch::receipts::read_routing(&f.store).unwrap().len(), 1);
    assert!(!f.store.join("checkpoints/one.json").exists());
    assert_eq!(orch::read_units(&f.store).unwrap().len(), 1);
}

#[test]
fn codex_usage_limit_resumes_same_attempt() { interrupted_attempt_resume("codex"); }

#[test]
fn claude_limit_caller_cancel_resumes_same_attempt() { interrupted_attempt_resume("claude_limit_caller_cancel"); }
