mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    orch,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::{Command, Stdio}, io::Write};

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
                format!("GOAL build\nRISK trivial\nSCOPE src/mod.py\nVERIFY {verify}\n"),
            )
            .unwrap();
            rows.push_str(&format!("{id}\tm\tpending\t\t\t\t{}\n", brief.display()));
        }
        fs::write(store.join("units.tsv"), rows).unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        let mut fake = common::fake("fake_codex");
        fake.push(format!("--fake-log={}", temp.path().join("codex.log").display()));
        config.codex = Ok(fake);
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
fn handoff_has_five_fields_and_cap() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    let first = f.cycle("one");
    // Resume fixtures can contain arbitrarily large, multibyte recorded paths.
    let huge = format!("src/{}", "界".repeat(5000));
    fs::write(f.store.join("reports/one.changed.json"), serde_json::to_vec(&vec![huge]).unwrap()).unwrap();
    let second = f.cycle("one");
    for r in [first, second] {
        let handoff = r["handoff"].as_str().expect("handoff");
        assert!(handoff.len() <= 1200);
        let lines: Vec<_> = handoff.lines().collect();
        assert_eq!(lines.len(), 5);
        for (line, key) in lines.iter().zip(["RESULT", "PATHS", "COMMANDS", "OPEN", "EVIDENCE"]) {
            assert_eq!(line.split_once(' ').unwrap().0, key);
            assert_eq!(lines.iter().filter(|l| l.starts_with(&format!("{key} "))).count(), 1);
        }
    }
}

impl Fixture {
    fn inline(&self) {
        axiom::engine::policy::set(&self.store, "reviewer", "astra-max").unwrap();
        let path = self.store.join("one.brief");
        let mut brief = fs::read_to_string(&path).unwrap();
        brief.push_str("FAKE_REVIEW {\"verdict\":\"pass\",\"findings\":[]}\n");
        fs::write(path, brief).unwrap();
    }
    fn turns(&self) -> Vec<Value> {
        fs::read_to_string(self.temp.path().join("codex.log")).unwrap().lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|r| r["method"] == "turn/start").collect()
    }
}

#[test]
fn inline_pass_is_done() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    f.inline();
    let r = f.cycle("one");
    assert_eq!(r["next"], "done", "{r}");
    assert_eq!(r["state"], "verified");
    assert_eq!(r["review"], json!([]));
    assert_eq!(f.turns().len(), 2, "one worker and one inline reviewer, no Claude");
    assert!(orch::evidence(&f.store, "one").unwrap().iter()
        .any(|r| r.kind == "review" && r.command.starts_with("engine-astra:") && r.exit_code == 0));
}

#[test]
fn review_only_while_claude_required() {
    for (mode, expected) in [("claude", "review"), ("both", "review"), ("astra-max", "done")] {
        let f = Fixture::new(&[("one", "echo ok")], "echo ok");
        f.inline();
        axiom::engine::policy::set(&f.store, "reviewer", mode).unwrap();
        let r = f.cycle("one");
        assert_eq!(r["next"], expected, "{mode}: {r}");
        if expected == "review" {
            assert!(r["review"].as_array().unwrap().iter().any(|v| v["reviewer"] == "chronos:reviewer"));
        }
    }
}

#[test]
fn ready_lists_unblocked() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo ok");
    f.inline();
    let rows = fs::read_to_string(f.store.join("units.tsv")).unwrap().replace("two\tm\t", "two\tn\t");
    fs::write(f.store.join("units.tsv"), rows).unwrap();
    fs::write(f.store.join("modules.md"), "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## n\nPURPOSE x\nPATHS other/\nDEPENDS m\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    fs::write(f.store.join("two.brief"), "GOAL dependent\nRISK trivial\nSCOPE other/\nVERIFY echo ok\n").unwrap();
    assert!(!orch::ready_units(&f.store).unwrap().contains(&"two".into()));
    let r = f.cycle("one");
    assert!(r["ready"].as_array().expect("ready array").contains(&json!("two")), "{r}");
    assert!(r["ready"].as_array().unwrap().contains(&json!("n")), "module ready: {r}");
    assert_eq!(f.cycle("one")["ready"], json!([]), "already ready is not newly ready");
}

#[test]
fn core_null_on_success_set_on_fix_rounds() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    let r = f.cycle("one");
    assert!(r.get("core").is_some_and(Value::is_null), "{r}");
    let f = Fixture::new(&[("one", "exit 1")], "echo ok");
    for _ in 0..2 {
        let r = f.cycle("one");
        assert_eq!(r["core"]["kind"], "fix_decision", "{r}");
        assert!(!r["core"]["reason"].as_str().unwrap().is_empty());
    }
    let r = f.cycle("one");
    assert_eq!(r["error"]["kind"], "fix_limit");
    assert_eq!(r["core"]["kind"], "fix_rounds", "{r}");
    assert!(!r["core"]["reason"].as_str().unwrap().is_empty());
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    fs::write(f.store.join("modules.md"), "## dep\nPURPOSE x\nPATHS dep/\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## m\nPURPOSE x\nPATHS src/\nDEPENDS dep\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    let r = f.cycle("one");
    assert_eq!(r["core"]["kind"], "blocked", "{r}");
    assert!(!r["core"]["reason"].as_str().unwrap().is_empty());
}

#[test]
fn existing_keys_kept() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    let r = f.cycle("one");
    for key in ["status", "error", "unit", "state", "registered", "model", "attempt", "attempt_id", "verify_exit", "verify_output", "tests", "failing", "changed", "outside_scope", "outside_scope_count", "accept", "report", "report_json", "pack", "usage", "slot_wait_ms", "slot_wait", "next", "reason", "verdict", "review", "paths", "module_done"] {
        assert!(r.get(key).is_some(), "missing {key}: {r}");
    }
    assert_eq!(r["status"], "completed");
    assert_eq!(r["unit"], "one");
    assert_eq!(r["state"], "built");
    assert_eq!(r["next"], "review");
    assert_eq!(r["verify_exit"], 0);
    assert_eq!(r["attempt"], 1);
    assert_eq!(r["registered"], false);
    assert_eq!(r["error"], Value::Null);
    assert_eq!(r["failing"], json!([]));
    assert_eq!(r["outside_scope_count"], 0);
    assert_eq!(r["accept"]["exit_code"], 0);
    assert!(PathBuf::from(r["pack"].as_str().unwrap()).is_file());
}

#[test]
fn one_cycle_one_dispatch() {
    let f = Fixture::new(&[("one", "echo ok")], "echo ok");
    assert_eq!(f.cycle("one")["status"], "completed");
    assert_eq!(f.turns().len(), 1);
    assert_eq!(f.cycle("one")["attempt"], 1);
    assert_eq!(f.turns().len(), 1, "resume never redispatches worker");
}

#[test]
fn crash_points_abort() {
    for point in [Some("after_worker"), Some("after_verify"), Some("after_selfcheck"), Some("after_review"), None, Some("unrelated")] {
        let f = Fixture::new(&[("one", "echo ok")], "echo ok");
        f.inline();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_axiom"));
        cmd.arg("bridge").env("AXIOM_DATA_HOME", f.temp.path()).env("AXIOM_HOME", f.temp.path())
            .env("AXIOM_CODEX_CMD", serde_json::to_string(&common::fake("fake_codex")).unwrap())
            .env_remove("ORCH_STORE").env_remove("AXIOM_TEST_CRASH_AT")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(point) = point { cmd.env("AXIOM_TEST_CRASH_AT", point); }
        let mut child = cmd.spawn().unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"codex_run","arguments":{"project":"demo","unit":"one","role":"terra","cwd":f.repo,"cycle":true}}});
        writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
        let out = child.wait_with_output().unwrap();
        let crashes = point.is_some_and(|p| p != "unrelated");
        assert_eq!(out.status.success(), !crashes, "{point:?}: {} {}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if crashes {
            let cp: Value = serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).unwrap()).unwrap();
            let stage = match point.unwrap() { "after_worker" => "worker_done", "after_verify" => "verified", "after_selfcheck" => "selfchecked", _ => "reviewed" };
            assert_eq!(cp["stage"], stage);
            if point == Some("after_worker") {
                assert!(orch::receipts::read_runs(&f.store).unwrap().is_empty());
                assert!(!orch::evidence(&f.store, "one").unwrap().iter().any(|r| r.kind == "run"));
            }
        } else {
            let stdout = String::from_utf8(out.stdout).unwrap();
            let response: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
            assert_eq!(response["result"]["structuredContent"]["next"], "done", "{response}");
        }
    }
}
