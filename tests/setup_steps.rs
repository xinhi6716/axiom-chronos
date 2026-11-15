mod common;
use axiom::{
    base::proc,
    setup::{self, StepStatus},
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let repo = temp.path().join("repo");
        let store = temp.path().join("store");
        fs::create_dir_all(repo.join(".axiom")).unwrap();
        assert!(proc::cmd("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        Self {
            _temp: temp,
            repo,
            store,
        }
    }
    fn manifest(&self, steps: Vec<Value>) {
        fs::write(
            self.repo.join(setup::MANIFEST),
            serde_json::to_vec(&json!({"version":1,"steps":steps})).unwrap(),
        )
        .unwrap();
    }
    fn script(&self, name: &str, source: &str) {
        fs::write(self.repo.join(name), source).unwrap();
    }
    fn step(&self, id: &str, script: &str, timeout: u64) -> Value {
        json!({"id":id,"argv":[common::python(), self.repo.join(script).to_string_lossy()],"timeout_s":timeout,"plan_independent":true})
    }
    fn no_index() -> Value {
        json!({"id":"index","argv":["unused"],"plan_independent":false})
    }
    fn status(&self, id: &str) -> StepStatus {
        setup::status(&self.store)
            .unwrap()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
    }
    fn wait(&self, id: &str, state: &str) -> StepStatus {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let s = self.status(id);
            if s.state == state {
                return s;
            }
            assert!(Instant::now() < until, "{id}: expected {state}, got {s:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn ready(&self, id: &str) -> u32 {
        let path = self.store.join("scratch/setup").join(id).join("child.pid");
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(s) = fs::read_to_string(&path) {
                if let Ok(pid) = s.parse() {
                    return pid;
                }
            }
            assert!(Instant::now() < until, "{id} not ready");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
const TREE: &str = r#"import os, pathlib, subprocess, sys, time
out = pathlib.Path(os.environ['AXIOM_SETUP_OUT'])
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
(out / 'child.pid').write_text(str(child.pid))
(out / 'output.txt').write_text('scratch')
time.sleep(60)
"#;
fn dead(pid: u32) {
    assert!(
        proc::wait_pid_exit(pid, Duration::from_secs(10)),
        "pid {pid} still alive"
    );
}
fn marker(store: &Path, id: &str) -> PathBuf {
    store.join("setup").join(format!("{id}.ok"))
}

fn evidence_snapshot(store: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let mut snapshot = BTreeMap::new();
    let reports = store.join("reports");
    match fs::read_dir(&reports) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(".evidence.tsv") {
                    return Err(format!(
                        "unexpected evidence file: {}",
                        entry.path().display()
                    ));
                }
                let path = entry.path();
                snapshot.insert(
                    PathBuf::from("reports").join(name),
                    fs::read(&path).map_err(|e| e.to_string())?,
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    for name in ["units.tsv", "ledger.tsv"] {
        match fs::read(store.join(name)) {
            Ok(contents) => {
                snapshot.insert(PathBuf::from(name), contents);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(snapshot)
}

#[test]
fn evidence_snapshot_detects_evidence_file() {
    let temp = common::temp();
    fs::create_dir_all(temp.path().join("reports")).unwrap();
    assert!(evidence_snapshot(temp.path()).is_ok());
    fs::write(temp.path().join("reports/x.evidence.tsv"), "row\n").unwrap();
    assert!(evidence_snapshot(temp.path()).is_err());
}

#[test]
fn manifests_validate_and_replace_builtins() {
    let f = Fixture::new();
    let (steps, warnings) = setup::load_steps(&f.repo).unwrap();
    assert_eq!(
        steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["index"]
    );
    assert!(warnings.is_empty());
    fs::write(f.repo.join("Cargo.toml"), "").unwrap();
    assert_eq!(
        setup::builtin_steps(&f.repo)
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["index", "cargo-fetch"]
    );
    let absolute = f.repo.to_string_lossy().to_string();
    f.manifest(vec![
        json!({"id":"Bad!","argv":["a"]}),
        json!({"id":"empty","argv":[]}),
        json!({"id":"absolute","argv":["a"],"cwd":absolute}),
        json!({"id":"up","argv":["a"],"cwd":"../x"}),
        json!({"id":"zero","argv":["a"],"timeout_s":0}),
        json!({"id":"cache","argv":["a"],"cache_dirs":["../cache"]}),
        json!({"id":"valid","argv":["a"]}),
        json!({"id":"valid","argv":["b"]}),
        json!({"id":"index","argv":["replacement"],"plan_independent":true}),
    ]);
    let (steps, warnings) = setup::load_steps(&f.repo).unwrap();
    assert_eq!(warnings.len(), 7, "{warnings:?}");
    assert_eq!(
        steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["index", "cargo-fetch", "valid"]
    );
    assert_eq!(steps[0].argv, ["replacement"]);
    assert!(!steps[0].builtin);
    assert_eq!(steps[2].cwd, ".");
    assert_eq!(steps[2].timeout_s, 600);
    assert!(!steps[2].plan_independent);
    fs::write(f.repo.join(setup::MANIFEST), "not json").unwrap();
    assert_eq!(
        setup::load_steps(&f.repo).unwrap_err().kind,
        "invalid_argument"
    );
    fs::write(f.repo.join(setup::MANIFEST), r#"{"version":2,"steps":[]}"#).unwrap();
    assert_eq!(
        setup::load_steps(&f.repo).unwrap_err().kind,
        "invalid_argument"
    );
}

#[test]
fn start_skips_and_never_restarts_and_succeeds() {
    let f = Fixture::new();
    f.script("success.py", "import os,pathlib,time\np=pathlib.Path(os.environ['AXIOM_SETUP_OUT'])\n(p/'started').write_text('yes')\ntime.sleep(1.5)\n(p/'finished').write_text('yes')\n");
    f.script("skip.py", "raise SystemExit('must not run')\n");
    f.manifest(vec![
        Fixture::no_index(),
        f.step("work", "success.py", 10),
        json!({"id":"skip","argv":[common::python(),f.repo.join("skip.py").to_string_lossy()]}),
    ]);
    let before = Instant::now();
    let started = setup::start(&f.store, &f.repo).unwrap();
    assert!(before.elapsed() < Duration::from_secs(1));
    let work = started.iter().find(|s| s.id == "work").unwrap();
    assert_eq!(work.state, "running");
    let pid = work.pid;
    let skipped = started.iter().find(|s| s.id == "skip").unwrap();
    assert_eq!(
        (&*skipped.state, skipped.reason.as_deref()),
        ("skipped", Some("not plan_independent"))
    );
    assert!(!skipped.scratch.exists());
    let again = setup::start(&f.store, &f.repo).unwrap();
    assert_eq!(again.iter().find(|s| s.id == "work").unwrap().pid, pid);
    let done = f.wait("work", "succeeded");
    assert_eq!(done.exit_code, Some(0));
    assert!(marker(&f.store, "work").exists());
    assert!(done.scratch.join("started").exists());
    assert!(done.scratch.join("finished").exists());
    assert_eq!(
        setup::start(&f.store, &f.repo)
            .unwrap()
            .iter()
            .find(|s| s.id == "work")
            .unwrap()
            .pid,
        pid
    );
}

#[test]
fn timeout_kills_tree_without_marker() {
    let f = Fixture::new();
    f.script("tree.py", TREE);
    f.manifest(vec![Fixture::no_index(), f.step("time", "tree.py", 1)]);
    setup::start(&f.store, &f.repo).unwrap();
    let descendant = f.ready("time");
    let state = f.wait("time", "timeout");
    dead(state.pid.unwrap());
    dead(descendant);
    assert!(!marker(&f.store, "time").exists());
}

fn porcelain(repo: &Path) -> Vec<u8> {
    let o = proc::cmd("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(o.status.success());
    o.stdout
}
#[test]
fn cancel_from_thread_preserves_other_step_repo_and_cache() {
    let f = Fixture::new();
    f.script("tree.py", TREE);
    f.script("other.py", "import os,pathlib,time\np=pathlib.Path(os.environ['AXIOM_SETUP_OUT'])\n(p/'ready').write_text('yes')\ntime.sleep(3)\n");
    fs::create_dir_all(f.repo.join("cache")).unwrap();
    fs::write(f.repo.join("cache/keep"), "retain").unwrap();
    let mut tree = f.step("kill", "tree.py", 30);
    tree["cache_dirs"] = json!(["cache"]);
    f.manifest(vec![
        Fixture::no_index(),
        tree,
        f.step("other", "other.py", 30),
    ]);
    let before = porcelain(&f.repo);
    fs::create_dir_all(f.store.join("reports")).unwrap();
    fs::write(f.store.join("reports/existing.txt"), "existing report").unwrap();
    fs::write(f.store.join("units.tsv"), "existing units\n").unwrap();
    fs::write(f.store.join("ledger.tsv"), "existing ledger\n").unwrap();
    let evidence_before = evidence_snapshot(&f.store).unwrap();
    setup::start(&f.store, &f.repo).unwrap();
    assert_eq!(evidence_snapshot(&f.store).unwrap(), evidence_before);
    let descendant = f.ready("kill");
    let parent = f.status("kill").pid.unwrap();
    let other = f.status("other").pid;
    let store = f.store.clone();
    thread::spawn(move || {
        setup::cancel(&store, Some(&["kill".into(), "unknown".into()]), "manual").unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(evidence_snapshot(&f.store).unwrap(), evidence_before);
    dead(parent);
    dead(descendant);
    let killed = f.status("kill");
    assert_eq!(killed.state, "cancelled");
    assert_eq!(killed.reason.as_deref(), Some("manual"));
    assert!(!killed.scratch.exists());
    assert!(!marker(&f.store, "kill").exists());
    assert_eq!(f.status("other").pid, other);
    assert_eq!(f.status("other").state, "running");
    assert_eq!(
        fs::read_to_string(f.repo.join("cache/keep")).unwrap(),
        "retain"
    );
    assert_eq!(porcelain(&f.repo), before);
    f.wait("other", "succeeded");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(f.status("kill").state, "cancelled");
    assert_eq!(evidence_snapshot(&f.store).unwrap(), evidence_before);
}

#[test]
fn cancel_running_state_with_dead_pid() {
    let f = Fixture::new();
    let pid = if cfg!(unix) { 2_000_000_000 } else { 0 };
    assert!(!proc::pid_alive(pid));
    let scratch = f.store.join("scratch/setup/lost");
    fs::create_dir_all(&scratch).unwrap();
    fs::create_dir_all(f.store.join("setup")).unwrap();
    fs::write(marker(&f.store, "lost"), "").unwrap();
    let state = StepStatus {
        id: "lost".into(),
        state: "running".into(),
        pid: Some(pid),
        started_at: Some("started".into()),
        ended_at: None,
        exit_code: None,
        scratch: scratch.clone(),
        reason: None,
    };
    fs::write(
        f.store.join("setup/lost.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let states = setup::cancel(&f.store, None, "manual").unwrap();
    let cancelled = states.iter().find(|state| state.id == "lost").unwrap();
    assert_eq!(cancelled.state, "cancelled");
    assert_eq!(cancelled.reason.as_deref(), Some("manual"));
    assert!(cancelled.ended_at.is_some());
    assert!(!scratch.exists());
    assert!(!marker(&f.store, "lost").exists());
}

#[test]
fn keep_then_cancel_all() {
    let f = Fixture::new();
    f.script("tree.py", TREE);
    f.manifest(vec![
        Fixture::no_index(),
        f.step("one", "tree.py", 30),
        f.step("two", "tree.py", 30),
        f.step("three", "tree.py", 30),
    ]);
    setup::start(&f.store, &f.repo).unwrap();
    let one_child = f.ready("one");
    let two_child = f.ready("two");
    let three_child = f.ready("three");
    setup::keep(&f.store, &["one".into()]).unwrap();
    assert_eq!(f.status("one").state, "running");
    assert_eq!(f.status("two").reason.as_deref(), Some("left out by plan"));
    assert_eq!(f.status("three").state, "cancelled");
    dead(two_child);
    dead(three_child);
    setup::cancel_all(&f.store, "shutdown").unwrap();
    dead(one_child);
    assert_eq!(f.status("one").reason.as_deref(), Some("shutdown"));
    assert_eq!(f.status("two").reason.as_deref(), Some("left out by plan"));
}

#[test]
fn cancel_in_another_process() {
    let f = Fixture::new();
    f.script("tree.py", TREE);
    f.manifest(vec![Fixture::no_index(), f.step("remote", "tree.py", 30)]);
    setup::start(&f.store, &f.repo).unwrap();
    let child = f.ready("remote");
    let result = proc::cmd(std::env::current_exe().unwrap())
        .args(["--exact", "subprocess_cancel_helper", "--nocapture"])
        .env("SETUP_TEST_STORE", &f.store)
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(result.success());
    dead(child);
    thread::sleep(Duration::from_millis(100));
    let state = f.status("remote");
    assert_eq!(state.state, "cancelled");
    assert_eq!(state.reason.as_deref(), Some("remote"));
    assert!(!state.scratch.exists());
}
#[test]
fn subprocess_cancel_helper() {
    if let Some(store) = std::env::var_os("SETUP_TEST_STORE") {
        setup::cancel_all(Path::new(&store), "remote").unwrap();
    }
}

#[test]
fn index_collects_tracked_files_and_lost_supervisor_is_reported() {
    let f = Fixture::new();
    fs::write(f.repo.join("tracked.txt"), "contents").unwrap();
    assert!(proc::cmd("git")
        .args(["add", "tracked.txt"])
        .current_dir(&f.repo)
        .status()
        .unwrap()
        .success());
    setup::start(&f.store, &f.repo).unwrap();
    let indexed = f.wait("index", "succeeded");
    assert_eq!(
        fs::read(indexed.scratch.join("files.txt")).unwrap(),
        b"tracked.txt\0"
    );
    assert!(marker(&f.store, "index").exists());
    let mut lost = indexed;
    lost.id = "lost".into();
    lost.state = "running".into();
    lost.pid = Some(0);
    lost.ended_at = None;
    lost.exit_code = None;
    // On Windows pid 0 is not an accessible user process.
    // On Unix use a PID beyond the operating system's supported range.
    if cfg!(unix) {
        lost.pid = Some(2_000_000_000);
    }
    fs::write(
        f.store.join("setup/lost.json"),
        serde_json::to_vec(&lost).unwrap(),
    )
    .unwrap();
    let lost = f.status("lost");
    assert_eq!(lost.state, "failed");
    assert_eq!(lost.reason.as_deref(), Some("supervisor lost"));
}

#[test]
fn setup_refuses_store_inside_repo_and_reports_launch_failure() {
    let f = Fixture::new();
    let error = setup::start(&f.repo.join("store"), &f.repo).unwrap_err();
    assert_eq!(error.kind, "invalid_argument");
    assert!(!f.repo.join("store").exists());
    f.manifest(vec![Fixture::no_index(), json!({"id":"missing", "argv":[f.repo.join("no-such-executable").to_string_lossy()], "plan_independent":true})]);
    setup::start(&f.store, &f.repo).unwrap();
    let state = f.status("missing");
    assert_eq!(state.state, "failed");
    assert!(state.reason.is_some());
    assert!(!marker(&f.store, "missing").exists());
}
