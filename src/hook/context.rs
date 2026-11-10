//! How large a dispatcher's context has grown, read from the tail of its own transcript.
//!
//! Anything unreadable yields None, so a missing or malformed transcript never blocks work.

use serde_json::Value;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

// Below the roughly 167k at which Claude Code compacts a 200k-window agent on its own.
pub const DEFAULT_HANDOFF_TOKENS: u64 = 150_000;
pub const HANDOFF_ENV: &str = "AXIOM_HANDOFF_TOKENS";
/// Bytes read from the end of a transcript; the rest of the file is never read.
pub const TAIL_BYTES: u64 = 1 << 20;
const HEAD_BYTES: u64 = 256 * 1024;
pub const DISPATCHER: &str = "chronos:dispatcher";

/// The handoff threshold: `AXIOM_HANDOFF_TOKENS`, 0 to turn handoffs off, 150k when unset.
pub fn handoff_limit() -> Option<u64> {
    let raw = std::env::var(HANDOFF_ENV).unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(DEFAULT_HANDOFF_TOKENS);
    }
    match raw.replace('_', "").parse::<u64>() {
        Ok(0) => None,
        Ok(n) => Some(n),
        Err(_) => {
            eprintln!("{HANDOFF_ENV}={raw} is not a token count; using {DEFAULT_HANDOFF_TOKENS}");
            Some(DEFAULT_HANDOFF_TOKENS)
        }
    }
}

/// At most `max` bytes from the end of the file, starting at a line boundary, and where they began.
pub fn read_tail(path: &Path, max: u64) -> Option<(u64, Vec<u8>)> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(max).read_to_end(&mut buf).ok()?;
    if start > 0 {
        let cut = buf
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buf.len(), |i| i + 1);
        buf.drain(..cut);
    }
    Some((start, buf))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Input plus cache tokens of one transcript line, when it is a real assistant message.
fn context_of(line: &[u8]) -> Option<u64> {
    if !contains(line, b"\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_slice(line).ok()?;
    if v["type"] != "assistant" || v["message"]["model"] == "<synthetic>" {
        return None;
    }
    crate::usage::call_usage(&v["message"]["usage"])
        .map(|u| u.context())
        .filter(|n| *n > 0)
}

/// Context of the last assistant message in the transcript's tail.
pub fn last_context(path: &Path) -> Option<u64> {
    let (_, tail) = read_tail(path, TAIL_BYTES)?;
    tail.split(|b| *b == b'\n').rev().find_map(context_of)
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    Some(head)
}

/// Context of the first assistant message: what the agent started with.
fn first_context(path: &Path) -> Option<u64> {
    read_head(path)?
        .split(|b| *b == b'\n')
        .find_map(context_of)
}

/// The text of the user messages in the transcript's head: the prompt the agent was started with.
pub fn opening_text(path: &Path) -> Option<String> {
    let head = read_head(path)?;
    let mut out = String::new();
    for line in head.split(|b| *b == b'\n') {
        if !contains(line, b"\"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["type"] != "user" {
            continue;
        }
        match &v["message"]["content"] {
            Value::String(text) => out.push_str(text),
            Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .for_each(|text| out.push_str(text)),
            _ => (),
        }
        out.push('\n');
    }
    (!out.is_empty()).then_some(out)
}

/// The id of the last compaction boundary in the transcript's tail.
pub fn last_compaction(path: &Path) -> Option<String> {
    let (start, tail) = read_tail(path, TAIL_BYTES)?;
    let mut offset = start + tail.len() as u64;
    for line in tail.split(|b| *b == b'\n').rev() {
        offset = offset.saturating_sub(line.len() as u64 + 1);
        if !contains(line, b"compact_boundary") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["type"] == "system" && v["subtype"] == "compact_boundary" {
            return Some(
                v["uuid"]
                    .as_str()
                    .or_else(|| v["timestamp"].as_str())
                    .map_or_else(|| format!("offset-{offset}"), str::to_owned),
            );
        }
    }
    None
}

/// The agent id, reduced to characters safe in a file name.
pub fn agent_key(event: &Value) -> Option<String> {
    let id: String = event
        .get("agent_id")?
        .as_str()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    (!id.is_empty()).then_some(id)
}

/// True when the hook fires inside a chronos dispatcher.
pub fn in_dispatcher(event: &Value) -> bool {
    agent_key(event).is_some()
        && event.get("agent_type").and_then(Value::as_str) == Some(DISPATCHER)
}

/// The subagent's own transcript. Hooks get the main session's `transcript_path`; Claude Code keeps
/// each subagent's transcript at `<session>/subagents/agent-<id>.jsonl` beside it. Never the
/// main transcript: that would measure the core's context instead.
pub fn agent_transcript(event: &Value) -> Option<PathBuf> {
    let given = PathBuf::from(event.get("transcript_path")?.as_str()?);
    let id = agent_key(event)?;
    let stem = given.file_stem()?.to_str()?;
    if stem.starts_with("agent-") {
        return given.is_file().then_some(given);
    }
    let dir = given.with_extension("").join("subagents");
    let bare = id.strip_prefix("agent-").unwrap_or(&id);
    let own = dir.join(format!("agent-{bare}.jsonl"));
    own.is_file().then_some(own)
}

/// The refusal a dispatcher gets once its context reaches `limit`, or None.
pub fn handoff_refusal(event: &Value, limit: u64, module: Option<&str>) -> Option<String> {
    if !in_dispatcher(event) {
        return None;
    }
    let transcript = agent_transcript(event)?;
    let context = last_context(&transcript)?;
    if context < limit {
        return None;
    }
    // A dispatcher that started near the limit would hand off forever.
    if first_context(&transcript).is_some_and(|first| first >= limit / 2) {
        return None;
    }
    let module = module.map_or(String::new(), |m| format!("\"--module\", \"{m}\", "));
    Some(format!(
        "Your context is {}k tokens, past the {}k handoff limit, so this call was not started. Start nothing new: call `orch` with [\"handoff\", {module}\"--repo\", <project dir>, \"--next\", <first step for your successor>, \"--in-flight\", <units started and not finished>, \"--stuck\", <why a unit is stuck>, \"--decisions\", <choices you made>] (at most 400, 600, 600 and 800 characters), then return your report. A fresh dispatcher continues from the handoff and the store.",
        context / 1000,
        limit / 1000
    ))
}
