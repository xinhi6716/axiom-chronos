mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    orch,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    log: PathBuf,
    bridge: Bridge,
    home: PathBuf,
}
impl Fixture {
    fn new(brief: &str) -> Self {
        // Keep reply-size assertions independent of an arbitrarily long inherited TEMP path.
        let temp = tempfile::tempdir_in(common::repo_root()).unwrap();
        let home = temp.path().to_path_buf();
        let repo = home.join("repo");
        let store = orch::store_for(&home, "demo").unwrap();
        let log = home.join("codex.log");
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
        let path = store.join("one.brief");
        fs::write(&path, brief).unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\none\tm\tpending\t\t\t\t{}\n",
                path.display()
            ),
        )
        .unwrap();
        let bridge = Self::bridge(&home, &log);
        Self {
            _temp: temp,
            repo,
            store,
            log,
            bridge,
            home,
        }
    }
    fn bridge(home: &Path, log: &Path) -> Bridge {
        let wrapper = home.join("logged.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        Bridge::new(Config {
            data_home: Some(home.into()),
            codex: Ok(vec![
                common::python(),
                wrapper.to_string_lossy().into_owned(),
                log.to_string_lossy().into_owned(),
                common::repo_root()
                    .join("tests/fakes/fake_codex.py")
                    .to_string_lossy()
                    .into_owned(),
            ]),
            wsl: None,
            on_windows: false,
        })
    }
    fn args(&self, unit: &str, cycle: bool) -> Value {
        json!({"project":"demo","unit":unit,"role":"terra","cwd":self.repo,"cycle":cycle})
    }
    fn run(&self, unit: &str, cycle: bool) -> Value {
        call(&self.bridge, "codex_run", &self.args(unit, cycle))["structuredContent"].clone()
    }
    fn turns(&self) -> usize {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .matches("turn/start")
            .count()
    }
    fn add(&self, unit: &str, brief: &str) {
        use std::io::Write;
        let path = self.store.join(format!("{unit}.brief"));
        fs::write(&path, brief).unwrap();
        writeln!(
            fs::OpenOptions::new()
                .append(true)
                .open(self.store.join("units.tsv"))
                .unwrap(),
            "{unit}\tm\tpending\t\t\t\t{}",
            path.display()
        )
        .unwrap();
    }
    fn spawn(&self, unit: &str, cycle: bool) -> thread::JoinHandle<Value> {
        let bridge = Self::bridge(&self.home, &self.log);
        let args = self.args(unit, cycle);
        thread::spawn(move || call(&bridge, "codex_run", &args)["structuredContent"].clone())
    }
    fn wait_turns(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.turns() < count {
            assert!(
                Instant::now() < deadline,
                "fake codex did not start {count} turns"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn gate(&self, unit: &str) -> PathBuf {
        let gate = self.home.join(format!("{unit}.gate"));
        let brief = self.store.join(format!("{unit}.brief"));
        let text = fs::read_to_string(&brief).unwrap();
        fs::write(&brief, format!("{text}\nFAKE_GATE {}\n", gate.display())).unwrap();
        gate
    }
}

#[test]
fn duplicate_dispatch_names_owner() {
    let f = Fixture::new(
        "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_DELAY_MS 2000\nFAKE_TEMPDIR test\n",
    );
    let b = Fixture::bridge(&f.home, &f.log);
    let args = f.args("one", false);
    let handle = thread::spawn(move || call(&b, "codex_run", &args)["structuredContent"].clone());
    let deadline = Instant::now() + Duration::from_secs(15);
    while f.turns() == 0 {
        assert!(Instant::now() < deadline, "fake codex did not start");
        thread::sleep(Duration::from_millis(20));
    }
    let owner = orch::unit_owner(&f.store, "one").unwrap().unwrap();
    let refusal = f.run("one", true);
    assert_eq!(refusal["status"], "refused", "{refusal}");
    assert_eq!(refusal["error"]["kind"], "already_running");
    assert_eq!(refusal["next"], "wait");
    assert_eq!(
        refusal["owner"],
        json!({"wish":"demo","module":"m","unit":"one","attempt":owner.attempt})
    );
    assert!(refusal["error"]["message"]
        .as_str()
        .unwrap()
        .contains(&owner.attempt));
    let completed = handle.join().unwrap();
    assert_eq!(completed["state"], "built", "{completed}");
    assert_eq!(f.turns(), 1);
    assert!(!orch::scratch_dir(&f.store, &["one", &owner.attempt]).exists());
    assert!(!f.repo.join("pytest-of-fake").exists());
}

#[test]
fn verified_dispatch_is_refused() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");
    assert_eq!(f.run("one", false)["state"], "built");
    orch::record_review(&f.store, &["one".into()], "reviewer", true, &f.repo).unwrap();
    for cycle in [false, true] {
        let refused = f.run("one", cycle);
        assert_eq!(refused["status"], "refused", "{refused}");
        assert_eq!(refused["error"]["kind"], "already_verified");
        assert_eq!(refused["state"], "verified");
        assert_eq!(
            refused["owner"],
            json!({"wish":"demo","module":"m","unit":"one","attempt":"1"})
        );
        if cycle {
            assert_eq!(refused["next"], "done");
        } else {
            assert!(refused.get("next").is_none());
        }
    }
    assert_eq!(f.turns(), 1);
}

#[test]
fn failed_unit_fix_chain_can_run() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY exit 1\n");
    assert_eq!(f.run("one", false)["state"], "failed");
    let second = f.run("one", false);
    assert_eq!(second["state"], "failed", "{second}");
    assert_eq!(second["attempt"], 2);
    let fix = f.store.join("fix.brief");
    fs::write(&fix, "GOAL fix\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
    let args: Vec<String> = [
        "unit",
        "add",
        "fix",
        "--track",
        "m",
        "--brief",
        fix.to_str().unwrap(),
        "--fixes",
        "one",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let result = orch::run(&f.store, &args, true, false);
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    let fixed = f.run("fix", false);
    assert_eq!(fixed["state"], "built", "{fixed}");
    assert_eq!(fixed["attempt"], 1);
    assert_eq!(f.turns(), 3);
}

#[test]
fn attempt_and_brief_use_isolated_scratch_and_clean_up() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_TEMPDIR test\n");
    let script = f.home.join("verify.py");
    fs::write(&script, "import os,pathlib\np=pathlib.Path(os.environ['TMP'])\nassert p==pathlib.Path(os.environ['TEMP'])==pathlib.Path(os.environ['TMPDIR'])\nassert p.is_dir()\nprint(p)\n").unwrap();
    fs::write(
        f.store.join("one.brief"),
        format!(
        "GOAL build\nSCOPE src/mod.py\nVERIFY \"{}\" \"{}\"\nFAKE_TEMPDIR test\nFAKE_TEMPFILE real\n",
            common::python(),
            script.display()
        ),
    )
    .unwrap();
    let result = f.run("one", false);
    assert_eq!(result["verify_exit"], 0, "{result}");
    let attempt = result["attempt_id"].as_str().unwrap();
    let attempt_dir = orch::scratch_dir(&f.store, &["one", attempt]);
    let verify_output = fs::read_to_string(f.store.join("reports/one.verify.txt")).unwrap();
    assert_eq!(
        Path::new(verify_output.trim()),
        attempt_dir.join("verify/tmp")
    );
    assert!(!attempt_dir.exists());
    let report = call(&f.bridge, "codex_run", &json!({"project":"demo", "role":"astra", "cwd":f.repo, "sandbox":"read-only", "brief":"FAKE_TEMPDIR brief"}))["structuredContent"].clone();
    assert_eq!(report["status"], "completed", "{report}");
    let log = fs::read_to_string(&f.log).unwrap();
    let starts: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["method"] == "thread/start")
        .collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(
        starts[0]["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
        json!([attempt_dir])
    );
    assert!(starts[1]["params"]["config"]["sandbox_workspace_write"]["writable_roots"].is_null());
    for (entry, prefix) in starts
        .iter()
        .zip([attempt_dir, orch::scratch_dir(&f.store, &["astra"])])
    {
        let tmp = PathBuf::from(
            entry["params"]["config"]["shell_environment_policy"]["set"]["TMP"]
                .as_str()
                .unwrap(),
        );
        assert!(tmp.starts_with(&prefix), "{}", tmp.display());
        assert!(!tmp.exists());
    }
    assert!(!f.repo.join("pytest-of-fake").exists());
    assert!(!f.repo.join(".axiom-tmp").exists());
}

#[test]
fn unit_runs_record_receipts_even_on_failure() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");
    let good = f.run("one", false);
    fs::write(
        f.store.join("one.brief"),
        "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nAUTH\n",
    )
    .unwrap();
    let bad = f.run("one", false);
    let rows = orch::receipts::read_runs(&f.store).unwrap();
    assert!(f.store.join("receipts/runs.jsonl").is_file());
    assert_eq!(rows.len(), 2);
    for (row, reply, status) in [(&rows[0], &good, "completed"), (&rows[1], &bad, "failed")] {
        assert_eq!(row.unit, "one");
        assert_eq!(row.attempt, reply["attempt_id"].as_str().unwrap());
        assert_eq!(row.status, status);
        assert!(row.started_ms > 0 && row.started_ms <= row.ended_ms);
    }
    let starts: Vec<Value> = fs::read_to_string(&f.log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["method"] == "thread/start")
        .collect();
    for (entry, row) in starts.iter().zip(&rows) {
        assert_eq!(
            entry["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
            json!([orch::scratch_dir(&f.store, &["one", &row.attempt])])
        );
    }
    assert_ne!(
        starts[0]["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
        starts[1]["params"]["config"]["sandbox_workspace_write"]["writable_roots"]
    );
}

#[test]
fn receipt_write_failure_does_not_fail_unit() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");
    fs::write(f.store.join("receipts"), "not a directory").unwrap();
    let result = f.run("one", false);
    assert_eq!(result["state"], "built", "{result}");
}

#[test]
fn interrupted_unit_records_receipt() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nWAIT_FOR_INTERRUPT\n");
    thread::scope(|scope| {
        let run = scope.spawn(|| f.run("one", false));
        f.wait_turns(1);
        f.bridge.cancel();
        let result = run.join().unwrap();
        let rows = orch::receipts::read_runs(&f.store).unwrap();
        assert_eq!(rows.len(), 1, "{result}");
        assert_eq!(rows[0].status, "interrupted");
        assert_eq!(rows[0].attempt, result["attempt_id"].as_str().unwrap());
        assert!(rows[0].started_ms <= rows[0].ended_ms);
    });
}

#[test]
fn parallel_units_use_private_scratch() {
    let f = Fixture::new("GOAL build\nSCOPE a.txt\nVERIFY echo ok\nFAKE_WRITE a.txt\nFAKE_TEMPDIR one\nFAKE_DELAY_MS 1200\n");
    f.add("two", "GOAL build\nSCOPE b.txt\nVERIFY echo ok\nFAKE_WRITE b.txt\nFAKE_TEMPDIR two\nFAKE_DELAY_MS 1200\n");
    let one = f.spawn("one", false);
    f.wait_turns(1);
    let two = f.spawn("two", false);
    let results = [one.join().unwrap(), two.join().unwrap()];
    for r in &results {
        assert_eq!(r["state"], "built", "{r}");
    }
    let starts: Vec<Value> = fs::read_to_string(&f.log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["method"] == "thread/start")
        .collect();
    assert_eq!(starts.len(), 2);
    let temps: Vec<PathBuf> = starts
        .iter()
        .map(|e| {
            PathBuf::from(
                e["params"]["config"]["shell_environment_policy"]["set"]["TMP"]
                    .as_str()
                    .unwrap(),
            )
        })
        .collect();
    assert_ne!(temps[0], temps[1]);
    for r in &results {
        let expected = orch::scratch_dir(
            &f.store,
            &[
                r["unit"].as_str().unwrap(),
                r["attempt_id"].as_str().unwrap(),
            ],
        )
        .join("tmp");
        assert!(temps.contains(&expected));
        assert!(!expected.starts_with(&f.repo));
        assert!(!expected.exists());
    }
    orch::record_review(
        &f.store,
        &["one".into(), "two".into()],
        "reviewer",
        true,
        &f.repo,
    )
    .unwrap();
    for unit in ["one", "two"] {
        assert_eq!(
            orch::derive_state(&orch::evidence(&f.store, unit).unwrap(), false),
            "verified"
        );
    }
}

#[test]
fn unit_verify_leaves_no_axiom_tmp() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");
    let script = f.home.join("temp_verify.py");
    fs::write(&script, "import os,pathlib,tempfile\np=pathlib.Path(tempfile.mkdtemp(prefix='pytest-of-fake-'))\n(p/'case.txt').write_text('ok')\n(pathlib.Path(os.environ['TMP'])/'direct.txt').write_text('ok')\nprint(p)\n").unwrap();
    fs::write(
        f.store.join("one.brief"),
        format!(
            "GOAL build\nSCOPE src/mod.py\nVERIFY \"{}\" \"{}\"\n",
            common::python(),
            script.display()
        ),
    )
    .unwrap();
    let r = f.run("one", false);
    assert_eq!(r["verify_exit"], 0, "{r}");
    let scratch = orch::scratch_dir(&f.store, &["one", r["attempt_id"].as_str().unwrap()]);
    assert!(!scratch.exists());
    assert!(!f.repo.join(".axiom-tmp").exists());
    assert!(!fs::read_dir(&f.repo).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("pytest-of-")));
}

#[test]
fn scope_records_ignored_paths() {
    let f = Fixture::new("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nFAKE_SHELL_WRITE ignored.txt\nFAKE_SHELL_WRITE pytest-of-fake/pytest-0/x/case.txt\nFAKE_SHELL_WRITE sub/pytest-of-x/pytest-0/f\nFAKE_SHELL_WRITE pytest-of-x/other/f\nFAKE_SHELL_WRITE .axiom-tmpx/f\n");
    let ignore = "ignored.txt\n";
    fs::write(f.repo.join(".gitignore"), ignore).unwrap();
    let r = f.run("one", false);
    assert_eq!(r["error"]["kind"], "outside_scope", "{r}");
    assert_eq!(
        r["outside_scope"],
        json!([
            ".axiom-tmpx/f",
            "pytest-of-x/other/f",
            "sub/pytest-of-x/pytest-0/f"
        ])
    );
    let ignored = json!([{ "path":"ignored.txt", "reason":"git-ignored" }, { "path":"pytest-of-fake/pytest-0/x/case.txt", "reason":"test-scratch" }]);
    assert_eq!(r["scope_ignored"], ignored);
    assert_eq!(r["scope_ignored_count"], 2);
    let scope: Value =
        serde_json::from_slice(&fs::read(f.store.join("reports/one.scope.json")).unwrap()).unwrap();
    assert_eq!(scope["outside"], r["outside_scope"]);
    assert_eq!(scope["ignored"], ignored);
    assert_eq!(scope["unattributed"], json!([]));
    let rows = orch::evidence(&f.store, "one").unwrap();
    let row = rows.iter().find(|row| row.kind == "scope-ignored").unwrap();
    assert_eq!(
        row.command,
        "ignored.txt (git-ignored); pytest-of-fake/pytest-0/x/case.txt (test-scratch)"
    );
    assert_eq!(row.exit_code, 0);
    assert_eq!(
        fs::read_to_string(f.repo.join(".gitignore")).unwrap(),
        ignore
    );

    let mut brief = String::from("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n");
    for n in 0..22 {
        brief.push_str(&format!("FAKE_WRITE .axiom-tmp/{n:02}.txt\n"));
    }
    brief.push_str("FAKE_WRITE pytest-of-fake/pytest-current/x.txt\n");
    let bounded = Fixture::new(&brief);
    let first = bounded.run("one", false);
    assert_eq!(first["state"], "built", "{first}");
    assert_eq!(first["scope_ignored_count"], 23);
    assert_eq!(
        first["scope_ignored"].as_array().unwrap().len(),
        orch::summary::MAX_PATH_SAMPLE
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(bounded.store.join("reports/one.scope.json")).unwrap())
            .unwrap();
    assert_eq!(saved["ignored"].as_array().unwrap().len(), 23);
    assert_eq!(
        saved["ignored"][22],
        json!({"path":"pytest-of-fake/pytest-current/x.txt", "reason":"test-scratch"})
    );
    let replay = bounded.run("one", true);
    assert_eq!(replay["scope_ignored"], first["scope_ignored"], "{replay}");
    assert_eq!(replay["scope_ignored_count"], 23);
}

#[test]
fn parallel_unit_scope_survives_peer_completion() {
    let f = Fixture::new("GOAL build\nSCOPE a.txt\nVERIFY echo ok\nFAKE_WRITE a.txt\n");
    f.add(
        "two",
        "GOAL build\nSCOPE b.txt\nVERIFY echo ok\nFAKE_WRITE b.txt\n",
    );
    let one_gate = f.gate("one");
    let two_gate = f.gate("two");
    let one = f.spawn("one", false);
    f.wait_turns(1);
    let two = f.spawn("two", false);
    f.wait_turns(2);
    // Both baselines and peer scopes exist before either unit can write.
    fs::write(one_gate, "").unwrap();
    let a = one.join().unwrap();
    assert_eq!(a["state"], "built", "{a}");
    assert!(orch::unit_owner(&f.store, "one").unwrap().is_none());
    fs::write(two_gate, "").unwrap();
    let b = two.join().unwrap();
    assert_eq!(b["state"], "built", "{b}");
    assert_eq!(b["outside_scope"], json!([]));
    assert_eq!(b["unattributed"], json!(["a.txt"]));
    assert_eq!(b["unattributed_count"], 1);
    let rows = orch::evidence(&f.store, "two").unwrap();
    let row = rows
        .iter()
        .find(|row| row.kind == "scope-unattributed")
        .unwrap();
    assert_eq!(row.command, "a.txt");
    assert_eq!(row.exit_code, 0);
    let scope: Value =
        serde_json::from_slice(&fs::read(f.store.join("reports/two.scope.json")).unwrap()).unwrap();
    assert_eq!(scope["unattributed"], json!(["a.txt"]));
    let replay = f.run("two", true);
    assert_eq!(replay["unattributed"], json!(["a.txt"]), "{replay}");
}

#[test]
fn own_outside_write_is_not_excused_by_peer_scope() {
    let f = Fixture::new("GOAL build\nSCOPE a.txt\nVERIFY echo ok\nFAKE_WRITE a.txt\n");
    f.add(
        "two",
        "GOAL build\nSCOPE b.txt\nVERIFY echo ok\nFAKE_WRITE a.txt\n",
    );
    let one_gate = f.gate("one");
    let one = f.spawn("one", false);
    f.wait_turns(1);
    let b = f.run("two", false);
    assert_eq!(b["error"]["kind"], "outside_scope", "{b}");
    assert_eq!(b["outside_scope"], json!(["a.txt"]));
    fs::write(one_gate, "").unwrap();
    assert_eq!(one.join().unwrap()["state"], "built");
}
