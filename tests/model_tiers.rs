mod common;

struct RunFixture {
    _temp: tempfile::TempDir,
    repo: std::path::PathBuf,
    log: std::path::PathBuf,
    bridge: axiom::bridge::Bridge,
}

impl RunFixture {
    fn new(tier: &str) -> Self {
        use std::{fs, process::Command};
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/mod.py"), "fixture").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "src/mod.py"],
            vec![
                "-c", "user.name=Test", "-c", "user.email=test@example.com",
                "commit", "-qm", "fixture",
            ],
        ] {
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let store = axiom::orch::store_for(temp.path(), "demo").unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        let brief = store.join("u1.brief");
        fs::write(
            &brief,
            format!("GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\nTIER {tier}\nEFFORT high\n"),
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let log = temp.path().join("codex.log");
        let wrapper = temp.path().join("fake.py");
        fs::write(
            &wrapper,
            "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nos.environ['FAKE_CODEX_MODELS']='gpt-6.1-sol,gpt-6.1-terra,gpt-6-astra,gpt-6-terra,gpt-6-luna'\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n",
        )
        .unwrap();
        let mut config = axiom::bridge::Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(vec![
            common::python(),
            wrapper.to_string_lossy().into_owned(),
            log.to_string_lossy().into_owned(),
            common::repo_root()
                .join("tests/fakes/fake_codex.py")
                .to_string_lossy()
                .into_owned(),
        ]);
        config.wsl = None;
        config.on_windows = false;
        Self {
            _temp: temp,
            repo,
            log,
            bridge: axiom::bridge::Bridge::new(config),
        }
    }

    fn run(&self) -> serde_json::Value {
        axiom::bridge::call(
            &self.bridge,
            "codex_run",
            &serde_json::json!({"project":"demo","unit":"u1","role":"terra","cwd":self.repo}),
        )["structuredContent"]
            .clone()
    }

    fn thread_models(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|request| request["method"] == "thread/start")
            .map(|request| request["params"]["model"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn check_run(tier: &str, model: &str) {
    let fixture = RunFixture::new(tier);
    let result = fixture.run();
    assert_eq!(result["status"], "completed", "{result}");
    let models = fixture.thread_models();
    assert!(!models.is_empty(), "codex_run must send thread/start");
    for actual in models {
        assert_eq!(actual, model, "wrong thread/start.model for TIER {tier}");
    }
}

#[test]
fn tier_sol_uses_gpt_6_1_sol() {
    check_run("sol", "gpt-6.1-sol");
}

#[test]
fn tier_terra_uses_gpt_6_1_terra() {
    check_run("terra", "gpt-6.1-terra");
}

#[test]
fn tier_astra_uses_gpt_6_astra() {
    check_run("astra", "gpt-6-astra");
}

#[test]
fn sol_is_not_astra_model() {
    let fixture = RunFixture::new("sol");
    let result = fixture.run();
    assert_eq!(result["status"], "completed", "{result}");
    let models = fixture.thread_models();
    assert!(!models.is_empty(), "codex_run must send thread/start");
    for model in models {
        assert_ne!(model, "gpt-6-astra", "sol must never use the astra model");
    }
}
