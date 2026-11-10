use super::{price::CallUsage, CodexTokens, LARGE_READ_BYTES};
use crate::base::{Failure, Res};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Deserialize)]
struct Head {
    #[serde(rename = "type", default)]
    kind: String,
    subtype: Option<String>,
    timestamp: Option<String>,
    id: Option<String>,
    #[serde(rename = "requestRef")]
    request_ref: Option<String>,
    parent_tool_use_id: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "totalCostUSD")]
    cost_state_usd: Option<f64>,
    total_cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct AssistantLine {
    message: AssistantMessage,
}
#[derive(Deserialize)]
struct AssistantMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
}
#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation: Option<RawCacheCreation>,
    server_tool_use: Option<RawServerToolUse>,
    speed: Option<String>,
    inference_geo: Option<String>,
    service_tier: Option<String>,
}
#[derive(Deserialize)]
struct RawCacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct RawServerToolUse {
    web_search_requests: Option<u64>,
}
impl From<RawUsage> for CallUsage {
    fn from(u: RawUsage) -> Self {
        Self {
            input: u.input_tokens.unwrap_or(0),
            cache_write: u.cache_creation_input_tokens.unwrap_or(0),
            cache_write_1h: u
                .cache_creation
                .and_then(|c| c.ephemeral_1h_input_tokens)
                .unwrap_or(0),
            cache_read: u.cache_read_input_tokens.unwrap_or(0),
            output: u.output_tokens.unwrap_or(0),
            web_searches: u
                .server_tool_use
                .and_then(|s| s.web_search_requests)
                .unwrap_or(0),
            fast: u.speed.as_deref() == Some("fast"),
            us_only: u.inference_geo.as_deref() == Some("us"),
            non_standard_tier: u.service_tier.is_some_and(|t| t != "standard"),
        }
    }
}

/// One `usage` object of a stream-json or transcript event; None when it is absent or unreadable.
pub(crate) fn call_usage(value: &Value) -> Option<CallUsage> {
    if !value.is_object() {
        return None;
    }
    serde_json::from_value::<RawUsage>(value.clone())
        .ok()
        .map(Into::into)
}

#[derive(Deserialize)]
struct ToolLine {
    message: ToolMessage,
}
#[derive(Deserialize)]
struct ToolMessage {
    #[serde(default)]
    content: Vec<ToolBlock>,
}
#[derive(Deserialize)]
struct ToolBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    input: Option<Value>,
    #[serde(default)]
    tool_use_id: String,
    content: Option<ResultContent>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum ResultContent {
    Text(String),
    Blocks(Vec<ResultBlock>),
    Other(serde::de::IgnoredAny),
}
#[derive(Deserialize)]
struct ResultBlock {
    #[serde(default)]
    text: String,
}
impl ResultContent {
    fn text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Blocks(blocks) => blocks.into_iter().map(|b| b.text).collect(),
            Self::Other(_) => String::new(),
        }
    }
}

enum Tool {
    Read { path: String, whole: bool },
    Codex,
}
struct Pending {
    tool: Tool,
    message: String,
}

pub(super) struct Call {
    pub id: String,
    pub model: String,
    pub usage: CallUsage,
    pub reported: [bool; 4],
    pub first_ms: Option<i64>,
    pub last_ms: Option<i64>,
    pub parent: Option<String>,
    pub gap_rewrite: bool,
    request_ref: Option<String>,
    line: usize,
}
impl Call {
    pub fn merge(&mut self, other: &Call) {
        self.usage.merge(&other.usage);
        for (reported, part) in self.reported.iter_mut().zip(other.reported) { *reported |= part; }
        self.first_ms = min_some(self.first_ms, other.first_ms);
        self.last_ms = self.last_ms.max(other.last_ms);
    }
}
fn min_some(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        _ => a.or(b),
    }
}
pub(super) struct LargeRead {
    pub tool_use_id: String,
    pub path: String,
    pub bytes: u64,
    pub parent: Option<String>,
    pub ms: Option<i64>,
    message: String,
}
pub(super) struct CodexRun {
    pub tool_use_id: String,
    pub model: Option<String>,
    pub tokens: Option<CodexTokens>,
    pub ms: Option<i64>,
    message: String,
}

#[derive(Default)]
pub(super) struct FileScan {
    pub calls: Vec<Call>,
    pub large_reads: Vec<LargeRead>,
    pub codex_runs: Vec<CodexRun>,
    pub subagents: HashMap<String, (String, Option<String>)>,
    pub copied_calls: u64,
    pub inseparable_copies: u64,
    pub skipped_lines: u64,
    pub compactions: u64,
    pub claude_code_usd: Option<f64>,
    positions: HashMap<String, usize>,
    pending: HashMap<String, Pending>,
    api_requests: HashSet<String>,
    compaction_lines: Vec<usize>,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

pub(super) fn parse_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (body, fraction) = match s.get(19..)?.strip_suffix('Z')? {
        "" => (&s[..19], 0),
        rest => {
            let digits = rest.strip_prefix('.')?;
            if digits.is_empty() || !digits.bytes().all(|d| d.is_ascii_digit()) {
                return None;
            }
            let ms: String = digits.chars().chain("00".chars()).take(3).collect();
            (&s[..19], ms.parse::<i64>().ok()?)
        }
    };
    let n = |range: std::ops::Range<usize>| body.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hour, minute, second) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000 + fraction)
}

impl FileScan {
    fn ingest(&mut self, line: &[u8], number: usize) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(head) = serde_json::from_slice::<Head>(line) else {
            self.skipped_lines += 1;
            return;
        };
        match head.kind.as_str() {
            "assistant" => self.assistant(line, head, number),
            "user" if contains(line, b"\"tool_result\"") => self.tool_results(
                line,
                head.parent_tool_use_id,
                head.timestamp.as_deref().and_then(parse_ms),
            ),
            "api-request" => {
                if let Some(id) = head.id {
                    self.api_requests.insert(id);
                }
            }
            "system" if head.subtype.as_deref() == Some("compact_boundary") => {
                self.compaction_lines.push(number);
            }
            "cost-state" => self.claude_code_usd = head.cost_state_usd.or(self.claude_code_usd),
            "result" => self.claude_code_usd = head.total_cost_usd.or(self.claude_code_usd),
            _ => {}
        }
    }

    fn assistant(&mut self, line: &[u8], head: Head, number: usize) {
        let Ok(parsed) = serde_json::from_slice::<AssistantLine>(line) else {
            self.skipped_lines += 1;
            return;
        };
        let message = parsed.message;
        let (Some(id), Some(model)) = (message.id, message.model) else {
            self.skipped_lines += 1;
            return;
        };
        if model == "<synthetic>" {
            return;
        }
        let ms = head.timestamp.as_deref().and_then(parse_ms);
        let reported = message.usage.as_ref().map_or([false; 4], |u| [u.input_tokens.is_some(), u.cache_read_input_tokens.is_some(), u.cache_creation_input_tokens.is_some(), u.output_tokens.is_some()]);
        let call = Call {
            id: id.clone(),
            model,
            usage: message.usage.map(CallUsage::from).unwrap_or_default(),
            reported,
            first_ms: ms,
            last_ms: ms,
            parent: head.parent_tool_use_id,
            gap_rewrite: false,
            request_ref: head.request_ref,
            line: number,
        };
        match self.positions.get(&id) {
            Some(&at) => self.calls[at].merge(&call),
            None => {
                self.positions.insert(id.clone(), self.calls.len());
                self.calls.push(call);
            }
        }
        if contains(line, b"\"tool_use\"") {
            self.tool_uses(line, &id);
        }
    }

    fn tool_uses(&mut self, line: &[u8], message: &str) {
        let Ok(parsed) = serde_json::from_slice::<ToolLine>(line) else {
            return;
        };
        for block in parsed.message.content {
            if block.kind != "tool_use" {
                continue;
            }
            let input = block.input.unwrap_or(Value::Null);
            let tool = match block.name.as_str() {
                "Read" => Tool::Read {
                    path: input["file_path"].as_str().unwrap_or("").to_owned(),
                    whole: input["limit"]
                        .as_f64()
                        .map_or(true, |limit| limit > crate::hook::MAX_READ_LINES as f64),
                },
                "Agent" | "Task" => {
                    let kind = input["subagent_type"].as_str().unwrap_or("general-purpose");
                    let description = input["description"].as_str().map(str::to_owned);
                    self.subagents
                        .insert(block.id, (kind.to_owned(), description));
                    continue;
                }
                name if name.ends_with("codex_run") => Tool::Codex,
                _ => continue,
            };
            self.pending.insert(
                block.id,
                Pending {
                    tool,
                    message: message.to_owned(),
                },
            );
        }
    }

    fn tool_results(&mut self, line: &[u8], parent: Option<String>, ms: Option<i64>) {
        let Ok(parsed) = serde_json::from_slice::<ToolLine>(line) else {
            return;
        };
        for block in parsed.message.content {
            if block.kind != "tool_result" {
                continue;
            }
            let Some(pending) = self.pending.remove(&block.tool_use_id) else {
                continue;
            };
            let text = block.content.map(ResultContent::text).unwrap_or_default();
            match pending.tool {
                Tool::Read { path, whole } => {
                    let bytes = text.len() as u64;
                    if whole && bytes >= LARGE_READ_BYTES {
                        self.large_reads.push(LargeRead {
                            tool_use_id: block.tool_use_id,
                            path,
                            bytes,
                            parent: parent.clone(),
                            ms,
                            message: pending.message,
                        });
                    }
                }
                Tool::Codex => {
                    let report: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    self.codex_runs.push(CodexRun {
                        tool_use_id: block.tool_use_id,
                        model: report["model"].as_str().map(str::to_owned),
                        tokens: CodexTokens::from_report(&report["usage"]),
                        ms,
                        message: pending.message,
                    });
                }
            }
        }
    }

    fn finish(&mut self) {
        let mut copied = HashSet::new();
        if !self.api_requests.is_empty() {
            let requests = &self.api_requests;
            self.calls.retain(|call| {
                let own = call
                    .request_ref
                    .as_ref()
                    .map_or(true, |r| requests.contains(r));
                if !own {
                    copied.insert(call.id.clone());
                }
                own
            });
        }
        let mut zeroed = HashSet::new();
        self.calls.retain(|call| {
            if call.usage.is_zero() {
                zeroed.insert(call.id.clone());
            }
            !call.usage.is_zero()
        });
        self.large_reads
            .retain(|r| !copied.contains(&r.message) && !zeroed.contains(&r.message));
        self.codex_runs
            .retain(|r| !copied.contains(&r.message) && !zeroed.contains(&r.message));
        let zeroed = zeroed.len() as u64;
        if self.api_requests.is_empty() {
            self.inseparable_copies = zeroed;
        }
        self.copied_calls = copied.len() as u64 + zeroed;
        let first_own = self.calls.iter().map(|c| c.line).min();
        self.compactions = self
            .compaction_lines
            .iter()
            .filter(|&&line| first_own.is_some_and(|first| line > first))
            .count() as u64;
    }
}

pub(super) fn scan(path: &Path) -> Res<FileScan> {
    let io_error = |e: std::io::Error| Failure::new("io", format!("{}: {e}", path.display()));
    let mut reader = BufReader::with_capacity(1 << 16, File::open(path).map_err(io_error)?);
    let mut scan = FileScan::default();
    let mut line = Vec::new();
    let mut number = 0;
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).map_err(io_error)? == 0 {
            break;
        }
        scan.ingest(&line, number);
        number += 1;
    }
    scan.finish();
    Ok(scan)
}

pub(super) fn log_identity(path: &Path) -> Option<(String, Option<String>)> {
    let mut reader = BufReader::with_capacity(1 << 16, File::open(path).ok()?);
    let mut line = Vec::new();
    for _ in 0..20 {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            return None;
        }
        let Ok(head) = serde_json::from_slice::<Head>(&line) else {
            continue;
        };
        if head.kind == "system" && head.subtype.as_deref() == Some("init") {
            return Some((head.session_id?, head.cwd));
        }
    }
    None
}

pub(super) fn transcript_cwd(path: &Path) -> Option<String> {
    let mut reader = BufReader::with_capacity(1 << 16, File::open(path).ok()?);
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line).ok()? == 0 {
            return None;
        }
        if let Ok(Head { cwd: Some(cwd), .. }) = serde_json::from_slice(&line) {
            return Some(cwd);
        }
    }
}
