mod common;
use axiom::{
    base::proc,
    seams::FakeApproval,
    workshop::{
        api::Api,
        host::{AckHandler, Host, HostConfig, Status},
        services::{FakeBackend, FakeRouter, FakeSlots, NoNetwork, Services},
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, Instant},
};

fn setup() -> tempfile::TempDir {
    let tmp = common::temp();
    copy_module(tmp.path(), "hello");
    tmp
}
fn copy_module(home: &Path, name: &str) {
    let source = common::repo_root().join("examples/workshop/hello");
    let dest = home.join("workshop").join(name);
    fs::create_dir_all(&dest).unwrap();
    fs::copy(source.join("main.py"), dest.join("main.py")).unwrap();
    let mut manifest: Value =
        serde_json::from_slice(&fs::read(source.join("manifest.json")).unwrap()).unwrap();
    manifest["name"] = json!(name);
    fs::write(
        dest.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}
fn host(home: &Path) -> Host {
    Host::new(
        HostConfig {
            home: home.to_owned(),
            python: Some(common::python()),
        },
        Arc::new(AckHandler),
        Arc::new(FakeApproval::approving()),
    )
}
fn files(home: &Path, name: &str) -> PathBuf {
    home.join("workshop-data").join(name).join("files")
}
fn probes(home: &Path, value: Value) {
    let path = files(home, "hello");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("probes.json"), value.to_string()).unwrap();
}
fn wait_until(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(20));
    }
}
fn done(home: &Path, name: &str) {
    wait_until(|| files(home, name).join("done").exists());
}
fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn start_runs_in_own_data_dir() {
    let tmp = setup();
    let home = tmp.path();
    std::env::set_var("AXIOM_TEST_SECRET", "secret");
    probes(home, json!([{"env":"AXIOM_TEST_SECRET"}]));
    let host = host(home);
    host.start("hello").unwrap();
    done(home, "hello");
    let started = json_file(&files(home, "hello").join("started.json"));
    let root = home.join("workshop-data/hello").canonicalize().unwrap();
    for path in [
        started["cwd"].as_str().unwrap(),
        started["env"]["HOME"].as_str().unwrap(),
    ] {
        assert!(Path::new(path).canonicalize().unwrap().starts_with(&root));
    }
    assert_eq!(started["env"]["AXIOM_WORKSHOP_MODULE"], "hello");
    assert!(started["env"]["AXIOM_TEST_SECRET"].is_null());
    let results = fs::read_to_string(files(home, "hello").join("results.jsonl")).unwrap();
    assert!(serde_json::from_str::<Value>(results.trim()).unwrap()["value"].is_null());
    std::env::remove_var("AXIOM_TEST_SECRET");
}
#[test]
fn request_reply_roundtrip() {
    let tmp = setup();
    probes(tmp.path(), json!([{"method":"test","params":{"x":1}}]));
    let host = host(tmp.path());
    host.start("hello").unwrap();
    done(tmp.path(), "hello");
    let line = fs::read_to_string(files(tmp.path(), "hello").join("results.jsonl")).unwrap();
    let result: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        result["request"],
        json!({"v":1,"id":1,"method":"test","params":{"x":1}})
    );
    assert_eq!(
        result["reply"],
        json!({"v":1,"id":1,"ok":true,"result":null})
    );
}
#[test]
fn default_probe_writes_log() {
    let tmp = setup();
    let home = tmp.path();
    let backend = Arc::new(FakeBackend::default());
    let host = Host::new(
        HostConfig {
            home: home.to_owned(),
            python: Some(common::python()),
        },
        Arc::new(Api::new(Services {
            engine_home: home.to_owned(),
            slots: Arc::new(FakeSlots::default()),
            claude: backend.clone(),
            codex: backend,
            router: Arc::new(FakeRouter::default()),
            network: Arc::new(NoNetwork),
        })),
        Arc::new(FakeApproval::approving()),
    );
    host.start("hello").unwrap();
    done(home, "hello");
    let results = fs::read_to_string(files(home, "hello").join("results.jsonl")).unwrap();
    let result: Value = serde_json::from_str(results.trim()).unwrap();
    assert_eq!(result["reply"]["ok"], true);
    let log = fs::read_to_string(home.join("workshop-data/hello/logs/module.log")).unwrap();
    assert!(log.lines().any(|line| {
        let entry: Value = serde_json::from_str(line).unwrap();
        entry["message"] == "hello" && entry["level"] == "info"
    }));
}
#[test]
fn stop_is_clean() {
    let tmp = setup();
    let host = host(tmp.path());
    let pid = host.start("hello").unwrap().pid.unwrap();
    done(tmp.path(), "hello");
    assert_eq!(host.stop("hello").unwrap().status, Status::Stopped);
    assert!(!proc::pid_alive(pid));
    assert_eq!(host.start("hello").unwrap().status, Status::Running);
}
#[test]
fn stop_kills_module_that_ignores_eof() {
    let tmp = setup();
    copy_module(tmp.path(), "stubborn");
    fs::write(
        tmp.path().join("workshop/stubborn/main.py"),
        "import pathlib, sys, time\npathlib.Path('ready').touch()\nsys.stdin.read()\npathlib.Path('eof').touch()\nwhile True:\n    time.sleep(1)\n",
    )
    .unwrap();
    let host = host(tmp.path());
    let pid = host.start("stubborn").unwrap().pid.unwrap();
    wait_until(|| files(tmp.path(), "stubborn").join("ready").exists());
    let started = Instant::now();
    let state = host.stop("stubborn").unwrap();
    let elapsed = started.elapsed();
    assert_eq!(state.status, Status::Stopped);
    assert!(
        elapsed >= Duration::from_secs(5),
        "stopped too early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(7),
        "stopped too late: {elapsed:?}"
    );
    assert!(files(tmp.path(), "stubborn").join("eof").exists());
    assert!(!proc::pid_alive(pid));
}
#[test]
fn drop_stops_started_modules() {
    let tmp = setup();
    copy_module(tmp.path(), "second");
    let host = host(tmp.path());
    let hello_pid = host.start("hello").unwrap().pid.unwrap();
    let second_pid = host.start("second").unwrap().pid.unwrap();
    done(tmp.path(), "hello");
    done(tmp.path(), "second");
    drop(host);
    for (name, pid) in [("hello", hello_pid), ("second", second_pid)] {
        assert!(!proc::pid_alive(pid));
        assert_eq!(
            json_file(
                &tmp.path()
                    .join("workshop-data")
                    .join(name)
                    .join("state.json")
            )["status"],
            "stopped"
        );
    }
}
#[test]
fn resolves_python_from_path() {
    use std::process::Stdio;
    if !["python3", "python"].into_iter().any(|python| {
        proc::command(&[python.into(), "--version".into()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }) {
        return;
    }
    let tmp = setup();
    let host = Host::new(
        HostConfig {
            home: tmp.path().to_owned(),
            python: None,
        },
        Arc::new(AckHandler),
        Arc::new(FakeApproval::approving()),
    );
    host.start("hello").unwrap();
    done(tmp.path(), "hello");
}

#[test]
fn busy_on_second_start() {
    let tmp = setup();
    let host = host(tmp.path());
    host.start("hello").unwrap();
    assert_eq!(host.start("hello").unwrap_err().kind, "busy");
}
#[test]
fn kill_is_persistent_and_isolated() {
    let tmp = setup();
    copy_module(tmp.path(), "second");
    let host = host(tmp.path());
    let pid = host.start("hello").unwrap().pid.unwrap();
    let other = host.start("second").unwrap().pid.unwrap();
    done(tmp.path(), "hello");
    done(tmp.path(), "second");
    let hello_ctx = host.ctx("hello").unwrap();
    let other_ctx = host.ctx("second").unwrap();
    assert_eq!(host.kill("hello").unwrap().status, Status::Killed);
    assert!(!proc::pid_alive(pid));
    assert!(proc::pid_alive(other));
    assert!(hello_ctx.cancel.load(Ordering::SeqCst));
    assert!(!other_ctx.cancel.load(Ordering::SeqCst));
    assert_eq!(
        json_file(&tmp.path().join("workshop-data/hello/state.json"))["killed"],
        true
    );
    assert_eq!(host.start("hello").unwrap_err().kind, "killed");
    assert_eq!(host.revive("hello").unwrap().status, Status::Stopped);
    assert_eq!(host.start("hello").unwrap().status, Status::Running);
    assert!(!host.ctx("hello").unwrap().cancel.load(Ordering::SeqCst));
}
#[test]
fn cross_process_kill() {
    let tmp = setup();
    let a = host(tmp.path());
    let b = host(tmp.path());
    let pid = a.start("hello").unwrap().pid.unwrap();
    done(tmp.path(), "hello");
    let ctx = a.ctx("hello").unwrap();
    b.kill("hello").unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while !ctx.cancel.load(Ordering::SeqCst) && Instant::now() < until {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(ctx.cancel.load(Ordering::SeqCst));
    assert!(!proc::pid_alive(pid));
}
#[test]
fn list_reports_all() {
    let tmp = setup();
    copy_module(tmp.path(), "second");
    let host = host(tmp.path());
    host.start("hello").unwrap();
    let list = host.list().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (&list[0].name, list[0].status),
        (&"hello".to_string(), Status::Running)
    );
    assert_eq!(
        (&list[1].name, list[1].status),
        (&"second".to_string(), Status::Stopped)
    );
}
#[test]
fn repo_untouched() {
    let repo = common::repo_root().canonicalize().unwrap();
    let snapshot = || {
        let mut entries: Vec<_> = fs::read_dir(repo.join("examples/workshop/hello"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name(),
                    entry.metadata().unwrap().modified().unwrap(),
                )
            })
            .collect();
        entries.sort();
        entries
    };
    let before = snapshot();
    let tmp = setup();
    let host = host(tmp.path());
    host.start("hello").unwrap();
    done(tmp.path(), "hello");
    assert!(!files(tmp.path(), "hello")
        .canonicalize()
        .unwrap()
        .starts_with(&repo));
    host.kill("hello").unwrap();
    assert_eq!(snapshot(), before);
}

#[test]
fn approval_denied_spawns_nothing() {
    let tmp = setup();
    let path = tmp.path().join("workshop/hello/manifest.json");
    let mut manifest = json_file(&path);
    manifest["permissions"]
        .as_array_mut()
        .unwrap()
        .push(json!("subprocess"));
    fs::write(path, manifest.to_string()).unwrap();
    let host = Host::new(
        HostConfig {
            home: tmp.path().to_owned(),
            python: Some(common::python()),
        },
        Arc::new(AckHandler),
        Arc::new(FakeApproval::denying()),
    );
    assert_eq!(host.start("hello").unwrap_err().kind, "approval_denied");
    assert!(!files(tmp.path(), "hello").join("started.json").exists());
}

#[test]
fn natural_exit_updates_status_and_logs_stdout() {
    let tmp = setup();
    fs::write(
        tmp.path().join("workshop/hello/main.py"),
        "import sys\nprint('plain output', flush=True)\nsys.exit(7)\n",
    )
    .unwrap();
    let host = host(tmp.path());
    host.start("hello").unwrap();
    wait_until(|| host.status("hello").unwrap().status == Status::Exited);
    assert_eq!(host.status("hello").unwrap().exit_code, Some(7));
    let log = tmp.path().join("workshop-data/hello/logs/stdout.log");
    wait_until(|| fs::read_to_string(&log).is_ok_and(|s| s.contains("plain output")));
}
