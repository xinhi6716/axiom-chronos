mod common;

use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const NEED: u64 = 100;
const RESERVE: u64 = 10;

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    disk: PathBuf,
    record: PathBuf,
}

#[derive(Debug)]
struct Output {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let temp = common::temp();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        fs::create_dir_all(home.join("storage")).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::write(
            home.join("storage/policy.json"),
            json!({
                "reserve_bytes": RESERVE, "low_disk_bytes": 1, "estimates": {"wish": 20}
            })
            .to_string(),
        )
        .unwrap();
        let disk = temp.path().join("disk.json");
        let record = temp.path().join("record");
        Self {
            _temp: temp,
            home,
            project,
            disk,
            record,
        }
    }

    fn disk(&self, value: Value) {
        fs::write(&self.disk, value.to_string()).unwrap();
    }

    fn run(&self, skill: &str, need_args: &[&str], message: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
        command
            .env("AXIOM_HOME", &self.home)
            .env("AXIOM_DATA_HOME", &self.home)
            .env("AXIOM_FAKE_DISK", &self.disk)
            .env(
                "AXIOM_CLAUDE_CMD",
                serde_json::to_string(&common::fake("fake_claude")).unwrap(),
            )
            .env("FAKE_CLAUDE_RECORD", &self.record)
            .env("FAKE_CLAUDE_SCENARIO", "ok")
            .env("AXIOM_HEARTBEAT_MS", "100")
            .arg(skill)
            .arg("--project")
            .arg(&self.project)
            .args(need_args);
        if message {
            command.arg("test");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = axiom::base::proc::spawn_group(&mut command).unwrap();
        // `capture` drains both pipes, enforces a deadline, and cleans up the
        // process group even when an output holder outlives the child.
        let result = axiom::base::proc::capture(child, Duration::from_secs(30)).unwrap();
        let output = Output {
            code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
        };
        assert!(
            !result.timed_out && !result.holders_ended,
            "axiom did not finish cleanly: {output:?}"
        );
        output
    }
}

fn outcome(
    output: &Output,
    record: &Path,
    code: i32,
    started: bool,
    text: Option<(&str, bool)>,
) -> bool {
    if output.code != code || record.exists() != started {
        return false;
    }
    if let Some((needle, stderr)) = text {
        let stream = if stderr {
            &output.stderr
        } else {
            &output.stdout
        };
        if !String::from_utf8_lossy(stream).contains(needle) {
            return false;
        }
    }
    true
}

fn verify(output: &Output, record: &Path, code: i32, started: bool, text: Option<(&str, bool)>) {
    assert!(
        outcome(output, record, code, started, text),
        "{output:?}, record exists: {}",
        record.exists()
    );
}

// Actual contrasting executions must be rejected by the same outcome assertion.
fn negative(output: &Output, record: &Path, code: i32, started: bool, text: Option<(&str, bool)>) {
    assert!(
        !outcome(output, record, code, started, text),
        "negative control unexpectedly matched: {output:?}"
    );
}

#[test]
fn usage_errors() {
    for skill in ["wish", "research"] {
        for arg in [None, Some("bad"), Some("1X"), Some("-1")] {
            let f = Fixture::new();
            f.disk(json!({"free_bytes": 10000}));
            let args = match arg {
                Some(s) => vec!["--need-space", s],
                None => vec!["--need-space"],
            };
            let out = f.run(skill, &args, arg.is_some());
            verify(&out, &f.record, 2, false, Some(("--need-space", true)));
            negative(&out, &f.record, 0, true, None);
        }
    }
}

#[test]
fn zero_override_preserves_existing_exit_codes() {
    for (skill, code) in [("wish", 2), ("research", 1)] {
        let f = Fixture::new();
        f.disk(json!({"free_bytes": 10000}));
        let out = f.run(skill, &["--need-space", "0"], true);
        verify(&out, &f.record, code, false, Some(("zero override", true)));
        negative(&out, &f.record, 0, true, None);
    }
}

#[test]
fn enough_space_and_boundary() {
    for skill in ["wish", "research"] {
        for free in [10000, NEED + RESERVE, NEED + RESERVE - 1] {
            let f = Fixture::new();
            f.disk(json!({"free_bytes": free}));
            let out = f.run(skill, &["--need-space", "100"], true);
            if free >= NEED + RESERVE {
                verify(&out, &f.record, 0, true, None);
                negative(&out, &f.record, 75, false, Some(("paused", false)));
            } else {
                verify(&out, &f.record, 75, false, Some(("paused", false)));
                negative(&out, &f.record, 0, true, None);
            }
        }
    }
}

#[test]
fn project_and_home_are_checked_independently() {
    for skill in ["wish", "research"] {
        for project_short in [true, false] {
            let f = Fixture::new();
            let mut paths = serde_json::Map::new();
            // An overlapping ancestor must lose to the more specific prefix.
            paths.insert(f._temp.path().to_string_lossy().into_owned(), json!(10000));
            paths.insert(
                f.project.to_string_lossy().into_owned(),
                json!(if project_short { 109 } else { 10000 }),
            );
            paths.insert(
                f.home.to_string_lossy().into_owned(),
                json!(if project_short { 10000 } else { 109 }),
            );
            f.disk(json!({"free_bytes": 10000, "paths": paths}));
            let out = f.run(skill, &["--need-space", "100"], true);
            verify(&out, &f.record, 75, false, Some(("paused", false)));
            negative(&out, &f.record, 0, true, None);
        }
    }
}

#[test]
fn probe_errors_preserve_existing_exit_codes() {
    for (skill, code) in [("wish", 2), ("research", 1)] {
        let f = Fixture::new();
        let out = f.run(skill, &["--need-space", "100"], true);
        verify(&out, &f.record, code, false, Some(("axiom:", true)));
        negative(&out, &f.record, 0, true, None);
    }
}

#[test]
fn fake_disk_fallback_respects_path_components() {
    for skill in ["wish", "research"] {
        let f = Fixture::new();
        let misleading = f._temp.path().join("pro");
        let mut paths = serde_json::Map::new();
        paths.insert(misleading.to_string_lossy().into_owned(), json!(10000));
        f.disk(json!({"free_bytes": 109, "paths": paths}));
        let out = f.run(skill, &["--need-space", "100"], true);
        verify(&out, &f.record, 75, false, Some(("paused", false)));
        negative(&out, &f.record, 0, true, None);
    }
}

#[test]
fn malformed_fake_disk_and_policy_are_errors() {
    for (skill, code) in [("wish", 2), ("research", 1)] {
        let f = Fixture::new();
        let mut invalid_values = serde_json::Map::new();
        invalid_values.insert(f.project.to_string_lossy().into_owned(), json!("bad"));
        for disk in [
            "{".to_owned(),
            r#"{"free_bytes":1000,"paths":[]}"#.to_owned(),
            r#"{"free_bytes":1000,"paths":{"relative":10}}"#.to_owned(),
            json!({"free_bytes": 1000, "paths": invalid_values}).to_string(),
        ] {
            let f = Fixture::new();
            fs::write(&f.disk, disk).unwrap();
            let out = f.run(skill, &["--need-space", "100"], true);
            verify(&out, &f.record, code, false, Some(("axiom:", true)));
            negative(&out, &f.record, 0, true, None);
        }
        let f = Fixture::new();
        f.disk(json!({"free_bytes": 10000}));
        fs::write(f.home.join("storage/policy.json"), "{").unwrap();
        let out = f.run(skill, &["--need-space", "100"], true);
        verify(&out, &f.record, code, false, Some(("axiom:", true)));
        negative(&out, &f.record, 0, true, None);
    }
}

#[test]
fn negative_controls_reject_contrasting_executions() {
    for skill in ["wish", "research"] {
        let ready = Fixture::new();
        ready.disk(json!({"free_bytes": NEED + RESERVE}));
        let ready_out = ready.run(skill, &["--need-space", "100"], true);
        verify(&ready_out, &ready.record, 0, true, None);

        let blocked = Fixture::new();
        blocked.disk(json!({"free_bytes": NEED + RESERVE - 1}));
        let blocked_out = blocked.run(skill, &["--need-space", "100"], true);
        verify(
            &blocked_out,
            &blocked.record,
            75,
            false,
            Some(("paused", false)),
        );

        // A started run must not satisfy any blocked/error invariant, and a
        // blocked run must not satisfy the successful-session invariant.
        negative(
            &ready_out,
            &ready.record,
            75,
            false,
            Some(("paused", false)),
        );
        negative(
            &ready_out,
            &ready.record,
            2,
            false,
            Some(("--need-space", true)),
        );
        negative(
            &ready_out,
            &ready.record,
            if skill == "wish" { 2 } else { 1 },
            false,
            Some(("axiom:", true)),
        );
        negative(&blocked_out, &blocked.record, 0, true, None);
        // Isolate the record and message checks from the exit-code check.
        negative(&ready_out, &ready.record, 0, false, None);
        negative(
            &blocked_out,
            &blocked.record,
            75,
            false,
            Some(("not a pause message", false)),
        );
        let probe_error = Fixture::new();
        let probe_out = probe_error.run(skill, &["--need-space", "100"], true);
        negative(
            &probe_out,
            &probe_error.record,
            if skill == "wish" { 2 } else { 1 },
            false,
            Some(("--need-space", true)),
        );
    }
}
