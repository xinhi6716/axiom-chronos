use crate::{
    base, engine,
    goal::{self, Goal},
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

pub mod context;

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
    let mut text = format!("Chronos goal {session} is still active. Take the next step through the chronos agents, or close it with the bridge goal tool: status done once it is verified, status blocked with what only the user can provide.");
    if goal.interjection.is_some() {
        text.push_str(" Classify the pending interjection with the bridge goal tool as amend, answer or stop.");
    }
    if let Some(waited) = &goal.waited {
        text.push(' ');
        text.push_str(&waited.prompt());
    }
    text
}

/// True while a Chronos agent runs in the background: the harness calls the session back when it finishes.
fn agent_running(event: &Value) -> bool {
    event
        .get("background_tasks")
        .and_then(Value::as_array)
        .is_some_and(|tasks| {
            tasks.iter().any(|task| {
                matches!(
                    task.get("status").and_then(Value::as_str),
                    Some("running" | "pending")
                ) && task
                    .get("agent_type")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.starts_with("chronos:"))
            })
        })
}

const MAX_WHOLE_READ: u64 = 256 * 1024;
pub(crate) const MAX_READ_LINES: u64 = 500;

fn waiting_loop(input: &Value) -> bool {
    static WAIT: OnceLock<regex::Regex> = OnceLock::new();
    let re = WAIT.get_or_init(|| regex::Regex::new(r"(?i)\b(sleep|wait-(process|job|event))\b").unwrap());
    re.is_match(input.get("command").and_then(Value::as_str).unwrap_or(""))
}

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
    let track = match units.iter().find(|u| u.id == unit) {
        Some(u) => u.track.as_str(),
        None => input.get("track")?.as_str()?,
    };
    let marker = store.join("agents").join(agent);
    // Parallel hooks can see the marker before its writer finishes, so an empty one counts as absent.
    let first = crate::orch::with_lock(&store, false, || {
        if let Ok(first) = std::fs::read_to_string(&marker) {
            if !first.trim().is_empty() {
                return Ok(first);
            }
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

fn store_of(home: &Path, project: Option<&str>) -> Option<PathBuf> {
    crate::orch::store_for(home, project?).ok()
}
/// The module this agent was bound to by its first `codex_run`, if any.
fn agent_module(home: &Path, g: &Goal, input: &Value, event: &Value) -> Option<String> {
    let key = context::agent_key(event)?;
    [input.get("project").and_then(Value::as_str), g.project.as_deref()]
        .into_iter()
        .filter_map(|p| store_of(home, p))
        .find_map(|store| {
            let module = std::fs::read_to_string(store.join("agents").join(&key)).ok()?;
            let module = module.trim();
            (!module.is_empty()).then(|| module.to_string())
        })
}
/// The engine's record of the work: the module's latest handoff and digest, or the whole wish's.
fn recall(home: &Path, g: &Goal, module: Option<&str>) -> Option<String> {
    let store = store_of(home, g.project.as_deref())?;
    crate::orch::resume_text(&store, g.cwd.as_deref().map(Path::new), module).ok()
}
fn names_module(prompt: &str, module: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    prompt.match_indices(module).any(|(i, _)| {
        !prompt[..i].chars().next_back().is_some_and(word)
            && !prompt[i + module.len()..].chars().next().is_some_and(word)
    })
}
/// Why no fresh dispatcher may start on a module its prompt names: it keeps handing off without progress.
fn stalled_module(home: &Path, g: &Goal, input: &Value) -> Option<String> {
    let store = store_of(home, g.project.as_deref())?;
    let prompt = input.get("prompt")?.as_str()?;
    crate::orch::pending_handoffs(&store)
        .ok()?
        .iter()
        .filter(|h| h.module.as_deref().is_some_and(|m| names_module(prompt, m)))
        .find_map(|h| crate::orch::stalled_out(&store, h))
}
/// A fresh dispatcher's input with the pending handoff of the module its prompt names appended.
fn with_handoff(home: &Path, g: &Goal, input: &Value) -> Option<Value> {
    let store = store_of(home, g.project.as_deref())?;
    let prompt = input.get("prompt")?.as_str()?;
    let pending = crate::orch::pending_handoffs(&store).ok()?;
    let named: Vec<_> = pending
        .iter()
        .filter(|h| h.module.as_deref().is_some_and(|m| names_module(prompt, m)))
        .collect();
    let chosen = if named.is_empty() {
        pending.iter().filter(|h| h.module.is_none()).collect()
    } else {
        named
    };
    if chosen.is_empty() {
        return None;
    }
    let mut text = prompt.to_string();
    for h in chosen {
        let repo = h.repo.as_deref().or(g.cwd.as_deref()).map(Path::new);
        let resume = crate::orch::resume_text(&store, repo, h.module.as_deref()).ok()?;
        text.push_str("\n\nA previous dispatcher handed this work off when its context grew too large. Continue from its handoff and the store; do not redo verified units.\n");
        text.push_str(&resume);
        crate::orch::take_handoff(&store, h.module.as_deref()).ok()?;
    }
    let mut updated = input.clone();
    updated["prompt"] = json!(text);
    Some(updated)
}
/// After Claude Code compacts a dispatcher, its next gated call brings back the work state once.
fn after_compaction(home: &Path, g: &Goal, event: &Value, input: &Value) -> Option<String> {
    let store = store_of(home, g.project.as_deref())?;
    let key = context::agent_key(event)?;
    let boundary = context::last_compaction(&context::agent_transcript(event)?)?;
    let marker = store.join("agents").join(format!("{key}.compact"));
    if std::fs::read_to_string(&marker).is_ok_and(|seen| seen.trim() == boundary) {
        return None;
    }
    let module = agent_module(home, g, input, event);
    let text = recall(home, g, module.as_deref())?;
    std::fs::create_dir_all(store.join("agents")).ok()?;
    base::atomic_write(&marker, boundary.as_bytes()).ok()?;
    Some(format!(
        "Your context was compacted. Your work state, from the engine:\n{text}"
    ))
}

fn wish_text(prompt: &str) -> Option<&str> {
    static WISH: OnceLock<regex::Regex> = OnceLock::new();
    let re = WISH.get_or_init(|| regex::Regex::new(r"(?s)^\s*/chronos:wish\b(.*)").unwrap());
    re.captures(prompt)
        .map(|c| c.get(1).map_or("", |m| m.as_str()))
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
            if let Some(wish) = wish_text(prompt) {
                let g = goal::open(
                    home,
                    session,
                    wish.trim(),
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
        "SessionStart" => {
            if event.get("source").and_then(Value::as_str) != Some("compact") {
                return None;
            }
            let home = home?;
            let g = goal::load(home, session).ok()??;
            if !matches!(g.status.as_str(), "active" | "waiting") {
                return None;
            }
            let module = agent_module(home, &g, &Value::Null, event);
            let text = recall(home, &g, module.as_deref())
                .unwrap_or_else(|| "No work state is recorded yet.\n".into());
            Some(json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":format!(
                "Context was compacted. Chronos goal {session} is {}; its work state, from the engine:\n{text}",
                g.status
            )}}))
        }
        "Stop" => {
            let g = goal::load(home?, session).ok()??;
            (g.status == "active" && (g.interjection.is_some() || !agent_running(event)))
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
                if !agent && tool == "PowerShell" && waiting_loop(input) {
                    return deny("Do not sleep or poll while a Chronos agent runs: end your turn. A background agent calls this session back when it finishes.");
                }
                if tool == CODEX {
                    if let Some(reason) = other_module(home, event, input) {
                        return deny(&reason);
                    }
                }
                if agent && matches!(tool, CODEX | "Agent" | "Task") {
                    if let Some(limit) = context::handoff_limit() {
                        let module = agent_module(home, &g, input, event);
                        if let Some(reason) =
                            context::handoff_refusal(event, limit, module.as_deref())
                        {
                            return deny(&reason);
                        }
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
            if dispatcher && !agent && g.status == "active" {
                if let Some(reason) = stalled_module(home, &g, input) {
                    return deny(&reason);
                }
                if let Some(updated) = with_handoff(home, &g, input) {
                    return Some(json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"Chronos added the pending handoff to the dispatcher's prompt.","updatedInput":updated}}));
                }
            }
            if agent && g.status == "active" && context::in_dispatcher(event) {
                if let Some(text) = after_compaction(home, &g, event, input) {
                    return Some(json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":text}}));
                }
            }
            None
        }
        _ => None,
    }
}

/// `handle` for a hook fired by a plugin: a wish opens only when that plugin belongs with this binary.
pub fn handle_from_plugin(
    event: &Value,
    home: Option<&Path>,
    headless: bool,
    plugin_root: Option<&Path>,
) -> Option<Value> {
    let starts_wish = event.get("hook_event_name").and_then(Value::as_str)
        == Some("UserPromptSubmit")
        && wish_text(event.get("prompt").and_then(Value::as_str).unwrap_or("")).is_some();
    let Some(root) = plugin_root.filter(|_| starts_wish) else {
        return handle(event, home, headless);
    };
    let plugin = match engine::check_plugin(root) {
        Ok(plugin) => plugin,
        Err(reason) => {
            return Some(
                json!({"decision":"block","reason":format!("Chronos did not start this wish: {reason}")}),
            )
        }
    };
    let mut reply = handle(event, home, headless)?;
    if !plugin.differing.is_empty() {
        if let Some(Value::String(text)) =
            reply.pointer_mut("/hookSpecificOutput/additionalContext")
        {
            text.push_str(&format!(
                " Warning: the plugin and the axiom binary are both version {}, but these plugin files differ from the ones built into the binary: {}. The rules you read may not match the gates that enforce them; say so in your final report.",
                plugin.version,
                plugin.differing.join(", ")
            ));
        }
    }
    Some(reply)
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
        Some("UserPromptSubmit" | "Stop" | "PreToolUse" | "SessionStart")
    ) {
        eprintln!("Unknown hook event");
        return 0;
    }
    let home = base::data_home().ok();
    let headless = std::env::var_os("AXIOM_HEADLESS").is_some_and(|v| !v.is_empty());
    let plugin_root = std::env::var_os("CLAUDE_PLUGIN_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    if let Some(reply) =
        handle_from_plugin(&event, home.as_deref(), headless, plugin_root.as_deref())
    {
        let _ = write!(std::io::stdout(), "{reply}");
    }
    0
}
