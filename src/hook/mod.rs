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
fn inside(home: &Path, path: &str, cwd: Option<&str>) -> bool {
    if path.is_empty() {
        return false;
    }
    let p = Path::new(path);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(cwd) = cwd {
        Path::new(cwd).join(p)
    } else {
        return false;
    };
    normalize(&p).starts_with(normalize(home))
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
    let mut text = format!("Chronos goal {session} is still active: {}. Take the next step through the chronos agents. When the wish is done and verified, call the bridge goal tool with status done. If it cannot go on without something only the user has, such as a credential, a payment or permission to delete their data, call it with status blocked and say what is needed.", goal.wish);
    if goal.interjection.is_some() {
        text.push_str(" Classify the pending interjection with the bridge goal tool as amend, answer or stop.");
    }
    text
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
            let role = event.get("agent_role").and_then(Value::as_str);
            let input = event.get("tool_input").unwrap_or(&Value::Null);
            if agent
                && matches!(tool, "Agent" | "Task")
                && !(input.get("subagent_type").and_then(Value::as_str) == Some("chronos:reviewer")
                    || (role == Some("dispatcher") && input["subagent_type"] == "chronos:content"))
            {
                return deny("Only designated reviewer/content children may be started; never forward engineering through another agent.");
            }
            if tool == CODEX && !agent {
                return deny("codex_run is for the chronos agents. Hand building to chronos:dispatcher and Astra plans to chronos:astra.");
            }
            let home = home?;
            let g = goal::load(home, session).ok()??;
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
            let restricted = role.is_some() || !agent || matches!(mode, Some("review" | "answer"));
            if restricted && ["Write", "Edit", "NotebookEdit"].contains(&tool) {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("notebook_path"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let content_output = role == Some("content")
                    && mode.is_some()
                    && event.get("authorized_output_path").and_then(Value::as_str) == Some(path);
                if !content_output && !inside(home, path, event.get("cwd").and_then(Value::as_str))
                {
                    return deny("Writes outside the data home are not allowed for this caller or goal mode.");
                }
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
                if goal::mark_asked(home, session).is_err() {
                    return deny("Could not record the question.");
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
