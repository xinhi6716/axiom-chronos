mod common;

use axiom::{
    engine,
    provenance::{self, collect, run_bounded, version_of, Provenance, Sources, UNKNOWN},
};
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn committed_repo(dir: &Path) -> String {
    git(dir, &["init", "-q"]);
    fs::write(dir.join("tracked.txt"), "one").unwrap();
    git(dir, &["add", "tracked.txt"]);
    git(dir, &["commit", "-q", "-m", "first"]);
    git(dir, &["rev-parse", "HEAD"])
}
fn of_project(dir: &Path) -> Provenance {
    collect(Sources {
        project: Some(dir),
        ..Sources::default()
    })
}
fn python(script: &str) -> Vec<String> {
    vec![common::python(), "-I".into(), "-c".into(), script.into()]
}

#[test]
fn commit_and_tree_state_come_from_the_project() {
    let temp = common::temp();
    let head = committed_repo(temp.path());
    let clean = of_project(temp.path());
    assert_eq!(clean.git_commit, head);
    assert_eq!(clean.git_tree, "clean");

    fs::write(temp.path().join("tracked.txt"), "two").unwrap();
    let modified = of_project(temp.path());
    assert_eq!(modified.git_commit, head);
    assert_eq!(modified.git_tree, "dirty");

    git(temp.path(), &["checkout", "-q", "--", "tracked.txt"]);
    assert_eq!(of_project(temp.path()).git_tree, "clean");
    fs::write(temp.path().join("untracked.txt"), "new").unwrap();
    assert_eq!(of_project(temp.path()).git_tree, "dirty");

    let sub = temp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    assert_eq!(of_project(&sub).git_commit, head);
}

#[test]
fn a_project_outside_git_is_unknown_not_clean() {
    let temp = common::temp();
    let p = of_project(temp.path());
    assert_eq!(p.git_commit, UNKNOWN);
    assert_eq!(p.git_tree, UNKNOWN);
}

#[test]
fn a_repository_with_no_commit_has_an_unknown_commit() {
    let temp = common::temp();
    git(temp.path(), &["init", "-q"]);
    let p = of_project(temp.path());
    assert_eq!(p.git_commit, UNKNOWN);
    assert_eq!(p.git_tree, "clean");
}

#[test]
fn a_project_directory_that_is_gone_is_unknown() {
    let temp = common::temp();
    let p = of_project(&temp.path().join("deleted"));
    assert_eq!(p.git_commit, UNKNOWN);
    assert_eq!(p.git_tree, UNKNOWN);
}

#[test]
fn nothing_supplied_records_only_what_the_binary_knows_about_itself() {
    let p = collect(Sources::default());
    assert_eq!(p.binary_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(p.engine_build, engine::build_id());
    for value in [
        &p.binary_build,
        &p.plugin_version,
        &p.plugin_rules,
        &p.claude_version,
        &p.codex_version,
        &p.git_commit,
        &p.git_tree,
    ] {
        assert_eq!(value, UNKNOWN);
    }
}

#[test]
fn versions_are_read_from_the_command_and_never_assumed() {
    let temp = common::temp();
    let codex = |command: Vec<String>| {
        collect(Sources {
            codex: Some(&command),
            ..Sources::default()
        })
        .codex_version
    };
    assert_eq!(codex(common::fake("fake_codex")), "codex-cli fake");
    let missing = temp.path().join("no-such-codex");
    assert_eq!(codex(vec![missing.to_string_lossy().into_owned()]), UNKNOWN);
    assert_eq!(codex(python("import sys; sys.exit(3)")), UNKNOWN);
    assert_eq!(codex(python("pass")), UNKNOWN);
    assert_eq!(
        version_of(&python(
            "import sys; print('warning: old node'); print(); print('tool 1.2'); print()"
        ))
        .as_deref(),
        Some("tool 1.2")
    );
    assert_eq!(
        version_of(&python("import sys; print('tool 3.4', file=sys.stderr)")).as_deref(),
        Some("tool 3.4")
    );
    assert_eq!(version_of(&[]), None);
}

#[test]
fn a_program_that_hangs_is_killed_at_the_limit() {
    let start = Instant::now();
    let output = run_bounded(
        &python("import time; time.sleep(30)"),
        None,
        Duration::from_millis(300),
    );
    assert!(output.is_none());
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_background_process_holding_the_streams_cannot_stall_the_probe() {
    let start = Instant::now();
    let output = run_bounded(
        &python("import subprocess, sys; subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(20)'], close_fds=False)"),
        None,
        Duration::from_secs(2),
    );
    assert!(output.is_none());
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn the_process_left_holding_the_streams_is_ended_with_the_probe() {
    let temp = common::temp();
    let pid_file = temp.path().join("holder.pid");
    let mut argv = python(
        "import subprocess, sys; p = subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(30)'], stdin=subprocess.DEVNULL, close_fds=False); open(sys.argv[1], 'w').write(str(p.pid))",
    );
    argv.push(pid_file.to_string_lossy().into_owned());
    let start = Instant::now();
    assert!(run_bounded(&argv, None, Duration::from_secs(2)).is_none());
    assert!(start.elapsed() < Duration::from_secs(10));
    let holder: u32 = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while axiom::base::proc::pid_alive(holder) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !axiom::base::proc::pid_alive(holder),
        "pid {holder} still holds the probe's streams"
    );
}

#[test]
fn captured_output_keeps_only_its_tail_up_to_the_cap() {
    let cap = axiom::base::proc::OUTPUT_CAP;
    let mut argv = python(
        "import sys; n = int(sys.argv[1]); sys.stdout.buffer.write(b'h' * 1000 + b'x' * n + b'END')",
    );
    argv.push(cap.to_string());
    let output = run_bounded(&argv, None, Duration::from_secs(60)).unwrap();
    assert!(output.success);
    assert_eq!(output.stdout.len(), cap);
    assert!(output.stdout.ends_with("xEND"));
    assert!(!output.stdout.contains('h'));
}

#[test]
fn binary_build_identifies_the_executable_itself() {
    let temp = common::temp();
    let build = |bytes: &[u8]| {
        let file = temp.path().join("axiom");
        fs::write(&file, bytes).unwrap();
        collect(Sources {
            binary: Some(&file),
            ..Sources::default()
        })
        .binary_build
    };
    let first = build(b"one build");
    assert_eq!(first.len(), 12);
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(build(b"one build"), first);
    assert_ne!(build(b"another build"), first);
    let missing = temp.path().join("gone");
    assert_eq!(
        collect(Sources {
            binary: Some(&missing),
            ..Sources::default()
        })
        .binary_build,
        UNKNOWN
    );
}

#[test]
fn plugin_version_and_rules_are_reported_as_found() {
    let temp = common::temp();
    let root = engine::plugin_dir(temp.path()).unwrap();
    let of_plugin = |root: &Path| {
        collect(Sources {
            plugin: Some(root),
            ..Sources::default()
        })
    };
    let matching = of_plugin(&root);
    assert_eq!(matching.plugin_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(matching.plugin_rules, "match");

    fs::write(root.join("DISPATCH.md"), "other rules").unwrap();
    assert_eq!(of_plugin(&root).plugin_rules, "differ");

    let manifest = root.join(".claude-plugin/plugin.json");
    fs::write(&manifest, r#"{"version":"0.3.0-codex"}"#).unwrap();
    let mismatched = of_plugin(&root);
    assert_eq!(mismatched.plugin_version, "0.3.0-codex");
    assert_eq!(mismatched.plugin_rules, "differ");

    fs::write(&manifest, "{").unwrap();
    let unreadable = of_plugin(&root);
    assert_eq!(unreadable.plugin_version, UNKNOWN);
    assert_eq!(unreadable.plugin_rules, UNKNOWN);
}

#[test]
fn the_log_record_is_one_tagged_json_line() {
    let p = collect(Sources::default());
    let line = p.log_line();
    assert_eq!(line.matches('\n').count(), 1);
    assert!(line.ends_with('\n'));
    let parsed: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(parsed["type"], "provenance");
    assert_eq!(parsed["provenance"], p.to_value());
}

fn fake_json(name: &str) -> String {
    serde_json::to_string(&common::fake(name)).unwrap()
}
fn wish(temp: &Path, project: &Path) -> (std::process::Output, Value) {
    let home = temp.join("home");
    let output = Command::new(env!("CARGO_BIN_EXE_axiom"))
        .args(["wish", "--project"])
        .arg(project)
        .arg("check the record")
        .env("AXIOM_HOME", &home)
        .env("AXIOM_DATA_HOME", &home)
        .env("FAKE_CLAUDE_RECORD", temp.join("claude.json"))
        .env("AXIOM_CLAUDE_CMD", fake_json("fake_claude"))
        .env("AXIOM_CODEX_CMD", fake_json("fake_codex"))
        .env_remove("ORCH_STORE")
        .env_remove(provenance::ENV)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = fs::read_dir(home.join("logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let first = fs::read_to_string(log)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    (output, serde_json::from_str(&first).unwrap())
}

#[test]
fn a_real_binary_run_records_every_field_it_can_establish() {
    let temp = common::temp();
    let project = temp.path().join("project");
    fs::create_dir(&project).unwrap();
    let head = committed_repo(&project);
    fs::write(project.join("wip.txt"), "unsaved").unwrap();
    let (output, record) = wish(temp.path(), &project);
    let block = &record["provenance"];
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(block["binary_version"], version);
    assert_eq!(
        block["binary_build"].as_str().unwrap(),
        engine::binary_id(Path::new(env!("CARGO_BIN_EXE_axiom"))).unwrap()
    );
    assert_eq!(block["engine_build"], engine::build_id());
    assert_eq!(block["plugin_version"], version);
    assert_eq!(block["plugin_rules"], "match");
    assert_eq!(block["claude_version"], "2.1.281");
    assert_eq!(block["codex_version"], "codex-cli fake");
    assert_eq!(block["git_commit"], head.as_str());
    assert_eq!(block["git_tree"], "dirty");
    let text = String::from_utf8_lossy(&output.stdout);
    let shown = text
        .lines()
        .find_map(|l| l.strip_prefix("provenance: "))
        .unwrap();
    assert!(shown.contains(&head), "{shown}");
    assert!(shown.contains("(dirty)"), "{shown}");
}

fn bridge_session(temp: &Path, extra_env: &[(&str, String)]) -> Value {
    use std::io::Write;
    let session = "bridge-session";
    let project = temp.join("project");
    fs::create_dir_all(&project).unwrap();
    axiom::goal::open(temp, session, "wish", project.to_str()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
    command
        .arg("bridge")
        .env("AXIOM_DATA_HOME", temp)
        .env("AXIOM_CODEX_CMD", fake_json("fake_codex"))
        .env_remove(provenance::ENV)
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    let input = format!(
        "{}\n{}\n",
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"claude-code","version":"2.1.311"}}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"goal","arguments":{"session":session,"mode":"change"}}}),
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let reply = text
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["id"] == 2)
        .unwrap();
    reply["result"]["structuredContent"]["goal"]["provenance"].clone()
}

#[test]
fn the_bridge_of_a_plugin_session_records_the_plugin_it_serves() {
    let temp = common::temp();
    let root = engine::plugin_dir(&temp.path().join("plugin-home")).unwrap();
    let records = bridge_session(
        temp.path(),
        &[("CLAUDE_PLUGIN_ROOT", root.to_string_lossy().into_owned())],
    );
    let block = &records[0];
    assert_eq!(records.as_array().unwrap().len(), 1);
    assert_eq!(block["claude_version"], "2.1.311");
    assert_eq!(block["plugin_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(block["plugin_rules"], "match");
    assert_eq!(block["codex_version"], "codex-cli fake");
}

#[test]
fn a_launchers_record_reaches_the_goal_unchanged() {
    let temp = common::temp();
    let launched = r#"{"binary_version":"from-launcher","git_tree":"clean"}"#;
    let records = bridge_session(temp.path(), &[(provenance::ENV, launched.to_owned())]);
    assert_eq!(records.as_array().unwrap().len(), 1);
    assert_eq!(records[0], serde_json::from_str::<Value>(launched).unwrap());
}

#[test]
fn a_malformed_launcher_record_is_ignored_for_a_fresh_one() {
    let temp = common::temp();
    let records = bridge_session(temp.path(), &[(provenance::ENV, "[1,2]".to_owned())]);
    assert_eq!(records[0]["binary_version"], env!("CARGO_PKG_VERSION"));
}
