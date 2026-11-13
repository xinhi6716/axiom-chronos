mod common;

use axiom::{engine::policy, orch};
use serde_json::{json, Value};
use std::{fs, io::Write, path::PathBuf, process::{Command, Stdio}};

struct Fixture {
    home: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    log: PathBuf,
    script: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home = common::temp();
        let repo = home.path().join("repo");
        let store = orch::store_for(home.path(), "demo").unwrap();
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture\n").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
            vec!["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "-qm", "fixture"],
        ] {
            assert!(Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
        }
        let brief = store.join("one.brief");
        fs::write(&brief, "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        fs::write(store.join("units.tsv"), format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\none\tm\tpending\t\t\t\t{}\n", brief.display()
        )).unwrap();
        policy::set(&store, "reviewer", "astra-max").unwrap();
        let log = home.path().join("codex.jsonl");
        let script = home.path().join("script.json");
        fs::write(&script, json!([{
            "match":"AXIOM-ENGINE reviewer review",
            "reply":{"verdict":"pass","findings":[]}
        }]).to_string()).unwrap();
        let f = Self { home, repo, store, log, script };
        // Build before installing settings, so even malformed settings are tested
        // at the new reviewer stage rather than during worker model routing.
        let built = f.run(false);
        assert_eq!(built["status"], "completed", "{built}");
        assert_eq!(built["state"], "built", "{built}");
        assert!(f.review_turns().is_empty());
        f
    }

    fn run(&self, cycle: bool) -> Value {
        let mut codex = common::fake("fake_codex");
        codex.push(format!("--fake-log={}", self.log.display()));
        codex.push(format!("--fake-script={}", self.script.display()));
        let mut child = Command::new(env!("CARGO_BIN_EXE_axiom"))
            .arg("bridge")
            .env("AXIOM_DATA_HOME", self.home.path())
            .env("AXIOM_HOME", self.home.path())
            .env("AXIOM_CODEX_CMD", serde_json::to_string(&codex).unwrap())
            .env("AXIOM_CLAUDE_CMD", serde_json::to_string(&common::fake("fake_claude")).unwrap())
            .env("FAKE_CODEX_MODELS", "gpt-6.1-sol,gpt-6-astra,gpt-6-terra")
            .env_remove("ORCH_STORE")
            .env_remove("AXIOM_TEST_CRASH_AT")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"codex_run","arguments":{
                "project":"demo","unit":"one","role":"terra","cwd":self.repo,"cycle":cycle
            }
        }});
        writeln!(child.stdin.take().unwrap(), "{request}").unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8(out.stdout).unwrap();
        let response: Value = serde_json::from_str(stdout.lines().last().expect("bridge reply")).unwrap();
        response["result"]["structuredContent"].clone()
    }

    fn global(&self) {
        let mut models = policy::Models::default();
        models.reviewer.tier = "sol".into();
        models.reviewer.effort = "high".into();
        policy::save_global_models(self.home.path(), &models).unwrap();
    }

    fn review_turns(&self) -> Vec<Value> {
        let rows: Vec<Value> = fs::read_to_string(&self.log).unwrap_or_default().lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect();
        rows.iter().enumerate()
            .filter(|(_, row)| row["method"] == "turn/start"
                && row["params"]["input"].to_string().contains("AXIOM-ENGINE reviewer review"))
            .map(|(i, row)| {
                // Codex sets the model on thread/start, effort on turn/start.
                let thread = rows[..i].iter().rev().find(|r| r["method"] == "thread/start").unwrap();
                json!({"model":thread["params"]["model"],"effort":row["params"]["effort"]})
            }).collect()
    }

    fn assert_review(&self, model: &str, effort: &str) {
        let result = self.run(true);
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(result["state"], "verified", "{result}");
        let turns = self.review_turns();
        assert_eq!(turns.len(), 1, "one new review: {turns:?}");
        assert_eq!(turns[0]["model"], model);
        assert_eq!(turns[0]["effort"], effort);
    }
}

#[test]
fn global_reviewer_setting_reaches_new_review() {
    let f = Fixture::new();
    f.global();
    f.assert_review("gpt-6.1-sol", "high");
}

#[test]
fn wish_reviewer_override_beats_global() {
    let f = Fixture::new();
    f.global();
    policy::set(&f.store, "models.reviewer", "terra/medium").unwrap();
    f.assert_review("gpt-6-terra", "medium");
}

#[test]
fn absent_settings_keep_default_reviewer() {
    let f = Fixture::new();
    assert!(!f.home.path().join("settings/models.json").exists());
    assert!(policy::load(&f.store).unwrap().models.is_none());
    f.assert_review("gpt-6-astra", "max");
}

#[test]
fn malformed_global_uses_effective_models_error() {
    let f = Fixture::new();
    fs::create_dir_all(f.home.path().join("settings")).unwrap();
    fs::write(f.home.path().join("settings/models.json"), "{broken").unwrap();
    let expected = policy::effective_models(Some(f.home.path()), &f.store).err().unwrap();
    let result = f.run(true);
    assert_eq!(result["status"], "bridge_error", "{result}");
    assert_eq!(result["error"], expected.to_json());
    assert!(f.review_turns().is_empty(), "invalid settings must not start a review");
}
