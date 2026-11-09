mod common;

use axiom::{
    codegraph,
    engine::FILES,
    session::{command, Config, TOOLS},
};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const ABSENT: [(bool, bool); 3] = [(false, true), (true, false), (false, false)];

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    path: OsString,
}

#[cfg(windows)]
fn install_fake(bin: &Path) -> PathBuf {
    let path = bin.join("codegraph.cmd");
    fs::write(&path, "@exit /b 0\r\n").unwrap();
    path
}
#[cfg(unix)]
fn install_fake(bin: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = bin.join("codegraph");
    fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
fn fixture(executable: bool, index: bool) -> Fixture {
    let temp = common::temp();
    let root = temp.path().to_path_buf();
    let project = root.join("project");
    let bin = root.join("bin");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&bin).unwrap();
    if index {
        fs::create_dir(project.join(".codegraph")).unwrap();
    }
    if executable {
        install_fake(&bin);
    }
    Fixture {
        _temp: temp,
        root,
        project,
        path: bin.into_os_string(),
    }
}
// The child PATH holds only the fixture's bin directory, so the interpreter must be absolute.
fn stand_in(name: &str) -> String {
    let mut stand_in = common::fake(name);
    let output = Command::new(&stand_in[0])
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    stand_in[0] = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    serde_json::to_string(&stand_in).unwrap()
}
fn axiom(f: &Fixture, home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom"));
    command
        .env("PATH", &f.path)
        .env("AXIOM_HOME", home)
        .env("AXIOM_DATA_HOME", home);
    command
}

fn claude_argv(f: &Fixture) -> Vec<String> {
    let home = f.root.join("home");
    let record = f.root.join("claude.json");
    let output = axiom(f, &home)
        .args(["wish", "--project"])
        .arg(&f.project)
        .arg("describe this project")
        .env("AXIOM_CLAUDE_CMD", stand_in("fake_claude"))
        .env("FAKE_CLAUDE_RECORD", &record)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
    record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}
fn today(f: &Fixture) -> Vec<String> {
    let home = f.root.join("home");
    let config = Config {
        claude: Ok(vec!["claude".into()]),
        home: home.clone(),
        bin: PathBuf::new(),
    };
    let plugin = home.join("engine").join(env!("CARGO_PKG_VERSION"));
    let mut argv = command(&config, &plugin, None, false).unwrap();
    argv.remove(0);
    argv
}

#[test]
fn claude_session_gets_the_server_and_tool_when_both_conditions_hold() {
    let f = fixture(true, true);
    let mut argv = claude_argv(&f);
    let at = argv.iter().position(|a| a == "--mcp-config").unwrap();
    let config: Value = serde_json::from_str(&argv[at + 1]).unwrap();
    assert_eq!(
        config,
        json!({"mcpServers":{"codegraph":{"type":"stdio","command":"codegraph","args":["serve","--mcp"],"env":{"CODEGRAPH_TELEMETRY":"0"},"alwaysLoad":true}}})
    );
    argv.drain(at..=at + 1);
    let mut expected = today(&f);
    let tools = expected.iter().position(|a| a == "--allowedTools").unwrap() + 1;
    expected[tools] = format!("{},mcp__codegraph__codegraph_explore", TOOLS.join(","));
    assert_eq!(argv, expected);
}

#[test]
fn claude_launch_is_untouched_unless_both_conditions_hold() {
    for (executable, index) in ABSENT {
        let f = fixture(executable, index);
        assert_eq!(claude_argv(&f), today(&f), "{executable} {index}");
    }
}

#[test]
fn a_plain_file_is_not_an_index() {
    let f = fixture(true, false);
    fs::write(f.project.join(".codegraph"), "not a directory").unwrap();
    assert_eq!(claude_argv(&f), today(&f));
}

fn codex_call(f: &Fixture, lean: bool, configured: bool) -> (Value, Value) {
    let home = f.root.join("home");
    let log = f.root.join("codex.log");
    let _ = fs::remove_file(&log);
    let mut bridge = axiom(f, &home);
    bridge
        .arg("bridge")
        .env("AXIOM_CODEX_CMD", stand_in("fake_codex"))
        .env("FAKE_CODEX_LOG", &log)
        .env_remove("FAKE_CODEX_CODEGRAPH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if configured {
        bridge.env("FAKE_CODEX_CODEGRAPH", "1");
    }
    let mut child = bridge.spawn().unwrap();
    let call = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"codex_run","arguments":{"brief":"GOAL read","cwd":f.project,"model":"gpt-6.1-terra","sandbox":"read-only","lean":lean}}});
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{call}").unwrap();
    drop(stdin);
    let reply: Value = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|line| serde_json::from_str::<Value>(&line.unwrap()).unwrap())
        .find(|message| message["id"] == 1)
        .unwrap();
    child.wait().unwrap();
    let report = reply["result"]["structuredContent"].clone();
    assert_eq!(report["status"], "completed", "{report}");
    let thread = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|entry| entry["method"] == "thread/start")
        .unwrap()["params"]
        .clone();
    (report, thread)
}
fn lean_settings() -> Value {
    json!({"features":{"apps":false,"goals":false},"plugins":{"keepme@personal":{"enabled":false},"panel@personal":{"enabled":false}},"mcp_servers":{"node_repl":{"enabled":false}},"skills":{"max_context_tokens":1},"personality":"none","web_search":"disabled"})
}

#[test]
fn codex_worker_gets_the_server_when_both_conditions_hold() {
    let f = fixture(true, true);
    let server = json!({"command":"codegraph","args":["serve","--mcp"],"env":{"CODEGRAPH_TELEMETRY":"0"},"cwd":f.project,"enabled":true});
    let mut expected = lean_settings();
    expected["mcp_servers"]["codegraph"] = server.clone();
    for configured in [false, true] {
        let (report, thread) = codex_call(&f, true, configured);
        assert_eq!(thread["config"], expected, "{configured}");
        assert_eq!(report["disabled"]["mcp_servers"], json!(["node_repl"]));
    }
    let (report, thread) = codex_call(&f, false, false);
    assert_eq!(thread["config"], json!({"mcp_servers":{"codegraph":server}}));
    assert!(report["disabled"].is_null());
}

#[test]
fn codex_thread_is_untouched_unless_both_conditions_hold() {
    for (executable, index) in ABSENT {
        let f = fixture(executable, index);
        let (report, thread) = codex_call(&f, true, false);
        assert_eq!(thread["config"], lean_settings(), "{executable} {index}");
        assert_eq!(report["disabled"]["mcp_servers"], json!(["node_repl"]));
        let (report, thread) = codex_call(&f, true, true);
        let mut expected = lean_settings();
        expected["mcp_servers"]["codegraph"] = json!({"enabled":false});
        assert_eq!(thread["config"], expected, "{executable} {index}");
        assert_eq!(
            report["disabled"]["mcp_servers"],
            json!(["codegraph", "node_repl"])
        );
        let (_, thread) = codex_call(&f, false, false);
        assert!(thread.get("config").is_none(), "{thread}");
    }
}

fn doctor_line(f: &Fixture) -> String {
    let home = f.root.join("home");
    let stand_in = stand_in("fake_claude");
    let output = axiom(f, &home)
        .arg("doctor")
        .current_dir(&f.project)
        .env("AXIOM_CLAUDE_CMD", &stand_in)
        .env("AXIOM_CODEX_CMD", &stand_in)
        .env("AXIOM_WSL_CMD", &stand_in)
        .env(
            "AXIOM_PWSH_CMD",
            serde_json::to_string(&vec!["pwsh"]).unwrap(),
        )
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{text}");
    text.lines()
        .find(|line| line.starts_with("ok   codegraph"))
        .unwrap_or_else(|| panic!("no codegraph line in {text}"))
        .to_owned()
}

#[test]
fn doctor_reports_what_it_found_and_never_fails_on_it() {
    let found = |f: &Fixture, line: &str| {
        let fake = Path::new(&f.path).join("codegraph");
        line.to_lowercase()
            .contains(&fake.to_string_lossy().to_lowercase())
    };
    let f = fixture(true, true);
    let line = doctor_line(&f);
    assert!(found(&f, &line), "{line}");
    assert!(line.contains("runs here use it"), "{line}");
    let f = fixture(true, false);
    let line = doctor_line(&f);
    assert!(found(&f, &line), "{line}");
    assert!(line.contains("`codegraph init`"), "{line}");
    for index in [true, false] {
        let f = fixture(false, index);
        let line = doctor_line(&f);
        assert!(line.contains("not on PATH"), "{line}");
    }
}

fn agent_tools(agent: &str) -> Vec<String> {
    let path = format!("agents/{agent}.md");
    let text = FILES.iter().find(|(name, _)| *name == path).unwrap().1;
    text.split("---")
        .nth(1)
        .unwrap()
        .lines()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix("tools:"))
        .unwrap()
        .split(',')
        .map(|tool| tool.trim().to_owned())
        .collect()
}

#[test]
fn only_the_code_reading_agents_list_the_graph_tool() {
    assert_eq!(codegraph::TOOL, "mcp__codegraph__codegraph_explore");
    for agent in ["architect", "dispatcher", "reviewer"] {
        assert!(
            agent_tools(agent).iter().any(|t| t == codegraph::TOOL),
            "{agent}"
        );
    }
    for agent in ["astra", "critic", "memory-keeper"] {
        assert!(
            !agent_tools(agent).iter().any(|t| t == codegraph::TOOL),
            "{agent}"
        );
    }
}
