use crate::{
    base,
    goal::{self, Goal},
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
const GOAL: &str = "mcp__plugin_chronos_bridge__goal";

fn deny(reason: &str) -> Option<Value> {
    Some(
        json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":reason}}),
    )
}
fn context(text: String) -> Option<Value> {
    Some(
        json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":text}}),
    )
}
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => (),
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
fn absolute(path: &str, cwd: Option<&str>) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        Some(normalize(p))
    } else {
        cwd.map(|cwd| normalize(&Path::new(cwd).join(p)))
    }
}
fn inside(home: &Path, path: &str, cwd: Option<&str>) -> bool {
    absolute(path, cwd).is_some_and(|p| p.starts_with(normalize(home)))
}
/// Goals, units, claims and evidence change only through the engine's own gates.
fn engine_state(home: &Path, path: &str, cwd: Option<&str>) -> bool {
    let fold = |p: &Path| -> PathBuf {
        if cfg!(windows) {
            p.to_string_lossy().to_lowercase().into()
        } else {
            p.to_path_buf()
        }
    };
    let Some(path) = absolute(path, cwd).map(|p| fold(&p)) else {
        return false;
    };
    let Ok(rest) = path.strip_prefix(fold(&normalize(home))) else {
        return false;
    };
    let parts: Vec<_> = rest.iter().map(|c| c.to_string_lossy()).collect();
    let parts: Vec<&str> = parts.iter().map(|c| c.as_ref()).collect();
    match parts.as_slice() {
        ["goals", ..] => true,
        ["orchestrate", _, file] => {
            ["units.tsv", "claims.tsv"].contains(file) || file.starts_with(".orch.lock")
        }
        ["orchestrate", _, "reports", file] => file.ends_with(".evidence.tsv"),
        _ => false,
    }
}
/// A variable that chooses which program runs makes even `git diff` a write.
fn steers_execution(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name == "PATH"
        || name == "RIPGREP_CONFIG_PATH"
        || ["GIT_", "LD_", "DYLD_"].iter().any(|p| name.starts_with(p))
}

pub fn read_only_bash(command: &str) -> bool {
    if command.contains("$(") || command.contains('`') {
        return false;
    }
    static REDIR: OnceLock<regex::Regex> = OnceLock::new();
    let redir =
        REDIR.get_or_init(|| regex::Regex::new(r"2>&1|[12]?>/dev/null(\s|$|[;&|])").unwrap());
    let cleaned = redir.replace_all(command, "$1");
    if cleaned.contains('>') || cleaned.contains('<') {
        return false;
    }
    for segment in cleaned.split(['&', '|', ';', '\n', '\r']) {
        let mut words = segment.split_whitespace().peekable();
        while words.peek().is_some_and(|w| {
            w.split_once('=').is_some_and(|(k, _)| {
                !k.is_empty()
                    && !steers_execution(k)
                    && k.chars().enumerate().all(|(i, c)| {
                        c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                    })
            })
        }) {
            words.next();
        }
        let Some(cmd) = words.next() else {
            if !segment.trim().is_empty() {
                return false;
            }
            continue;
        };
        match cmd {
            "ls" | "cat" | "head" | "tail" | "wc" | "pwd" | "echo" | "grep" | "which" | "type" => {
                ()
            }
            "rg" => {
                if words.any(|w| w == "--pre" || w.starts_with("--pre=")) {
                    return false;
                }
            }
            "find" => {
                if words.any(|w| {
                    w == "-delete"
                        || w.starts_with("-exec")
                        || w.starts_with("-fprint")
                        || w == "-ok"
                        || w == "-okdir"
                        || w == "-fls"
                }) {
                    return false;
                }
            }
            "git" => {
                let subcommand = words.next();
                if !subcommand.is_some_and(|w| {
                    [
                        "status",
                        "diff",
                        "log",
                        "show",
                        "rev-parse",
                        "ls-files",
                        "blame",
                    ]
                    .contains(&w)
                }) {
                    return false;
                }
                if matches!(subcommand, Some("diff" | "log" | "show"))
                    && words.any(|w| w == "--output" || w.starts_with("--output="))
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    !command.trim().is_empty()
}

pub fn stop_reason(session: &str, goal: &Goal) -> String {
    let mut chars = goal.wish.chars();
    let mut wish: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        wish.push('…');
    }
    let mut text = format!("Chronos goal {session} is still active: {wish}. Take the next step through the chronos agents. When the wish is done and verified, call the bridge goal tool with status done. If it cannot go on without something only the user has, such as a credential, a payment or permission to delete their data, call it with status blocked and say what is needed.");
    if goal.interjection.is_some() {
        text.push_str(" Classify the pending interjection with the bridge goal tool as amend, answer or stop.");
    }
    text
}

const MAX_WHOLE_READ: u64 = 256 * 1024;
const MAX_READ_LINES: u64 = 500;

fn oversized_read(input: &Value) -> bool {
    let path = Path::new(input.get("file_path").and_then(Value::as_str).unwrap_or(""));
    let binary = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp", "pdf", "ipynb"].contains(&e.as_str()));
    !binary
        && std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_WHOLE_READ)
        && input
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(true, |limit| limit > MAX_READ_LINES)
}

/// With a module map, a dispatcher stays on the first module it dispatches, so its context stays small.
fn other_module(home: &Path, event: &Value, input: &Value) -> Option<String> {
    let agent: String = event
        .get("agent_id")?
        .as_str()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if agent.is_empty() || input.get("role").and_then(Value::as_str) == Some("sol") {
        return None;
    }
    let unit = input.get("unit")?.as_str()?;
    let store = crate::orch::store_for(home, input.get("project")?.as_str()?).ok()?;
    if !store.join("modules.md").is_file() {
        return None;
    }
    let units = crate::orch::read_units(&store).ok()?;
    let track = &units.iter().find(|u| u.id == unit)?.track;
    let marker = store.join("agents").join(agent);
    let first = crate::orch::with_lock(&store, false, || {
        if let Ok(first) = std::fs::read_to_string(&marker) {
            return Ok(first);
        }
        let _ = std::fs::create_dir_all(store.join("agents"));
        let _ = base::atomic_write(&marker, track.as_bytes());
        Ok(track.to_string())
    })
    .ok()?;
    (first.trim() != track).then(|| format!(
        "This dispatcher already works on module {}; unit {unit} belongs to {track}. Report back: each module gets a fresh dispatcher.",
        first.trim()
    ))
}

pub fn handle(event: &Value, home: Option<&Path>, headless: bool) -> Option<Value> {
    let kind = event.get("hook_event_name")?.as_str()?;
    let session = event
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        "UserPromptSubmit" => {
            let home = home?;
            let prompt = event.get("prompt").and_then(Value::as_str).unwrap_or("");
            static WISH: OnceLock<regex::Regex> = OnceLock::new();
            let re =
                WISH.get_or_init(|| regex::Regex::new(r"(?s)^\s*/chronos:wish\b(.*)").unwrap());
            if let Some(c) = re.captures(prompt) {
                let g = goal::open(
                    home,
                    session,
                    c[1].trim(),
                    event.get("cwd").and_then(Value::as_str),
                )
                .ok()?;
                let mut text = format!("Chronos goal {session} is open for this wish. Program slug: {}. Reply in the language of the wish.", g.project.as_deref().unwrap_or("project"));
                if headless {
                    text.push_str(" This run is headless: ask the user nothing.");
                }
                context(text)
            } else {
                goal::interject(home, session, prompt).ok()??;
                context(format!("This message amends Chronos goal {session}. The user's words take precedence over the plan. Classify it with the bridge goal tool as amend, answer or stop."))
            }
        }
        "Stop" => {
            let g = goal::load(home?, session).ok()??;
            (g.status == "active")
                .then(|| json!({"decision":"block", "reason":stop_reason(session, &g)}))
        }
        "PreToolUse" => {
            let tool = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
            let agent = event.get("agent_id").is_some();
            let input = event.get("tool_input").unwrap_or(&Value::Null);
            let subagent = input.get("subagent_type").and_then(Value::as_str);
            if matches!(tool, "Agent" | "Task")
                && matches!(
                    subagent,
                    Some("chronos:memory-keeper" | "chronos:dispatcher")
                )
                && input.get("run_in_background").and_then(Value::as_bool) == Some(true)
                && home
                    .and_then(|h| goal::load(h, session).ok().flatten())
                    .is_some_and(|g| g.status == "active")
            {
                return deny(&format!(
                    "Run {} in the foreground; waiting needs no polling.",
                    subagent.unwrap_or("this agent")
                ));
            }
            if agent
                && matches!(tool, "Agent" | "Task")
                && input.get("subagent_type").and_then(Value::as_str) != Some("chronos:reviewer")
            {
                return deny("Inside an agent only chronos:reviewer may be started. Call codex_run yourself; never forward work through another agent.");
            }
            if tool == CODEX && !agent {
                return deny("codex_run is for the chronos agents. Hand building to chronos:dispatcher and Astra plans to chronos:astra.");
            }
            if tool == GOAL && agent {
                return deny("Only the core session may change the goal; report back instead.");
            }
            let home = home?;
            let g = match goal::load(home, session) {
                Ok(Some(g)) => g,
                Ok(None) => return None,
                Err(e) if e.kind == "invalid_argument" => return None,
                Err(e) => {
                    return deny(&format!(
                        "The goal could not be read, so this call cannot be checked: {}",
                        e.message
                    ))
                }
            };
            if g.status == "active" {
                if !agent
                    && matches!(tool, "Agent" | "Task")
                    && !subagent.is_some_and(|s| s.starts_with("chronos:"))
                {
                    return deny("While a goal is active only chronos agents may be started.");
                }
                if tool == "Read" && oversized_read(input) {
                    return deny("This file is too large to read whole: use Grep, or Read with offset and a limit of at most 500 lines.");
                }
                if tool == CODEX {
                    if let Some(reason) = other_module(home, event, input) {
                        return deny(&reason);
                    }
                }
            }
            let mode = g.mode.as_deref();
            let dispatcher = (tool == "Agent" || tool == "Task")
                && input.get("subagent_type").and_then(Value::as_str) == Some("chronos:dispatcher");
            if mode != Some("change")
                && (dispatcher
                    || (tool == CODEX
                        && input.get("sandbox").and_then(Value::as_str) != Some("read-only")))
            {
                return deny(if mode.is_none() {
                    "Declare the mode with the goal tool before dispatching or using a writable sandbox."
                } else {
                    "The goal mode is review/answer; dispatch and writable sandboxes require change mode."
                });
            }
            let restricted = !agent || mode != Some("change");
            if ["Write", "Edit", "NotebookEdit"].contains(&tool) {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("notebook_path"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let cwd = event.get("cwd").and_then(Value::as_str);
                if engine_state(home, path, cwd) {
                    return deny("Goals, units, claims and evidence change only through the engine.");
                }
                if restricted && !inside(home, path, cwd) {
                    return deny("Writes outside the data home are not allowed for this caller or goal mode.");
                }
            }
            if restricted && tool == "PowerShell" {
                return deny(
                    "PowerShell cannot be checked as read-only; use read-only Bash commands.",
                );
            }
            if restricted
                && tool == "Bash"
                && !read_only_bash(input.get("command").and_then(Value::as_str).unwrap_or(""))
            {
                return deny(
                    "Only read-only Bash commands are allowed for this caller or goal mode.",
                );
            }
            if tool == "AskUserQuestion" {
                let count = input
                    .get("questions")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                if headless || agent || g.asked || g.dispatched || count > 4 {
                    return deny(
                        "Questions are unavailable in this session or exceed the limit of four.",
                    );
                }
                match goal::mark_asked(home, session) {
                    Ok(true) => (),
                    Ok(false) => {
                        return deny(
                            "Questions are unavailable in this session or exceed the limit of four.",
                        )
                    }
                    Err(_) => return deny("Could not record the question."),
                }
            }
            if dispatcher
                && !agent
                && mode == Some("change")
                && goal::mark_dispatched(home, session).is_err()
            {
                return deny("Could not record dispatch.");
            }
            None
        }
        _ => None,
    }
}

pub fn main_stdin() -> i32 {
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        eprintln!("Could not read hook input");
        return 0;
    }
    let event: Value = match serde_json::from_str(&String::from_utf8_lossy(&bytes)) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("Invalid hook input");
            return 0;
        }
    };
    if !matches!(
        event.get("hook_event_name").and_then(Value::as_str),
        Some("UserPromptSubmit" | "Stop" | "PreToolUse")
    ) {
        eprintln!("Unknown hook event");
        return 0;
    }
    let home = base::data_home().ok();
    let headless = std::env::var_os("AXIOM_HEADLESS").is_some_and(|v| !v.is_empty());
    if let Some(reply) = handle(&event, home.as_deref(), headless) {
        let _ = write!(std::io::stdout(), "{reply}");
    }
    0
}
