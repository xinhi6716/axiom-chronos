//! One member's process: spawn it, meter it, and turn what it prints into an `Answer`.
use super::{Status, ARGON_MODEL, FABLE_MODEL};
use crate::{
    base::{proc, schema, Failure, Res},
    usage::{self, price},
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

const STDERR_KEEP: usize = 64 * 1024;
pub(super) const ARGV_LIMIT_UTF16: usize = 30_000;
/// Parent settings that would change a nested claude: the one-hour cache TTL bills cache
/// writes at a higher rate, and the rest mark it as a child of the wish.
const FABLE_ENV_CLEARED: [&str; 8] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_PROMPT_CACHE_TTL",
    "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL",
    super::ENV,
    "AXIOM_HEADLESS",
    "AXIOM_BIN",
    "AXIOM_PROVENANCE",
];

pub(super) struct Answer {
    pub status: Status,
    pub output: Option<Value>,
    pub raw: Option<String>,
    pub tokens: u64,
    pub detail: Option<String>,
}
impl Answer {
    fn bad(status: Status, tokens: u64, detail: impl Into<String>) -> Self {
        Self {
            status,
            output: None,
            raw: None,
            tokens,
            detail: Some(detail.into()),
        }
    }
}

/// Fable's spend across all dialogue turns, priced from reported usage. Persisted per brief, so a
/// retry after a crash continues the same budget instead of starting a fresh one.
pub(super) struct Meter {
    cap_usd: f64,
    ledger: Option<PathBuf>,
    key: String,
    spent_usd: f64,
    unpriced_tokens: u64,
    tokens: u64,
    observed: bool,
    broken: bool,
    calls: HashMap<String, Call>,
    anonymous: u64,
    runs: u64,
}
#[derive(Default, Clone, Copy)]
struct Call {
    usage: price::CallUsage,
    usd: f64,
    unpriced: u64,
    tokens: u64,
    priced: bool,
}
impl Meter {
    pub fn open(ledger: &Path, key: &str, cap_usd: f64) -> Res<Self> {
        let failure = |e: &dyn std::fmt::Display| {
            Failure::new("council_ledger", format!("{}: {e}", ledger.display()))
        };
        let (spent_usd, unpriced_tokens) = match std::fs::read(ledger) {
            Ok(bytes) => {
                let v: Value = serde_json::from_slice(&bytes).map_err(|e| failure(&e))?;
                if v["key"] == key {
                    (
                        v["usd"].as_f64().unwrap_or(0.0),
                        v["unpriced_tokens"].as_u64().unwrap_or(0),
                    )
                } else {
                    (0.0, 0)
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (0.0, 0),
            Err(e) => return Err(failure(&e)),
        };
        Ok(Self {
            cap_usd,
            ledger: Some(ledger.to_path_buf()),
            key: key.to_owned(),
            spent_usd,
            unpriced_tokens,
            tokens: 0,
            observed: false,
            broken: false,
            calls: HashMap::new(),
            anonymous: 0,
            runs: 0,
        })
    }
    fn single_call() -> Self {
        Self {
            cap_usd: f64::INFINITY,
            ledger: None,
            key: String::new(),
            spent_usd: 0.0,
            unpriced_tokens: 0,
            tokens: 0,
            observed: false,
            broken: false,
            calls: HashMap::new(),
            anonymous: 0,
            runs: 0,
        }
    }
    fn observe(&mut self, id: Option<&str>, model: &str, usage: price::CallUsage) {
        self.observed = true;
        let id = id.map(str::to_owned).unwrap_or_else(|| {
            self.anonymous += 1;
            format!("anonymous-{}", self.anonymous)
        });
        let call = self.calls.entry(id).or_default();
        let old = *call;
        call.usage.merge(&usage);
        call.tokens = call.usage.context() + call.usage.output;
        match price::usd(model, &call.usage) {
            Some(usd) => (call.usd, call.unpriced, call.priced) = (usd, 0, true),
            None => (call.usd, call.unpriced, call.priced) = (0.0, call.tokens, false),
        }
        let new = *call;
        self.spent_usd += new.usd - old.usd;
        self.unpriced_tokens += new.unpriced;
        self.unpriced_tokens -= old.unpriced;
        self.tokens += new.tokens;
        self.tokens -= old.tokens;
        let record =
            json!({"key":self.key,"usd":self.spent_usd,"unpriced_tokens":self.unpriced_tokens});
        if let Some(ledger) = &self.ledger {
            if crate::base::atomic_write(ledger, record.to_string().as_bytes()).is_err() {
                self.broken = true;
            }
        }
    }
    fn output_tokens(&self) -> u64 {
        self.calls.values().map(|c| c.usage.output).sum()
    }
    /// Adds whatever output the run's closing total shows beyond what was already metered.
    fn settle_output(&mut self, output_before: u64, run_total: u64, model: &str) {
        let metered = self.output_tokens().saturating_sub(output_before);
        if run_total > metered {
            let hidden = price::CallUsage {
                output: run_total - metered,
                ..Default::default()
            };
            self.observe(None, model, hidden);
        }
    }
    /// A model the price table cannot price still counts: its tokens run against the cap at
    /// the highest output rate any Fable has had ($50 per million), so the cap fails closed.
    fn unpriced_cap(&self) -> u64 {
        (self.cap_usd * 20_000.0) as u64
    }
    pub fn exceeded(&self) -> bool {
        self.broken || self.spent_usd >= self.cap_usd || self.unpriced_tokens >= self.unpriced_cap()
    }
    pub fn observed(&self) -> bool {
        self.observed
    }
    pub fn remaining_usd(&self) -> f64 {
        if self.unpriced_tokens > 0 {
            return 0.0;
        }
        self.cap_usd - self.spent_usd
    }
    pub fn spent_usd(&self) -> f64 {
        self.spent_usd
    }
    pub fn cap_usd(&self) -> f64 {
        self.cap_usd
    }
    pub fn unpriced_tokens(&self) -> u64 {
        self.unpriced_tokens
    }
    pub fn tokens(&self) -> u64 {
        self.tokens
    }
}

enum End {
    Exited(Option<i32>),
    Timeout,
    Stopped,
    Capped,
}
struct Finished {
    end: End,
    stdout: Vec<u8>,
    stderr: String,
}

fn run_process(
    mut command: Command,
    stdin: Option<String>,
    timeout: Duration,
    stop: &AtomicBool,
    member: &str,
    raw_path: Option<&Path>,
    on_line: &mut dyn FnMut(&[u8]) -> bool,
) -> std::io::Result<Finished> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    // Reporting failure must not abort a paid call or invent an occupancy.
    let _running = super::running::Member::start(&crate::codex::slots::home(), member).ok();
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let stdout = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            match reader.read_until(b'\n', &mut line) {
                Ok(n) if n > 0 && tx.send(line).is_ok() => {}
                _ => break,
            }
        }
    });
    let kept = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&kept);
    let errors = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = stderr_pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut kept = sink.lock().unwrap_or_else(|e| e.into_inner());
            kept.extend_from_slice(&chunk[..n]);
            let excess = kept.len().saturating_sub(STDERR_KEEP);
            kept.drain(..excess);
        }
    });
    let start = Instant::now();
    let mut stdout = Vec::new();
    let mut raw_file: Option<std::fs::File> = None;
    let mut exited_at = None;
    let end = loop {
        if stop.load(Ordering::SeqCst) {
            proc::kill_tree(&mut child);
            break End::Stopped;
        }
        if start.elapsed() >= timeout {
            proc::kill_tree(&mut child);
            break End::Timeout;
        }
        match lines.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                if let Some(path) = raw_path {
                    let write = (|| -> std::io::Result<()> {
                        if raw_file.is_none() {
                            raw_file = Some(std::fs::File::create(path)?);
                        }
                        raw_file.as_mut().unwrap().write_all(&line)
                    })();
                    if let Err(e) = write {
                        proc::kill_tree(&mut child);
                        return Err(e);
                    }
                }
                stdout.extend_from_slice(&line);
                if !on_line(&line) {
                    proc::kill_tree(&mut child);
                    break End::Capped;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // A grandchild can hold the pipe open after the member itself has ended.
                if let Ok(Some(status)) = child.try_wait() {
                    let at = *exited_at.get_or_insert_with(Instant::now);
                    if at.elapsed() > Duration::from_secs(2) {
                        break End::Exited(status.code());
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => match child.try_wait() {
                Ok(Some(status)) => break End::Exited(status.code()),
                _ => std::thread::sleep(Duration::from_millis(20)),
            },
        }
    };
    let waited = Instant::now();
    while !errors.is_finished() && waited.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(10));
    }
    let stderr =
        String::from_utf8_lossy(&kept.lock().unwrap_or_else(|e| e.into_inner())).replace('\0', "");
    Ok(Finished {
        end,
        stdout,
        stderr,
    })
}

/// Bytes of generated content in a message: text, thinking (or its signature) and tool input.
fn content_bytes(content: &Value) -> u64 {
    content
        .as_array()
        .into_iter()
        .flatten()
        .map(|block| {
            let text = ["text", "thinking", "signature"]
                .iter()
                .filter_map(|key| block[*key].as_str())
                .map(str::len)
                .sum::<usize>();
            let input = if block["input"].is_null() {
                0
            } else {
                block["input"].to_string().len()
            };
            (text + input) as u64
        })
        .sum()
}

fn quota(text: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)usage limit|session limit|rate.?limit|quota|resource.?exhausted|credit balance|\b429\b|limit.*resets",
        )
        .unwrap()
    })
    .is_match(text)
}

/// The JSON object in a model's reply, which may be wrapped in a code fence or prose.
fn json_in(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return v.is_object().then_some(v);
    }
    let (start, end) = (trimmed.find('{')?, trimmed.rfind('}')?);
    serde_json::from_str::<Value>(trimmed.get(start..=end)?)
        .ok()
        .filter(Value::is_object)
}

fn checked(candidate: Option<Value>, shape: &Value, tokens: u64, what: &str) -> Answer {
    let Some(value) = candidate else {
        return Answer::bad(Status::Malformed, tokens, format!("{what}: no JSON object"));
    };
    match schema::validate(shape, &value) {
        Ok(()) => Answer {
            status: Status::Ok,
            output: Some(value),
            raw: None,
            tokens,
            detail: None,
        },
        Err(e) => Answer::bad(Status::Malformed, tokens, format!("{what}: {e}")),
    }
}

fn spawn_failure(error: std::io::Error) -> Answer {
    let status = if error.kind() == std::io::ErrorKind::NotFound {
        Status::Missing
    } else {
        Status::Failed
    };
    Answer::bad(status, 0, error.to_string())
}

pub(super) struct Ask<'a> {
    pub prompt: String,
    pub cwd: &'a Path,
    pub schema: &'a Value,
    pub timeout: Duration,
    pub stop: &'a AtomicBool,
    pub raw_path: Option<&'a Path>,
}

/// `tools` empty means the round that may use none; the prompt goes on stdin.
pub(super) fn ask_fable(
    command: &Res<Vec<String>>,
    ask: Ask,
    tools: &str,
    meter: &mut Meter,
) -> Answer {
    ask_claude(command, ask, tools, meter, FABLE_MODEL)
}

/// Usage observed for a single Claude invocation. Input includes cache reads and writes.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct MemberUsage {
    pub model: String,
    pub usd: Option<f64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Run one Claude invocation with tools disabled and validate its structured answer.
#[allow(dead_code)]
pub fn claude_call(
    command: &Res<Vec<String>>,
    model: &str,
    prompt: String,
    cwd: &Path,
    schema: &Value,
    timeout: Duration,
) -> Res<(Value, MemberUsage)> {
    if let Err(error) = command {
        return Err(error.clone());
    }
    if command
        .as_ref()
        .is_ok_and(|argv| argv.is_empty() || argv[0].is_empty())
    {
        return Err(Failure::new("not_installed", "Claude command is empty"));
    }
    let mut meter = Meter::single_call();
    let stop = AtomicBool::new(false);
    let answer = ask_claude(
        command,
        Ask {
            prompt,
            cwd,
            schema,
            timeout,
            stop: &stop,
            raw_path: None,
        },
        "",
        &mut meter,
        model,
    );
    let output = answer.output.ok_or_else(|| {
        Failure::new(
            "claude_call",
            answer.detail.unwrap_or_else(|| answer.status.word().into()),
        )
    })?;
    let usd = (meter.observed
        && meter.calls.values().all(|call| call.priced)
        && price::usd(model, &price::CallUsage::default()).is_some())
    .then_some(meter.spent_usd);
    let usage = MemberUsage {
        model: model.to_owned(),
        usd,
        input_tokens: meter.calls.values().map(|call| call.usage.context()).sum(),
        output_tokens: meter.output_tokens(),
    };
    Ok((output, usage))
}

fn ask_claude(
    command: &Res<Vec<String>>,
    ask: Ask,
    tools: &str,
    meter: &mut Meter,
    model: &str,
) -> Answer {
    // A spent budget (also one loaded from an earlier attempt) starts no paid request.
    if meter.exceeded() {
        return Answer::bad(
            Status::BudgetCut,
            0,
            format!("the ${:.2} cap is already spent", meter.cap_usd()),
        );
    }
    meter.runs += 1;
    // Each run's closing report is its own call, so separate runs add up.
    let result_id = format!("result-{}", meter.runs);
    let tokens_before = meter.tokens();
    let output_before = meter.output_tokens();
    let mut argv = match command {
        Ok(argv) => argv.clone(),
        Err(e) => return Answer::bad(Status::Missing, 0, e.message.clone()),
    };
    argv.extend(
        [
            "-p",
            "--model",
            model,
            "--output-format",
            "stream-json",
            "--verbose",
            "--strict-mcp-config",
            "--setting-sources",
            "local",
            "--tools",
            tools,
        ]
        .map(String::from),
    );
    if !tools.is_empty() {
        argv.extend(["--allowedTools".to_string(), tools.to_string()]);
    }
    let mut child = proc::command(&argv);
    child.current_dir(ask.cwd);
    for name in FABLE_ENV_CLEARED {
        child.env_remove(name);
    }
    let mut result: Option<Value> = None;
    let mut usage_in_this_run = false;
    let finished = run_process(
        child,
        Some(ask.prompt),
        ask.timeout,
        ask.stop,
        "fable",
        ask.raw_path,
        &mut |line| {
            let Ok(event) = serde_json::from_slice::<Value>(line) else {
                return true;
            };
            match event["type"].as_str() {
                Some("assistant") => {
                    let message = &event["message"];
                    if let Some(mut u) = usage::call_usage(&message["usage"]) {
                        usage_in_this_run = true;
                        // Stream-json stamps usage at stream start, so its output count is far
                        // below what is billed; the content so far is the better bound.
                        u.output = u.output.max(content_bytes(&message["content"]) / 3);
                        let reported_model = message["model"].as_str().unwrap_or(model);
                        meter.observe(message["id"].as_str(), reported_model, u);
                    }
                }
                Some("result") => {
                    if let Some(total) = usage::call_usage(&event["usage"]) {
                        if usage_in_this_run {
                            // The result carries the true output total: bill what the stream hid.
                            meter.settle_output(output_before, total.output, model);
                        } else {
                            usage_in_this_run = true;
                            meter.observe(Some(&result_id), model, total);
                        }
                    }
                    result = Some(event);
                }
                _ => {}
            }
            !meter.exceeded()
        },
    );
    let finished = match finished {
        Ok(f) => f,
        Err(e) => return spawn_failure(e),
    };
    let raw = (!finished.stdout.is_empty())
        .then(|| String::from_utf8_lossy(&finished.stdout).into_owned());
    let tokens = meter.tokens() - tokens_before;
    let reply = result.as_ref().filter(|r| r["is_error"] != true);
    let mut answer = match finished.end {
        // A reply delivered with the event that crossed the cap is already paid for.
        End::Capped if reply.is_none() => Answer::bad(
            Status::BudgetCut,
            tokens,
            format!("stopped at the ${:.2} cap", meter.cap_usd()),
        ),
        End::Timeout => Answer::bad(Status::Timeout, tokens, "timed out"),
        End::Stopped => Answer::bad(Status::Failed, tokens, "stopped"),
        End::Exited(_) | End::Capped => {
            if let Some(reply) = reply {
                let text = reply["result"].as_str().unwrap_or("");
                let mut answer = checked(json_in(text), ask.schema, tokens, "fable");
                answer.raw = raw;
                return answer;
            }
            let text = match &result {
                Some(r) => r["result"].as_str().unwrap_or("").to_owned(),
                None => finished.stderr.clone(),
            };
            let status = if quota(&text) {
                Status::Quota
            } else {
                Status::Failed
            };
            let detail = if text.trim().is_empty() {
                "exited without a result".to_owned()
            } else {
                text
            };
            Answer::bad(status, tokens, detail)
        }
    };
    answer.raw = raw;
    answer
}

fn token_count(value: &Value) -> Option<u64> {
    let find = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| value.get(*name).and_then(Value::as_u64))
    };
    if let Some(total) = find(&["totalTokenCount", "total_tokens", "totalTokens"]) {
        return Some(total);
    }
    let input = find(&["promptTokenCount", "input_tokens", "inputTokens"]);
    let output = find(&["candidatesTokenCount", "output_tokens", "outputTokens"]);
    if input.is_some() || output.is_some() {
        return Some(input.unwrap_or(0) + output.unwrap_or(0));
    }
    match value {
        Value::Object(map) => map.values().find_map(token_count),
        Value::Array(list) => list.iter().find_map(token_count),
        _ => None,
    }
}

/// `agy --output-format json` is not documented, so accept the object at the top level or
/// under the keys such envelopes use, and let the schema decide which one is the answer.
fn envelope_candidates(value: &Value) -> Vec<Value> {
    let mut found = vec![value.clone()];
    for key in [
        "structured_output",
        "structuredOutput",
        "result",
        "response",
        "output",
        "text",
    ] {
        match value.get(key) {
            Some(Value::String(text)) => found.extend(json_in(text)),
            Some(v) if v.is_object() => found.push(v.clone()),
            _ => {}
        }
    }
    found
}

/// The prompt goes last, after `--print`, so it reads correctly whether `--print` takes the
/// prompt as its value or is a switch followed by the prompt.
pub(super) fn ask_argon(command: &Res<Vec<String>>, ask: Ask, schema_path: &Path) -> Answer {
    let mut argv = match command {
        Ok(argv) => argv.clone(),
        Err(e) => return Answer::bad(Status::Missing, 0, e.message.clone()),
    };
    if ask.prompt.encode_utf16().count() > ARGV_LIMIT_UTF16 {
        return Answer::bad(Status::Failed, 0, "prompt is too long for a command line");
    }
    argv.extend([
        "--model".to_string(),
        ARGON_MODEL.to_string(),
        "--output-format".to_string(),
        "json".to_string(),
        "--json-schema".to_string(),
        schema_path.to_string_lossy().into_owned(),
        "--print-timeout".to_string(),
        format!("{}s", ask.timeout.as_secs().max(1)),
        "--sandbox".to_string(),
        "--mode".to_string(),
        "plan".to_string(),
        "--print".to_string(),
        ask.prompt.clone(),
    ]);
    let mut child = proc::command(&argv);
    child.current_dir(ask.cwd);
    child.env_remove(super::ENV);
    // A margin past --print-timeout, so agy can report its own timeout first.
    let limit = ask.timeout + Duration::from_secs(30);
    let finished = match run_process(
        child,
        None,
        limit,
        ask.stop,
        "argon",
        ask.raw_path,
        &mut |_| true,
    ) {
        Ok(f) => f,
        Err(e) => return spawn_failure(e),
    };
    let stdout = String::from_utf8_lossy(&finished.stdout).into_owned();
    let raw = (!finished.stdout.is_empty()).then(|| stdout.clone());
    let mut answer = match finished.end {
        End::Timeout => Answer::bad(Status::Timeout, 0, "timed out"),
        End::Stopped | End::Capped => Answer::bad(Status::Failed, 0, "stopped"),
        End::Exited(Some(0)) => {
            let Ok(parsed) = serde_json::from_str::<Value>(stdout.trim()) else {
                let mut answer = Answer::bad(Status::Malformed, 0, "argon: stdout is not JSON");
                answer.raw = raw;
                return answer;
            };
            let tokens = token_count(&parsed).unwrap_or(0);
            let candidates = envelope_candidates(&parsed);
            let valid = candidates
                .iter()
                .find(|c| schema::validate(ask.schema, c).is_ok())
                .cloned();
            checked(
                valid.or_else(|| candidates.into_iter().last()),
                ask.schema,
                tokens,
                "argon",
            )
        }
        End::Exited(code) => {
            let text = format!("{}\n{stdout}", finished.stderr);
            let status = if quota(&text) {
                Status::Quota
            } else {
                Status::Failed
            };
            Answer::bad(status, 0, format!("exit {code:?}: {text}"))
        }
    };
    answer.raw = raw;
    answer
}

#[cfg(test)]
mod claude_call_tests {
    use super::*;

    fn fake(extra: &[String]) -> Res<Vec<String>> {
        let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
            let names = if cfg!(windows) {
                ["python", "python3"]
            } else {
                ["python3", "python"]
            };
            names
                .into_iter()
                .find(|name| {
                    crate::base::proc::cmd(name)
                        .arg("--version")
                        .output()
                        .is_ok_and(|o| o.status.success())
                })
                .expect("Python not found")
                .to_owned()
        });
        let mut argv = vec![
            python,
            "-I".into(),
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fakes/fake_member.py")
                .to_string_lossy()
                .into_owned(),
        ];
        argv.extend_from_slice(extra);
        Ok(argv)
    }

    #[test]
    fn single_call_returns_scripted_answer_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("script.json");
        let log = dir.path().join("log.jsonl");
        std::fs::write(
            &script,
            json!([{"match":"hello", "reply":{"answer":"world"}}]).to_string(),
        )
        .unwrap();
        let command = fake(&[
            format!("--fake-script={}", script.display()),
            format!("--fake-log={}", log.display()),
        ]);
        let shape = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}});
        let (answer, usage) = claude_call(
            &command,
            FABLE_MODEL,
            "hello".into(),
            dir.path(),
            &shape,
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(answer, json!({"answer":"world"}));
        assert_eq!(usage.model, FABLE_MODEL);
        assert_eq!((usage.input_tokens, usage.output_tokens), (7000, 500));
        assert!((usage.usd.unwrap() - 0.0684).abs() < 1e-10);
        let logged: Value =
            serde_json::from_str(std::fs::read_to_string(log).unwrap().trim()).unwrap();
        assert!(logged["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|v| v[0] == "--model" && v[1] == FABLE_MODEL));
        assert_eq!(logged["prompt"], "hello");
    }

    #[test]
    fn missing_command_and_timeout_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let shape = json!({"type":"object"});
        let missing = Err(Failure::new("not_installed", "missing"));
        assert!(claude_call(
            &missing,
            FABLE_MODEL,
            String::new(),
            dir.path(),
            &shape,
            Duration::from_secs(5)
        )
        .is_err());
        let absent = Ok(vec![dir
            .path()
            .join("no-such-command")
            .to_string_lossy()
            .into_owned()]);
        assert!(claude_call(
            &absent,
            FABLE_MODEL,
            String::new(),
            dir.path(),
            &shape,
            Duration::from_secs(5)
        )
        .is_err());
        let hanging = fake(&["--fake-scenario=hang".into()]);
        assert!(claude_call(
            &hanging,
            FABLE_MODEL,
            String::new(),
            dir.path(),
            &shape,
            Duration::from_millis(250)
        )
        .is_err());
    }

    #[test]
    fn unpriced_usage_has_no_usd() {
        let dir = tempfile::tempdir().unwrap();
        let shape = json!({"type":"object"});
        let command = fake(&["--fake-scenario=unpriced".into()]);
        let (_, usage) = claude_call(
            &command,
            "claude-experimental-x",
            String::new(),
            dir.path(),
            &shape,
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(usage.usd, None);
        assert_eq!((usage.input_tokens, usage.output_tokens), (7000, 500));
    }
}

// p8: BEGIN paper-only Argon stream transport
pub(super) fn ask_argon_stdin(command: &Res<Vec<String>>, ask: Ask, schema_path: &Path) -> Answer {
    let mut argv = match command {
        Ok(argv) => argv.clone(),
        Err(e) => return Answer::bad(Status::Missing, 0, e.message.clone()),
    };
    argv.extend([
        "--model".to_owned(), ARGON_MODEL.to_owned(),
        "--input-format".to_owned(), "stream-json".to_owned(),
        "--output-format".to_owned(), "stream-json".to_owned(),
        "--json-schema".to_owned(), schema_path.to_string_lossy().into_owned(),
        "--print-timeout".to_owned(), format!("{}s", ask.timeout.as_secs().max(1)),
        "--sandbox".to_owned(), "--mode".to_owned(), "plan".to_owned(),
    ]);
    let input = format!("{}\n", json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":ask.prompt}]}}));
    let mut child = proc::command(&argv);
    child.current_dir(ask.cwd);
    child.env_remove(super::ENV);
    let finished = match run_process(child, Some(input), ask.timeout, ask.stop, "argon", ask.raw_path, &mut |_| true) {
        Ok(f) => f,
        Err(e) => return spawn_failure(e),
    };
    let stdout = String::from_utf8_lossy(&finished.stdout).into_owned();
    let raw = (!finished.stdout.is_empty()).then(|| stdout.clone());
    let mut answer = match finished.end {
        End::Timeout => Answer::bad(Status::Timeout, 0, "timed out"),
        End::Stopped | End::Capped => Answer::bad(Status::Failed, 0, "stopped"),
        End::Exited(Some(0)) => {
            let result = stdout.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| event["type"] == "result").last();
            if let Some(result) = result {
                let tokens = token_count(&result).unwrap_or(0);
                let candidates = envelope_candidates(&result);
                let valid = candidates.iter().find(|c| schema::validate(ask.schema, c).is_ok()).cloned();
                checked(valid.or_else(|| candidates.into_iter().last()), ask.schema, tokens, "argon")
            } else {
                Answer::bad(Status::Malformed, 0, "argon: no stream result")
            }
        }
        End::Exited(code) => {
            let text = format!("{}\n{stdout}", finished.stderr);
            let status = if quota(&text) { Status::Quota } else { Status::Failed };
            Answer::bad(status, 0, format!("exit {code:?}: {text}"))
        }
    };
    answer.raw = raw;
    answer
}
// p8: END paper-only Argon stream transport
