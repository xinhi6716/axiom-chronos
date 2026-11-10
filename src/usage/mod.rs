//! Per-run Claude and Codex usage, measured from the records a run leaves behind.
//!
//! A run is one Claude session. Its transcript and subagent transcripts are preferred over the
//! headless stream-json log: the log holds every assistant event at stream start, so its
//! `output_tokens` are far below what was billed.

pub mod price;
mod report;
mod scan;

pub use report::render;
pub(crate) use scan::call_usage;

use crate::base::{self, Failure, Res};
use scan::{Call, FileScan};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub const CONTEXT_WARN: u64 = 500_000;
pub const LARGE_CACHE_WRITE: u64 = 50_000;
pub const CACHE_GAP_MS: i64 = 5 * 60 * 1000;
pub const LARGE_READ_BYTES: u64 = 50_000;

const MAIN: &str = "main";
const UNKNOWN_AGENT: &str = "unknown";

#[derive(Serialize, Default, Clone, Debug)]
pub struct Tally {
    pub calls: u64,
    pub input: u64,
    pub cache_write: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    pub web_searches: u64,
    pub usd: Option<f64>,
    pub priced_usd: f64,
    pub unpriced_calls: u64,
    pub peak_context: u64,
    pub gap_rewrites: u64,
    pub gap_rewrite_tokens: u64,
    pub gap_rewrite_usd: f64,
}
impl Tally {
    fn add(&mut self, call: &Call) {
        let u = &call.usage;
        self.calls += 1;
        self.input += u.input;
        self.cache_write += u.cache_write;
        self.cache_write_5m += u.cache_write_5m();
        self.cache_write_1h += u.cache_write_1h;
        self.cache_read += u.cache_read;
        self.output += u.output;
        self.web_searches += u.web_searches;
        match price::usd(&call.model, u) {
            Some(usd) => self.priced_usd += usd,
            None => self.unpriced_calls += 1,
        }
        self.usd = (self.unpriced_calls == 0).then_some(self.priced_usd);
        self.peak_context = self.peak_context.max(u.context());
        if call.gap_rewrite {
            self.gap_rewrites += 1;
            self.gap_rewrite_tokens += u.cache_write;
            self.gap_rewrite_usd += price::cache_write_usd(&call.model, u).unwrap_or(0.0);
        }
    }
}

#[derive(Serialize, Debug)]
pub struct AgentRow {
    pub agent_type: String,
    pub model: String,
    #[serde(flatten)]
    pub tally: Tally,
}

#[derive(Serialize, Debug)]
pub struct Instance {
    pub id: String,
    pub agent_type: String,
    pub description: Option<String>,
    pub peak_model: String,
    pub compactions: u64,
    #[serde(flatten)]
    pub tally: Tally,
}

#[derive(Serialize, Default, Clone, Copy, Debug, PartialEq)]
pub struct CodexTokens {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
    pub reasoning_output: u64,
    pub total: u64,
}
impl CodexTokens {
    fn from_report(usage: &Value) -> Option<Self> {
        usage.as_object()?;
        let n = |key: &str| usage[key].as_u64().unwrap_or(0);
        Some(Self {
            input: n("inputTokens"),
            cached_input: n("cachedInputTokens"),
            output: n("outputTokens"),
            reasoning_output: n("reasoningOutputTokens"),
            total: n("totalTokens"),
        })
    }
}

#[derive(Serialize, Debug)]
pub struct CodexRow {
    pub model: String,
    pub runs: u64,
    #[serde(flatten)]
    pub tokens: CodexTokens,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Warning {
    ContextHigh {
        agent_type: String,
        instance: String,
        model: String,
        peak_context: u64,
    },
    CacheRewritesAfterGap {
        agent_type: String,
        instance: String,
        count: u64,
        tokens: u64,
        usd: f64,
    },
    LargeFileRead {
        agent_type: String,
        instance: String,
        path: String,
        reads: u64,
        largest_bytes: u64,
    },
    CopiedHistoryNotSeparable {
        file: String,
        zeroed_calls: u64,
    },
}

impl Warning {
    fn severity(&self) -> (u8, u64) {
        match self {
            Self::ContextHigh { peak_context, .. } => (3, *peak_context),
            Self::CacheRewritesAfterGap { tokens, .. } => (2, *tokens),
            Self::LargeFileRead { largest_bytes, .. } => (1, *largest_bytes),
            Self::CopiedHistoryNotSeparable { zeroed_calls, .. } => (0, *zeroed_calls),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct Thresholds {
    pub context_warn: u64,
    pub large_cache_write: u64,
    pub cache_gap_seconds: i64,
    pub large_read_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct Prices {
    pub as_of: &'static str,
    pub source: &'static str,
}

#[derive(Serialize, Debug)]
pub struct Ledger {
    pub session: String,
    pub cwd: Option<String>,
    pub source: &'static str,
    pub subagent_files: u64,
    pub output_tokens_partial: bool,
    pub totals: Tally,
    pub by_agent: Vec<AgentRow>,
    pub instances: Vec<Instance>,
    pub codex: Vec<CodexRow>,
    pub codex_runs_without_usage: u64,
    pub warnings: Vec<Warning>,
    pub copied_calls: u64,
    pub compactions: u64,
    pub skipped_lines: u64,
    pub claude_code_usd: Option<f64>,
    pub prices: Prices,
    pub thresholds: Thresholds,
}

pub struct Locations {
    pub home: PathBuf,
    pub projects: PathBuf,
}
impl Locations {
    pub fn from_env() -> Self {
        let config = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(
                    std::env::var_os("USERPROFILE")
                        .or_else(|| std::env::var_os("HOME"))
                        .unwrap_or_default(),
                )
                .join(".claude")
            });
        Self {
            home: base::axiom_home(),
            projects: config.join("projects"),
        }
    }
}

/// Claude Code's project directory name: every character outside `[A-Za-z0-9]` becomes `-`.
pub fn transcript_dir_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[derive(Default)]
struct Run {
    cwd: Option<String>,
    transcript: Option<PathBuf>,
    logs: Vec<PathBuf>,
    activity: Option<SystemTime>,
}
impl Run {
    fn touch(&mut self, path: &Path) {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.activity = self.activity.max(modified);
    }
    fn has_records(&self) -> bool {
        self.transcript.is_some() || !self.logs.is_empty()
    }
}

fn io_failure(path: &Path, e: std::io::Error) -> Failure {
    Failure::new("io", format!("{}: {e}", path.display()))
}

fn entries_with_suffix(dir: &Path, suffix: &str) -> Res<Vec<PathBuf>> {
    let reader = match std::fs::read_dir(dir) {
        Ok(reader) => reader,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_failure(dir, e)),
    };
    let mut paths = Vec::new();
    for entry in reader {
        let path = entry.map_err(|e| io_failure(dir, e))?.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(suffix))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn find_transcript(projects: &Path, session: &str, cwd: Option<&str>) -> Res<Option<PathBuf>> {
    let name = format!("{session}.jsonl");
    if let Some(cwd) = cwd {
        let path = projects.join(transcript_dir_name(cwd)).join(&name);
        if path.is_file() {
            return Ok(Some(path));
        }
    }
    let reader = match std::fs::read_dir(projects) {
        Ok(reader) => reader,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_failure(projects, e)),
    };
    let mut dirs = Vec::new();
    for entry in reader {
        dirs.push(entry.map_err(|e| io_failure(projects, e))?.path());
    }
    dirs.sort();
    Ok(dirs
        .into_iter()
        .map(|d| d.join(&name))
        .find(|p| p.is_file()))
}

fn discover(locations: &Locations) -> Res<BTreeMap<String, Run>> {
    let mut runs = BTreeMap::<String, Run>::new();
    for log in entries_with_suffix(&locations.home.join("logs"), ".jsonl")? {
        let Some((session, cwd)) = scan::log_identity(&log) else {
            continue;
        };
        let run = runs.entry(session).or_default();
        run.cwd = run.cwd.take().or(cwd);
        run.touch(&log);
        run.logs.push(log);
    }
    for goal in entries_with_suffix(&locations.home.join("goals"), ".json")? {
        let Some(session) = goal.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let cwd = std::fs::read(&goal)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|v| v["cwd"].as_str().map(str::to_owned));
        let run = runs.entry(session.to_owned()).or_default();
        run.cwd = run.cwd.take().or(cwd);
        run.touch(&goal);
    }
    for (session, run) in &mut runs {
        run.transcript = find_transcript(&locations.projects, session, run.cwd.as_deref())?;
        if let Some(transcript) = run.transcript.clone() {
            run.touch(&transcript);
        }
    }
    Ok(runs)
}

fn locate(locations: &Locations, session: Option<&str>) -> Res<(String, Run)> {
    if let Some(session) = session {
        base::require_slug("session", session)?;
    }
    let mut runs = discover(locations)?;
    let Some(session) = session else {
        return runs
            .into_iter()
            .filter(|(_, run)| run.has_records())
            .max_by_key(|(_, run)| run.activity)
            .ok_or_else(|| {
                Failure::new("not_found", "no runs found; name one with --session <id>")
            });
    };
    if let Some(run) = runs.remove(session).filter(Run::has_records) {
        return Ok((session.to_owned(), run));
    }
    let transcript = find_transcript(&locations.projects, session, None)?
        .ok_or_else(|| Failure::new("not_found", format!("no records for session {session}")))?;
    Ok((
        session.to_owned(),
        Run {
            transcript: Some(transcript),
            ..Run::default()
        },
    ))
}

struct Agent {
    id: String,
    agent_type: String,
    description: Option<String>,
    calls: Vec<Call>,
    compactions: u64,
}

#[derive(Default)]
struct Agents {
    agents: Vec<Agent>,
    index: HashMap<String, usize>,
}
impl Agents {
    fn slot(&mut self, id: &str, agent_type: &str, description: Option<&str>) -> usize {
        if let Some(&at) = self.index.get(id) {
            return at;
        }
        self.agents.push(Agent {
            id: id.to_owned(),
            agent_type: agent_type.to_owned(),
            description: description.map(str::to_owned),
            calls: Vec::new(),
            compactions: 0,
        });
        self.index.insert(id.to_owned(), self.agents.len() - 1);
        self.agents.len() - 1
    }
}

struct SourceFile {
    path: PathBuf,
    instance: String,
    agent_type: String,
    description: Option<String>,
}

fn subagent_sources(transcript: &Path) -> Res<Vec<SourceFile>> {
    let mut sources = Vec::new();
    for path in entries_with_suffix(&transcript.with_extension("").join("subagents"), ".jsonl")? {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(instance) = stem.strip_prefix("agent-") else {
            continue;
        };
        let meta = std::fs::read(path.with_extension("meta.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        sources.push(SourceFile {
            instance: instance.to_owned(),
            agent_type: meta["agentType"]
                .as_str()
                .unwrap_or(UNKNOWN_AGENT)
                .to_owned(),
            description: meta["description"].as_str().map(str::to_owned),
            path,
        });
    }
    Ok(sources)
}

fn mark_gap_rewrites(calls: &mut [Call]) {
    calls.sort_by_key(|call| call.first_ms.unwrap_or(i64::MAX));
    let mut previous_end: Option<i64> = None;
    for call in calls.iter_mut() {
        if let (Some(end), Some(start)) = (previous_end, call.first_ms) {
            call.gap_rewrite =
                start - end > CACHE_GAP_MS && call.usage.cache_write >= LARGE_CACHE_WRITE;
        }
        previous_end = previous_end.max(call.last_ms);
    }
}

fn build(session: String, run: Run) -> Res<Ledger> {
    let transcript_source = run.transcript.is_some();
    let mut sources = Vec::new();
    if let Some(transcript) = &run.transcript {
        sources.push(SourceFile {
            path: transcript.clone(),
            instance: MAIN.to_owned(),
            agent_type: MAIN.to_owned(),
            description: None,
        });
        sources.extend(subagent_sources(transcript)?);
    } else {
        sources.extend(run.logs.iter().map(|path| SourceFile {
            path: path.clone(),
            instance: MAIN.to_owned(),
            agent_type: MAIN.to_owned(),
            description: None,
        }));
    }
    let subagent_files = if transcript_source {
        sources.len() as u64 - 1
    } else {
        0
    };

    let mut agents = Agents::default();
    let mut seen_calls = HashMap::<String, (usize, usize)>::new();
    let mut seen_tools = HashSet::<String>::new();
    let mut warnings = Vec::new();
    let mut reads = BTreeMap::<(String, String, String), (u64, u64)>::new();
    let mut codex = BTreeMap::<String, CodexRow>::new();
    let mut codex_runs_without_usage = 0;
    let (mut copied_calls, mut compactions, mut skipped_lines) = (0, 0, 0);
    let mut claude_code_usd = None;

    for source in &sources {
        let FileScan {
            calls,
            large_reads,
            codex_runs,
            subagents,
            copied_calls: copied,
            inseparable_copies,
            skipped_lines: skipped,
            compactions: compacted,
            claude_code_usd: reported,
            ..
        } = scan::scan(&source.path)?;
        copied_calls += copied;
        compactions += compacted;
        skipped_lines += skipped;
        if source.instance == MAIN {
            claude_code_usd = reported.or(claude_code_usd);
        }
        if inseparable_copies > 0 {
            warnings.push(Warning::CopiedHistoryNotSeparable {
                file: source.path.display().to_string(),
                zeroed_calls: inseparable_copies,
            });
        }
        let own = agents.slot(
            &source.instance,
            &source.agent_type,
            source.description.as_deref(),
        );
        agents.agents[own].compactions += compacted;
        let attribute = |agents: &mut Agents, parent: Option<&str>| match parent {
            Some(parent) => {
                let (kind, description) = subagents
                    .get(parent)
                    .map(|(k, d)| (k.as_str(), d.as_deref()))
                    .unwrap_or((UNKNOWN_AGENT, None));
                agents.slot(parent, kind, description)
            }
            None => own,
        };
        for call in calls {
            match seen_calls.get(&call.id) {
                Some(&(agent, at)) => agents.agents[agent].calls[at].merge(&call),
                None => {
                    let agent = attribute(&mut agents, call.parent.as_deref());
                    seen_calls.insert(call.id.clone(), (agent, agents.agents[agent].calls.len()));
                    agents.agents[agent].calls.push(call);
                }
            }
        }
        for read in large_reads {
            if !seen_tools.insert(read.tool_use_id.clone()) {
                continue;
            }
            let at = attribute(&mut agents, read.parent.as_deref());
            let key = (
                agents.agents[at].agent_type.clone(),
                agents.agents[at].id.clone(),
                read.path,
            );
            let (count, largest) = reads.entry(key).or_default();
            *count += 1;
            *largest = (*largest).max(read.bytes);
        }
        for run in codex_runs {
            if !seen_tools.insert(run.tool_use_id.clone()) {
                continue;
            }
            let Some(tokens) = run.tokens else {
                codex_runs_without_usage += 1;
                continue;
            };
            let model = run.model.unwrap_or_else(|| UNKNOWN_AGENT.to_owned());
            let row = codex.entry(model.clone()).or_insert_with(|| CodexRow {
                model,
                runs: 0,
                tokens: CodexTokens::default(),
            });
            row.runs += 1;
            row.tokens.input += tokens.input;
            row.tokens.cached_input += tokens.cached_input;
            row.tokens.output += tokens.output;
            row.tokens.reasoning_output += tokens.reasoning_output;
            row.tokens.total += tokens.total;
        }
    }

    let mut totals = Tally {
        usd: Some(0.0),
        ..Tally::default()
    };
    let mut rows = BTreeMap::<(String, String), Tally>::new();
    let mut instances = Vec::new();
    for agent in &mut agents.agents {
        if agent.calls.is_empty() {
            continue;
        }
        mark_gap_rewrites(&mut agent.calls);
        let mut instance = Tally::default();
        let mut peak_model = "";
        for call in &agent.calls {
            if call.usage.context() > instance.peak_context {
                peak_model = &call.model;
            }
            instance.add(call);
            totals.add(call);
            rows.entry((agent.agent_type.clone(), call.model.clone()))
                .or_default()
                .add(call);
        }
        if instance.peak_context > CONTEXT_WARN {
            warnings.push(Warning::ContextHigh {
                agent_type: agent.agent_type.clone(),
                instance: agent.id.clone(),
                model: peak_model.to_owned(),
                peak_context: instance.peak_context,
            });
        }
        if instance.gap_rewrites > 0 {
            warnings.push(Warning::CacheRewritesAfterGap {
                agent_type: agent.agent_type.clone(),
                instance: agent.id.clone(),
                count: instance.gap_rewrites,
                tokens: instance.gap_rewrite_tokens,
                usd: instance.gap_rewrite_usd,
            });
        }
        instances.push(Instance {
            id: agent.id.clone(),
            agent_type: agent.agent_type.clone(),
            description: agent.description.clone(),
            peak_model: peak_model.to_owned(),
            compactions: agent.compactions,
            tally: instance,
        });
    }
    instances.sort_by(|a, b| {
        b.tally
            .peak_context
            .cmp(&a.tally.peak_context)
            .then(a.id.cmp(&b.id))
    });
    let mut by_agent: Vec<AgentRow> = rows
        .into_iter()
        .map(|((agent_type, model), tally)| AgentRow {
            agent_type,
            model,
            tally,
        })
        .collect();
    by_agent.sort_by(|a, b| b.tally.priced_usd.total_cmp(&a.tally.priced_usd));
    warnings.extend(reads.into_iter().map(
        |((agent_type, instance, path), (reads, largest_bytes))| Warning::LargeFileRead {
            agent_type,
            instance,
            path,
            reads,
            largest_bytes,
        },
    ));
    warnings.sort_by_key(|w| std::cmp::Reverse(w.severity()));

    Ok(Ledger {
        session,
        cwd: run.cwd,
        source: if transcript_source {
            "transcript"
        } else {
            "log"
        },
        subagent_files,
        output_tokens_partial: !transcript_source,
        totals,
        by_agent,
        instances,
        codex: codex.into_values().collect(),
        codex_runs_without_usage,
        warnings,
        copied_calls,
        compactions,
        skipped_lines,
        claude_code_usd,
        prices: Prices {
            as_of: price::PRICES_AS_OF,
            source: price::PRICES_SOURCE,
        },
        thresholds: Thresholds {
            context_warn: CONTEXT_WARN,
            large_cache_write: LARGE_CACHE_WRITE,
            cache_gap_seconds: CACHE_GAP_MS / 1000,
            large_read_bytes: LARGE_READ_BYTES,
        },
    })
}

pub fn ledger(locations: &Locations, session: Option<&str>) -> Res<Ledger> {
    let (session, mut run) = locate(locations, session)?;
    if run.cwd.is_none() {
        if let Some(transcript) = &run.transcript {
            run.cwd = scan::transcript_cwd(transcript);
        }
    }
    build(session, run)
}
