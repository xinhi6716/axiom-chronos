mod common;
use axiom::{
    goal,
    session::{self, trigger_interrupt, Config, CONTINUE_TAG},
};
use serde_json::Value;
use std::{
    io::{BufRead, Cursor, Write},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

static ENV_LOCK: Mutex<()> = Mutex::new(());
const SESSION: &str = "12345678-1234-1234-1234-123456789abc";

struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvRestore {
    fn new() -> Self {
        Self(
            [
                "FAKE_CLAUDE_RECORD",
                "FAKE_CLAUDE_SCENARIO",
                "FAKE_CLAUDE_SESSION",
                "FAKE_CLAUDE_PIDS",
                "FAKE_CLAUDE_BATCHES",
            ]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect(),
        )
    }
}
impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            if let Some(v) = v {
                std::env::set_var(k, v);
            } else {
                std::env::remove_var(k);
            }
        }
    }
}
fn empty() -> Box<dyn BufRead + Send> {
    Box::new(Cursor::new(Vec::<u8>::new()))
}

struct Fixture {
    temp: tempfile::TempDir,
    config: Config,
}
impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let config = Config {
            claude: Ok(common::fake("fake_claude")),
            home: temp.path().join("home"),
            bin: temp.path().join("axiom"),
            clock: axiom::session::Clock::system(),
        };
        std::env::set_var("FAKE_CLAUDE_RECORD", temp.path().join("record"));
        std::env::set_var("FAKE_CLAUDE_SCENARIO", "batch");
        std::env::set_var("FAKE_CLAUDE_BATCHES", "[]");
        std::env::remove_var("FAKE_CLAUDE_SESSION");
        goal::open(&config.home, SESSION, "wish", temp.path().to_str()).unwrap();
        Self { temp, config }
    }
    fn batch(&self, name: &str, pid: u32) -> goal::Batch {
        let file = |suffix: &str| {
            self.temp
                .path()
                .join(format!("{name}.{suffix}"))
                .to_string_lossy()
                .into_owned()
        };
        goal::Batch {
            progress_file: file("progress"),
            marker_file: file("done"),
            pid,
        }
    }
    fn plan(&self, batches: &[&goal::Batch]) {
        std::env::set_var(
            "FAKE_CLAUDE_BATCHES",
            serde_json::to_string(batches).unwrap(),
        );
    }
    fn wait_on(&self, batch: &goal::Batch) {
        goal::set_status_with_batch(
            &self.config.home,
            SESSION,
            "waiting",
            Some("screening 7800 papers"),
            Some(batch.clone()),
        )
        .unwrap();
    }
    fn run(&self, out: &mut dyn Write) -> i32 {
        session::run(
            &self.config,
            "wish",
            "wish",
            self.temp.path(),
            Some(9),
            empty(),
            out,
        )
        .unwrap()
    }
    fn record(&self, run: usize) -> Value {
        let path = self.temp.path().join(format!("record.{run}"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
    fn ran(&self, run: usize) -> bool {
        self.temp.path().join(format!("record.{run}")).exists()
    }
    fn prompt(&self, run: usize) -> String {
        self.record(run)["messages"][0]["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn resumed(&self, run: usize) -> bool {
        let record = self.record(run);
        let argv = record["argv"].as_array().unwrap();
        argv.iter()
            .position(|v| v == "--resume")
            .is_some_and(|i| argv[i + 1] == SESSION)
    }
    fn goal(&self) -> goal::Goal {
        goal::load(&self.config.home, SESSION).unwrap().unwrap()
    }
    fn logs(&self) -> usize {
        std::fs::read_dir(self.config.home.join("logs")).map_or(0, |entries| entries.count())
    }
}
fn dead_pid() -> u32 {
    let mut child = Command::new(common::python())
        .args(["-I", "-c", ""])
        .spawn()
        .unwrap();
    child.wait().unwrap();
    child.id()
}
fn locked() -> (std::sync::MutexGuard<'static, ()>, EnvRestore) {
    (
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
        EnvRestore::new(),
    )
}

#[test]
fn headless_wish_waits_on_the_batch_then_resumes_with_what_happened() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", std::process::id());
    std::fs::write(&b.progress_file, "7/10\n").unwrap();
    std::fs::write(&b.marker_file, "").unwrap();
    f.plan(&[&b]);
    let mut out = Vec::new();
    assert_eq!(f.run(&mut out), 0);
    let output = String::from_utf8(out).unwrap();
    for expected in [
        "goal waiting: batch running",
        "batch: 7/10",
        "batch finished",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert!(!f.resumed(0));
    assert!(f.resumed(1));
    let record = f.record(1);
    let argv = record["argv"].as_array().unwrap();
    let turns = argv.iter().position(|v| v == "--max-turns").unwrap();
    assert_eq!(argv[turns + 1], "9");
    let prompt = f.prompt(1);
    assert!(prompt.starts_with(CONTINUE_TAG), "{prompt}");
    for fact in [
        "batch running",
        b.progress_file.as_str(),
        b.marker_file.as_str(),
        "the batch finished",
    ] {
        assert!(prompt.contains(fact), "{fact}: {prompt}");
    }
    assert!(!prompt.contains("failure"), "{prompt}");
    assert_eq!(f.goal().status, "done");
    assert_eq!(f.logs(), 2);
}

#[test]
fn a_batch_that_died_is_resumed_as_a_failure() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", dead_pid());
    f.plan(&[&b]);
    let mut out = Vec::new();
    assert_eq!(f.run(&mut out), 0);
    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("batch died without its completion marker"),
        "{output}"
    );
    assert!(f.resumed(1));
    let prompt = f.prompt(1);
    for fact in [
        "died without writing",
        "Treat this as a failure",
        b.marker_file.as_str(),
    ] {
        assert!(prompt.contains(fact), "{fact}: {prompt}");
    }
    assert!(!prompt.contains("the batch finished"), "{prompt}");
}

#[test]
fn resuming_a_waiting_goal_by_hand_says_what_it_was_waiting_on() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", std::process::id());
    f.wait_on(&b);
    let mut out = Vec::new();
    assert_eq!(
        session::resume(&f.config, SESSION, None, None, empty(), &mut out).unwrap(),
        0
    );
    assert!(f.resumed(0));
    let prompt = f.prompt(0);
    assert!(prompt.starts_with(CONTINUE_TAG), "{prompt}");
    for fact in [
        "screening 7800 papers",
        b.progress_file.as_str(),
        b.marker_file.as_str(),
        &b.pid.to_string(),
        "may still be running",
        "do not start it again",
    ] {
        assert!(prompt.contains(fact), "{fact}: {prompt}");
    }
}

#[test]
fn what_a_goal_waited_on_survives_a_crash_of_the_resumed_session() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", dead_pid());
    f.wait_on(&b);
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "crash");
    std::env::set_var("FAKE_CLAUDE_RECORD", f.temp.path().join("crashed"));
    assert_eq!(
        session::wait_and_resume(&f.config, SESSION, empty(), &mut Vec::new()).unwrap(),
        3
    );
    let crashed = f.goal();
    assert_eq!(crashed.status, "blocked");
    assert_eq!(crashed.reason.as_deref(), Some("interrupted"));
    let waited = crashed.waited.clone().unwrap();
    assert_eq!(waited.note, "screening 7800 papers");
    assert_eq!(waited.batch.as_ref(), Some(&b));
    assert_eq!(waited.end, Some(goal::BatchEnd::Died));
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "batch");
    std::env::set_var("FAKE_CLAUDE_RECORD", f.temp.path().join("record"));
    assert_eq!(
        session::resume(&f.config, SESSION, None, None, empty(), &mut Vec::new()).unwrap(),
        0
    );
    let prompt = f.prompt(0);
    for fact in [
        "screening 7800 papers",
        b.marker_file.as_str(),
        "Treat this as a failure",
    ] {
        assert!(prompt.contains(fact), "{fact}: {prompt}");
    }
}

struct InterruptOnProgress(Vec<u8>);
impl Write for InterruptOnProgress {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(bytes);
        if String::from_utf8_lossy(bytes).contains("batch: ") {
            trigger_interrupt();
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn ctrl_c_during_the_wait_leaves_the_goal_waiting_and_names_axiom_wait() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", std::process::id());
    std::fs::write(&b.progress_file, "1/10").unwrap();
    f.plan(&[&b]);
    let mut out = InterruptOnProgress(Vec::new());
    assert_eq!(f.run(&mut out), 130);
    let output = String::from_utf8(out.0).unwrap();
    assert!(
        output
            .lines()
            .any(|l| l == format!("goal waiting; continue with: axiom wait {SESSION}")),
        "{output}"
    );
    let g = f.goal();
    assert_eq!(g.status, "waiting");
    assert_eq!(g.batch.as_ref(), Some(&b));
    assert_eq!(g.note.as_deref(), Some("batch running"));
    assert!(g.waited.is_none());
    assert!(!f.ran(1));
    assert_eq!(f.logs(), 1);
}

#[test]
fn a_run_interrupted_while_the_goal_waits_does_not_start_watching() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", std::process::id());
    std::fs::write(&b.marker_file, "").unwrap();
    f.wait_on(&b);
    let pids = f.temp.path().join("pids");
    std::env::set_var("FAKE_CLAUDE_SCENARIO", "interrupt");
    std::env::set_var("FAKE_CLAUDE_PIDS", &pids);
    let trigger = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if std::fs::read_to_string(&pids).is_ok_and(|s| s.lines().last() == Some("ready")) {
                trigger_interrupt();
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("fake did not signal readiness");
    });
    let mut out = Vec::new();
    let code = f.run(&mut out);
    trigger.join().unwrap();
    assert_eq!(code, 130);
    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains(&format!("axiom wait {SESSION}")),
        "{output}"
    );
    assert!(!output.contains("batch finished"), "{output}");
    assert_eq!(f.goal().status, "waiting");
    assert_eq!(f.logs(), 1);
}

#[test]
fn each_batch_of_a_multi_stage_wish_is_waited_on() {
    let _env = locked();
    let f = Fixture::new();
    let (first, second) = (
        f.batch("one", std::process::id()),
        f.batch("two", std::process::id()),
    );
    for b in [&first, &second] {
        std::fs::write(&b.marker_file, "").unwrap();
    }
    f.plan(&[&first, &second]);
    let mut out = Vec::new();
    assert_eq!(f.run(&mut out), 0);
    let output = String::from_utf8(out).unwrap();
    assert_eq!(output.matches("batch finished").count(), 2, "{output}");
    assert!(f.prompt(1).contains(&first.marker_file));
    assert!(f.prompt(2).contains(&second.marker_file));
    assert!(!f.prompt(2).contains(&first.marker_file));
    assert_eq!(f.goal().status, "done");
    assert_eq!(f.logs(), 3);
}

#[test]
fn resuming_a_wish_also_waits_when_it_ends_waiting() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", dead_pid());
    f.plan(&[&b]);
    let mut out = Vec::new();
    assert_eq!(
        session::resume(&f.config, SESSION, None, None, empty(), &mut out).unwrap(),
        0
    );
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("batch died"), "{output}");
    assert!(f.resumed(0) && f.resumed(1));
    assert_eq!(f.goal().status, "done");
}

#[test]
fn a_goal_that_stopped_waiting_is_not_resumed_when_the_batch_ends() {
    let _env = locked();
    let f = Fixture::new();
    let b = f.batch("one", std::process::id());
    std::fs::write(&b.progress_file, "1/10").unwrap();
    f.wait_on(&b);
    struct TakeOver<'a>(&'a Fixture, &'a goal::Batch, Vec<u8>);
    impl Write for TakeOver<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.2.extend_from_slice(bytes);
            if String::from_utf8_lossy(bytes).contains("batch: ") {
                goal::reopen(&self.0.config.home, SESSION).unwrap();
                std::fs::write(&self.1.marker_file, "").unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = TakeOver(&f, &b, Vec::new());
    let code = session::wait_and_resume(&f.config, SESSION, empty(), &mut out).unwrap();
    assert_eq!(code, 0);
    let output = String::from_utf8(out.2).unwrap();
    assert!(output.contains("no longer waits on this batch"), "{output}");
    assert!(!f.ran(0));
    assert_eq!(f.logs(), 0);
    assert_eq!(f.goal().status, "active");
}

#[test]
fn axiom_wait_refuses_what_it_could_not_resume_before_waiting() {
    let _env = locked();
    let f = Fixture::new();
    let home = &f.config.home;
    let attempt = |config: &Config, id: &str| {
        session::wait_and_resume(config, id, empty(), &mut Vec::new()).unwrap_err()
    };
    assert_eq!(attempt(&f.config, "missing").kind, "not_found");
    let active = attempt(&f.config, SESSION);
    assert_eq!(active.kind, "invalid_argument");
    assert!(
        active.message.contains("is active, not waiting"),
        "{}",
        active.message
    );
    goal::set_status(home, SESSION, "waiting", Some("old style")).unwrap();
    let note_only = attempt(&f.config, SESSION);
    assert_eq!(note_only.kind, "invalid_argument");
    assert!(
        note_only
            .message
            .contains(&format!("axiom wish --resume {SESSION}")),
        "{}",
        note_only.message
    );
    let b = f.batch("one", std::process::id());
    f.wait_on(&b);
    let no_claude = Config {
        claude: Err(axiom::base::Failure::new("not_installed", "no claude")),
        home: home.clone(),
        bin: f.config.bin.clone(),
        clock: axiom::session::Clock::system(),
    };
    assert_eq!(attempt(&no_claude, SESSION).kind, "not_installed");
    goal::open(home, SESSION, "wish", None).unwrap();
    f.wait_on(&b);
    let no_cwd = attempt(&f.config, SESSION);
    assert_eq!(no_cwd.kind, "invalid_argument");
    assert!(
        no_cwd.message.contains("has no project directory"),
        "{}",
        no_cwd.message
    );
    assert_eq!(f.goal().status, "waiting");
    assert!(!f.ran(0));
}

fn axiom(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_axiom"));
    c.env("AXIOM_HOME", home).env("AXIOM_DATA_HOME", home);
    c
}

#[test]
fn wait_command_checks_its_arguments_and_the_goal() {
    let t = common::temp();
    for (args, message) in [
        (vec!["wait"], "axiom: wait takes only a session id"),
        (
            vec!["wait", "a", "b"],
            "axiom: wait takes only a session id",
        ),
        (
            vec!["wait", "missing"],
            "axiom: no goal for session missing",
        ),
    ] {
        let o = axiom(t.path()).args(args).output().unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(String::from_utf8_lossy(&o.stderr).trim_end(), message);
    }
    goal::open(t.path(), "s", "wish", None).unwrap();
    let o = axiom(t.path()).args(["wait", "s"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("is active, not waiting"));
}

#[test]
fn wait_command_follows_a_real_batch_and_resumes_the_session() {
    let t = common::temp();
    let project = t.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let progress = t.path().join("progress");
    let marker = t.path().join("done");
    let script = "import pathlib,sys,time; p,m=map(pathlib.Path,sys.argv[1:]); p.write_text('1/2'); time.sleep(1); p.write_text('2/2'); m.write_text('')";
    let mut child = Command::new(common::python())
        .args(["-I", "-c", script])
        .args([&progress, &marker])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    goal::open(t.path(), SESSION, "wish", project.to_str()).unwrap();
    goal::set_status_with_batch(
        t.path(),
        SESSION,
        "waiting",
        Some("the batch"),
        Some(goal::Batch {
            progress_file: progress.to_string_lossy().into_owned(),
            marker_file: marker.to_string_lossy().into_owned(),
            pid: child.id(),
        }),
    )
    .unwrap();
    let o = axiom(t.path())
        .args(["wait", SESSION])
        .env(
            "AXIOM_CLAUDE_CMD",
            serde_json::to_string(&common::fake("fake_claude")).unwrap(),
        )
        .env("FAKE_CLAUDE_RECORD", t.path().join("record"))
        .env("FAKE_CLAUDE_SCENARIO", "batch")
        .env("FAKE_CLAUDE_BATCHES", "[]")
        .env_remove("FAKE_CLAUDE_SESSION")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    child.wait().unwrap();
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(
        o.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    for expected in ["batch: 2/2", "batch finished"] {
        assert!(stdout.contains(expected), "{stdout}");
    }
    let v: Value =
        serde_json::from_slice(&std::fs::read(t.path().join("record.0")).unwrap()).unwrap();
    let argv = v["argv"].as_array().unwrap();
    let i = argv.iter().position(|a| a == "--resume").unwrap();
    assert_eq!(argv[i + 1], SESSION);
    let prompt = v["messages"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(
        prompt.contains("the batch finished") && prompt.contains(marker.to_str().unwrap()),
        "{prompt}"
    );
    assert_eq!(
        goal::load(t.path(), SESSION).unwrap().unwrap().status,
        "done"
    );
}
