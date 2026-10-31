use axiom::bridge::{serve, Config};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
#[path = "../common/mod.rs"]
mod common;
#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Input(mpsc::Receiver<String>);
impl io::Read for Input {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        unreachable!()
    }
}
impl io::BufRead for Input {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        unreachable!()
    }
    fn consume(&mut self, _: usize) {
        unreachable!()
    }
    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        match self.0.recv() {
            Ok(s) => {
                line.push_str(&s);
                Ok(s.len())
            }
            Err(_) => Ok(0),
        }
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(
            Instant::now() < until,
            "timed out waiting for observed event"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn response(c: &Capture, id: i64) -> Value {
    let mut result = None;
    wait(|| {
        result = String::from_utf8_lossy(&c.0.lock().unwrap())
            .lines()
            .filter_map(|s| serde_json::from_str::<Value>(s).ok())
            .find(|v| v["id"] == id);
        result.is_some()
    });
    result.unwrap()["result"]["structuredContent"].clone()
}
fn send(tx: &mpsc::Sender<String>, id: i64, args: Value) {
    tx.send(format!(
        "{}\n",
        json!({"id":id,"method":"tools/call","params":{"name":"session","arguments":args}})
    ))
    .unwrap();
}
fn git(repo: &Path, args: &[&str]) {
    assert!(std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap()
        .status
        .success());
}
fn setup(repo: &Path, verify: &Path, signal: &Path, slow: bool) {
    fs::create_dir(repo).unwrap();
    let script = if slow {
        format!("import os,time,pathlib\npathlib.Path({}).write_text(str(os.getpid()))\ntime.sleep(120)\n",serde_json::to_string(signal.to_str().unwrap()).unwrap())
    } else {
        "from pathlib import Path\nassert Path('result.txt').read_text()=='CHRONOS_SMOKE_OK'\n"
            .into()
    };
    fs::write(verify, script).unwrap();
    git(repo, &["init", "-q"]);
    git(repo, &["add", "."]);
    git(
        repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "fixture",
        ],
    );
}
#[test]
fn readonly_attached_dispatcher_observes_tools_launches_sol_and_keeps_reports_local() {
    exercise("attached-smoke", true);
}
#[test]
fn cancellation_and_eof_reach_attached_model_nested_agent_and_callback_verifier() {
    for scenario in [
        "attached-model",
        "attached-late",
        "attached-nested",
        "attached-verify",
    ] {
        for notify in [true, false] {
            exercise(scenario, notify);
        }
    }
}
#[test]
fn cancelled_dispatcher_retains_report_when_notice_store_is_busy() {
    for notify in [true, false] {
        exercise_with_busy_store("attached-late", notify, true);
    }
}
#[test]
fn attached_bash_cancellation_child() {
    if std::env::var_os("CHRONOS_TEST_ISOLATED_BASH").is_none() {
        return;
    }
    for notify in [true, false] {
        exercise("attached-bash", notify);
    }
}
#[test]
fn cancellation_and_eof_reap_host_bash_and_descendants() {
    // Shell overrides are process-local; never race other tests' environment.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "dispatcher_cases::attached_bash_cancellation_child",
            "--nocapture",
        ])
        .env("CHRONOS_TEST_ISOLATED_BASH", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn exercise(scenario: &str, notify: bool) {
    exercise_with_busy_store(scenario, notify, false);
}
fn exercise_with_busy_store(scenario: &str, notify: bool, busy: bool) {
    let temp = common::temp();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    let verify = repo.join("verify_smoke.py");
    let signal = temp.path().join("started");
    setup(&repo, &verify, &signal, scenario == "attached-verify");
    if scenario == "attached-bash" {
        let shell = common::repo_root().join("tests/bridge/fake_shell.py");
        std::env::set_var("CHRONOS_TEST_SHELL_SIGNAL", &signal);
        #[cfg(windows)]
        std::env::set_var(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec![
                common::python(),
                "-I".into(),
                shell.to_string_lossy().into_owned(),
            ])
            .unwrap(),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = temp.path().join("bin");
            fs::create_dir(&bin).unwrap();
            // WSL may run a Windows checkout with CRLF; a shebang must use LF.
            fs::write(bin.join("sh"),fs::read_to_string(shell).unwrap().replace("\r\n","\n")).unwrap();
            fs::set_permissions(bin.join("sh"), fs::Permissions::from_mode(0o755)).unwrap();
            let mut paths = vec![bin];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        }
    }
    let verify_command = format!("\"{}\" -I \"{}\"", common::python(), verify.display());
    let cfg = Config {
        data_home: Some(home.clone()),
        codex: Ok(vec![
            common::python(),
            "-I".into(),
            common::repo_root()
                .join("tests/bridge/fake_dispatcher.py")
                .to_string_lossy()
                .into_owned(),
            "--scenario".into(),
            scenario.into(),
            "--home".into(),
            home.to_string_lossy().into_owned(),
            "--project".into(),
            repo.to_string_lossy().into_owned(),
            "--verify".into(),
            verify_command,
            "--signal".into(),
            signal.to_string_lossy().into_owned(),
        ]),
        agy: Err(axiom::base::Failure::new("disabled", "test")),
        wsl: None,
        on_windows: cfg!(windows),
    };
    let capture = Capture(Arc::new(Mutex::new(vec![])));
    let output = capture.clone();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || serve(cfg, Input(rx), output));
    send(
        &tx,
        1,
        json!({"action":"begin","project":repo,"text":"smoke dispatch"}),
    );
    let begin=response(&capture,1);
    assert_eq!(begin["status"],"completed");
    // Test downstream callbacks from a trusted, approved execution checkpoint.
    let dir=home.join("authority").join(begin["session"].as_str().unwrap());
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"),json!({"phase":"execution","revision":1,"research":null,"reason":"trusted downstream fixture"}).to_string()).unwrap();
    send(
        &tx,
        2,
        json!({"action":"call","tool":"goal","arguments":{"mode":"change"}}),
    );
    assert_eq!(response(&capture, 2)["goal"]["mode"], "change");
    send(
        &tx,
        3,
        json!({"action":"call","tool":"Agent","arguments":{"subagent_type":"chronos:dispatcher","project":"demo","cwd":repo,"prompt":"Run authorized smoke only"}}),
    );
    if scenario == "attached-smoke" {
        let reply = response(&capture, 3);
        // The transport completed, but this legacy unit has only been built:
        // independent review/release remains blocked, never outer success.
        assert_eq!(reply["status"], "blocked", "{reply}\nreport: {}",
            fs::read_to_string(reply["saved_to"].as_str().unwrap_or("")).unwrap_or_default());
        assert_eq!(reply["transport_status"], "completed");
        assert_eq!(reply["outcome"], "blocked");
        assert!(reply.to_string().len() < 8192);
        assert_eq!(reply["recipient"], "core");
        assert!(reply.get("final_message").is_none());
        assert!(!reply.to_string().contains("PRIVATE_"));
        assert!(fs::read_to_string(reply["saved_to"].as_str().unwrap())
            .unwrap()
            .contains("PRIVATE_WORKER_SENTINEL"));
        assert_eq!(
            fs::read_to_string(repo.join("result.txt")).unwrap(),
            "CHRONOS_SMOKE_OK"
        );
        assert_eq!(
            axiom::orch::evidence(&home.join("orchestrate/demo"), "smoke")
                .unwrap()
                .last()
                .unwrap()
                .kind,
            "verify"
        );
        drop(tx);
        assert_eq!(server.join().unwrap(), 0);
        return;
    }
    wait(|| {
        fs::read_to_string(&signal).is_ok_and(|s| {
            if matches!(scenario, "attached-verify" | "attached-bash") {
                !s.is_empty() && s.split_whitespace().all(|p| p.parse::<u32>().is_ok())
            } else {
                !s.is_empty()
            }
        })
    });
    let pids: Vec<u32> = if matches!(scenario, "attached-verify" | "attached-bash") {
        fs::read_to_string(&signal)
            .unwrap()
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect()
    } else {
        vec![]
    };
    let start = Instant::now();
    let store_lock = if busy {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)] {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(home.join("orchestrate/demo/.orch.mutex")).unwrap();
        #[cfg(unix)] {
            use std::os::fd::AsRawFd;
            assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0);
        }
        Some(file)
    } else { None };
    if notify {
        tx.send(format!(
            "{}\n",
            json!({"method":"notifications/cancelled","params":{"requestId":3}})
        ))
        .unwrap();
        let result = response(&capture, 3);
        assert_ne!(result["status"], "completed", "{scenario}: {result}");
        if scenario == "attached-late" {
            let report: Value = serde_json::from_str(
                &fs::read_to_string(result["saved_to"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(report["terminal_status"], "completed");
            assert_eq!(report["status"], "interrupted");
        }
        drop(store_lock);
        // Other requests on the attachment retain their own cancellation domain.
        send(&tx, 4, json!({"action":"status"}));
        assert_eq!(response(&capture, 4)["status"], "completed");
    }
    drop(tx);
    assert_eq!(server.join().unwrap(), 0);
    assert_ne!(
        response(&capture, 3)["status"],
        "completed",
        "{scenario}: cancellation must not credit a late completion"
    );
    if scenario == "attached-late" {
        // EOF must retain the same durable terminal evidence as explicit cancellation,
        // including when the store mutex stays held until serve has returned.
        let result = response(&capture, 3);
        let report: Value = serde_json::from_str(
            &fs::read_to_string(result["saved_to"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(report["terminal_status"], "completed");
        assert_eq!(report["status"], "interrupted");
    }
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "cancellation did not terminate promptly"
    );
    for pid in pids {
        wait(|| !axiom::base::proc::pid_alive(pid));
    }
    let log = fs::read_dir(home.join("logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let events: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(events.iter().any(|e| e["method"] == "turn/completed"
        && match scenario {
            "attached-late" => e["params"]["turn"]["status"] == "completed",
            // Shell reap can return its callback before the owner's interrupt.
            "attached-bash" => matches!(
                e["params"]["turn"]["status"].as_str(),
                Some("completed" | "interrupted")
            ),
            _ => e["params"]["turn"]["status"] == "interrupted",
        }), "scenario={scenario}, notify={notify}, events={events:?}");
    if scenario == "attached-nested" {
        assert!(events.iter().any(|e| e["hostEvent"] == "threadStarted"
            && e["role"] == "memory-keeper"
            && e["model"] == "gpt-6-terra"));
    }
}
