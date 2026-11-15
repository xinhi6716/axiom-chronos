mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    goal, orch,
    storage::preflight,
};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, process::Command, sync::Mutex};

static ENV: Mutex<()> = Mutex::new(());

struct Fixture {
    temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    disk: PathBuf,
    log: PathBuf,
    bridge: Bridge,
    old_disk: Option<std::ffi::OsString>,
}

impl Fixture {
    fn new(with_goal: bool) -> Self {
        let temp = common::temp();
        let home = temp.path();
        let repo = home.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
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
        fs::create_dir_all(home.join("storage")).unwrap();
        fs::write(
            home.join("storage/policy.json"),
            r#"{"reserve_bytes":0,"estimates":{"wish":20}}"#,
        )
        .unwrap();
        let disk = home.join("disk.json");
        fs::write(&disk, r#"{"free_bytes":50}"#).unwrap();
        let old_disk = std::env::var_os("AXIOM_FAKE_DISK");
        std::env::set_var("AXIOM_FAKE_DISK", &disk);
        let store = if with_goal {
            let mut g = goal::open(home, "session", "wish", repo.to_str()).unwrap();
            g.project = Some("demo".into());
            g.need_bytes = Some(100);
            fs::write(
                home.join("goals/session.json"),
                serde_json::to_vec(&g).unwrap(),
            )
            .unwrap();
            orch::store_for_goal(home, &g).unwrap()
        } else {
            orch::store_for(home, "demo").unwrap()
        };
        fs::create_dir_all(store.join("inbox")).unwrap();
        fs::write(
            store.join("modules.md"),
            "## m\nPURPOSE x\nPATHS src/mod.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n",
        )
        .unwrap();
        fs::write(
            store.join("one.brief"),
            "GOAL build\nRISK trivial\nSCOPE src/mod.py\nVERIFY echo verified > verify-marker\n",
        )
        .unwrap();
        fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\none\tm\tpending\t\t\t\t{}\n",
                store.join("one.brief").display()
            ),
        )
        .unwrap();
        if with_goal {
            orch::record_stage(&store, "astra", "gpt-6-astra", true).unwrap();
            fs::create_dir_all(store.join("astra")).unwrap();
            for name in ["plan.json", "review.json", "ruling.md", "plan-final.md"] {
                fs::write(store.join("astra").join(name), "ok").unwrap();
            }
        }
        let wrapper = home.join("logged_codex.py");
        fs::write(&wrapper, "import os,runpy,sys\nos.environ['FAKE_CODEX_LOG']=sys.argv[1]\nsys.argv=sys.argv[2:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n").unwrap();
        let log = home.join("codex.log");
        let bridge = Bridge::new(Config {
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
        });
        Self {
            temp,
            repo,
            store,
            disk,
            log,
            bridge,
            old_disk,
        }
    }
    fn space(&self, free: u64) {
        fs::write(&self.disk, json!({"free_bytes":free}).to_string()).unwrap();
    }
    fn cycle(&self) -> Value {
        call(
            &self.bridge,
            "codex_run",
            &json!({"project":"demo","unit":"one","role":"terra","cwd":self.repo,"cycle":true}),
        )["structuredContent"]
            .clone()
    }
    fn rows(&self) -> Vec<orch::Evidence> {
        orch::evidence(&self.store, "one").unwrap()
    }
    fn count(&self, kind: &str) -> usize {
        self.rows().iter().filter(|r| r.kind == kind).count()
    }
    fn calls(&self, method: &str) -> usize {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .filter(|line| serde_json::from_str::<Value>(line).is_ok_and(|v| v["method"] == method))
            .count()
    }
    fn verify_cli(&self) -> orch::Output {
        orch::commands::run_extra(
            &self.store,
            &[
                "verify".into(),
                "one".into(),
                "--repo".into(),
                self.repo.to_string_lossy().into_owned(),
            ],
            true,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(value) = &self.old_disk {
            std::env::set_var("AXIOM_FAKE_DISK", value);
        } else {
            std::env::remove_var("AXIOM_FAKE_DISK");
        }
    }
}

#[test]
fn bridge_pause_and_resume_reuses_worker_checkpoint() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(true);
    let first = f.cycle();
    assert_eq!(first["status"], "interrupted", "{first}");
    assert_eq!(first["error"]["kind"], "disk_space", "{first}");
    assert_eq!(first["next"], "wait");
    let checkpoint: Value =
        serde_json::from_slice(&fs::read(f.store.join("checkpoints/one.json")).unwrap()).unwrap();
    assert_eq!(checkpoint["stage"], "worker_done");
    assert_eq!(checkpoint["attempt_id"], first["attempt_id"]);
    assert_eq!(f.count("run"), 1);
    assert_eq!(f.count("verify"), 0);
    assert_eq!(f.count("fixes"), 0);
    assert_eq!(orch::read_units(&f.store).unwrap().len(), 1);
    assert!(!orch::scratch_dir(&f.store, &["one", first["attempt_id"].as_str().unwrap()]).exists());
    assert!(!f.repo.join("verify-marker").exists());
    let g = goal::load(f.temp.path(), "session").unwrap().unwrap();
    assert_eq!(g.status, "blocked");
    assert_eq!(g.reason.as_deref(), Some("disk_space"));
    let events = orch::events::read_events(&f.store, 0).unwrap();
    let pause = events
        .iter()
        .find(|e| e.kind == preflight::EVENT_DISK_PAUSE)
        .unwrap();
    assert_eq!(pause.data["line"], first["error"]["message"]);
    assert_eq!(g.note.as_deref(), pause.data["line"].as_str());
    assert_eq!(f.calls("thread/start"), 1);
    let repeated = f.cycle();
    assert_eq!(repeated["error"]["kind"], "disk_space", "{repeated}");
    assert_eq!(repeated["attempt_id"], first["attempt_id"]);
    assert_eq!(f.count("verify"), 0);
    assert_eq!(f.calls("thread/start"), 1);
    f.space(1000);
    let second = f.cycle();
    assert_eq!(second["attempt_id"], first["attempt_id"], "{second}");
    assert_eq!(f.count("run"), 1);
    assert_eq!(f.count("scope"), 1);
    assert_eq!(f.count("verify"), 1);
    assert_eq!(f.count("fixes"), 0);
    assert_eq!(f.calls("thread/start"), 1);
    assert_eq!(f.calls("initialize"), 1);
    assert!(f.repo.join("verify-marker").exists());
}

#[test]
fn orch_verify_pauses_then_succeeds() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(false);
    f.space(1);
    assert_eq!(f.cycle()["error"]["kind"], "disk_space");
    let before = f.rows();
    let error = orch::record_verify(&f.store, "one", &f.repo)
        .err()
        .expect("verify should pause");
    assert_eq!(error.kind, "disk_space");
    let result = f.verify_cli();
    assert_ne!(result.exit_code, 0);
    assert!(result.stderr.contains(&error.message));
    assert_eq!(f.rows().len(), before.len());
    assert_eq!(f.count("verify"), 0);
    assert_eq!(f.count("fixes"), 0);
    assert!(!f.repo.join("verify-marker").exists());
    f.space(1000);
    let result = f.verify_cli();
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    assert_eq!(f.count("verify"), 1);
    assert!(f.repo.join("verify-marker").exists());
}

#[test]
fn ample_space_runs_verify_and_checks_both_paths() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(false);
    fs::write(&f.disk, json!({"free_bytes":1000,"paths":{f.temp.path().to_string_lossy().to_string(): 1, f.repo.to_string_lossy().to_string():1000}}).to_string()).unwrap();
    let first = f.cycle();
    assert_eq!(first["error"]["kind"], "disk_space", "{first}");
    assert_eq!(f.count("verify"), 0);
    let pause = orch::events::read_events(&f.store, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == preflight::EVENT_DISK_PAUSE)
        .unwrap();
    assert_eq!(pause.data["check"]["path"], json!(f.temp.path()));
    f.space(1000);
    let second = f.cycle();
    assert_eq!(second["state"], "built", "{second}");
    assert_eq!(f.count("verify"), 1);
    assert_eq!(f.calls("thread/start"), 1);
}

#[test]
fn ample_space_from_start_runs_verify() {
    let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new(false);
    f.space(1000);
    let result = f.cycle();
    assert_eq!(result["state"], "built", "{result}");
    assert_eq!(f.count("run"), 1);
    assert_eq!(f.count("verify"), 1);
    assert!(f.repo.join("verify-marker").exists());
    assert!(!orch::events::read_events(&f.store, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == preflight::EVENT_DISK_PAUSE));
}
