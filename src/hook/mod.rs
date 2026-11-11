use crate::{
    base, engine,
    goal::{self, Goal},
    orch::Handoff,
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
const ORCH: &str = "mcp__plugin_chronos_bridge__orch";

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
/// Where a write to `path` lands: its longest existing ancestor with symlinks and junctions resolved.
fn real(path: &Path) -> PathBuf {
    let mut base = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(found) = std::fs::canonicalize(base) {
            return rest.iter().rev().fold(found, |p, name| p.join(name));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                base = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}
fn inside(home: &Path, path: &str, cwd: Option<&str>) -> bool {
    absolute(path, cwd).is_some_and(|p| real(&p).starts_with(real(&normalize(home))))
}
/// Goals, units, claims and evidence change only through the engine's own gates, under the
/// path as written or the one a link leads to.
fn engine_state(home: &Path, path: &str, cwd: Option<&str>) -> bool {
    let Some(path) = absolute(path, cwd) else {
        return false;
    };
    let home = normalize(home);
    state_file(&home, &path) || state_file(&real(&home), &real(&path))
}
fn state_file(home: &Path, path: &Path) -> bool {
    let fold = |p: &Path| -> PathBuf {
        if cfg!(windows) {
            p.to_string_lossy().to_lowercase().into()
        } else {
            p.to_path_buf()
        }
    };
    let Ok(rest) = fold(path).strip_prefix(fold(home)).map(Path::to_path_buf) else {
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

pub fn stop_reason_in(home: &Path, session: &str, goal: &Goal) -> String {
    let Some(store) = store_of(home, goal.project.as_deref()) else {
        return stop_reason(session, goal);
    };
    match crate::orch::pending_handoffs(&store) {
        Ok(pending) if !pending.is_empty() => {
            let mut text = format!("Chronos goal {session} is still active.");
            for h in pending {
                text.push(' ');
                text.push_str(&crate::orch::handoff_blocker(&store, &h));
            }
            if goal.interjection.is_some() {
                text.push_str(" Classify the pending interjection with the bridge goal tool as amend, answer or stop.");
            }
            text
        }
        Err(e) => format!("Chronos goal {session} is still active. Cannot read handoffs in {}: {e}. Report this blocker with the bridge goal tool.", store.display()),
        _ => {
            let mut text = stop_reason(session, goal);
            if let Err(e) = crate::orch::done_gate(&store, goal.mode.as_deref() == Some("change")) {
                text.push_str(&format!(" Completion blocker: {e}."));
            }
            text
        }
    }
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
    let re = WAIT
        .get_or_init(|| regex::Regex::new(r"(?i)\b(sleep|wait-(process|job|event))\b").unwrap());
    re.is_match(input.get("command").and_then(Value::as_str).unwrap_or(""))
}

fn binary_read(path: &Path) -> bool {
    let binary = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|e| {
            ["png", "jpg", "jpeg", "gif", "webp", "pdf", "ipynb"].contains(&e.as_str())
        });
    binary
}
fn oversized_read(input: &Value) -> bool {
    let path = Path::new(input.get("file_path").and_then(Value::as_str).unwrap_or(""));
    !binary_read(path)
        && std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_WHOLE_READ)
        && input
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(true, |limit| limit > MAX_READ_LINES)
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

// Resolve links before removing parent components, as the filesystem does.
fn resolved_path(path: &str, cwd: Option<&str>) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(cwd?).join(p)
    };
    if !p.is_absolute() {
        return None;
    }
    Some(normalize(&real(&p)))
}

fn reviewer_binding(store: &Path, event: &Value, input: &Value, tool: &str) -> Option<String> {
    let key = context::agent_key(event)?;
    let marker = store.join("agents").join(format!("{key}.reviewer"));
    let read_binding = || {
        std::fs::read_to_string(&marker)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(bound) = read_binding() {
        return Some(bound);
    }
    if tool != "Read" {
        return None;
    }
    let given = input.get("file_path")?.as_str()?;
    let path = resolved_path(given, event.get("cwd").and_then(Value::as_str))?;
    let unit = [Path::new(given), path.as_path()]
        .into_iter()
        .find_map(|p| {
            let filename = p.file_name()?.to_str()?;
            let filename = if cfg!(windows) {
                filename.to_lowercase()
            } else {
                filename.to_string()
            };
            let unit = filename.strip_suffix(".review.md")?;
            (base::require_slug("unit", unit).is_ok()
                && same_path(&path, &real(&crate::orch::pack_path(store, unit))))
            .then(|| unit.to_string())
        })?;
    crate::orch::with_lock(store, false, || {
        if let Some(bound) = read_binding() {
            return Ok(bound);
        }
        std::fs::create_dir_all(store.join("agents"))
            .map_err(|e| base::Failure::new("io", e.to_string()))?;
        base::atomic_write(&marker, unit.as_bytes())
            .map_err(|e| base::Failure::new("io", e.to_string()))?;
        Ok(unit)
    })
    .ok()
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
    [
        input.get("project").and_then(Value::as_str),
        g.project.as_deref(),
    ]
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
/// The work a fresh dispatcher carries out, judged by module names in its prompt.
enum Target {
    /// No module is named: the whole wish.
    Wish,
    Module(String),
    /// Several are named and the description does not single one out.
    Unclear(Vec<String>),
}
/// The only module the prompt names; among several, the only one the description names too.
/// Other names are background or exclusions, never assignments.
fn target(store: &Path, input: &Value, pending: &[Handoff]) -> Target {
    let text = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    let mut modules = crate::orch::map_modules(store)
        .ok()
        .flatten()
        .unwrap_or_default();
    modules.extend(pending.iter().filter_map(|h| h.module.clone()));
    modules.sort();
    modules.dedup();
    let named: Vec<String> = modules
        .into_iter()
        .filter(|m| names_module(text("prompt"), m))
        .collect();
    let described: Vec<String> = named
        .iter()
        .filter(|m| names_module(text("description"), m))
        .cloned()
        .collect();
    match (named.len(), described.len()) {
        (0, _) => Target::Wish,
        (1, _) => Target::Module(named[0].clone()),
        (_, 1) => Target::Module(described[0].clone()),
        _ => Target::Unclear(named),
    }
}
const HANDED_OFF: &str = "\n\nA previous dispatcher handed this work off when its context grew too large. Continue from its handoff and the store; do not redo verified units.\n";
/// A fresh dispatcher's input with the pending handoff of the work it carries out appended, or
/// why it may not start. The note stays pending until the dispatcher is seen running with it.
fn start_dispatcher(
    home: &Path,
    session: &str,
    g: &Goal,
    input: &Value,
) -> Option<Result<Value, String>> {
    let store = store_of(home, g.project.as_deref())?;
    let prompt = input.get("prompt")?.as_str()?;
    let pending = crate::orch::pending_handoffs(&store).ok()?;
    if pending.is_empty() {
        return None;
    }
    let target = target(&store, input, &pending);
    let module = match &target {
        Target::Module(m) => Some(m.as_str()),
        _ => None,
    };
    let chosen: Vec<&Handoff> = match &target {
        Target::Wish => pending.iter().filter(|h| h.module.is_none()).collect(),
        Target::Module(m) => pending
            .iter()
            .filter(|h| h.module.is_none() || h.module.as_deref() == Some(m.as_str()))
            .collect(),
        Target::Unclear(named) => {
            if pending
                .iter()
                .any(|h| h.module.as_ref().is_some_and(|m| named.contains(m)))
            {
                return Some(Err(format!(
                    "This dispatcher's prompt names modules {}; name the one it carries out in its description, so it gets that module's handoff.",
                    named.join(", ")
                )));
            }
            pending.iter().filter(|h| h.module.is_none()).collect()
        }
    };
    if let Some(reason) = chosen
        .iter()
        .find_map(|h| crate::orch::stalled_out(&store, h))
    {
        return Some(Err(reason));
    }
    if chosen.is_empty() {
        return None;
    }
    let mut text = prompt.to_string();
    // Module digests must not pull in another module's pending notes through a wish resume.
    // Append each chosen note explicitly, then one digest for the dispatcher's target.
    for h in &chosen {
        text.push_str(HANDED_OFF);
        text.push_str(&crate::orch::handoff_text(h));
    }
    if let Some(h) = chosen.first() {
        let repo = h.repo.as_deref().or(g.cwd.as_deref()).map(Path::new);
        let digest = repo
            .and_then(|repo| crate::orch::digest(&store, repo, module).ok())
            .unwrap_or_else(|| "unavailable: pass --repo <project directory>\n".into());
        text.push_str("DIGEST\n");
        text.extend(
            digest
                .chars()
                .take(crate::orch::MAX_RESUME.saturating_sub(text[prompt.len()..].chars().count())),
        );
    }
    let wish_file = home.join("goals").join(format!("{session}.json"));
    text.push('\n');
    text.push_str(&crate::orch::anchor_text(
        &store,
        &crate::orch::Anchor {
            rules: false,
            wish: Some(&g.wish),
            wish_file: Some(&wish_file),
            module,
        },
    ));
    let mut updated = input.clone();
    updated["prompt"] = json!(text);
    for h in chosen {
        if let Err(e) = crate::orch::offer_handoff(&store, h) {
            return Some(Err(format!("Could not record the handoff offer: {e}")));
        }
    }
    Some(Ok(updated))
}
/// Marks taken each pending handoff this dispatcher was started with: its transcript opens with
/// the prompt that carried it, so a launch that failed or was cancelled consumes nothing.
fn confirm_handoffs(home: &Path, g: &Goal, event: &Value) {
    let Some(store) = store_of(home, g.project.as_deref()) else {
        return;
    };
    let pending = crate::orch::pending_handoffs(&store).unwrap_or_default();
    if pending.is_empty() {
        return;
    }
    let Some(opening) = context::agent_transcript(event).and_then(|t| context::opening_text(&t))
    else {
        return;
    };
    for h in pending {
        let delivered = format!("{HANDED_OFF}{}", crate::orch::handoff_header(&h));
        if opening.contains(&delivered) {
            let _ = crate::orch::take_delivered(&store, &h);
        }
    }
}
/// Find a compaction boundary that has not yet been marked.
fn pending_compaction(store: &Path, event: &Value) -> Option<(String, PathBuf)> {
    let key = context::agent_key(event)?;
    let boundary = context::last_compaction(&context::agent_transcript(event)?)?;
    let marker = store.join("agents").join(format!("{key}.compact"));
    if std::fs::read_to_string(&marker).is_ok_and(|seen| seen.trim() == boundary) {
        return None;
    }
    Some((boundary, marker))
}

/// Mark a compaction boundary and return its anchor when the project store exists.
fn compaction_anchor(
    store: &Path,
    boundary: &str,
    marker: &Path,
    module: Option<&str>,
) -> Option<Option<String>> {
    let has_store = store.is_dir();
    std::fs::create_dir_all(store.join("agents")).ok()?;
    base::atomic_write(marker, boundary.as_bytes()).ok()?;
    Some(has_store.then(|| {
        crate::orch::anchor_text(
            store,
            &crate::orch::Anchor {
                rules: true,
                wish: None,
                wish_file: None,
                module,
            },
        )
    }))
}

/// After Claude Code compacts a dispatcher, its next gated call brings back the work state once.
fn after_compaction(home: &Path, g: &Goal, event: &Value, input: &Value) -> Option<String> {
    let store = store_of(home, g.project.as_deref())?;
    let (boundary, marker) = pending_compaction(&store, event)?;
    let module = agent_module(home, g, input, event);
    let text = match recall(home, g, module.as_deref()) {
        Some(text) => text,
        None if store.is_dir() => "No work state is recorded yet.\n".into(),
        None => return None,
    };
    let Some(anchor) = compaction_anchor(&store, &boundary, &marker, module.as_deref())? else {
        return Some(format!(
            "Your context was compacted. Your work state, from the engine:\n{text}"
        ));
    };
    Some(format!("Your context was compacted. Your binding rules and your module's contract follow verbatim; they bind you as before.\n{anchor}Your work state, from the engine:\n{text}"))
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
            let mut text = format!(
                "Context was compacted. Chronos goal {session} is {}; its work state, from the engine:\n{text}",
                g.status
            );
            if context::in_dispatcher(event) {
                if let Some(anchor) = store_of(home, g.project.as_deref())
                    .filter(|store| store.is_dir())
                    .and_then(|store| {
                        let (boundary, marker) = pending_compaction(&store, event)?;
                        compaction_anchor(&store, &boundary, &marker, module.as_deref())
                    })
                    .flatten()
                {
                    text.push('\n');
                    text.push_str(&anchor);
                }
            }
            Some(
                json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":text}}),
            )
        }
        "Stop" => {
            let home = home?;
            let g = goal::load(home, session).ok()??;
            if g.status != "active" || (g.interjection.is_none() && agent_running(event)) {
                return None;
            }
            match goal::progress::block_stop(home, session) {
                Ok(true) => {
                    Some(json!({"decision":"block", "reason":stop_reason_in(home, session, &g)}))
                }
                Ok(false) => None,
                Err(e) => {
                    eprintln!("Chronos Stop progress guard: {e}");
                    None
                }
            }
        }
        "PreToolUse" => {
            let tool = event.get("tool_name").and_then(Value::as_str).unwrap_or("");
            let previously_routed = matches!(
                tool,
                "Write"
                    | "Edit"
                    | "NotebookEdit"
                    | "Bash"
                    | "PowerShell"
                    | "Read"
                    | "AskUserQuestion"
                    | "Agent"
                    | "Task"
                    | CODEX
                    | GOAL
            );
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
            if tool == ORCH && agent {
                let args = input.get("args").and_then(Value::as_array);
                let first = args.and_then(|a| a.first()).and_then(Value::as_str);
                let second = args.and_then(|a| a.get(1)).and_then(Value::as_str);
                if first == Some("waive") || (first == Some("--json") && second == Some("waive")) {
                    return deny("Only the core session may waive findings; report back instead.");
                }
            }
            let home = home?;
            let g = match goal::load(home, session) {
                Ok(Some(g)) => g,
                Ok(None) => return None,
                Err(e) if e.kind == "invalid_argument" => return None,
                Err(_) if !previously_routed => return None,
                Err(e) => {
                    return deny(&format!(
                        "The goal could not be read, so this call cannot be checked: {}",
                        e.message
                    ))
                }
            };
            let reviewer = agent
                && event.get("agent_type").and_then(Value::as_str) == Some("chronos:reviewer");
            let store = reviewer
                .then(|| store_of(home, g.project.as_deref()))
                .flatten();
            let bound = store
                .as_deref()
                .and_then(|store| reviewer_binding(store, event, input, tool));
            if reviewer && matches!(tool, "Write" | "Edit" | "NotebookEdit") {
                let allowed = store.as_ref().zip(bound.as_ref()).map(|(store, unit)| {
                    store.join("reports").join(format!("{unit}.verdict.json"))
                });
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("notebook_path"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let actual = resolved_path(path, event.get("cwd").and_then(Value::as_str));
                if !allowed
                    .as_ref()
                    .zip(actual.as_ref())
                    .is_some_and(|(allowed, actual)| same_path(&real(allowed), actual))
                {
                    return deny(&allowed.map_or_else(
                        || "Read your review pack first before writing a verdict.".to_string(),
                        |path| format!("A reviewer may write only its verdict: {}", path.display()),
                    ));
                }
            }
            if reviewer && tool == "Read" && input.get("limit").is_none() {
                let path = input.get("file_path").and_then(Value::as_str).unwrap_or("");
                let path = resolved_path(path, event.get("cwd").and_then(Value::as_str));
                let own_pack = store
                    .as_ref()
                    .zip(bound.as_ref())
                    .zip(path.as_ref())
                    .is_some_and(|((store, unit), path)| {
                        same_path(&real(&crate::orch::pack_path(store, unit)), path)
                    });
                if path.as_ref().is_some_and(|path| {
                    !binary_read(path) && std::fs::metadata(path).is_ok_and(|m| m.len() > 50_000)
                }) && !own_pack
                {
                    return deny("Reviewers may not read files larger than 50,000 bytes whole; use Read with a limit.");
                }
            }
            let mut warning = None;
            if g.status == "active" {
                if context::in_dispatcher(event) {
                    confirm_handoffs(home, &g, event);
                }
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
                if agent && !context::lock_allows(tool, input) {
                    if let Some(limit) = context::handoff_limit() {
                        let module = agent_module(home, &g, input, event);
                        if let Some(reason) =
                            context::handoff_refusal(event, limit, module.as_deref())
                        {
                            return deny(&reason);
                        }
                        warning = context::warn_percent()
                            .and_then(|p| context::handoff_warning(event, limit, p));
                    }
                }
                // Calls newly routed by the wildcard have no other hook behavior.
                if !previously_routed {
                    return warning.map(|line| json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":line}}));
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
                    return deny(
                        "Goals, units, claims and evidence change only through the engine.",
                    );
                }
                if restricted && !reviewer && !inside(home, path, cwd) {
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
                    Ok(false) => return deny(
                        "Questions are unavailable in this session or exceed the limit of four.",
                    ),
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
                match start_dispatcher(home, session, &g, input) {
                    Some(Err(reason)) => return deny(&reason),
                    Some(Ok(updated)) => {
                        return Some(
                            json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"Chronos added the pending handoff to the dispatcher's prompt.","updatedInput":updated}}),
                        )
                    }
                    None => (),
                }
            }
            if agent && g.status == "active" && context::in_dispatcher(event) {
                if let Some(mut text) = after_compaction(home, &g, event, input) {
                    if let Some(line) = warning {
                        text.push('\n');
                        text.push_str(&line);
                    }
                    return Some(
                        json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":text}}),
                    );
                }
            }
            warning.map(|line| json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":line}}))
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
