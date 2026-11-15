mod common;

use axiom::{
    bridge::{call, Bridge, Config},
    orch::{
        self, events,
        receipts::{self, Receipts},
    },
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, Once},
    thread,
    time::{Duration, Instant},
};

static SERIAL: Mutex<()> = Mutex::new(());
static INIT: Once = Once::new();
const MODULES: [(&str, usize, bool, &str); 5] = [
    ("A", 2, true, "none"),
    ("B", 1, false, "none"),
    ("C", 1, true, "B"),
    ("D", 2, false, "B"),
    ("E", 2, true, "D"),
];

fn unit(module: &str, n: usize) -> String {
    format!("{}{n}", module.to_ascii_lowercase())
}
fn file(unit: &str) -> String {
    format!("{unit}.txt")
}
fn command(mode: &str, files: &[String]) -> String {
    let script = common::repo_root().join("tests/fixtures/wall_clock/check.py");
    format!(
        "\"{}\" \"{}\" {} {}",
        common::python(),
        script.display(),
        mode,
        files.join(" ")
    )
}
fn orch_call(bridge: &Bridge, args: Value) -> Value {
    let body =
        call(bridge, "orch", &json!({"project":"demo", "args":args}))["structuredContent"].clone();
    assert_eq!(body["status"], "completed", "{body}");
    body
}
struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
    bridge: Arc<Bridge>,
    levels: Vec<Vec<String>>,
}
impl Fixture {
    fn new(slow: u64, fast: u64) -> Self {
        let temp = common::temp();
        let home = temp.path();
        let repo = home.join("repo");
        let store = home.join("orchestrate/demo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(store.join("inbox")).unwrap();
        let accept_script = store.join("accept.py");
        fs::write(&accept_script, format!(
            "import pathlib, runpy, sys\nwith pathlib.Path(__file__).with_name(sys.argv[2] + '.accept-count').open('a') as count:\n    count.write('accept\\n')\nrunpy.run_path({:?}, run_name='__main__')\n",
            common::repo_root().join("tests/fixtures/wall_clock/check.py").to_string_lossy()
        )).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
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
            store.join("units.tsv"),
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n",
        )
        .unwrap();
        let mut map = String::new();
        for (module, count, is_slow, depends) in MODULES {
            let files: Vec<_> = (1..=count).map(|n| file(&unit(module, n))).collect();
            let accept = format!("\"{}\" \"{}\" accept {}", common::python(), accept_script.display(), files.join(" "));
            map.push_str(&format!("## {module}\nPURPOSE fixture\nPATHS {}\nDEPENDS {depends}\nCONTRACT fixture\nACCEPT test `{accept}`\n", files.join(", ")));
            for n in 1..=count {
                let id = unit(module, n);
                let path = file(&id);
                fs::write(store.join(format!("{id}.brief")), format!("GOAL build\nSCOPE {path}\nVERIFY {}\nRISK trivial\nFAKE_DELAY_MS {}\nFAKE_WRITE {path}\n", command("verify", &[path.clone()]), if is_slow { slow } else { fast })).unwrap();
            }
        }
        fs::write(store.join("modules.md"), &map).unwrap();
        let parsed = orch::parse_map(&map).unwrap();
        let mut remaining = parsed.modules;
        let mut complete = HashSet::new();
        let mut levels = Vec::new();
        while !remaining.is_empty() {
            let level: Vec<_> = remaining
                .iter()
                .filter(|m| m.depends.iter().all(|d| complete.contains(d)))
                .map(|m| m.id.clone())
                .collect();
            assert!(!level.is_empty(), "cyclic map");
            remaining.retain(|m| !level.contains(&m.id));
            complete.extend(level.iter().cloned());
            levels.push(level);
        }
        let bridge = Arc::new(Bridge::new(Config {
            data_home: Some(home.into()),
            codex: Ok(common::fake_codex(&[])),
            wsl: None,
            on_windows: false,
        }));
        Self {
            _temp: temp,
            repo,
            store,
            bridge,
            levels,
        }
    }
    fn dispatch(&self, module: String) -> thread::JoinHandle<()> {
        let bridge = Arc::clone(&self.bridge);
        let store = self.store.clone();
        let repo = self.repo.clone();
        thread::spawn(move || {
            let count = MODULES.iter().find(|m| m.0 == module).unwrap().1;
            let ids: Vec<_> = (1..=count).map(|n| unit(&module, n)).collect();
            for id in &ids {
                let brief = store.join(format!("{id}.brief"));
                orch_call(
                    &bridge,
                    json!(["unit", "add", id, "--track", module, "--brief", brief]),
                );
            }
            let workers: Vec<_> = ids.into_iter().map(|id| {
                let bridge = Arc::clone(&bridge);
                let store = store.clone();
                let repo = repo.clone();
                thread::spawn(move || {
                    let body = call(&bridge, "codex_run", &json!({"project":"demo", "unit":id, "role":"terra", "cwd":repo, "cycle":true}))["structuredContent"].clone();
                    assert_eq!(body["status"], "completed", "{id}: {body}");
                    let deadline = Instant::now() + Duration::from_secs(60);
                    let verify = loop {
                        let evidence = orch::evidence(&store, &id).unwrap();
                        if evidence.iter().any(|e| e.kind == "accept" && e.exit_code == 0) {
                            break evidence.into_iter().rev().find(|e| e.kind == "verify").unwrap();
                        }
                        assert!(Instant::now() < deadline, "{id}: no acceptance: {body}");
                        thread::sleep(Duration::from_millis(20));
                    };
                    let verdict = store.join(format!("{id}.verdict.json"));
                    fs::write(&verdict, json!({"unit":id,"tree":verify.commit,"verdict":"pass","findings":[]}).to_string()).unwrap();
                    orch_call(&bridge, json!(["review", id, "--repo", repo, "--reviewer", "chronos:reviewer", "--verdict-file", verdict]));
                })
            }).collect();
            for worker in workers {
                worker.join().unwrap();
            }
            assert_eq!(fs::read_to_string(store.join(format!("{}.accept-count", file(&unit(&module, 1))))).unwrap().lines().collect::<Vec<_>>(), vec!["accept"], "{module}: ACCEPT must run exactly once");
            // record_accept fans one module run out to every unit: each must
            // have exactly one row, including the unit whose reply skipped it.
            for n in 1..=count {
                let id = unit(&module, n);
                assert_eq!(orch::evidence(&store, &id).unwrap().iter()
                    .filter(|e| e.kind == "accept").count(), 1, "{id}");
            }
            assert!(
                orch::lifecycle::scheduler::module_verified(&store, &module).unwrap(),
                "{module}"
            );
        })
    }
    fn wave(&self) {
        for level in &self.levels {
            let mut workers = Vec::new();
            for module in level {
                orch_call(&self.bridge, json!(["schedule", "claim", module]));
                workers.push(self.dispatch(module.clone()));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        }
    }
    fn schedule(&self) {
        let mut cursor = 0;
        let mut started = HashSet::new();
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let body = orch_call(
                &self.bridge,
                json!([
                    "--json",
                    "schedule",
                    "next",
                    "--cursor",
                    cursor.to_string(),
                    "--timeout",
                    "1"
                ]),
            );
            let result: Value = serde_json::from_str(body["stdout"].as_str().unwrap()).unwrap();
            cursor = result["cursor"].as_u64().unwrap();
            for module in result["start"].as_array().unwrap() {
                let module = module.as_str().unwrap().to_string();
                assert!(
                    started.insert(module.clone()),
                    "duplicate dispatch: {module}"
                );
                workers.push(self.dispatch(module));
            }
            let mut i = 0;
            while i < workers.len() {
                if workers[i].is_finished() {
                    workers.swap_remove(i).join().unwrap();
                } else {
                    i += 1;
                }
            }
            if result["done"] == true {
                break;
            }
            assert!(Instant::now() < deadline, "scheduler stalled: {result}");
        }
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(started.len(), MODULES.len());
    }
    fn finish(&self) -> Receipts {
        let files: Vec<_> = MODULES
            .iter()
            .flat_map(|(m, count, _, _)| (1..=*count).map(move |n| file(&unit(m, n))))
            .collect();
        let full = orch_call(
            &self.bridge,
            json!([
                "fulltest",
                "--repo",
                self.repo,
                "--command",
                command("full", &files)
            ]),
        );
        assert!(
            full["stdout"].as_str().unwrap().contains("exit=0"),
            "{full}"
        );
        let report = orch_call(&self.bridge, json!(["report", "--repo", self.repo]));
        let digest = report["stdout"].as_str().unwrap();
        assert!(digest.contains("units: 8 (verified 8)"), "{digest}");
        assert!(digest.contains("full test: pass"), "{digest}");
        for (m, _, _, _) in MODULES {
            assert!(digest.contains(&format!("  {m}  pass")), "{digest}");
        }
        let result = receipts::compute(&self.store, 8).unwrap();
        assert_eq!(result.verified_units, 8);
        assert_eq!(result.runs, 8);
        assert_eq!(result.units.len(), 8);
        assert!(result.concurrency.peak <= 8);
        result
    }
}
fn setup() -> std::sync::MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    INIT.call_once(|| std::env::set_var("AXIOM_CODEX_MAX_PARALLEL", "8"));
    guard
}
fn check_barrier(f: &Fixture, receipt: &Receipts) {
    let events = events::read_events(&f.store, 0).unwrap();
    let units = orch::read_units(&f.store).unwrap();
    let tracks: HashMap<_, _> = units
        .iter()
        .map(|u| (u.id.as_str(), u.track.as_str()))
        .collect();
    for pair in f.levels.windows(2) {
        let mut verified_max = 0;
        let mut verified_seq = 0;
        for module in &pair[0] {
            let event = events
                .iter()
                .find(|e| e.kind == events::MODULE_VERIFIED && e.module.as_deref() == Some(module))
                .expect("module verified event");
            verified_seq = verified_seq.max(event.seq);
            verified_max = verified_max.max(event.ts_ms);
            for clock in receipt
                .units
                .iter()
                .filter(|u| tracks[u.unit.as_str()] == module)
            {
                verified_max = verified_max.max(clock.verified_ms);
            }
        }
        for module in &pair[1] {
            let claim = events
                .iter()
                .find(|e| e.kind == events::MODULE_CLAIMED && e.module.as_deref() == Some(module))
                .unwrap();
            assert!(
                claim.seq > verified_seq && claim.ts_ms >= verified_max,
                "{module} claimed before previous wave verified"
            );
            let first = receipt
                .units
                .iter()
                .filter(|u| tracks[u.unit.as_str()] == module)
                .map(|u| u.first_ms)
                .min()
                .unwrap();
            assert!(
                first >= verified_max,
                "{module} started at {first} before prior wave verified at {verified_max}"
            );
        }
    }
}
fn save_events(store: &Path, label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("axiom-wallclock-all");
    fs::create_dir_all(&dir).unwrap();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = dir.join(format!(
        "{label}-{}-{unique}.events.jsonl",
        std::process::id()
    ));
    fs::copy(store.join("events.jsonl"), &path).unwrap();
    path
}
fn compare(slow: u64, fast: u64, acceptance: bool) {
    let wave = Fixture::new(slow, fast);
    wave.wave();
    let before = wave.finish();
    let wave_events = save_events(&wave.store, if acceptance { "wave" } else { "dev-wave" });
    let scheduler = Fixture::new(slow, fast);
    scheduler.schedule();
    let after = scheduler.finish();
    let scheduler_events = save_events(
        &scheduler.store,
        if acceptance {
            "scheduler"
        } else {
            "dev-scheduler"
        },
    );
    for (label, r, path) in [
        ("WAVE (before)", &before, wave_events),
        ("SCHEDULER (after)", &after, scheduler_events),
    ] {
        println!(
            "{label}: T/N={:.2} ms, mean concurrency={:.3}, peak={}, cap={}, events={}",
            r.per_unit_ms.unwrap(),
            r.concurrency.mean,
            r.concurrency.peak,
            r.concurrency.cap,
            path.display()
        );
    }
    let ratio = before.per_unit_ms.unwrap() / after.per_unit_ms.unwrap();
    if acceptance {
        assert!(ratio >= 2.0, "T/N ratio {ratio}");
    } else {
        assert!(ratio > 1.0, "T/N ratio {ratio}");
    }
    assert!(
        after.concurrency.mean > before.concurrency.mean,
        "wave {:?}, scheduler {:?}",
        before.concurrency,
        after.concurrency
    );
}
#[test]
fn wave_barrier() {
    let _guard = setup();
    let f = Fixture::new(900, 60);
    f.wave();
    let receipt = f.finish();
    check_barrier(&f, &receipt);
}
#[test]
fn scheduler_wall_clock_acceptance() {
    let _guard = setup();
    // Keep simulated work dominant over durable resume/self-check bookkeeping.
    compare(24000, 200, true);
}
#[test]
#[ignore]
fn scheduler_wall_clock_development() {
    let _guard = setup();
    compare(900, 60, false);
}

#[test]
fn acceptance_retakes_finished_peer_but_not_running_owner() {
    let _guard = setup();
    let f = Fixture::new(0, 0);
    let mut attempts = Vec::new();
    for id in ["a1", "a2"] {
        orch_call(&f.bridge, json!(["unit", "add", id, "--track", "A",
            "--brief", f.store.join(format!("{id}.brief"))]));
        let orch::Begin::Run { attempt_id, .. } =
            orch::begin_unit(&f.store, id, "worker", &f.repo, false).unwrap()
        else { panic!("expected new attempt"); };
        attempts.push(attempt_id);
        fs::write(f.repo.join(file(id)), "built").unwrap();
        for kind in ["run", "scope", "verify"] {
            orch::append_evidence(&f.store, id, &orch::Evidence {
                kind: kind.into(), command: "fixture".into(), exit_code: 0,
                commit: String::new(), ts: axiom::base::now_iso(),
            }).unwrap();
        }
    }
    // Both have VERIFY evidence, but a1's live checkpoint still owns work.
    assert!(matches!(orch::accept_if_ready(&f.store, "a2", &f.repo).unwrap(),
        orch::AutoAccept::Skipped { reason, detail } if reason == "claimed" && detail == "a1"));
    assert!(orch::evidence(&f.store, "a2").unwrap().iter().all(|e| e.kind != "accept"));

    let path = f.store.join("checkpoints/a1.json");
    let mut cp: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    cp["stage"] = json!("selfchecked");
    cp["finished"] = serde_json::to_value(orch::Finished {
        state: "built".into(), verify_exit: Some(0), verify_tail: String::new(),
        verify_output: None, outside_scope: vec![], ignored: vec![],
        unattributed: vec![], changed: vec![], pack: None, pack_error: None,
    }).unwrap();
    fs::write(path, cp.to_string()).unwrap();
    assert!(matches!(orch::accept_if_ready(&f.store, "a2", &f.repo).unwrap(),
        orch::AutoAccept::Ran { exit_code: 0, .. }));
    assert!(matches!(orch::accept_if_ready(&f.store, "a2", &f.repo).unwrap(),
        orch::AutoAccept::Skipped { reason, .. } if reason == "already_accepted"));
    for (id, attempt) in ["a1", "a2"].into_iter().zip(attempts) {
        assert_eq!(orch::evidence(&f.store, id).unwrap().iter()
            .filter(|e| e.kind == "accept").count(), 1, "{id}");
        orch::release_claim(&f.store, id, &attempt).unwrap();
    }
}
