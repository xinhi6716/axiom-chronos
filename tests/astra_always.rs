mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    goal, orch,
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
    fn new() -> Self {
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
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        let brief = store.join("one.brief");
        fs::write(&brief, "GOAL build\nSCOPE src/mod.py\nVERIFY echo ok\n").unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\none\tm\tpending\t\t\t\t{}\n",
                brief.to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let mut config = Config::from_env();
        config.data_home = Some(temp.path().into());
        config.codex = Ok(common::fake("fake_codex"));
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

    fn write_goal(&self, wish: &str, status: &str, project: &str) {
        let mut g = goal::open(
            self.temp.path(),
            "session",
            wish,
            Some(self.repo.to_str().unwrap()),
        )
        .unwrap();
        g.project = Some(project.into());
        g.status = status.into();
        fs::write(
            self.temp.path().join("goals/session.json"),
            serde_json::to_vec(&g).unwrap(),
        )
        .unwrap();
    }

    fn run(&self) -> Value {
        call(
            &self.bridge,
            "codex_run",
            &json!({"project":"demo", "unit":"one", "role":"terra", "cwd":self.repo}),
        )["structuredContent"]
            .clone()
    }

    fn assert_gated(&self) {
        let result = self.run();
        assert_eq!(result["status"], "bridge_error", "{result}");
        assert_eq!(result["error"]["kind"], "astra_gate", "{result}");
        assert!(!orch::evidence(&self.store, "one")
            .unwrap()
            .iter()
            .any(|row| row.kind == "run"));
        let claims = fs::read_to_string(self.store.join("claims.tsv")).unwrap_or_default();
        assert!(
            claims.lines().skip(1).all(|line| line.trim().is_empty()),
            "{claims}"
        );
    }

    fn prerequisites(&self) {
        orch::record_stage(&self.store, "astra", "gpt-6-astra", true).unwrap();
        fs::create_dir_all(self.store.join("astra")).unwrap();
        for file in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
            fs::write(self.store.join("astra").join(file), "ok").unwrap();
        }
    }

    fn assert_completed(&self) {
        let result = self.run();
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(result["attempt"], 1, "{result}");
    }
}

#[test]
fn every_wish_requires_astra_before_dispatch() {
    for wish in [
        "build",
        "[astra] build",
        "[no-architect] build",
        "[no-architect] [astra] build",
        "[creative] build",
    ] {
        let f = Fixture::new();
        f.write_goal(wish, "active", "demo");
        f.assert_gated();
        f.prerequisites();
        f.assert_completed();
    }
}

#[test]
fn runs_without_active_goal_are_not_gated() {
    for goal in [
        None,
        Some(("done", "demo")),
        Some(("blocked", "demo")),
        Some(("active", "other")),
    ] {
        let f = Fixture::new();
        if let Some((status, project)) = goal {
            f.write_goal("build", status, project);
        }
        assert!(!f.store.join("astra").exists());
        f.assert_completed();
        assert!(!f.store.join("astra").exists());
    }
}

#[test]
fn legacy_goal_json_and_resume_keep_gate() {
    let f = Fixture::new();
    fs::create_dir_all(f.temp.path().join("goals")).unwrap();
    let file = f.temp.path().join("goals/session.json");
    let raw = serde_json::to_vec(&json!({"wish":"[astra] 修理", "cwd": f.repo, "status":"active", "amendments":[], "project":"demo", "extra":42})).unwrap();
    fs::write(&file, &raw).unwrap();
    let g = goal::load(f.temp.path(), "session").unwrap().unwrap();
    assert_eq!(g.other["extra"], 42);
    assert!(!goal::creative_requested(&g));
    f.assert_gated();
    assert_eq!(fs::read(&file).unwrap(), raw);
    f.write_goal("build", "active", "demo");
    goal::interrupt(f.temp.path(), "session", "crash").unwrap();
    goal::reopen(f.temp.path(), "session").unwrap();
    f.assert_gated();
}
