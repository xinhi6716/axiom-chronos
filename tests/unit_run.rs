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
        let bridge = Bridge::new(Config {
            data_home: Some(temp.path().into()),
            codex: Ok(common::fake("fake_codex")),
            agy: Ok(common::fake("fake_agy")),
            wsl: None,
            on_windows: false,
        });
        Self {
            temp,
            repo,
            store,
            bridge,
        }
    }
    fn run(&self, unit: &str) -> Value {
        call(
            &self.bridge,
            "codex_run",
            &json!({"project":"demo","unit":unit,"role":"terra","cwd":self.repo}),
        )["structuredContent"]
            .clone()
    }
    fn accepts(&self, unit: &str) -> usize {
        orch::evidence(&self.store, unit)
            .unwrap()
            .iter()
            .filter(|row| row.kind == "accept")
            .count()
    }
    fn goal(&self) {
        let mut g = goal::open(
            self.temp.path(),
            "session",
            "build",
            Some(self.repo.to_str().unwrap()),
        )
        .unwrap();
        g.project = Some("demo".into());
        fs::write(
            self.temp.path().join("goals/session.json"),
            serde_json::to_vec(&g).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn auto_accept_and_verify_failure() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    let result = f.run("one");
    assert_eq!(result["status"], "completed", "{result}");
    assert_eq!(result["accept"]["ran"], true, "{result}");
    assert_eq!(result["accept"]["exit_code"], 0);
    assert_eq!(f.accepts("one"), 1);
    let f = Fixture::new(&[("one", "exit 1")], "echo accepted");
    let result = f.run("one");
    assert_eq!(
        result["accept"],
        json!({"ran":false,"reason":"not_built","detail":""})
    );
    assert_eq!(f.accepts("one"), 0);
}

#[test]
fn goal_holds() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    assert_eq!(goal::accept_hold(f.temp.path(), "demo").unwrap(), None);
    f.goal();
    assert_eq!(goal::accept_hold(f.temp.path(), "demo").unwrap(), None);
    goal::interject(f.temp.path(), "session", "stop").unwrap();
    assert_eq!(
        goal::accept_hold(f.temp.path(), "demo").unwrap().as_deref(),
        Some("amend_pending")
    );
    let result = f.run("one");
    assert_eq!(result["accept"]["reason"], "amend_pending", "{result}");
    assert_eq!(f.accepts("one"), 0);
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    f.goal();
    goal::interject(f.temp.path(), "session", "stop").unwrap();
    goal::classify(f.temp.path(), "session", "stop").unwrap();
    let result = f.run("one");
    assert_eq!(result["accept"]["reason"], "stopped", "{result}");
    assert_eq!(f.accepts("one"), 0);
}

#[test]
fn waits_for_all_module_units() {
    let f = Fixture::new(&[("one", "echo ok"), ("two", "echo ok")], "echo accepted");
    let first = f.run("one");
    assert_eq!(first["accept"]["reason"], "units_pending", "{first}");
    assert_eq!(f.accepts("one"), 0);
    let second = f.run("two");
    assert_eq!(second["accept"]["ran"], true, "{second}");
    assert_eq!(f.accepts("one"), 1);
    assert_eq!(f.accepts("two"), 1);
    assert!(second["accept"]["states"].get("one").is_some());
    assert_eq!(second["state"], second["accept"]["states"]["two"]);
}

#[test]
fn mixed_mode_message() {
    let f = Fixture::new(&[("one", "echo ok")], "echo accepted");
    let r = call(
        &f.bridge,
        "codex_run",
        &json!({"project":"demo","unit":"one","role":"terra","cwd":f.repo,"brief":"bad"}),
    )["structuredContent"]
        .clone();
    assert_eq!(r["error"]["kind"], "invalid_argument");
    assert_eq!(r["error"]["message"], "unit mode takes exactly project, unit, role (terra or sol) and cwd; brief and save_to belong to astra brief mode");
}

#[test]
fn accept_failure_does_not_fail_run() {
    let f = Fixture::new(&[("one", "echo ok")], "exit 1");
    let r = f.run("one");
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["accept"]["exit_code"], 1);
    assert_eq!(r["state"], "accept-failed");
}
