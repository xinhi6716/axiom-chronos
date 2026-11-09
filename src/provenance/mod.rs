//! What produced a run: one record per run, in the log's first line and in the wish's goal file.
//! Every value is a plain string; a value that could not be established is `unknown`.

use crate::{
    base::{self, proc},
    engine,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    io::Read,
    path::Path,
    process::Stdio,
    sync::mpsc,
    time::{Duration, Instant},
};

/// Carries a launcher's record to the bridge it starts, so a run has one record, not two.
pub const ENV: &str = "AXIOM_PROVENANCE";
pub const UNKNOWN: &str = "unknown";
const PROBE_LIMIT: Duration = Duration::from_secs(5);

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Provenance {
    pub binary_version: String,
    pub binary_build: String,
    pub engine_build: String,
    pub plugin_version: String,
    pub plugin_rules: String,
    pub claude_version: String,
    pub codex_version: String,
    pub git_commit: String,
    pub git_tree: String,
}

#[derive(Default)]
pub struct Sources<'a> {
    pub binary: Option<&'a Path>,
    pub plugin: Option<&'a Path>,
    pub claude: Option<String>,
    pub codex: Option<&'a [String]>,
    pub project: Option<&'a Path>,
}

pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn drain(mut stream: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stream.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    rx
}

/// None when the program cannot start or has not finished within `limit`. The streams are read
/// under the same limit because a background process it leaves behind can hold them open.
pub fn run_bounded(argv: &[String], cwd: Option<&Path>, limit: Duration) -> Option<Output> {
    if argv.is_empty() {
        return None;
    }
    let mut program = proc::command(argv);
    if let Some(cwd) = cwd {
        program.current_dir(cwd);
    }
    let mut child = program
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = drain(child.stdout.take()?);
    let stderr = drain(child.stderr.take()?);
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                proc::kill_tree(&mut child);
                return None;
            }
        }
    };
    let remaining = || limit.saturating_sub(start.elapsed());
    let stdout = stdout.recv_timeout(remaining()).ok()?;
    let stderr = stderr.recv_timeout(remaining()).ok()?;
    Some(Output {
        success: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// The last line a program prints for `--version`, or None if it does not answer cleanly.
pub fn version_of(command: &[String]) -> Option<String> {
    let mut argv = command.to_vec();
    argv.push("--version".into());
    let output = run_bounded(&argv, None, PROBE_LIMIT).filter(|o| o.success)?;
    let text = if output.stdout.trim().is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .map(str::to_owned)
}

/// `claude --version` prints `2.1.311 (Claude Code)`; the MCP handshake gives only the number.
fn claude_number(line: String) -> String {
    match line.split_whitespace().next() {
        Some(first) if first.starts_with(|c: char| c.is_ascii_digit()) => first.to_owned(),
        _ => line,
    }
}

fn git(project: &Path, args: &[&str]) -> Option<Output> {
    let argv: Vec<String> = std::iter::once("git")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    run_bounded(&argv, Some(project), PROBE_LIMIT).filter(|o| o.success)
}

fn git_commit(project: &Path) -> Option<String> {
    let output = git(project, &["rev-parse", "--verify", "HEAD"])?;
    Some(output.stdout.trim().to_owned()).filter(|commit| !commit.is_empty())
}

fn git_tree(project: &Path) -> Option<&'static str> {
    let output = git(
        project,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    )?;
    Some(if output.stdout.trim().is_empty() {
        "clean"
    } else {
        "dirty"
    })
}

fn known(value: Option<String>) -> String {
    value.unwrap_or_else(|| UNKNOWN.into())
}

pub fn collect(from: Sources) -> Provenance {
    let plugin_version = from
        .plugin
        .and_then(|root| engine::plugin_version(root).ok());
    let plugin_rules = from
        .plugin
        .filter(|_| plugin_version.is_some())
        .map(|root| {
            if engine::differing_files(root).is_empty() {
                "match".to_owned()
            } else {
                "differ".to_owned()
            }
        });
    Provenance {
        binary_version: env!("CARGO_PKG_VERSION").into(),
        binary_build: known(from.binary.and_then(engine::binary_id)),
        engine_build: engine::build_id(),
        plugin_version: known(plugin_version),
        plugin_rules: known(plugin_rules),
        claude_version: known(from.claude),
        codex_version: known(from.codex.and_then(version_of)),
        git_commit: known(from.project.and_then(git_commit)),
        git_tree: known(from.project.and_then(git_tree).map(str::to_owned)),
    }
}

/// The record for a run this binary launches itself, where `claude` is the very command it starts.
pub fn of_session(claude: &[String], binary: &Path, plugin: &Path, project: &Path) -> Provenance {
    let codex = base::resolve_command("AXIOM_CODEX_CMD", "codex").ok();
    collect(Sources {
        binary: Some(binary),
        plugin: Some(plugin),
        claude: version_of(claude).map(claude_number),
        codex: codex.as_deref(),
        project: Some(project),
    })
}

/// The record a launcher passed down, if this process was started by one.
pub fn inherited() -> Option<Value> {
    let text = std::env::var(ENV).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()
        .filter(Value::is_object)
}

impl Provenance {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
    pub fn log_line(&self) -> String {
        format!("{}\n", json!({"type": "provenance", "provenance": self}))
    }
    pub fn line(&self) -> String {
        format!(
            "axiom {} (build {}, engine {}) · plugin {} (rules {}) · claude {} · codex {} · git {} ({})",
            self.binary_version,
            self.binary_build,
            self.engine_build,
            self.plugin_version,
            self.plugin_rules,
            self.claude_version,
            self.codex_version,
            self.git_commit,
            self.git_tree
        )
    }
}
