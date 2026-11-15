mod common;

use axiom::{
    goal, orch,
    service::{client::Client, protocol::StartRequest},
    storage::{
        ledger,
        retention::{Action, AuditRecord},
        Class, Ledger, Owner,
    },
};
use serde_json::json;
use std::{
    fs,
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for fixture");
        thread::sleep(Duration::from_millis(30));
    }
}

fn output(command: &mut Command) -> Output {
    let mut child = ChildGuard(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    wait_until(|| child.0.try_wait().unwrap().is_some());
    // Read the bounded CLI output after the process has exited.
    let stdout = std::io::read_to_string(child.0.stdout.take().unwrap()).unwrap();
    let stderr = std::io::read_to_string(child.0.stderr.take().unwrap()).unwrap();
    Output {
        status: child.0.wait().unwrap(),
        stdout: stdout.into_bytes(),
        stderr: stderr.into_bytes(),
    }
}

fn assert_success(out: &Output) {
    assert!(
        out.status.success(),
        "status: {}; stdout: {}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn preflight_pauses_fake_unpack_on_small_disk() {
    let temp = common::temp();
    let home = temp.path().join("home");
    let disk = temp.path().join("disk.json");
    fs::write(&disk, json!({"free_bytes": 1_u64 << 30}).to_string()).unwrap();
    let initial = goal::open(
        &home,
        SESSION,
        "unpack",
        Some(temp.path().to_str().unwrap()),
    )
    .unwrap();
    let store = orch::store_for_goal(&home, &initial).unwrap();
    let marker = temp.path().join("writer-ran");
    let writer = temp.path().join("writer.py");
    fs::write(
        &writer,
        "import pathlib, sys\npathlib.Path(sys.argv[1]).write_text('ran')\n",
    )
    .unwrap();
    let out = output(
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args([
                "launch",
                "--kind",
                "unpack",
                "--need",
                "8G",
                "--session",
                SESSION,
                "--",
            ])
            .arg(common::python())
            .arg("-I")
            .arg(&writer)
            .arg(&marker)
            .current_dir(temp.path())
            .env("AXIOM_HOME", &home)
            .env("AXIOM_FAKE_DISK", &disk),
    );
    assert_eq!(
        out.status.code(),
        Some(75),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!marker.exists());
    let goal = goal::load(&home, SESSION).unwrap().unwrap();
    assert_eq!(goal.status, "blocked");
    assert_eq!(goal.reason.as_deref(), Some("disk_space"));
    let pending = &goal.other["pending_launch"];
    assert_eq!(pending["session"], SESSION);
    assert_eq!(pending["need_bytes"], 8_u64 << 30);
    assert_eq!(pending["argv"][2], writer.to_str().unwrap());
    let events = fs::read_to_string(store.join("events.jsonl")).unwrap();
    let event = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["kind"] == "disk_preflight_paused")
        .unwrap();
    assert_eq!(event["data"]["pending"], *pending);
    let report = fs::read_to_string(store.join("reports/disk.txt")).unwrap();
    assert!(
        report
            .lines()
            .any(|line| line.contains("unpack") && line.contains("paused")),
        "{report}"
    );
}

#[test]
fn pressure_cleanup_frees_only_safe_classes() {
    let temp = common::temp();
    let home = temp.path().join("home");
    let ledger = Ledger::open(&home).unwrap();
    let owner = Owner {
        project: "project".into(),
        wish: Some("wish".into()),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut paths = Vec::new();
    for class in Class::ALL {
        let path = home.join(class.as_str());
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("content"), class.as_str()).unwrap();
        let meta = if class == Class::BuildCache {
            json!({"worktree_state":"merged", "state_since_ms":now})
        } else {
            json!({})
        };
        ledger.register(&path, class, &owner, meta).unwrap();
        ledger.touch(&path, now - 1_000_000_000).unwrap();
        paths.push((class, path));
    }
    // The newest non-current engine is retained; seed a newer copy so the tested one is obsolete.
    let previous = home.join("engine-previous");
    fs::create_dir_all(&previous).unwrap();
    fs::write(previous.join("content"), "previous").unwrap();
    ledger
        .register(&previous, Class::EngineCopy, &owner, json!({}))
        .unwrap();
    ledger.touch(&previous, now - 1_000).unwrap();
    let project = home.join("untracked-project-data");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("content"), "keep").unwrap();
    drop(ledger);

    let run = |yes: bool| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_axiom"));
        cmd.args(["storage", "clean", "--pressure"])
            .env("AXIOM_HOME", &home);
        if yes {
            cmd.arg("--yes");
        }
        output(&mut cmd)
    };
    assert_success(&run(false));
    for (_, path) in &paths {
        assert!(path.join("content").exists(), "{}", path.display());
    }
    assert_eq!(
        fs::read_to_string(previous.join("content")).unwrap(),
        "previous"
    );
    assert_eq!(fs::read_to_string(project.join("content")).unwrap(), "keep");
    assert!(!home.join("storage/deletions.jsonl").exists());

    assert_success(&run(true));
    let audit = fs::read_to_string(home.join("storage/deletions.jsonl")).unwrap();
    let records: Vec<AuditRecord> = audit
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    for (class, path) in &paths {
        if [Class::Scratch, Class::BuildCache, Class::EngineCopy].contains(class) {
            assert!(!path.exists(), "{}", path.display());
            let record = records
                .iter()
                .find(|record| record.path == ledger::normalize(path).to_string_lossy())
                .unwrap();
            assert_eq!(record.class, *class);
            assert_eq!(record.action, Action::Delete);
            assert_eq!(record.outcome, "done");
        } else {
            assert_eq!(
                fs::read_to_string(path.join("content")).unwrap(),
                class.as_str()
            );
        }
    }
    assert_eq!(
        fs::read_to_string(previous.join("content")).unwrap(),
        "previous"
    );
    assert_eq!(fs::read_to_string(project.join("content")).unwrap(), "keep");
}

#[test]
fn fake_wish_survives_detach_and_reattaches_over_service() {
    let temp = common::temp();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let disk = temp.path().join("disk.json");
    fs::write(&disk, json!({"free_bytes": 1_u64 << 50}).to_string()).unwrap();
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_axiom"))
            .args(["serve", "--no-tray"])
            .env("AXIOM_HOME", &home)
            .env("AXIOM_FAKE_DISK", &disk)
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .env("FAKE_CLAUDE_SCENARIO", "tick")
            .env("FAKE_CLAUDE_RECORD", temp.path().join("claude-record.json"))
            .env("FAKE_CLAUDE_TICK_MS", "100")
            .env_remove("FAKE_CLAUDE_SESSION")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let client = loop {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "service exited before connecting"
        );
        match Client::connect(&home) {
            Ok(client) => break client,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "service connection timed out: {error}; accept error: {}",
                    fs::read_to_string(home.join("service/accept-error.txt")).unwrap_or_default()
                );
                thread::sleep(Duration::from_millis(30));
            }
        }
    };
    let handle = client
        .start(StartRequest {
            wish: "test wish".into(),
            cwd: temp.path().to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let mut before = None;
    wait_until(|| {
        let snapshot = client.snapshot().unwrap();
        before = snapshot.wishes.into_iter().find(|w| {
            w.handle == handle
                && w.session.as_deref() == Some(SESSION)
                && w.state == "running"
                && w.log.iter().any(|l| l.text.contains("tick "))
        });
        before.is_some()
    });
    let before = before.unwrap();
    // The fake does not invoke the goal-opening hook; seed the goal as in the service crash fixture.
    goal::open(
        &home,
        SESSION,
        "test wish",
        Some(temp.path().to_str().unwrap()),
    )
    .unwrap();
    drop(client);
    let client = Client::connect(&home).unwrap();
    let mut after = None;
    wait_until(|| {
        let snapshot = client.snapshot().unwrap();
        after = snapshot.wishes.into_iter().find(|w| {
            w.handle == handle
                && w.session.as_deref() == Some(SESSION)
                && w.state == "running"
                && w.log.len() > before.log.len()
                && w.log.iter().filter(|l| l.text.contains("tick ")).count()
                    > before
                        .log
                        .iter()
                        .filter(|l| l.text.contains("tick "))
                        .count()
        });
        after.is_some()
    });
    assert_eq!(after.unwrap().key, before.key);
    client.quit(true).unwrap();
    wait_until(|| child.0.try_wait().unwrap().is_some());
    assert!(child.0.wait().unwrap().success());
    let resumed = goal::load(&home, SESSION).unwrap().unwrap();
    assert_eq!(resumed.status, "blocked");
    assert_eq!(resumed.reason.as_deref(), Some("interrupted"));
    assert!(resumed
        .note
        .unwrap()
        .contains(&format!("axiom wish --resume {SESSION}")));
}
