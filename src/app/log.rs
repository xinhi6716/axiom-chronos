use crate::usage::quota::{self, Provider, ProviderQuota, Quotas};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(crate) const BUDGET: u64 = 4 << 20;
const HEAD: u64 = 64 << 10;
const TAIL: u64 = 512 << 10;
const BRIDGE: &str = crate::session::BRIDGE;

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub log: PathBuf,
    pub skill: String,
    /// Local-time stamp from the file name, `YYYYMMDD-HHMMSS`.
    pub started: Option<String>,
    pub started_unix: Option<i64>,
    pub session: Option<String>,
    pub cwd: Option<String>,
    pub bytes: u64,
    pub modified: Option<String>,
    pub modified_unix: Option<u64>,
    pub summary: Option<Summary>,
    pub scan: Option<Scan>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total_cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub num_turns: Option<u64>,
    pub is_error: bool,
    /// The session's final message.
    pub result: Option<String>,
    pub models: BTreeMap<String, ModelUsage>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelUsage {
    pub cost_usd: Option<f64>,
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_creation: Option<u64>,
    pub context_window: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    pub scanned: u64,
    pub complete: bool,
    pub events: u64,
    pub malformed: u64,
    pub codex: BTreeMap<String, Tokens>,
    pub agents: Vec<AgentView>,
    pub open: Vec<OpenCall>,
    pub finished: BTreeMap<String, u64>,
    #[serde(skip)]
    pub rate_limit: Option<ProviderQuota>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub calls: u64,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    #[serde(skip)]
    pub missing: [bool; 3],
}
#[derive(Debug, Clone, Serialize)]
pub struct AgentView {
    pub id: String,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub peak_context: Option<u64>,
    pub output_tokens: u64,
    pub messages: u64,
    pub open: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct OpenCall {
    pub id: String,
    pub tool: String,
    pub kind: Option<String>,
    pub detail: Option<String>,
}

#[derive(Default)]
pub(crate) struct State {
    seen: Option<(u64, Option<SystemTime>)>,
    head: Option<Head>,
    summary: Option<Summary>,
    scan: Option<ScanState>,
}
#[derive(Default)]
struct Head {
    session: Option<String>,
    cwd: Option<String>,
}
#[derive(Default)]
struct Agent {
    kind: Option<String>,
    description: Option<String>,
    call: Option<String>,
    peak: Option<u64>,
    output: u64,
    messages: u64,
}
struct Call {
    tool: String,
    kind: Option<String>,
    detail: Option<String>,
}
struct Task {
    kind: Option<String>,
    description: Option<String>,
    call: Option<String>,
}
#[derive(Default)]
struct ScanState {
    offset: u64,
    partial: Vec<u8>,
    complete: bool,
    events: u64,
    malformed: u64,
    summary: Option<Summary>,
    codex: BTreeMap<String, Tokens>,
    agents: BTreeMap<String, Agent>,
    calls: Vec<(String, Call)>,
    tasks: HashMap<String, Task>,
    finished: BTreeMap<String, u64>,
    last_timestamp: Option<String>,
    log_start_unix: Option<i64>,
    quotas: Quotas,
}

fn s(v: &Value) -> Option<String> {
    v.as_str().filter(|x| !x.is_empty()).map(str::to_owned)
}
fn read_range(path: &Path, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(len.min(BUDGET) as usize);
    f.take(len).read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn name_parts(name: &str) -> (String, Option<String>) {
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let b = stem.as_bytes();
    let stamped = b.len() > 16
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'-';
    if !stamped {
        return (stem.to_string(), None);
    }
    let mut skill = &stem[16..];
    if let Some((head, tail)) = skill.rsplit_once('-') {
        if !tail.is_empty() && tail.bytes().all(|c| c.is_ascii_digit()) {
            skill = head;
        }
    }
    (skill.to_string(), Some(stem[..15].to_string()))
}
fn stamp_unix(stamp: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| stamp.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(4..6)?, n(6..8)?);
    let (hh, mm, ss) = (n(9..11)?, n(11..13)?, n(13..15)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let local = (era * 146097 + doe - 719468) * 86400 + hh * 3600 + mm * 60 + ss;
    Some(local - i64::from(crate::session::local_offset_secs(local)))
}
fn summary(event: &Value) -> Summary {
    let models = event["modelUsage"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        ModelUsage {
                            cost_usd: v["costUSD"].as_f64(),
                            input: v["inputTokens"].as_u64(),
                            output: v["outputTokens"].as_u64(),
                            cache_read: v["cacheReadInputTokens"].as_u64(),
                            cache_creation: v["cacheCreationInputTokens"].as_u64(),
                            context_window: v["contextWindow"].as_u64(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Summary {
        total_cost_usd: event["total_cost_usd"].as_f64(),
        duration_ms: event["duration_ms"].as_u64(),
        num_turns: event["num_turns"].as_u64(),
        is_error: event["is_error"].as_bool().unwrap_or(false),
        result: event["result"].as_str().map(str::to_owned),
        models,
    }
}
fn head(path: &Path, size: u64, label: &str, warnings: &mut Vec<String>) -> Head {
    let bytes = match read_range(path, 0, size.min(HEAD)) {
        Ok(b) => b,
        Err(e) => {
            warnings.push(format!("{label}: {e}"));
            return Head::default();
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(session) = s(&v["session_id"]) {
                return Head {
                    session: Some(session),
                    cwd: s(&v["cwd"]),
                };
            }
        }
    }
    Head::default()
}
fn tail(path: &Path, size: u64, label: &str, warnings: &mut Vec<String>) -> Option<Summary> {
    let start = size.saturating_sub(TAIL);
    let bytes = match read_range(path, start, size - start) {
        Ok(b) => b,
        Err(e) => {
            warnings.push(format!("{label}: {e}"));
            return None;
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let text = if start > 0 {
        &text[text.find('\n')? + 1..]
    } else {
        &text
    };
    let mut found = None;
    for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if v["type"] == "result" {
                found = Some(summary(&v));
            }
        }
    }
    found
}
impl ScanState {
    fn advance(&mut self, path: &Path, size: u64, budget: u64, label: &str, warnings: &mut Vec<String>) {
        let want = size.saturating_sub(self.offset).min(budget);
        let bytes = match read_range(path, self.offset, want) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(format!("{label}: {e}"));
                return;
            }
        };
        self.offset += bytes.len() as u64;
        let mut buf = std::mem::take(&mut self.partial);
        buf.extend_from_slice(&bytes);
        let mut start = 0;
        while let Some(i) = buf[start..].iter().position(|b| *b == b'\n') {
            self.line(&buf[start..start + i]);
            start += i + 1;
        }
        let rest = &buf[start..];
        self.complete = self.offset >= size;
        self.partial = if self.complete && !rest.is_empty() {
            match serde_json::from_slice::<Value>(rest) {
                Ok(v) => {
                    self.event(&v);
                    Vec::new()
                }
                Err(_) => rest.to_vec(),
            }
        } else {
            rest.to_vec()
        };
    }
    fn line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        match serde_json::from_slice::<Value>(line) {
            Ok(v) => self.event(&v),
            Err(_) => self.malformed += 1,
        }
    }
    fn event(&mut self, v: &Value) {
        self.events += 1;
        match v["type"].as_str().unwrap_or("") {
            "rate_limit_event" => {
                if let Some(observed_at) =
                    quota::claude_log_time(self.last_timestamp.as_deref(), self.log_start_unix)
                {
                    if let Some(update) = quota::parse_claude_event(v, observed_at) {
                        self.quotas.merge(Provider::Claude, update);
                    }
                }
            }
            "system" if v["subtype"] == "task_started" => {
                if let Some(id) = s(&v["task_id"]) {
                    self.tasks.insert(
                        id,
                        Task {
                            kind: s(&v["subagent_type"]),
                            description: s(&v["description"]),
                            call: s(&v["tool_use_id"]),
                        },
                    );
                }
            }
            "assistant" => self.assistant(v),
            "user" => self.user(v),
            "result" => self.summary = Some(summary(v)),
            _ => {}
        }
        if let Some(timestamp) = v["timestamp"].as_str() {
            self.last_timestamp = Some(timestamp.to_owned());
        }
    }
    fn assistant(&mut self, v: &Value) {
        let id = s(&v["agent_id"]).unwrap_or_else(|| "main".into());
        let parent = s(&v["parent_tool_use_id"]);
        let agent = self.agents.entry(id.clone()).or_default();
        agent.messages += 1;
        if let Some(u) = v["message"]["usage"].as_object() {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let context =
                n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
            agent.peak = Some(agent.peak.unwrap_or(0).max(context));
            agent.output += n("output_tokens");
        }
        if agent.kind.is_none() && id != "main" {
            if let Some(task) = self.tasks.get(&id) {
                agent.kind = task.kind.clone();
                agent.description = task.description.clone();
                agent.call = task.call.clone();
            } else if let Some(p) = &parent {
                if let Some((_, c)) = self.calls.iter().find(|(cid, c)| cid == p && c.tool == "Agent")
                {
                    agent.kind = c.kind.clone();
                    agent.description = c.detail.clone();
                    agent.call = Some(p.clone());
                }
            }
        }
        let Some(blocks) = v["message"]["content"].as_array() else {
            return;
        };
        for b in blocks.iter().filter(|b| b["type"] == "tool_use") {
            let Some(cid) = s(&b["id"]) else { continue };
            let name = b["name"].as_str().unwrap_or("");
            let input = &b["input"];
            let call = if matches!(name, "Agent" | "Task") {
                Call {
                    tool: "Agent".into(),
                    kind: s(&input["subagent_type"]),
                    detail: s(&input["description"]),
                }
            } else if let Some(tool) = name.strip_prefix(BRIDGE) {
                match tool {
                    "codex_run" => Call {
                        tool: tool.into(),
                        kind: s(&input["role"]).or_else(|| s(&input["model"])),
                        detail: s(&input["unit"]).or_else(|| {
                            input["brief"].as_str().and_then(|brief| {
                                brief.lines().find_map(|l| {
                                    l.strip_prefix("GOAL")
                                        .map(str::trim)
                                        .filter(|g| !g.is_empty())
                                        .map(|g| g.chars().take(100).collect())
                                })
                            })
                        }),
                    },
                    "orch" => {
                        let args: Vec<&str> = input["args"]
                            .as_array()
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        Call {
                            tool: tool.into(),
                            kind: args
                                .iter()
                                .find(|a| !a.starts_with("--"))
                                .map(|a| format!("orch:{a}")),
                            detail: Some(args.join(" ").chars().take(100).collect()),
                        }
                    }
                    _ => Call {
                        tool: tool.into(),
                        kind: None,
                        detail: None,
                    },
                }
            } else {
                continue;
            };
            self.calls.push((cid, call));
        }
    }
    fn user(&mut self, v: &Value) {
        let Some(blocks) = v["message"]["content"].as_array() else {
            return;
        };
        for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
            let id = b["tool_use_id"].as_str().unwrap_or("");
            let Some(i) = self.calls.iter().position(|(cid, _)| cid == id) else {
                continue;
            };
            let (_, call) = self.calls.remove(i);
            match call.tool.as_str() {
                "Agent" => {
                    *self
                        .finished
                        .entry(call.kind.unwrap_or_else(|| "agent".into()))
                        .or_default() += 1;
                }
                "codex_run" => {
                    let content = &b["content"];
                    let text = match content {
                        Value::String(t) => t.clone(),
                        Value::Array(a) => a
                            .iter()
                            .filter(|x| x["type"] == "text")
                            .filter_map(|x| x["text"].as_str())
                            .collect(),
                        _ => continue,
                    };
                    let Ok(report) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    let key = s(&report["model"])
                        .or(call.kind)
                        .unwrap_or_else(|| "codex".into());
                    let u = report["usage"].as_object();
                    let n = |k: &str| u.and_then(|u| u.get(k)).and_then(Value::as_u64);
                    let t = self.codex.entry(key).or_default();
                    t.calls += 1;
                    for (ix, value) in [n("inputTokens"), n("cachedInputTokens"), n("outputTokens")].iter().enumerate() {
                        t.missing[ix] |= value.is_none();
                    }
                    t.input += n("inputTokens").unwrap_or(0);
                    t.cached += n("cachedInputTokens").unwrap_or(0);
                    t.output += n("outputTokens").unwrap_or(0);
                }
                _ => {}
            }
        }
    }
    fn view(&self) -> Scan {
        let open = |a: &Agent| {
            a.call
                .as_ref()
                .is_some_and(|c| self.calls.iter().any(|(id, _)| id == c))
        };
        let agent = |(id, a): (&String, &Agent)| AgentView {
            id: id.clone(),
            kind: a.kind.clone(),
            description: a.description.clone(),
            peak_context: a.peak,
            output_tokens: a.output,
            messages: a.messages,
            open: open(a),
        };
        let mut agents: Vec<AgentView> = self
            .agents
            .iter()
            .filter(|(id, _)| *id == "main")
            .map(agent)
            .collect();
        agents.extend(self.agents.iter().filter(|(id, _)| *id != "main").map(agent));
        Scan {
            scanned: self.offset,
            complete: self.complete,
            events: self.events,
            malformed: self.malformed,
            codex: self.codex.clone(),
            agents,
            open: self
                .calls
                .iter()
                .map(|(id, c)| OpenCall {
                    id: id.clone(),
                    tool: c.tool.clone(),
                    kind: c.kind.clone(),
                    detail: c.detail.clone(),
                })
                .collect(),
            finished: self.finished.clone(),
            rate_limit: (self.quotas.claude.five_hour.is_some()
                || self.quotas.claude.weekly.is_some())
            .then(|| self.quotas.claude.clone()),
        }
    }
}
pub(crate) fn refresh(
    states: &mut HashMap<PathBuf, State>,
    dir: &Path,
    deep: &dyn Fn(&str) -> bool,
    budget: u64,
    warnings: &mut Vec<String>,
) -> Vec<Run> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(e) => {
            warnings.push(format!("logs: {e}"));
            return vec![];
        }
    };
    let mut runs = Vec::new();
    let mut present = HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        present.insert(path.clone());
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = format!("logs/{name}");
        let (skill, started) = name_parts(&name);
        let started_unix = started.as_deref().and_then(stamp_unix);
        let (size, modified) = (meta.len(), meta.modified().ok());
        let state = states.entry(path.clone()).or_default();
        let changed = state.seen != Some((size, modified));
        if state.head.is_none() || (changed && state.head.as_ref().is_some_and(|h| h.session.is_none()))
        {
            state.head = Some(head(&path, size, &label, warnings));
        }
        let session = state.head.as_ref().and_then(|h| h.session.clone());
        if session.as_deref().is_some_and(deep) {
            let scan = state.scan.get_or_insert_with(ScanState::default);
            if size < scan.offset {
                warnings.push(format!("{label}: log shrank; rescanning"));
                *scan = ScanState::default();
            }
            scan.log_start_unix = started_unix;
            if scan.offset < size || !scan.complete {
                scan.advance(&path, size, budget, &label, warnings);
            }
            if scan.summary.is_some() {
                state.summary = scan.summary.clone();
            }
        } else if changed {
            state.summary = tail(&path, size, &label, warnings);
        }
        if let Some(scan) = &state.scan {
            if scan.malformed > 0 {
                warnings.push(format!("{label}: {} malformed line(s) skipped", scan.malformed));
            }
        }
        state.seen = Some((size, modified));
        runs.push(Run {
            log: path,
            skill,
            started_unix,
            started,
            session,
            cwd: state.head.as_ref().and_then(|h| h.cwd.clone()),
            bytes: size,
            modified: modified.map(super::iso),
            modified_unix: modified.and_then(super::unix),
            summary: state.summary.clone(),
            scan: state.scan.as_ref().map(ScanState::view),
        });
    }
    states.retain(|p, _| present.contains(p));
    runs
}
