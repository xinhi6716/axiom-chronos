#[path = "../common/mod.rs"]
mod common;
use axiom::{
    goal,
    session::{resume, run, Config},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};

struct Harness {
    // run() serializes process-wide signal handling. Acquire the corresponding
    // test lease before fixtures, watchdogs and asynchronous marker waits too;
    // otherwise parallel tests time out while still queued outside run().
    _session: std::sync::MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
    config: Config,
    record: PathBuf,
}
impl Harness {
    fn new(scenario: &str) -> Self {
        static SESSIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let session = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
        let temp = common::temp();
        let home = temp.path().join("home");
        let record = temp.path().join("record.json");
        if scenario=="checkpoint-authority" {
            let store=axiom::orch::store_for(&home,"demo").unwrap();
            assert_eq!(axiom::orch::run(&store,&["init".into()],true,false).exit_code,0);
            assert_eq!(axiom::orch::run(&store,&["wait".into(),"enroll".into()],true,false).exit_code,0);
        }

        // These tests start at an already approved execution checkpoint and test
        // downstream routing/cancellation only. authority_cases covers the real prefix.
        if matches!(scenario,"agents" | "fallback" | "verify-stop" | "verify-interrupt" | "checkpoint-authority") {
            let dir=home.join("authority/thread-1");std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("state.json"),json!({"phase":"execution","revision":1,"research":null,"reason":"trusted downstream test fixture"}).to_string()).unwrap();
        }

        std::fs::write(temp.path().join("input.txt"), "readable source").unwrap();
        let mut cmd = vec![
            common::python(),
            "-I".into(),
            common::repo_root()
                .join("tests/session/fake_session.py")
                .to_string_lossy()
                .into_owned(),
        ];
        for (k, v) in [
            ("--scenario", scenario.to_owned()),
            ("--record", record.to_string_lossy().into_owned()),
            ("--home", home.to_string_lossy().into_owned()),
            ("--project", temp.path().to_string_lossy().into_owned()),
        ] {
            cmd.extend([k.into(), v]);
        }
        Self {
            _session: session,
            config: Config {
                codex: Ok(cmd),
                home,
                bin: temp.path().join("axiom"),
            },
            record,
            temp,
        }
    }
    fn arg(&mut self, key: &str, value: &Path) {
        self.config
            .codex
            .as_mut()
            .unwrap()
            .extend([key.into(), value.to_string_lossy().into_owned()]);
    }
    fn execute(&self, input: Box<dyn BufRead + Send>, limit: Option<u32>) -> (i32, String) {
        let mut output = Vec::new();
        let code = run(
            &self.config,
            "wish",
            "幫 todo 加統計功能",
            self.temp.path(),
            limit,
            input,
            &mut output,
        )
        .unwrap();
        (code, String::from_utf8(output).unwrap())
    }
    fn records(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.record).unwrap()).unwrap()
    }
    fn requests(&self, method: &str) -> Vec<Value> {
        self.records()["requests"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == method)
            .map(|r| r["params"].clone())
            .collect()
    }
    fn goal(&self) -> goal::Goal {
        goal::load(&self.config.home, "thread-1").unwrap().unwrap()
    }
    fn logs(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.config.home.join("logs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }
}
fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}
#[test]
fn enrolled_change_waits_without_model_polling_and_wakes_for_interjection() {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    struct IdleInput { record: PathBuf, count: Arc<AtomicUsize>, sent: bool }
    impl Read for IdleInput {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.sent { return Ok(0); }
            std::thread::sleep(Duration::from_millis(800));
            let record: Value = serde_json::from_slice(&std::fs::read(&self.record)?).unwrap();
            self.count.store(record["requests"].as_array().unwrap().iter()
                .filter(|r| r["method"] == "turn/start").count(), Ordering::SeqCst);
            self.sent = true;
            let line = b"new requirement\n";
            buf[..line.len()].copy_from_slice(line);
            Ok(line.len())
        }
    }
    let h = Harness::new("idle-wake");
    idle_store(&h);
    let count = Arc::new(AtomicUsize::new(0));
    let input = IdleInput { record: h.record.clone(), count: count.clone(), sent: false };
    let (code, out) = h.execute(Box::new(BufReader::new(input)), None);
    assert_eq!(code, 1, "{out}");
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert!(turns[1]["input"][0]["text"].as_str().unwrap().contains("new requirement"));
    assert!(h.goal().interjection.is_none());
}
// Synthetic dispatch reports are fixtures, not evidence of live engineering.
// They pass the real persisted journal replay and session RPC delivery paths.
fn idle_store(h: &Harness) -> PathBuf {
    let store = h.config.home.join("orchestrate").join(axiom::base::slug_for_dir(h.temp.path()));
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n").unwrap();
    std::fs::write(store.join("wait-required"), "1\n").unwrap();
    let state = axiom::orch::WaitState { engineering_enrolled: true, ..Default::default() };
    axiom::base::atomic_write(&store.join("core-wait.json"), &serde_json::to_vec(&state).unwrap()).unwrap();
    store
}
fn publish_idle_notice(store: &Path, task: &str) -> String {
    let mut state = axiom::orch::wait_state(store).unwrap();
    let c: axiom::orch::Checkpoint = serde_json::from_value(json!({
        "task":task,"module":"m","round":1,"min_rounds":5,"owner":"dispatcher",
        "dependencies":[],"outputs":["out"],"evidence":[],"next_action":"core verdict",
        "issue":"i","proposal":"p","evidence_version":1,"contract":"fixture-contract",
        "repo":"","severity":"normal","findings":[]
    })).unwrap();
    state.checkpoints.push(c);
    let dispatch = format!("{task}-dispatch");
    for (kind, actor, model) in [
        ("dispatch", "lifecycle", ""), ("build", "worker", "gpt-6.1-terra"),
        ("assembly_intent", "lifecycle", ""), ("assembly", "sol", "gpt-6.1-sol"),
        ("package_intent", "lifecycle", ""), ("package", "lifecycle", ""),
        ("verification_intent", "lifecycle", ""), ("verification", "astra-verify", "gpt-6-astra"),
        ("notification", "lifecycle", ""),
    ] {
        let report = json!({"status":"completed","actual_effort":"high","model":model,
            "actual_sandbox":if kind == "verification" {"readOnly"} else {"workspaceWrite"},
            "thread_id":"fixture-thread","turn_id":kind,"source_identity":"fixture-source",
            "binding":{"task":task,"round":1,"attempt":dispatch,"artifact":"fixture-artifact",
                "issue":"i","evidence_version":1,"contract":"fixture-contract"},
            "package_hash":"sha256:fixture","verification_event":format!("{task}-verification"),
            "structured_output":{"issues":[],"severity":"normal"}});
        state.events.push(axiom::orch::Event {
            id:format!("{task}-{kind}"), task:task.into(), round:1, issue:"i".into(),
            proposal:"p".into(), evidence_version:1, kind:kind.into(), evidence:format!("host:{report}"),
            actor:actor.into(), dispatch_id:dispatch.clone(), contract:"fixture-contract".into(),
            artifact:"fixture-artifact".into(),
        });
    }
    axiom::base::atomic_write(&store.join("core-wait.json"), &serde_json::to_vec(&state).unwrap()).unwrap();
    axiom::orch::wait_state(store).unwrap(); // Reject malformed fixtures before exercising the loop.
    format!("{task}-notification")
}
fn wait_file(path: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(12);
    while !path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}
fn marker(h: &Harness, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}.{suffix}", h.record.display()))
}
fn start_idle(h: &Harness, input: Box<dyn BufRead + Send>) -> std::thread::JoinHandle<(i32, String)> {
    let config = Config {
        codex: Ok(h.config.codex.as_ref().unwrap().clone()),
        home: h.config.home.clone(), bin: h.config.bin.clone(),
    };
    let project = h.temp.path().to_path_buf();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let code = run(&config, "wish", "durable idle test", &project, Some(20), input, &mut output).unwrap();
        (code, String::from_utf8(output).unwrap())
    })
}
fn assert_idle_turns(h: &Harness, count: usize) {
    // Several 50ms polling periods must not schedule another model turn.
    std::thread::sleep(Duration::from_millis(350));
    assert_eq!(h.requests("turn/start").len(), count);
}
struct ChannelInput(std::sync::mpsc::Receiver<String>);
impl Read for ChannelInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Ok(line) = self.0.recv() else { return Ok(0) };
        let bytes = line.as_bytes();
        assert!(bytes.len() <= buf.len());
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
#[test]
fn enrolled_idle_notice_wakes_once_and_replays_pending_notice_on_user_wake() {
    let h = Harness::new("idle-state");
    let store = idle_store(&h);
    let (tx, rx) = std::sync::mpsc::channel();
    let running = start_idle(&h, Box::new(BufReader::new(ChannelInput(rx))));
    wait_file(&marker(&h, "completed-1"));
    assert_idle_turns(&h, 1);
    let first = publish_idle_notice(&store, "first");
    wait_file(&marker(&h, "completed-2"));
    assert_idle_turns(&h, 2);
    // A second distinct durable event wakes once even with the old notice pending.
    let second = publish_idle_notice(&store, "second");
    wait_file(&marker(&h, "completed-3"));
    assert_idle_turns(&h, 3);
    let before = std::fs::read(store.join("core-wait.json")).unwrap();
    assert_eq!(h.goal().status, "active");
    tx.send("new requirement while idle\n".into()).unwrap();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 4);
    assert!(turns[1]["input"].to_string().contains(&first));
    for i in [2, 3] {
        assert!(turns[i]["input"].to_string().contains(&first));
        assert!(turns[i]["input"].to_string().contains(&second));
        assert!(turns[i]["input"].to_string().contains("verification_event"));
    }
    let persisted: Value = serde_json::from_slice(&std::fs::read(marker(&h, "interjection")).unwrap()).unwrap();
    assert_eq!(persisted["interjection"]["prompt"], "new requirement while idle");
    assert!(h.goal().interjection.is_none());
    assert!(h.goal().amendments.iter().any(|a| a == "new requirement while idle"));
    // Delivery is not a verdict and must not rewrite or acknowledge the journal.
    assert_eq!(std::fs::read(store.join("core-wait.json")).unwrap(), before);
}
#[test]
fn enrolled_idle_cancellation_and_durable_stop_do_not_start_turns() {
    for stop in [false, true] {
        let h = Harness::new("idle-state");
        idle_store(&h);
        let running = start_idle(&h, empty());
        wait_file(&marker(&h, "completed-1"));
        assert_idle_turns(&h, 1);
        if stop {
            goal::interject(&h.config.home, "thread-1", "stop idle work").unwrap();
            goal::classify(&h.config.home, "thread-1", "stop").unwrap();
        } else {
            axiom::session::trigger_interrupt();
        }
        let (code, out) = running.join().unwrap();
        assert_eq!(code, if stop { 1 } else { 130 }, "{out}");
        assert_eq!(h.requests("turn/start").len(), 1);
        assert_eq!(h.goal().status, "blocked");
        assert_eq!(h.goal().reason.as_deref(), Some(if stop { "user_stop" } else { "interrupted" }));
    }
}
#[test]
fn enrolled_idle_delivers_turn_boundary_notice_without_duplicate_wake() {
    let h = Harness::new("idle-state-boundary");
    let store = idle_store(&h);
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "boundary"));
    let id = publish_idle_notice(&store, "boundary");
    // The notice was absent when turn 1 was constructed, but durable before its completion.
    std::fs::write(marker(&h, "release"), "go").unwrap();
    wait_file(&marker(&h, "completed-2"));
    assert_idle_turns(&h, 2);
    axiom::session::trigger_interrupt();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 130, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert!(!turns[0]["input"].to_string().contains(&id));
    assert!(turns[1]["input"].to_string().contains(&id));
}
#[test]
fn enrolled_idle_dispatcher_reply_counts_as_notice_delivery_not_a_new_wake() {
    let h = Harness::new("idle-state-callback");
    let store = idle_store(&h);
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "boundary"));
    let id = publish_idle_notice(&store, "callback");
    std::fs::write(marker(&h, "release"), "go").unwrap();
    wait_file(&marker(&h, "completed-1"));
    // One core turn and the synchronous dispatcher turn; no follow-up core turn.
    assert_idle_turns(&h, 2);
    axiom::session::trigger_interrupt();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 130, "{out}");
    assert_eq!(h.requests("turn/start").len(), 2);
    assert!(h.records()["responses"].to_string().contains(&id));
    assert!(axiom::orch::wait_state(&store).unwrap().events.iter().any(|e| e.id == id));
}
#[test]
fn enrolled_idle_propagates_durable_state_errors_without_waking_model() {
    let h = Harness::new("idle-state");
    let store = idle_store(&h);
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "completed-1"));
    assert_idle_turns(&h, 1);
    std::fs::write(store.join("core-wait.json"), "{").unwrap();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("invalid core-wait state"), "{out}");
    assert_eq!(h.requests("turn/start").len(), 1);
}
struct Delayed(bool);
#[test]
fn enrolled_steered_interjection_is_classified_after_unclassified_turn_completion() {
    let h = Harness::new("idle-fix-steered");
    idle_store(&h);
    let (tx, rx) = std::sync::mpsc::channel();
    let running = start_idle(&h, Box::new(BufReader::new(ChannelInput(rx))));
    wait_file(&marker(&h, "boundary"));
    tx.send("successfully steered but not classified\n".into()).unwrap();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    assert_eq!(h.requests("turn/steer").len(), 1);
    assert_eq!(h.requests("turn/start").len(), 2);
    assert!(h.requests("turn/start")[1]["input"].to_string().contains("successfully steered"));
    assert!(h.goal().interjection.is_none());
    assert!(h.goal().amendments.iter().any(|a| a == "successfully steered but not classified"));
}

fn actionable_store(h: &Harness) -> PathBuf {
    let store = idle_store(h);
    let call = |args: &[&str]| {
        let out = axiom::orch::run(&store, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true, false);
        assert_eq!(out.exit_code, 0, "{}", out.stderr);
    };
    call(&["init"]);
    std::fs::write(store.join("brief"), "GOAL progress\nSCOPE input.txt\nOUTPUTS input.txt\nDELIVERY implementation\nVERIFY echo ok\n").unwrap();
    call(&["unit", "add", "u", "--track", "m", "--brief", store.join("brief").to_str().unwrap()]);
    call(&["wait", "prepare", "--task", "u", "--repo", h.temp.path().to_str().unwrap(),
        "--issue", "i", "--proposal", "p"]);
    store
}

#[test]
fn enrolled_producerless_todo_recovery_and_dispatcher_failure_deliver_stable_blocked_outcome() {
    for scenario in ["idle-fix-todo", "idle-fix-recovered", "idle-fix-dispatcher-completed",
        "idle-fix-dispatcher-failed", "idle-fix-ignored", "idle-fix-abandoned"] {
        let h = Harness::new(scenario);
        let store = if scenario == "idle-fix-dispatcher-failed" { idle_store(&h) } else { actionable_store(&h) };
        if scenario == "idle-fix-recovered" {
            axiom::orch::commit_dispatch(&store, "u").unwrap();
            let state = axiom::orch::wait_state(&store).unwrap();
            let mut recovery = state.events.last().unwrap().clone();
            recovery.kind = "recovery".into();
            recovery.id = "confirmed-recovery".into();
            recovery.actor = "core".into();
            recovery.evidence = "operator confirmed no previous writer launched".into();
            axiom::orch::recover_dispatch(&store, recovery, "not_started").unwrap();
        }
        if scenario == "idle-fix-abandoned" {
            axiom::orch::commit_dispatch(&store, "u").unwrap();
            std::fs::write(store.join(".worker.claim"), "unconfirmed stale ownership").unwrap();
        }
        let (code, out) = h.execute(empty(), Some(20));
        assert_eq!(code, 1, "{scenario}: {out}");
        assert_eq!(h.goal().status, "blocked", "{scenario}");
        let turns = h.requests("turn/start");
        let core_turns: Vec<_> = turns.iter().filter(|t| t["threadId"] == "thread-1").collect();
        assert_eq!(core_turns.len(), 2, "{scenario}");
        assert!(core_turns[1]["input"].to_string().contains("Rust idle liveness failure"));
        if scenario == "idle-fix-ignored" {
            assert!(h.goal().note.unwrap().contains("idle_blocked"));
        }
        // A liveness failure is never forged into independent verification or acceptance.
        let state = axiom::orch::wait_state(&store).unwrap();
        assert!(!state.events.iter().any(|e| matches!(e.kind.as_str(), "verification" | "core_approve" | "release")));
    }
}

// Hold the actual nonblocking OS store mutex, not synthetic error strings.
fn hold_store_mutex(store: &Path) -> std::fs::File {
    // The live idle poller can own the real lock at this boundary. Retry only
    // contention; acquiring the fixture must not race an ordinary poll.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)] {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        match options.open(store.join(".orch.mutex")) {
            Ok(file) => {
                #[cfg(unix)] {
                    use std::os::fd::AsRawFd;
                    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                        let e = std::io::Error::last_os_error();
                        assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}");
                        assert!(std::time::Instant::now() < deadline, "store lock remained busy");
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                }
                return file;
            }
            Err(e) => {
                #[cfg(windows)]
                let busy = e.raw_os_error() == Some(32);
                #[cfg(not(windows))]
                let busy = e.kind() == std::io::ErrorKind::WouldBlock;
                assert!(busy, "{e}");
                assert!(std::time::Instant::now() < deadline, "store lock remained busy: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
fn enrolled_idle_retries_real_store_lock_without_model_wake_then_delivers_notice() {
    let h = Harness::new("idle-state");
    let store = idle_store(&h);
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "completed-1"));
    assert_idle_turns(&h, 1);
    let lock = hold_store_mutex(&store);
    assert_eq!(axiom::orch::wait_state(&store).unwrap_err().kind, "busy");
    assert_idle_turns(&h, 1);
    assert!(!running.is_finished());
    drop(lock);
    let id = publish_idle_notice(&store, "after-lock");
    wait_file(&marker(&h, "completed-2"));
    assert_idle_turns(&h, 2);
    axiom::session::trigger_interrupt();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 130, "{out}");
    assert!(h.requests("turn/start")[1]["input"].to_string().contains(&id));
}

#[test]
fn dispatcher_completion_retries_actual_store_lock_without_false_failure_or_core_wake() {
    let h = Harness::new("dispatcher-boundary");
    let store = idle_store(&h);
    let id = publish_idle_notice(&store, "dispatcher-boundary");
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "dispatcher-ready"));
    let lock = hold_store_mutex(&store);
    assert_eq!(axiom::orch::wait_state(&store).unwrap_err().kind, "busy");
    std::fs::write(marker(&h, "release"), "").unwrap();
    wait_file(&marker(&h, "dispatcher-completed"));
    assert_idle_turns(&h, 2); // One core turn and one dispatcher turn.
    assert!(!marker(&h, "callback").exists());
    drop(lock);
    wait_file(&marker(&h, "completed-1"));
    let response = std::fs::read_to_string(marker(&h, "callback")).unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["success"], true, "{response}");
    assert!(response.to_string().contains(&id), "{response}");
    assert_idle_turns(&h, 2);
    assert!(!running.is_finished());
    axiom::session::trigger_interrupt();
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 130, "{out}");
    assert_eq!(h.requests("turn/start").len(), 2);
}

#[test]
fn dispatcher_completion_propagates_corruption_after_real_lock_releases() {
    let h = Harness::new("dispatcher-boundary-corrupt");
    let store = idle_store(&h);
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "dispatcher-ready"));
    let lock = hold_store_mutex(&store);
    std::fs::write(store.join("core-wait.json"), "{").unwrap();
    std::fs::write(marker(&h, "release"), "").unwrap();
    wait_file(&marker(&h, "dispatcher-completed"));
    assert_idle_turns(&h, 2);
    assert!(!marker(&h, "callback").exists());
    drop(lock);
    wait_file(&marker(&h, "callback"));
    let response = std::fs::read_to_string(marker(&h, "callback")).unwrap();
    let response: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["success"], false, "{response}");
    assert!(response.to_string().contains("invalid core-wait state"), "{response}");
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("invalid core-wait state"), "{out}");
    assert_eq!(h.requests("turn/start").len(), 2);
}

#[test]
fn enrolled_idle_waits_for_actual_worker_ownership_not_just_durable_claim() {
    let h = Harness::new("idle-fix-owned");
    let store = actionable_store(&h);
    assert!(std::process::Command::new("git").args(["init", "-q"])
        .current_dir(h.temp.path()).status().unwrap().success());
    assert!(matches!(axiom::orch::begin_unit(&store, "u", "worker", h.temp.path(), false).unwrap(),
        axiom::orch::Begin::Run { .. }));
    let running = start_idle(&h, empty());
    wait_file(&marker(&h, "boundary"));
    assert_idle_turns(&h, 1);
    assert!(!running.is_finished());
    axiom::orch::abort_unit(&store, "u");
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    assert_eq!(h.goal().status, "blocked");
    assert_eq!(h.requests("turn/start").len(), 2);
}

#[test]
fn enrolled_idle_missing_unreadable_and_corrupt_files_fail_without_model_wake() {
    for (file, damage) in [("core-wait.json", "missing"), ("core-wait.json", "directory"),
        ("units.tsv", "missing"), ("units.tsv", "directory"), ("units.tsv", "invalid-utf8"),
        ("core-wait.json", "corrupt-after-lock")] {
        let h = Harness::new("idle-state");
        let store = idle_store(&h);
        let running = start_idle(&h, empty());
        wait_file(&marker(&h, "completed-1"));
        assert_idle_turns(&h, 1);
        let lock = hold_store_mutex(&store);
        std::fs::remove_file(store.join(file)).unwrap();
        match damage {
            "directory" => std::fs::create_dir(store.join(file)).unwrap(),
            "invalid-utf8" => std::fs::write(store.join(file), [0xff]).unwrap(),
            "corrupt-after-lock" => std::fs::write(store.join(file), "{").unwrap(),
            _ => (),
        }
        drop(lock);
        let (code, out) = running.join().unwrap();
        assert_eq!(code, 1, "{file}/{damage}: {out}");
        assert!(out.contains("session error"), "{file}/{damage}: {out}");
        assert_eq!(h.requests("turn/start").len(), 1);
    }
}
impl Read for Delayed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Ok(0);
        }
        std::thread::sleep(Duration::from_millis(350));
        self.0 = true;
        let bytes = "補充中文\r\n".as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
#[test]
fn launch_progress_log_eof_and_readonly_authority() {
    let h = Harness::new("ok");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("完成中文"));
    assert!(out.contains("codex tokens: gpt-6-astra 100 in (20 cached) / 10 out"));
    assert!(!out.contains('$'));
    assert_eq!(h.goal().status, "done");
    assert_eq!(h.goal().wish, "幫 todo 加統計功能");
    let start = &h.requests("thread/start")[0];
    assert_eq!(start["model"], "gpt-6-astra");
    assert_eq!(start["sandbox"], "read-only");
    assert_eq!(start["ephemeral"], false);
    assert_eq!(start["approvalPolicy"], "never");
    assert_eq!(start["config"]["plugins"]["fixture@test"]["enabled"], false);
    assert_eq!(start["config"]["mcp_servers"]["fixture"]["enabled"], false);
    let tools = start["dynamicTools"].as_array().unwrap();
    assert!(tools
        .iter()
        .all(|t| t["type"] == "function" && t["inputSchema"]["type"] == "object"));
    assert!(tools
        .iter()
        .all(|t| !t["name"].as_str().unwrap().starts_with("mcp__")));
    assert!(tools.iter().any(|t| t["name"] == "chronos_goal"));
    assert!(!tools
        .iter()
        .any(|t| t["name"].as_str().unwrap().ends_with("codex_run")));
    assert!(!start["config"].get("tools").is_some()); // No invented native-tool removal keys.
    let turn = &h.requests("turn/start")[0];
    assert_eq!(turn["threadId"], "thread-1");
    assert_eq!(turn["effort"], "medium");
    assert_eq!(turn["disabledPluginIds"], json!(["fixture@test"]));
    assert_eq!(turn["input"][0]["text"], "/chronos:wish 幫 todo 加統計功能");
    let logs = h.logs();
    assert_eq!(logs.len(), 1);
    assert!(regex::Regex::new(r"^\d{8}-\d{6}-wish\.jsonl$")
        .unwrap()
        .is_match(logs[0].file_name().unwrap().to_str().unwrap()));
    let events: Vec<Value> = std::fs::read_to_string(&logs[0])
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(events.iter().any(|e| e["method"] == "turn/completed"));
    assert!(events
        .iter()
        .any(|e| e["method"] == "thread/tokenUsage/updated"));
    assert_eq!(h.requests("thread/start").len(), 1);
    // A forged session in the callback must not close a different root.
    assert!(goal::load(&h.config.home, "forged-root").unwrap().is_none());
}

#[test]
fn server_must_confirm_actual_role_model_and_readonly_sandbox() {
    for (scenario, kind) in [
        ("unsafe-sandbox", "permission_denied"),
        ("model-mismatch", "model_mismatch"),
    ] {
        let h = Harness::new(scenario);
        let error = run(
            &h.config,
            "wish",
            "test",
            h.temp.path(),
            None,
            empty(),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(h.requests("turn/start").is_empty());
    }
}
#[test]
fn continuation_does_not_change_wish_or_amendments() {
    let h = Harness::new("continue");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0]["threadId"], turns[1]["threadId"]);
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    let prompt = turns[1]["input"][0]["text"].as_str().unwrap();
    assert!(prompt.starts_with(axiom::session::CONTINUE_TAG));
    assert!(prompt.contains("still active"));
    assert!(h.goal().amendments.is_empty());
    assert_eq!(out.matches("codex tokens:").count(), 1);
}
#[test]
fn interjection_updates_goal_and_steers_expected_live_turn() {
    let h = Harness::new("interject");
    let (code, out) = h.execute(Box::new(BufReader::new(Delayed(false))), None);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("補充中文"));
    assert_eq!(h.goal().amendments, ["補充中文"]);
    assert!(h.goal().interjection.is_none());
    let steer = &h.requests("turn/steer")[0];
    assert_eq!(steer["threadId"], "thread-1");
    assert_eq!(steer["expectedTurnId"], "turn-1");
    assert_eq!(steer["input"][0]["text"], "補充中文");
    assert!(h.records()["responses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["result"]["success"] == false
            && r["result"]["contentItems"][0]["text"]
                .as_str()
                .unwrap_or("")
                .contains("pending_interjection")));
}

#[test]
fn completion_racing_steer_defers_without_losing_the_message() {
    let h = Harness::new("stale");
    let (code, out) = h.execute(Box::new(BufReader::new(Delayed(false))), Some(8));
    assert_eq!(code, 0, "{out}");
    let turns = h.requests("turn/start");
    assert_eq!(turns.len(), 2);
    assert!(turns[1]["input"][0]["text"]
        .as_str()
        .unwrap()
        .contains("補充中文"));
    assert_eq!(h.goal().amendments, ["補充中文"]);
    assert_eq!(h.goal().status, "done");
}
#[test]
fn forwarding_skips_blank_lines() {
    let h = Harness::new("forward");
    let (code, out) = h.execute(
        Box::new(Cursor::new(b"\r\n\n  \nA\r\n\nB\n".to_vec())),
        None,
    );
    assert_eq!(code, 0, "{out}");
    let steers = h.requests("turn/steer");
    assert_eq!(steers.len(), 2);
    assert_eq!(steers[0]["input"][0]["text"], "A");
    assert_eq!(steers[1]["input"][0]["text"], "B");
}
#[test]
fn blocked_is_not_process_success() {
    let h = Harness::new("blocked");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1);
    assert!(out.contains("goal blocked: need input"));
    assert_eq!(h.goal().reason, None);
}
#[test]
fn capacity_crash_and_error_mark_interrupted_and_offer_resume() {
    for scenario in ["limit", "crash", "error"] {
        let h = Harness::new(scenario);
        let (code, out) = h.execute(empty(), None);
        assert_eq!(code, 1, "{scenario}: {out}");
        assert_eq!(h.goal().status, "blocked");
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("resume: axiom wish --resume thread-1"));
        assert!(h.logs()[0].is_file());
        if scenario == "limit" {
            assert!(out.contains("usage capacity limit"));
        }
        if scenario == "crash" {
            assert!(out.contains("codex_exited"));
        }
    }
}
#[test]
fn bounded_turns_including_zero() {
    for limit in [0, 2] {
        let h = Harness::new("forever");
        let (code, out) = h.execute(empty(), Some(limit));
        assert_eq!(code, 1);
        assert_eq!(h.requests("turn/start").len(), limit as usize);
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("maximum turns reached"));
    }
}
#[test]
fn corrupt_goal_is_reported_not_silently_successful() {
    let h = Harness::new("bad-goal");
    let (code, out) = h.execute(empty(), None);
    // Corruption must remain visible, even when an interrupted marker cannot be persisted.
    assert_eq!(code, 1);
    assert!(out.contains("invalid_json"));
}
#[test]
fn existing_log_is_not_overwritten() {
    let h = Harness::new("ok");
    std::fs::create_dir_all(h.config.home.join("logs")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for second in now - 60..=now + 60 {
        let stamp = axiom::session::log_stamp(second, axiom::session::local_offset_secs(second));
        std::fs::write(
            h.config
                .home
                .join("logs")
                .join(format!("{stamp}-wish.jsonl")),
            "sentinel",
        )
        .unwrap();
    }
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let log = Path::new(out.lines().find_map(|l| l.strip_prefix("log: ")).unwrap());
    let stamp = log
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_suffix("-wish-1.jsonl")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(
            h.config
                .home
                .join("logs")
                .join(format!("{stamp}-wish.jsonl"))
        )
        .unwrap(),
        "sentinel"
    );
}
#[test]
fn interrupt_kills_process_tree_and_preserves_resume() {
    let mut h = Harness::new("interrupt");
    let pids = h.temp.path().join("pids");
    h.arg("--pids", &pids);
    let wait = pids.clone();
    let trigger = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !wait.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait.exists());
        axiom::session::trigger_interrupt();
    });
    let (code, out) = h.execute(empty(), None);
    trigger.join().unwrap();
    assert_eq!(code, 130, "{out}");
    assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
    assert!(out.contains("interrupted by the user"));
    assert!(out.contains("resume:"));
    for id in std::fs::read_to_string(pids)
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse::<u32>().unwrap())
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while axiom::base::proc::pid_alive(id) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!axiom::base::proc::pid_alive(id), "process {id} survived");
    }
}
#[test]
fn resume_uses_actual_persistent_thread_and_original_goal() {
    let mut h = Harness::new("limit");
    assert_eq!(h.execute(empty(), None).0, 1);
    goal::interject(&h.config.home, "thread-1", "amendment").unwrap();
    goal::interrupt(&h.config.home, "thread-1", "test interruption").unwrap();
    let cmd = h.config.codex.as_mut().unwrap();
    let idx = cmd.iter().position(|s| s == "--scenario").unwrap();
    cmd[idx + 1] = "ok".into();
    let mut out = Vec::new();
    assert_eq!(
        resume(&h.config, "thread-1", Some(17), empty(), &mut out).unwrap(),
        0
    );
    assert_eq!(h.requests("thread/start").len(), 0);
    assert_eq!(h.requests("thread/resume")[0]["threadId"], "thread-1");
    assert_eq!(
        h.requests("thread/resume")[0]["config"]["features"]["multi_agent_v2"],
        false
    );
    assert!(h
        .requests("turn/start")
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert!(h.requests("turn/start")[0]["input"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with(axiom::session::CONTINUE_TAG));
    assert_eq!(h.goal().status, "done");
    assert_eq!(h.goal().wish, "幫 todo 加統計功能");
    assert_eq!(h.goal().amendments, ["amendment"]);
}
#[test]
fn resume_rejects_missing_done_corrupt_and_missing_cwd() {
    let h = Harness::new("ok");
    let attempt = |id| resume(&h.config, id, None, empty(), &mut Vec::new()).unwrap_err();
    assert_eq!(attempt("missing").kind, "not_found");
    goal::open(&h.config.home, "done", "wish", None).unwrap();
    goal::set_status(&h.config.home, "done", "done", None).unwrap();
    assert!(attempt("done").message.contains("already done"));
    for cwd in [
        None,
        Some(
            h.temp
                .path()
                .join("nonexistent")
                .to_string_lossy()
                .into_owned(),
        ),
    ] {
        goal::open(&h.config.home, "no-cwd", "wish", cwd.as_deref()).unwrap();
        assert!(attempt("no-cwd").message.contains("no project directory"));
    }
    std::fs::write(h.config.home.join("goals/no-cwd.json"), "{").unwrap();
    assert_eq!(attempt("no-cwd").kind, "invalid_json");
}
#[test]
fn core_dispatcher_and_actual_reviewer_models_have_distinct_threads() {
    let h = Harness::new("agents");
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}");
    let threads = h.requests("thread/start");
    assert_eq!(threads.len(), 4, "{threads:?}");
    assert_eq!(
        threads
            .iter()
            .map(|p| p["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gpt-6-astra", "gpt-6-terra", "gpt-6-sol", "gpt-6-terra"]
    );
    let dispatcher_tools = threads[1]["dynamicTools"].as_array().unwrap();
    let codex = dispatcher_tools
        .iter()
        .find(|t| t["name"] == "chronos_codex_run")
        .unwrap();
    assert!(codex["inputSchema"]["properties"]["role"]["enum"]
        .as_array()
        .unwrap()
        .contains(&json!("astra-verify")));
    assert_eq!(
        codex["inputSchema"]["properties"]["task_kind"]["enum"],
        json!(["verification", "measurement"])
    );
    for child in &threads[1..] {
        assert_eq!(child["ephemeral"], true);
        assert_eq!(child["sandbox"], "read-only");
    }
    let turns = h.requests("turn/start");
    for thread in &threads {
        assert_eq!(thread["config"]["features"]["multi_agent"], false);
        assert_eq!(thread["config"]["features"]["multi_agent_v2"], false);
        assert_eq!(thread["config"]["features"]["shell_tool"], false);
        assert_eq!(thread["config"]["features"]["unified_exec"], false);
        assert_eq!(thread["config"]["features"]["view_image"], true);
        let instructions = thread["developerInstructions"].as_str().unwrap();
        assert!(!instructions.starts_with("---"));
        assert!(!instructions.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(!instructions.contains("mcp__"));
    }
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert_eq!(turns[1]["effort"], "high");
    assert_eq!(turns[2]["effort"], "high");
    assert_eq!(turns[3]["effort"], "medium");
    assert_eq!(turns[1]["input"][0]["text"], "dispatch directly");
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
}

#[test]
fn sol_writer_fallback_uses_astra_max_and_returns_actual_model_while_critic_stays_sol_high() {
    let h = Harness::new("fallback");
    let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
    std::fs::create_dir_all(store.join("reports")).unwrap();
    let brief = store.join("u.brief");
    std::fs::write(&brief, "GOAL x\nSCOPE input.txt\nVERIFY echo ok\n").unwrap();
    std::fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tbuilt\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "input.txt"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        if args[0] == "init" {
            std::fs::write(h.temp.path().join("input.txt"), "fixture").unwrap();
        }
        assert!(std::process::Command::new("git")
            .current_dir(h.temp.path())
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    let identity = axiom::orch::snapshot(h.temp.path(), &["input.txt".into()]);
    // Sol is historical, not latest; it still contributed to the reviewed unit.
    std::fs::write(store.join("reports/u.evidence.tsv"), format!("kind\tcommand\texit_code\tcommit\tts\nrun\tgpt-6.1-sol\t0\told\t1\nrun\tgpt-6.1-terra\t0\tnew\t2\nverify\ttest\t0\t{identity}\t3\n")).unwrap();
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}");
    let threads = h.requests("thread/start");
    assert_eq!(
        threads
            .iter()
            .map(|t| t["model"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["gpt-6-astra", "gpt-6-terra", "gpt-6-astra", "gpt-6-sol"]
    );
    let turns = h.requests("turn/start");
    assert_eq!(turns[2]["effort"], "max");
    assert_eq!(turns[3]["effort"], "high");
    let records = h.records();
    let reports = records["responses"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| {
            r["result"]["contentItems"][0]["text"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
        })
        .collect::<Vec<_>>();
    let reviewer = reports
        .iter()
        .find(|r| r["thread_id"] == "thread-3")
        .unwrap();
    assert_eq!(reviewer["model"], "gpt-6-astra");
    assert_eq!(reviewer["status"], "completed");
    let states = axiom::orch::record_review(
        &store,
        &["u".into()],
        reviewer["model"].as_str().unwrap(),
        true,
        h.temp.path(),
    )
    .unwrap();
    assert_eq!(states[0].1, "verified");
    assert_eq!(
        axiom::orch::evidence(&store, "u")
            .unwrap()
            .last()
            .unwrap()
            .command,
        "gpt-6-astra"
    );
}
#[test]
fn independent_planner_thread_output_schema_saved_and_compressed() {
    let mut h = Harness::new("planner");
    let schema = h.temp.path().join("schema.json");
    std::fs::write(&schema,json!({"type":"object","properties":{"plan":{"type":"string"},"coverage":{"type":"object"}},"required":["plan","coverage"]}).to_string()).unwrap();
    let output = axiom::orch::store_for(&h.config.home, "demo")
        .unwrap()
        .join("research/plan.json");
    let second_cwd = h.temp.path().join("second-project");
    std::fs::create_dir(&second_cwd).unwrap();
    std::fs::write(second_cwd.join("input.txt"), "separate planner evidence").unwrap();
    h.arg("--planner-cwd", &second_cwd);
    h.arg("--schema", &schema);
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    let threads:Vec<_> = h.requests("thread/start").into_iter().filter(|p| p["model"] != "gpt-6-terra").collect();
    assert_eq!(threads.len(), 3);
    assert_eq!(Path::new(threads[2]["cwd"].as_str().unwrap()), second_cwd);
    assert!(!h.config.home.join("sessions/thread-1.planner").exists());
    assert_eq!(threads[0]["model"], threads[1]["model"]);
    assert_eq!(threads[1]["ephemeral"], true);
    assert_eq!(threads[2]["ephemeral"], true);
    assert!(h.requests("thread/resume").is_empty());
    assert_eq!(threads[1]["config"]["web_search"], "disabled");
    assert!(threads[2]["config"].get("web_search").is_none());
    assert!(!threads[1]["dynamicTools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "Agent" || t["name"].as_str().unwrap().ends_with("codex_run")));
    let turns:Vec<_> = h.requests("turn/start").into_iter().filter(|p| p["effort"] != "high").collect();
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
    assert_ne!(turns[1]["threadId"], turns[2]["threadId"]);
    assert!(turns
        .iter()
        .all(|t| t["disabledPluginIds"] == json!(["fixture@test"])));
    assert!(turns[1]["outputSchema"].is_object());
    assert_eq!(turns[1]["effort"], "max");
    let saved: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(saved["plan"], "independent");
    let responses = h.records()["responses"].as_array().unwrap().clone();
    let reports: Vec<Value> = responses
        .iter()
        .filter_map(|r| r["result"]["contentItems"][0]["text"].as_str())
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .filter(|r| r.get("saved_to").is_some() && r["model"] == "gpt-6-astra")
        .collect();
    assert_eq!(reports.len(), 2);
    for r in reports {
        assert!(r.get("final_message").is_none());
        assert!(r.get("structured_output").is_none());
        assert_eq!(r["model"], "gpt-6-astra");
        assert!(r["coverage"].is_object());
        assert!(r["usage"].is_object());
    }
}
#[test]
fn planner_rejects_invalid_schema_and_outside_store_destination() {
    for outside in [false, true] {
        let mut h = Harness::new("planner-invalid");
        let schema = h.temp.path().join("schema.json");
        std::fs::write(
            &schema,
            json!({"type":"object","required":["missing"]}).to_string(),
        )
        .unwrap();
        let output = if outside {
            h.temp.path().join("escape.json")
        } else {
            axiom::orch::store_for(&h.config.home, "demo")
                .unwrap()
                .join("bad.json")
        };
        h.arg("--schema", &schema);
        h.arg("--output", &output);
        assert_eq!(h.execute(empty(), None).0, 0);
        assert!(!output.exists());
        assert!(h.records()["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["result"]["success"] == false));
    }
}

#[test]
fn resume_keeps_research_bound_with_fresh_independent_planners() {
    let mut h = Harness::new("planner-limit");
    let schema = h.temp.path().join("schema.json");
    std::fs::write(
        &schema,
        json!({"type":"object","required":["plan"]}).to_string(),
    )
    .unwrap();
    let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
    let output = store.join("research/plan.json");
    h.arg("--schema", &schema);
    h.arg("--output", &output);
    let mut out = Vec::new();
    assert_eq!(
        run(
            &h.config,
            "research",
            "research question",
            h.temp.path(),
            Some(16),
            empty(),
            &mut out
        )
        .unwrap(),
        1
    );
    assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
    let ledger = std::fs::read_to_string(store.join("reports/_stages.evidence.tsv")).unwrap();
    assert!(ledger.contains("astra\tgpt-6-astra\t0\t"));
    let cmd = h.config.codex.as_mut().unwrap();
    let i = cmd.iter().position(|s| s == "--scenario").unwrap();
    cmd[i + 1] = "planner".into();
    assert_eq!(
        resume(&h.config, "thread-1", None, empty(), &mut Vec::new()).unwrap(),
        0
    );
    assert_eq!(
        h.requests("thread/start").iter().filter(|p| p["model"] != "gpt-6-terra").count(),
        2,
        "requests: {} goal: {:?}",
        h.records(),
        h.goal()
    );
    let resumed = h.requests("thread/resume");
    assert_eq!(
        resumed
            .iter()
            .map(|p| p["threadId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["thread-1"]
    );
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(h.config.home.join("sessions/thread-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["skill"], "research");
    assert_eq!(metadata["max_turns"], 16);
    assert_eq!(h.goal().wish, "research question");
    let turns:Vec<_> = h.requests("turn/start").into_iter().filter(|p| p["effort"] != "high").collect();
    assert!(turns[0]["input"][1]["text"]
        .as_str()
        .unwrap()
        .contains("# Research"));
    assert_ne!(turns[0]["threadId"], turns[1]["threadId"]);
}
#[test]
fn luna_content_has_reads_exact_outputs_and_no_arbitrary_nesting() {
    let mut h = Harness::new("authority");
    let output = h.temp.path().join("authorized.artifact");
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "explicit document artifact"
    );
    assert!(!PathBuf::from(format!("{}.other", output.display())).exists());
    assert!(!PathBuf::from(format!("{}.core", output.display())).exists());
    let threads = h.requests("thread/start");
    assert_eq!(threads[2]["model"], "gpt-6-luna");
    assert_eq!(threads[2]["sandbox"], "read-only");
    let tools = threads[2]["dynamicTools"].as_array().unwrap();
    for name in ["Read", "Glob", "Grep", "Write"] {
        assert!(tools.iter().any(|t| t["name"] == name));
    }
    assert!(!tools
        .iter()
        .any(|t| t["name"] == "Agent" || t["name"].as_str().unwrap().ends_with("codex_run")));
    let failures = h.records()["responses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["result"]["success"] == false)
        .count();
    assert!(
        failures >= 4,
        "expected core-write, core-codex, escaped content write and nesting denials"
    );
}
#[test]
fn classified_stop_cancels_root_and_outstanding_child_calls() {
    let h = Harness::new("cancel");
    let (tx, rx) = std::sync::mpsc::channel();
    let running = start_idle(&h, Box::new(BufReader::new(ChannelInput(rx))));
    // An outstanding child is the precondition, not an elapsed 350 ms. A slow
    // runner may otherwise receive the interjection before child dispatch,
    // correctly refuse to launch it, and have nothing to interrupt.
    wait_file(&marker(&h, "cancel-child-ready"));
    tx.send("stop\n".into()).unwrap();
    drop(tx);
    let (code, out) = running.join().unwrap();
    assert_eq!(code, 1, "{out}");
    assert_eq!(h.goal().reason.as_deref(), Some("user_stop"));
    let interrupts = h.requests("turn/interrupt");
    assert!(interrupts.iter().any(|r| r["threadId"] == "thread-1"));
    assert!(interrupts.iter().any(|r| r["threadId"] == "thread-2"));
}

struct StopWhenVerifierRuns(PathBuf, bool);
impl Read for StopWhenVerifierRuns {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1 {
            return Ok(0);
        }
        wait_for_verifier(&self.0);
        self.1 = true;
        let bytes = b"stop\n";
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}
fn wait_for_verifier(path: &Path) -> Vec<u32> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(ids) = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()
            })
            .filter(|ids| ids.len() == 2)
        {
            return ids;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "verifier never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn stop_and_ctrl_c_kill_verifier_tree_after_worker_model_completes() {
    for scenario in ["verify-stop", "verify-interrupt"] {
        let mut h = Harness::new(scenario);
        let wrapper = h.temp.path().join("verifier_fake.py");
        let source = serde_json::to_string(
            &common::repo_root()
                .join("tests/fakes/fake_codex_session.py")
                .to_string_lossy(),
        )
        .unwrap();
        std::fs::write(&wrapper, format!("from pathlib import Path\np={source}\ns=Path(p).read_text(encoding='utf-8').replace(\"'workspaceWrite' if scenario=='unsafe-sandbox' else 'readOnly'\",\"'workspaceWrite' if scenario=='unsafe-sandbox' or params.get('sandbox')=='workspace-write' else 'readOnly'\")\ns=s.replace(\"    elif method in ('thread/start','thread/resume'):\",\"    elif method=='skills/list': reply(msg,{{'data':[{{'cwd':params['cwds'][0],'errors':[],'skills':[]}}]}})\\n    elif method in ('thread/start','thread/resume'):\")\nexec(compile(s,p,'exec'))\n")).unwrap();
        h.config.codex.as_mut().unwrap()[2] = wrapper.to_string_lossy().into_owned();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(h.temp.path())
            .status()
            .unwrap()
            .success());
        std::fs::write(
            h.temp.path().join(".git/info/exclude"),
            "home/\nrecord.json\nverifier-pids\nverifier.py\n",
        )
        .unwrap();
        let store = axiom::orch::store_for(&h.config.home, "demo").unwrap();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir(store.join("inbox")).unwrap();
        let pids = h.temp.path().join("verifier-pids");
        let script = h.temp.path().join("verifier.py");
        std::fs::write(&script, "import os,sys,time,subprocess\nfrom pathlib import Path\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])\nPath(sys.argv[1]).write_text(f'{os.getpid()} {p.pid}')\ntime.sleep(60)\n").unwrap();
        let brief = store.join("u.brief");
        std::fs::write(
            &brief,
            format!(
                "GOAL build\nSCOPE input.txt\nVERIFY \"{}\" \"{}\" \"{}\"\n",
                common::python(),
                script.display(),
                pids.display()
            ),
        )
        .unwrap();
        std::fs::write(
            store.join("units.tsv"),
            format!(
                "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tm\tpending\t\t\t\t{}\n",
                brief.display()
            ),
        )
        .unwrap();
        let trigger = if scenario == "verify-interrupt" {
            let pids = pids.clone();
            Some(std::thread::spawn(move || {
                wait_for_verifier(&pids);
                axiom::session::trigger_interrupt();
            }))
        } else {
            None
        };
        let input = if scenario == "verify-stop" {
            Box::new(BufReader::new(StopWhenVerifierRuns(pids.clone(), false)))
                as Box<dyn BufRead + Send>
        } else {
            empty()
        };
        let (code, out) = h.execute(input, None);
        if let Some(trigger) = trigger {
            trigger.join().unwrap();
        }
        assert_eq!(
            code,
            if scenario == "verify-stop" { 1 } else { 130 },
            "{out}"
        );
        assert_eq!(h.goal().status, "blocked");
        for pid in wait_for_verifier(&pids) {
            assert!(
                !axiom::base::proc::pid_alive(pid),
                "{scenario}: verifier {pid} survived"
            );
        }
        let report: Value =
            serde_json::from_slice(&std::fs::read(store.join("reports/u.json")).unwrap()).unwrap();
        assert_eq!(
            report["status"], "completed",
            "worker had finished before cancellation"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let evidence = axiom::orch::evidence(&store, "u").unwrap();
            if evidence
                .iter()
                .any(|r| r.kind == "verify" && r.exit_code != 0)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing cancelled VERIFY evidence"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            axiom::orch::read_units(&store).unwrap()[0].state,
            "verified"
        );
    }
}
#[test]
fn local_log_stamp() {
    assert_eq!(
        axiom::session::log_stamp(1793312048, 28800),
        "20261030-061408"
    );
    assert_eq!(axiom::session::log_stamp(1793312048, 0), "20261029-221408");
}

#[test]
fn tool_iterations_inside_one_turn_are_bounded_and_resumable() {
    for scenario in ["tool-loop", "native-loop"] {
        let h = Harness::new(scenario);
        let (code, out) = h.execute(empty(), Some(3));
        assert_eq!(code, 1, "{scenario}: {out}");
        assert_eq!(h.requests("turn/start").len(), 1);
        assert!(out.contains("maximum turns reached"), "{out}");
        assert!(h
            .requests("turn/interrupt")
            .iter()
            .any(|p| p["threadId"] == "thread-1"));
        assert_eq!(h.goal().reason.as_deref(), Some("interrupted"));
        assert!(out.contains("resume: axiom wish --resume thread-1"));
        if scenario == "tool-loop" {
            assert_eq!(
                h.records()["responses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|r| r["result"]["success"] == true)
                    .count(),
                2
            );
        }
    }
}
#[test]
fn workflow_templates_expand_at_load_without_rewriting_user_text() {
    let h = Harness::new("ok");
    let text = "verbatim $ARGUMENTS ${CLAUDE_PLUGIN_ROOT} mcp__keep_this";
    assert_eq!(
        run(
            &h.config,
            "wish",
            text,
            h.temp.path(),
            None,
            empty(),
            &mut Vec::new()
        )
        .unwrap(),
        0
    );
    let turns = h.requests("turn/start");
    let contract = turns[0]["input"][1]["text"].as_str().unwrap();
    assert!(contract.contains(&format!("Wish: {text}")));
    assert!(contract.contains(&format!(
        "{}/research/audit.schema.json",
        h.config
            .home
            .join("engine")
            .join(env!("CARGO_PKG_VERSION"))
            .display()
    )));
    assert!(!contract.contains("disable-model-invocation:"));
}
#[test]
fn dispatcher_can_delegate_only_root_authorized_content_outputs() {
    let mut h = Harness::new("delegated-authority");
    let output = h.temp.path().join("image-description.md");
    h.arg("--output", &output);
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}"); // Explicit blocked end, not a forged engineering completion.
    assert_eq!(
        std::fs::read_to_string(&output).unwrap(),
        "explicit document artifact"
    );
    assert!(!PathBuf::from(format!("{}.other", output.display())).exists());
    let threads = h.requests("thread/start");
    assert_eq!(threads.len(), 3);
    assert_eq!(threads[1]["model"], "gpt-6-terra");
    assert_eq!(threads[2]["model"], "gpt-6-luna");
    assert_eq!(threads[2]["config"]["features"]["view_image"], true);
    let log = std::fs::read_to_string(&h.logs()[0]).unwrap();
    assert!(log.contains("imageView"));
    assert!(log.contains("content output requires exact root authorization"));
}

#[test]
fn core_cannot_write_plans_or_directly_call_specialists() {
    let mut h=Harness::new("planning-policy");
    h.config.codex.as_mut().unwrap()[2]=common::repo_root().join("tests/session/planning_fake.py").to_string_lossy().into_owned();
    let output=h.temp.path().join("artifact.md");h.arg("--output",&output);
    let (code,out)=h.execute(empty(),None);assert_eq!(code,0,"{out}");
    let core:Value=serde_json::from_slice(&std::fs::read(format!("{}.core",h.record.display())).unwrap()).unwrap();
    assert_eq!(core["mode"]["success"],true);
    for key in ["inside","outside","architect","content_before","content_after","dispatch"] {
        assert_eq!(core[key]["success"],false,"{key}: {core}");
    }
    assert!(!h.config.home.join("core-plan.md").exists());
    assert!(!h.config.home.join("architect-plan.md").exists());
    assert!(!output.exists());
    assert_eq!(h.requests("thread/start").len(),1);
}

#[test]
fn readonly_workflows_ignore_old_units_without_weakening_change_gates() {
    for skill in ["research", "wish"] {
        let h = Harness::new("ok");
        let project = axiom::base::slug_for_dir(h.temp.path());
        // Pre-existing engineering work in this project is unrelated to a research plan.
        let store = axiom::orch::store_for(&h.config.home, &project).unwrap();
        assert_eq!(
            axiom::orch::run(&store, &["init".into()], false, false).exit_code,
            0
        );
        let brief = store.join("brief.md");
        std::fs::write(&brief, "GOAL pending\nSCOPE src/\nVERIFY cargo test\n").unwrap();
        assert_eq!(
            axiom::orch::run(
                &store,
                &[
                    "unit".into(),
                    "add".into(),
                    "pending".into(),
                    "--track".into(),
                    "build".into(),
                    "--brief".into(),
                    brief.to_string_lossy().into_owned()
                ],
                false,
                false
            )
            .exit_code,
            0
        );
        let mut output = Vec::new();
        let code = run(
            &h.config,
            skill,
            "question",
            h.temp.path(),
            Some(8),
            empty(),
            &mut output,
        )
        .unwrap();
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&output));
        assert_eq!(h.goal().status, "done");
        assert_eq!(h.goal().mode.as_deref(), Some("answer"));
        assert_eq!(h.requests("turn/start").len(), 1);
        if skill == "research" {
            let turns = h.requests("turn/start");
            let workflow = turns[0]["input"][1]["text"].as_str().unwrap();
            assert!(workflow.contains("Declare goal mode answer"));
            assert!(workflow.contains("plan-only"));
        }
        axiom::goal::open(
            &h.config.home,
            "change-control",
            "implement",
            h.temp.path().to_str(),
        )
        .unwrap();
        axiom::goal::set_mode(&h.config.home, "change-control", "change").unwrap();
        assert_eq!(
            axiom::goal::set_status(&h.config.home, "change-control", "done", None)
                .unwrap_err()
                .kind,
            "not_done"
        );
    }
}

#[test]
fn host_binds_checkpoint_authority_to_caller_and_blocks_direct_state_writes() {
    let mut h = Harness::new("checkpoint-authority");
    let wrapper = h.temp.path().join("authority.py");
    let source_path = serde_json::to_string(
        &common::repo_root()
            .join("tests/fakes/fake_codex_session.py")
            .to_string_lossy(),
    )
    .unwrap();
    let probe = r#"    if scenario == 'checkpoint-authority':
        core = 'You are Chronos core' in threads[tid].get('developerInstructions','')
        home = Path(option('--home'))
        results = {}
        if core:
            results['mode'] = tool(tid,turn,'goal',{'mode':'change'})
            results['plan'] = call(tid,turn,'Write',{'file_path':str(home/'orchestrate'/'demo'/'planning.md'),'content':'plan'})
            results['init'] = tool(tid,turn,'orch',{'project':'demo','args':['init']})
            results['enroll'] = tool(tid,turn,'orch',{'project':'demo','args':['wait','enroll']})
            results['child'] = call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'probe checkpoint authority','project':'demo','cwd':option('--project')})
            results['blocked'] = tool(tid,turn,'goal',{'status':'blocked','note':'authority probe complete'})
        else:
            repo = option('--project')
            brief = str(home/'orchestrate'/'demo'/'brief.md')
            results['brief'] = call(tid,turn,'Write',{'file_path':brief,'content':'GOAL test\nSCOPE a.txt\nOUTPUTS a.txt\nDELIVERY implementation\nVERIFY echo ok\n'})
            results['unit'] = tool(tid,turn,'orch',{'project':'demo','args':['unit','add','t','--track','build','--brief',brief]})
            prepare = ['wait','prepare','--task','t','--repo',repo,'--issue','issue','--proposal','proposal','--min-rounds','3']
            results['unenrolled'] = tool(tid,turn,'orch',{'project':'other','args':prepare})
            results['prepare'] = tool(tid,turn,'orch',{'project':'demo','args':prepare})
            results['unit_default'] = tool(tid,turn,'orch',{'project':'demo','args':['unit','add','default','--track','build','--brief',brief]})
            results['prepare_default'] = tool(tid,turn,'orch',{'project':'demo','args':['--json','wait','prepare','--task','default','--repo',repo,'--issue','issue','--proposal','proposal']})
            results['policy_change'] = tool(tid,turn,'orch',{'project':'demo','args':prepare[:-1]+['2']})
            results['bad_option'] = tool(tid,turn,'orch',{'project':'demo','args':prepare+['--min-rounds','0']})
            for action in ('verdict','complete','recover','checkpoint','enroll'):
                results[action] = tool(tid,turn,'orch',{'project':'demo','args':['wait',action,'--task','t']})
            results['malformed'] = tool(tid,turn,'orch',{'project':'demo','args':['wait',None,'event','--file','x']})
            for flags in ([], ['--json'], ['--force','--json']):
                results[str(flags)] = tool(tid,turn,'orch',{'project':'demo','args':flags+['wait','event','--file','does-not-exist']})
            results['write'] = call(tid,turn,'Write',{'file_path':str(home/'orchestrate'/'demo'/'core-wait.json'),'content':'{"forged":true}'})
            results['relative'] = call(tid,turn,'Write',{'file_path':'core-wait.json','content':'{"forged":true}'})
        Path(option('--record')+'.authority-'+('core' if core else 'child')).write_text(json.dumps(results),encoding='utf-8')
        complete(tid,turn,'authority probe')
        return
"#;
    std::fs::write(&wrapper, format!("from pathlib import Path\np={source_path}\ns=Path(p).read_text(encoding='utf-8')\ns=s.replace(\"    elif method in ('thread/start','thread/resume'):\",\"    elif method=='skills/list': reply(msg,{{'data':[{{'cwd':params['cwds'][0],'errors':[],'skills':[]}}]}})\\n    elif method in ('thread/start','thread/resume'):\")\ns=s.replace(\"    role=threads[tid].get('model','')\",{}+\"    role=threads[tid].get('model','')\")\nexec(compile(s,p,'exec'))\n",serde_json::to_string(probe).unwrap())).unwrap();
    h.config.codex.as_mut().unwrap()[2] = wrapper.to_string_lossy().into_owned();
    let (code, out) = h.execute(empty(), None);
    assert_eq!(code, 1, "{out}");
    let child: Value = serde_json::from_slice(
        &std::fs::read(format!("{}.authority-child", h.record.display())).unwrap(),
    )
    .unwrap();
    for name in ["brief", "unit", "prepare", "unit_default", "prepare_default"] {
        assert_eq!(child[name]["success"], true, "{name}: {}", child[name]);
    }
    for name in ["policy_change", "bad_option"] {
        assert_eq!(child[name]["success"], false, "{name}: {}", child[name]);
        assert!(child[name].to_string().contains("orch_failed"), "{}", child[name]);
    }
    for (name, value) in child.as_object().unwrap().iter().filter(|(name, _)| !["brief", "unit", "prepare", "unit_default", "prepare_default", "policy_change", "bad_option"].contains(&name.as_str())) {
        assert_eq!(value["success"], false, "{name}: {value}");
        assert!(
            value.to_string().contains("permission_denied"),
            "{name}: {value}"
        );
    }
    let state = axiom::orch::wait_state(&h.config.home.join("orchestrate/demo")).unwrap();
    assert_eq!(state.checkpoints.len(), 2);
    assert_eq!(axiom::orch::current_checkpoint(&state, "t").unwrap().min_rounds, 3);
    assert_eq!(axiom::orch::current_checkpoint(&state, "default").unwrap().min_rounds, 5);
    assert!(state.events.is_empty());
}
