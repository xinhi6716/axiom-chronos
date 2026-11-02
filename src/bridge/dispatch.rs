//! Public boundary for the checkpoint-backed dispatcher.
//!
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use super::Bridge;
use crate::base::{Failure, Res};
#[cfg(test)]
use crate::orch::checkpoint::scratch_change;
use crate::orch::checkpoint::{self, AttemptState, CoreDecision};
pub use crate::orch::checkpoint::{ArtifactRef, Phase, RecoveryDecision, RunStatus};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootContext {
    pub repo_root: PathBuf,
    pub project: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DispatchAction {
    Start {
        intent: String,
    },
    Decide {
        run_id: String,
        research_digest: String,
        pass: bool,
        reason: String,
    },
    Resume {
        run_id: String,
        recovery: Option<RecoveryDecision>,
    },
    Status {
        run_id: String,
    },
    Cancel {
        run_id: String,
        reason: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRequest {
    pub request_id: String,
    pub action: DispatchAction,
}

pub struct DispatchOptions {
    pub max_wait_ms: u64,
    pub cancel_grace_ms: u64,
    pub cancel: Arc<AtomicBool>,
}

impl Default for DispatchOptions {
    fn default() -> Self {
        Self {
            max_wait_ms: 0,
            cancel_grace_ms: 10_000,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryResult {
    pub run_id: String,
    pub phase: Phase,
    pub status: RunStatus,
    pub short_reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
    #[serde(default)]
    pub research: Option<ArtifactRef>,
    pub artifact_refs: Vec<ArtifactRef>,
    pub next_action: Option<String>,
}

pub trait DispatchObserver: Send + Sync {
    fn on_boundary(&self, result: &BoundaryResult);
}

use crate::codex::{
    self, AppServer, DynamicTool, RunEvent, RunEventKind, RunObserver, RunOptions, RunRequest,
    ThreadMode, ToolCall, ToolHandler, ToolReply, TransportTerminal,
};
use crate::orch::checkpoint::{
    AttemptEvent, AttemptEventKind, AttemptOutcome, CheckResult, Checkpoint, ClaimDisposition,
    ExecutionPolicy, ExecutorIdentity, FrozenCheck, IndependentReview, Purpose, Snapshot, StepSpec,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

fn fail(message: impl Into<String>) -> Failure {
    Failure::new("dispatch", message)
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn command_path(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        // Canonical verbatim paths are identities, not portable shell literals.
        // PowerShell rejects mixed separators after the verbatim prefix.
        let value = if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            format!("//{unc}")
        } else {
            value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
        };
        value.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        value
    }
}
fn same_scratch_path(actual: &Path, expected: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // Scratch components are ASCII slugs or hexadecimal hashes. Compare
        // UTF-16 units without lossy conversion.
        let fold = |c: u16| {
            if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) {
                c + u16::from(b'a' - b'A')
            } else {
                c
            }
        };
        actual
            .as_os_str()
            .encode_wide()
            .map(fold)
            .eq(expected.as_os_str().encode_wide().map(fold))
    }
    #[cfg(not(windows))]
    {
        actual == expected
    }
}

fn reject_scratch_link(path: &Path) -> Res<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(io)?;
    let is_link = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let is_link = {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points need not report is_symlink().
        is_link || metadata.file_attributes() & 0x400 != 0
    };
    if is_link {
        return Err(fail("scratch path is a link or reparse point"));
    }
    Ok(())
}

/// Keep scratch and its output evidence in the project store. The complete
/// SHA-256 of the run/attempt/check identity names each directory; no partial
/// marker can block a retry or expose one attempt's files to another. Existing
/// dev.9 paths are retained for resumed runs.
fn scratch_path(store: &Path, run: &str, attempt: &str, check: Option<&str>) -> Res<PathBuf> {
    crate::base::require_slug("run_id", run)?;
    crate::base::require_slug("attempt_id", attempt)?;
    if let Some(id) = check {
        crate::base::require_slug("check_id", id)?;
    }
    let store_root = store.canonicalize().map_err(io)?;
    let legacy = store
        .join("runs")
        .join(run)
        .join("scratch")
        .join(attempt)
        .join(check.unwrap_or("work"));
    if legacy.try_exists().map_err(io)? {
        let resolved = legacy.canonicalize().map_err(io)?;
        let mut expected = store_root.clone();
        for component in ["runs", run, "scratch", attempt, check.unwrap_or("work")] {
            expected.push(component);
            reject_scratch_link(&expected)?;
        }
        if !same_scratch_path(&resolved, &expected) || !resolved.is_dir() {
            return Err(fail("legacy scratch does not match its assigned identity"));
        }
        return Ok(legacy);
    }

    let root = store.join("s");
    std::fs::create_dir_all(&root).map_err(io)?;
    reject_scratch_link(&root)?;
    let resolved_root = root.canonicalize().map_err(io)?;
    if !same_scratch_path(&resolved_root, &store_root.join("s")) {
        return Err(fail(
            "scratch root is outside its expected project store location",
        ));
    }
    let identity = serde_json::to_vec(&(run, attempt, check)).map_err(io)?;
    let name = digest(&identity);
    let path = root.join(&name);
    std::fs::create_dir_all(&path).map_err(io)?;
    reject_scratch_link(&path)?;
    if !same_scratch_path(
        &path.canonicalize().map_err(io)?,
        &resolved_root.join(&name),
    ) {
        return Err(fail("scratch path does not match its assigned identity"));
    }
    Ok(path)
}
fn reference(path: &Path) -> Res<ArtifactRef> {
    Ok(ArtifactRef {
        path: path.into(),
        digest: digest(&std::fs::read(path).map_err(io)?),
    })
}
fn artifact(dir: &Path, bytes: &[u8]) -> Res<ArtifactRef> {
    std::fs::create_dir_all(dir).map_err(io)?;
    let path = dir.join(format!("{}.json", digest(bytes)));
    if !path.exists() {
        crate::base::atomic_write(&path, bytes).map_err(io)?;
    }
    let r = reference(&path)?;
    if r.digest != digest(bytes) {
        return Err(fail("immutable artifact changed"));
    }
    Ok(r)
}

/// Stored in an immutable artifact referenced by an existing receipt slot so
/// dev.9 readers can still deserialize the unchanged checkpoint schema.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryFailure {
    revision: u64,
    phase: Phase,
    status: RunStatus,
    error: Failure,
}

fn saved_failure(store: &Path, c: &Checkpoint) -> Res<Option<Failure>> {
    if !matches!(c.status, RunStatus::Blocked | RunStatus::OutcomeUnknown) {
        return Ok(None);
    }
    let Some(receipt) = c.receipts.get(checkpoint::DISPATCH_FAILURE) else {
        return Ok(None);
    };
    let root = store.join("runs").join(&c.run_id).join("artifacts");
    if !receipt
        .path
        .canonicalize()
        .map_err(io)?
        .starts_with(root.canonicalize().map_err(io)?)
    {
        return Err(Failure::new(
            "checkpoint",
            "dispatch failure artifact outside its run",
        ));
    }
    let bytes = std::fs::read(&receipt.path).map_err(io)?;
    if digest(&bytes) != receipt.digest {
        return Err(Failure::new(
            "checkpoint",
            "dispatch failure artifact digest mismatch",
        ));
    }
    let saved: BoundaryFailure = serde_json::from_slice(&bytes).map_err(io)?;
    if saved.revision != c.revision || saved.phase != c.phase || saved.status != c.status {
        return Ok(None);
    }
    Ok(Some(saved.error))
}

fn unknown_failure() -> Failure {
    Failure::new(
        "unknown",
        "The blocked boundary did not record its cause; inspect the saved reports and receipts.",
    )
}

/// Project a historical checkpoint through the current acceptance policy
/// without changing its stored phase, status or request receipts.
fn historical_boundary(c: &Checkpoint, failure: Failure) -> BoundaryResult {
    let active = c
        .attempts
        .values()
        .any(|a| !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared));
    let status = if active {
        RunStatus::OutcomeUnknown
    } else {
        RunStatus::Blocked
    };
    let reason = format!(
        "Historical checkpoint status was {:?}; it is not authorization under the current acceptance policy. {}{}",
        c.status,
        failure.message,
        if active { " An existing attempt remains unresolved; reconcile it without redispatch." } else { "" }
    );
    let mut refs = vec![c.intent.clone()];
    refs.extend(c.research.clone());
    refs.extend(c.plan.clone());
    BoundaryResult {
        run_id: c.run_id.clone(),
        phase: c.phase.clone(),
        status,
        short_reason: reason.clone(),
        error: Some(Failure::new(if active { "outcome_unknown" } else { &failure.kind }, reason)),
        research: c.research.clone(),
        artifact_refs: refs,
        next_action: Some("Preserve the historical evidence. Reconcile any unresolved attempt, then start a new run with a complete baseline and independent verification.".into()),
    }
}
fn tool(name: &str, properties: Value) -> DynamicTool {
    let required = properties
        .as_object()
        .map(|m| m.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    DynamicTool {
        name: name.into(),
        description: format!(
            "Execute captured run's {name}. No caller identity or command overrides."
        ),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}
fn runner_tool_reply(receipt: &crate::orch::runner::RunnerReceipt) -> Res<Value> {
    use crate::orch::runner::{RunnerSentinel, RunnerStatus};
    if receipt.status != RunnerStatus::Ready {
        return Ok(
            json!({"status":"failed","reason":"The single managed Rust compile/link did not succeed. Return preparation status failed; do not substitute another check.","failure":receipt.status}),
        );
    }
    let binary = receipt
        .binary
        .as_ref()
        .ok_or_else(|| fail("prepared runner binary missing"))?;
    let sentinel = RunnerSentinel {
        source_digest: receipt.source.digest.clone(),
        binary_digest: binary.digest.clone(),
    };
    Ok(
        json!({"status":"ready","program":sentinel.program()?,"meaning":"Source compiled and linked only; no runner or product cases were executed."}),
    )
}
/// Typed business result of a writing leaf. A normally terminated turn is only
/// transport evidence; dependencies follow this status.
fn writer_schema() -> Value {
    json!({"type":"object","required":["status","summary","artifact"],"additionalProperties":false,"properties":{
        "status":{"type":"string","enum":["completed","blocked"]},
        "summary":{"type":"string"},
        "artifact":{"type":["string","null"]}}})
}
fn writes_work(purpose: &Purpose) -> bool {
    matches!(
        purpose,
        Purpose::Engineer | Purpose::Content | Purpose::Assembler
    )
}
/// Reason a writer did not achieve its goal; `None` only for a typed completion.
fn work_blocked(report: &Value) -> Option<String> {
    // History replays carry the typed result only as the final message text.
    let parsed = report["final_message"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let out = if report["structured_output"].is_null() {
        parsed.as_ref().unwrap_or(&Value::Null)
    } else {
        &report["structured_output"]
    };
    match out["status"].as_str() {
        Some("completed") => None,
        Some("blocked") => Some(
            out["summary"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("no reason given")
                .to_owned(),
        ),
        _ => Some("missing machine-readable work status".into()),
    }
}
fn blocked_failure(step: &str, reason: &str) -> Failure {
    Failure::new(
        "work_blocked",
        format!("work unit {step} reported blocked: {reason}. Dependents, assembly and verification were not started on it. The frozen plan and this evidence are retained; fix the stated prerequisite and start a new run instead of resuming this one for a different result."),
    )
}
/// Concrete typed failure a tool handler raised during a leaf attempt. The
/// transport keeps it beside the wrapper message so a blocked run reports the
/// business reason instead of "tool_handler_failed".
fn cause_failure(report: &Value) -> Option<Failure> {
    report["error"]["causes"].as_array()?.iter().find_map(|c| {
        let data = &c["error"]["data"];
        let kind = data["kind"].as_str()?;
        Some(Failure::new(
            kind,
            data["message"].as_str().unwrap_or("no message"),
        ))
    })
}
struct Bound {
    pipeline: Arc<Pipeline>,
    purpose: Purpose,
    attempt: String,
    execution: Mutex<Option<codex::ExecutionKey>>,
    tools: Vec<String>,
    model: String,
    effort: String,
    operation: Mutex<()>,
}
impl RunObserver for Bound {
    fn on_event(&self, event: &RunEvent) -> Res<()> {
        if event.attempt_id != self.attempt {
            return Err(fail("observer attempt mismatch"));
        }
        if event.requested_model != self.model
            || event.configured_model != self.model
            || event.requested_effort.as_deref() != Some(self.effort.as_str())
        {
            return Err(Failure::new(
                "model_identity",
                "observer model request/configuration mismatch",
            ));
        }
        if matches!(
            &event.kind,
            RunEventKind::ConnectionLost { .. }
                | RunEventKind::Terminal {
                    report: codex::RunReport {
                        terminal: TransportTerminal::OutcomeUnknown,
                        ..
                    }
                }
        ) {
            self.pipeline.stopped.store(true, Ordering::Release);
            self.pipeline.cancel.store(true, Ordering::Release);
        }
        if matches!(event.kind, RunEventKind::BeforeTurn)
            && (self.pipeline.stopped.load(Ordering::Acquire)
                || event.execution.generation != self.pipeline.generation.load(Ordering::Acquire))
        {
            return Err(Failure::new(
                "outcome_unknown",
                "run connection changed; no new turn may be dispatched",
            ));
        }
        let mut key = self.execution.lock().map_err(io)?;
        if let Some(old) = key.as_ref() {
            if old.generation != event.execution.generation
                || old.thread_id != event.execution.thread_id
                || old.turn_id.is_some() && old.turn_id != event.execution.turn_id
            {
                return Err(fail("execution identity changed"));
            }
        }
        *key = Some(event.execution.clone());
        let kind = match &event.kind {
            RunEventKind::BeforeTurn => Some(AttemptEventKind::Dispatching {
                generation: event.execution.generation,
                thread_id: event.execution.thread_id.clone(),
            }),
            RunEventKind::TurnStarted => Some(AttemptEventKind::Running {
                turn_id: event
                    .execution
                    .turn_id
                    .clone()
                    .ok_or_else(|| fail("missing turn"))?,
            }),
            RunEventKind::ConnectionLost { reason } => Some(AttemptEventKind::OutcomeUnknown {
                reason: reason.clone(),
            }),
            RunEventKind::Terminal { report }
                if matches!(report.terminal, TransportTerminal::OutcomeUnknown) =>
            {
                Some(AttemptEventKind::OutcomeUnknown {
                    reason: "terminal outcome unknown".into(),
                })
            }
            _ => None,
        };
        if let Some(kind) = kind {
            self.pipeline.edit(|c| {
                // A disconnected transport first reports ConnectionLost and
                // then an unknown Terminal for the same attempt. The first
                // event already persisted the unknown state and its reason.
                // Still record a lone unknown Terminal (e.g. timeout).
                if matches!(&event.kind, RunEventKind::Terminal { report } if matches!(report.terminal, TransportTerminal::OutcomeUnknown))
                    && c.attempts.get(&self.attempt).is_some_and(|a| a.state == AttemptState::OutcomeUnknown)
                {
                    return Ok(());
                }
                let attempt_event = AttemptEvent {
                        event_id: format!("{}-{}", self.attempt, event.sequence),
                        attempt_id: self.attempt.clone(),
                        sequence: event.sequence,
                        kind,
                    };
                if matches!(event.kind, RunEventKind::BeforeTurn) {
                    let mut authority = self.pipeline.authority.lock().map_err(io)?;
                    let mut owned = c.attempts.get(&self.attempt).cloned().ok_or_else(|| fail("unknown dispatched attempt"))?;
                    checkpoint::record_attempt_dispatch(
                        &self.pipeline.store, &self.pipeline.run, c.revision, &attempt_event,
                        &event.requested_model, &event.configured_model, event.requested_effort.as_deref(),
                    )?;
                    owned.generation = Some(event.execution.generation);
                    owned.thread_id = Some(event.execution.thread_id.clone());
                    owned.turn_id = event.execution.turn_id.clone();
                    authority.executions.insert(self.attempt.clone(), owned);
                    Ok(())
                } else if matches!(event.kind, RunEventKind::TurnStarted) {
                    let mut authority = self.pipeline.authority.lock().map_err(io)?;
                    let owned = authority.executions.get_mut(&self.attempt).ok_or_else(|| fail("turn started without owned dispatch"))?;
                    checkpoint::record_attempt_event(
                        &self.pipeline.store, &self.pipeline.run, c.revision, &attempt_event,
                    )?;
                    owned.turn_id = event.execution.turn_id.clone();
                    Ok(())
                } else {
                    checkpoint::record_attempt_event(
                        &self.pipeline.store, &self.pipeline.run, c.revision, &attempt_event,
                    ).map(|_| ())
                }
            })?;
        }
        Ok(())
    }
}
impl ToolHandler for Bound {
    fn handle(&self, call: ToolCall) -> Res<ToolReply> {
        let _operation = self.operation.lock().map_err(io)?;
        let key = self
            .execution
            .lock()
            .map_err(io)?
            .clone()
            .ok_or_else(|| fail("callback before dispatch"))?;
        if key.generation != call.execution.generation
            || key.thread_id != call.execution.thread_id
            || key.turn_id != call.execution.turn_id
            || !self.tools.contains(&call.name)
        {
            return Err(fail("callback execution or tool mismatch"));
        }
        let p = &self.pipeline;
        let c = p.load()?;
        let a = c
            .attempts
            .get(&self.attempt)
            .ok_or_else(|| fail("unknown callback attempt"))?;
        if a.state != AttemptState::Running
            || a.thread_id.as_ref() != Some(&key.thread_id)
            || a.turn_id != key.turn_id
        {
            return Err(fail("callback for closed or different execution"));
        }
        let value = match (self.purpose.clone(), call.name.as_str()) {
            (Purpose::Dispatcher, "run_research")
                if c.phase == Phase::Research && call.arguments == json!({}) =>
            {
                p.research()?;
                json!({"research":p.load()?.research})
            }
            (Purpose::Dispatcher, "run_plan")
                if c.phase == Phase::Planning && call.arguments == json!({}) =>
            {
                p.plan()?;
                json!({"plan":p.load()?.plan})
            }
            (Purpose::Dispatcher, "dispatch_batch") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    unit_ids: Vec<String>,
                }
                let args: Args = serde_json::from_value(call.arguments).map_err(io)?;
                let caller = checkpoint::ScopeBatchCaller {
                    attempt_id: self.attempt.clone(),
                    generation: key.generation,
                    thread_id: key.thread_id.clone(),
                    turn_id: key
                        .turn_id
                        .clone()
                        .ok_or_else(|| fail("batch callback missing turn"))?,
                    call_id: call.call_id.clone(),
                };
                p.batch(&args.unit_ids, &caller)?;
                json!({"terminal":args.unit_ids,"note":"Rust recorded each unit's typed work outcome; blocked units do not satisfy dependents."})
            }
            (Purpose::Verifier, "verify_run") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    check_id: String,
                }
                let args: Args = serde_json::from_value(call.arguments).map_err(io)?;
                p.check(
                    &args.check_id,
                    ExecutorIdentity {
                        attempt_id: self.attempt.clone(),
                        thread_id: key.thread_id,
                        model: self.model.clone(),
                        purpose: Purpose::Verifier,
                        policy: ExecutionPolicy::ScratchCheck,
                    },
                )?
            }
            (Purpose::Verifier, "prepare_rust_runner")
                if c.phase == Phase::Planning
                    && a.step_id
                        == format!("evaluation-prep-{}", checkpoint::planning_epoch(&c)) =>
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    source_utf8: String,
                }
                let args: Args = serde_json::from_value(call.arguments).map_err(io)?;
                let identity = crate::orch::runner::RunnerIdentity {
                    run_id: p.run.clone(),
                    epoch: checkpoint::planning_epoch(&c),
                    attempt_id: self.attempt.clone(),
                    generation: key.generation,
                    thread_id: key.thread_id.clone(),
                    turn_id: key
                        .turn_id
                        .clone()
                        .ok_or_else(|| fail("runner callback missing turn"))?,
                };
                p.prepare_runner(&identity, args.source_utf8.as_bytes())?
            }
            (Purpose::Memory, "record_memory") if c.phase == Phase::Memory => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    evidence_id: String,
                    content: String,
                }
                let args: Args = serde_json::from_value(call.arguments).map_err(io)?;
                if checkpoint::verified_review(&c).is_none_or(|r| r.digest != args.evidence_id) {
                    return Err(fail("memory evidence is not the current verified review"));
                }
                let mut result = crate::memory::Memory::open(&p.home)?.record_run_with_evidence(
                    &p.project,
                    &p.run,
                    &args.content,
                    &args.evidence_id,
                )?;
                // Preserve the selected review alongside the memory receipt.
                result["review_digest"] = json!(args.evidence_id);
                let r = artifact(&p.artifacts(), &serde_json::to_vec(&result).map_err(io)?)?;
                p.receipt("memory", &r)?;
                result
            }
            _ => return Err(fail("tool not allowed for captured purpose and phase")),
        };
        Ok(ToolReply {
            success: true,
            content_items: vec![
                json!({"type":"inputText","text":serde_json::to_string(&value).map_err(io)?}),
            ],
        })
    }
}
impl Bridge {
    pub fn dispatch(
        &self,
        root: &RootContext,
        request: &DispatchRequest,
        options: &DispatchOptions,
        observer: Option<&dyn DispatchObserver>,
    ) -> Res<BoundaryResult> {
        crate::base::require_slug("project", &root.project)?;
        if options.max_wait_ms != 0 {
            return Err(Failure::new("invalid_argument","dispatch awaits the boundary; max_wait_ms must be 0, with finite execution budgets"));
        }
        if !root.repo_root.is_absolute()
            || !root.repo_root.is_dir()
            || request.request_id.trim().is_empty()
        {
            return Err(Failure::new(
                "invalid_argument",
                "absolute repo directory and request_id required",
            ));
        }
        let repo = root.repo_root.canonicalize().map_err(io)?;
        let store = crate::orch::store_for(self.home()?, &root.project)?;
        let _guard = if matches!(request.action, DispatchAction::Status { .. }) {
            None
        } else {
            Some(self.dispatch_lock.write().map_err(io)?)
        };
        let input = serde_json::to_vec(&(root, request)).map_err(io)?;
        let request_path = store
            .join("requests")
            .join(format!("{}.json", digest(request.request_id.as_bytes())));
        if !matches!(
            request.action,
            DispatchAction::Status { .. } | DispatchAction::Resume { .. }
        ) && request_path.is_file()
        {
            let old: Value =
                serde_json::from_slice(&std::fs::read(&request_path).map_err(io)?).map_err(io)?;
            if old["input"] != digest(&input) {
                return Err(fail("request_id reused with different input"));
            }
            if !old["result"].is_null() {
                let mut result: BoundaryResult =
                    serde_json::from_value(old["result"].clone()).map_err(io)?;
                if !old["boundary_error"].is_null() {
                    result.error =
                        Some(serde_json::from_value(old["boundary_error"].clone()).map_err(io)?);
                } else if result.error.is_none()
                    && matches!(
                        result.status,
                        RunStatus::Blocked | RunStatus::OutcomeUnknown
                    )
                {
                    result.error = Some(unknown_failure());
                }
                if matches!(
                    result.status,
                    RunStatus::ReadyToCommit | RunStatus::Released
                ) {
                    let historical = checkpoint::load_run(&store, &result.run_id)?;
                    if historical.repo.canonical_root != repo {
                        return Err(Failure::new(
                            "invalid_argument",
                            "repository does not match run",
                        ));
                    }
                    if let Err(failure) = checkpoint::current_frozen_sources(&historical)
                        .and_then(|_| checkpoint::baseline_authorized(&historical))
                    {
                        return Ok(historical_boundary(&historical, failure));
                    }
                    if !checkpoint::ready_evidence(&historical) {
                        return Ok(historical_boundary(&historical, Failure::new(
                            "unverified_review", "Stored acceptance evidence no longer satisfies the current independent verification requirements."
                        )));
                    }
                }
                return Ok(result);
            }
        }
        let run = match &request.action {
            DispatchAction::Start { intent } => {
                if intent.trim().is_empty() {
                    return Err(Failure::new("invalid_argument", "intent required"));
                }
                let run = format!("run-{}", &digest(&input)[..24]);
                let intent = artifact(
                    &store.join("runs").join(&run).join("artifacts"),
                    intent.as_bytes(),
                )?;
                checkpoint::create_run(
                    &store,
                    &checkpoint::RunSeed {
                        run_id: run.clone(),
                        repo: checkpoint::RepoIdentity {
                            canonical_root: repo.clone(),
                            identity_digest: digest(repo.to_string_lossy().as_bytes()),
                        },
                        intent,
                    },
                )?;
                run
            }
            DispatchAction::Decide { run_id, .. }
            | DispatchAction::Status { run_id }
            | DispatchAction::Resume { run_id, .. }
            | DispatchAction::Cancel { run_id, .. } => run_id.clone(),
        };
        let mut c = checkpoint::load_run(&store, &run)?;
        if c.repo.canonical_root != repo {
            return Err(Failure::new(
                "invalid_argument",
                "repository does not match run",
            ));
        }
        // A pre-v2 frozen run may still be inspected and reconciled, but no
        // current runtime may treat its historical result as full acceptance
        // or schedule another role under the old baseline.
        let compatibility = if c.plan.is_some() && c.phase != Phase::Cancelled {
            checkpoint::current_frozen_sources(&c).err()
        } else {
            None
        };
        if let Some(failure) = compatibility {
            if !matches!(request.action, DispatchAction::Cancel { .. }) {
                if let DispatchAction::Resume { recovery, .. } = &request.action {
                    if let Some(decision) = recovery {
                        if !matches!(decision, checkpoint::RecoveryDecision::RetryPlanning { .. }) {
                            checkpoint::resolve_attempt(&store, &run, c.revision, decision)?;
                            c = checkpoint::load_run(&store, &run)?;
                        }
                    }
                    if c.attempts.values().any(|a| {
                        !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared)
                    }) {
                        checkpoint::reconcile_run(&store, &run, c.revision, &[])?;
                        c = checkpoint::load_run(&store, &run)?;
                    }
                }
                return Ok(historical_boundary(&c, failure));
            }
        }
        if !matches!(request.action, DispatchAction::Cancel { .. })
            && matches!(c.phase, Phase::ReadyToCommit | Phase::Released)
        {
            if let Err(failure) = checkpoint::baseline_authorized(&c) {
                return Ok(historical_boundary(&c, failure));
            }
            if !checkpoint::ready_evidence(&c) {
                return Ok(historical_boundary(&c, Failure::new(
                    "unverified_review", "Stored acceptance evidence no longer satisfies the current independent verification requirements."
                )));
            }
        }
        // Recovery itself changes the revision. Read its old diagnostic first
        // so a no-op repair boundary does not lose the original cause.
        let prior_failure = if matches!(request.action, DispatchAction::Resume { .. }) {
            saved_failure(&store, &c)?
        } else {
            None
        };
        // Reconciliation can advance only some attempts. A previous diagnostic
        // remains authoritative only if the phase and complete attempt state
        // are unchanged, not merely because the final status is still blocked.
        let prior_phase = c.phase.clone();
        let prior_status = c.status.clone();
        let prior_attempts = c.attempts.clone();
        match &request.action {
            DispatchAction::Decide {
                research_digest,
                pass,
                reason,
                ..
            } => {
                let decided = c.core_decision.as_ref().is_some_and(|d| {
                    d.research_digest == *research_digest && d.pass == *pass && d.reason == *reason
                }) || (!pass
                    && c.receipts
                        .iter()
                        .filter(|(k, _)| k.starts_with("intent-rejection-"))
                        .any(|(_, r)| {
                            std::fs::read(&r.path)
                                .ok()
                                .and_then(|b| serde_json::from_slice::<CoreDecision>(&b).ok())
                                .is_some_and(|d| {
                                    d.research_digest == *research_digest
                                        && !d.pass
                                        && d.reason == *reason
                                })
                        }));
                if (!decided
                    && (c.phase != Phase::IntentReview
                        || c.research.as_ref().map(|r| &r.digest) != Some(research_digest)))
                    || reason.trim().is_empty()
                {
                    return Err(Failure::new(
                        "invalid_argument",
                        "matching research digest, reason and intent review phase required",
                    ));
                }
                if !decided {
                    checkpoint::record_core_decision(
                        &store,
                        &run,
                        c.revision,
                        &CoreDecision {
                            research_digest: research_digest.clone(),
                            pass: *pass,
                            reason: reason.clone(),
                        },
                    )?;
                }
                c = checkpoint::load_run(&store, &run)?;
                if *pass && c.phase == Phase::IntentReview {
                    checkpoint::advance_phase(&store, &run, c.revision, Phase::Planning)?;
                }
            }
            DispatchAction::Cancel { reason, .. } => {
                if reason.trim().is_empty() {
                    return Err(Failure::new(
                        "invalid_argument",
                        "cancellation reason required",
                    ));
                }
                checkpoint::cancel_run(&store, &run, c.revision, reason)?;
            }
            DispatchAction::Resume { recovery, .. } => {
                if let Some(d) = recovery {
                    checkpoint::resolve_attempt(&store, &run, c.revision, d)?;
                    c = checkpoint::load_run(&store, &run)?;
                }
                let reconciled = checkpoint::reconcile_run(&store, &run, c.revision, &[])?;
                checkpoint::retry_prepared_batches(&store, &run, reconciled.revision)?;
            }
            _ => {}
        }
        let execute = matches!(
            request.action,
            DispatchAction::Start { .. }
                | DispatchAction::Decide { pass: true, .. }
                | DispatchAction::Resume { .. }
        ) || (matches!(request.action, DispatchAction::Decide { pass: false, .. })
            && c.phase == Phase::Research);
        if execute
            && !matches!(request.action, DispatchAction::Resume { .. })
            && !request_path.exists()
        {
            std::fs::create_dir_all(request_path.parent().unwrap()).map_err(io)?;
            crate::base::atomic_write(
                &request_path,
                &serde_json::to_vec(&json!({"input":digest(&input),"result":null})).map_err(io)?,
            )
            .map_err(io)?;
        }
        let mut reason = "checkpoint loaded".to_string();
        let mut error = None;
        let mut boundary_pipeline = None;
        if execute {
            let origin = checkpoint::load_run(&store, &run)?;
            let p = Arc::new(Pipeline {
                server: self.codex()?,
                store: store.clone(),
                home: self.home()?.into(),
                repo,
                project: root.project.clone(),
                run: run.clone(),
                cancel: options.cancel.clone(),
                grace: options.cancel_grace_ms,
                mutations: Mutex::new(()),
                authority: Mutex::new(checkpoint::DispatchBoundaryAuthority {
                    phase: Some(origin.phase),
                    original_attempts: origin.attempts,
                    ..Default::default()
                }),
                sealed: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                generation: AtomicU64::new(0),
            });
            if matches!(request.action, DispatchAction::Resume { .. }) {
                for a in p
                    .load()?
                    .attempts
                    .values()
                    .filter(|a| a.state != AttemptState::Terminal)
                {
                    if a.state == AttemptState::Prepared {
                        continue;
                    }
                    if let Err(cause) =
                        checkpoint::attempt_model(&p.store, &p.load()?, &a.attempt_id)
                    {
                        error = Some(cause);
                        break;
                    }
                    if let (Some(thread), Some(turn)) = (&a.thread_id, &a.turn_id) {
                        if let Ok(codex::AttemptRead::Terminal(report)) =
                            p.server.read_attempt(thread, Some(turn))
                        {
                            // History cannot reconstruct a lost in-memory before
                            // snapshot. Only an already persisted original-role
                            // audit (handled by reconcile_run) can close a writer.
                            if p.load()?.steps.get(&a.step_id).is_some_and(|s| {
                                matches!(s.purpose, Purpose::Engineer | Purpose::Assembler)
                            }) {
                                continue;
                            }
                            if let Err(cause) = checkpoint::validate_attempt_report(
                                &p.store,
                                &p.load()?,
                                &a.attempt_id,
                                &report.report,
                            ) {
                                error = Some(cause);
                                break;
                            }
                            let outcome = match report.terminal {
                                TransportTerminal::Completed
                                    if p.load()?.steps.get(&a.step_id).is_some_and(|s| {
                                        writes_work(&s.purpose)
                                            && work_blocked(&report.report).is_some()
                                    }) =>
                                {
                                    AttemptOutcome::Blocked
                                }
                                TransportTerminal::Completed => AttemptOutcome::Completed,
                                TransportTerminal::Failed => AttemptOutcome::Failed,
                                TransportTerminal::Cancelled => AttemptOutcome::Cancelled,
                                _ => continue,
                            };
                            p.edit(|c| {
                                let mut authority = p.authority.lock().map_err(io)?;
                                let reconciled = c
                                    .attempts
                                    .get(&a.attempt_id)
                                    .cloned()
                                    .ok_or_else(|| fail("unknown reconciled attempt"))?;
                                checkpoint::commit_result(
                                    &p.store,
                                    &p.run,
                                    c.revision,
                                    &a.attempt_id,
                                    outcome,
                                    &report.report,
                                )?;
                                authority
                                    .executions
                                    .insert(a.attempt_id.clone(), reconciled);
                                Ok(())
                            })?;
                        }
                    }
                }
            }
            let c = p.load()?;
            let unchanged_attempts = c.phase == prior_phase && c.attempts == prior_attempts;
            if c.attempts
                .values()
                .any(|a| a.state != AttemptState::Terminal && a.state != AttemptState::Prepared)
            {
                let missing_writer_audit = c.attempts.values().any(|a| {
                    !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared)
                        && c.steps.get(&a.step_id).is_some_and(|s| {
                            matches!(s.purpose, Purpose::Engineer | Purpose::Assembler)
                        })
                });
                let previous = prior_failure.as_ref().filter(|failure| {
                    prior_status == RunStatus::OutcomeUnknown
                        && unchanged_attempts
                        && failure.kind == "outcome_unknown"
                        && !missing_writer_audit
                });
                if let Some(identity_error) = error.as_ref().filter(|e| e.kind == "model_identity")
                {
                    reason = identity_error.message.clone();
                } else if let Some(previous) = previous {
                    reason = previous.message.clone();
                    error = Some(previous.clone());
                } else {
                    let unresolved = c
                        .attempts
                        .values()
                        .filter(|a| {
                            a.state != AttemptState::Terminal && a.state != AttemptState::Prepared
                        })
                        .map(|a| {
                            format!(
                                "step {} (attempt {}, thread {}, turn {}, state {:?})",
                                a.step_id,
                                a.attempt_id,
                                a.thread_id.as_deref().unwrap_or("unknown"),
                                a.turn_id.as_deref().unwrap_or("unknown"),
                                a.state
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    reason = format!("unknown execution retained; no writing attempt redispatched; unresolved: {unresolved}. If the original writer scope audit is unavailable, provide explicit stop and repair evidence before retry.");
                    error = Some(Failure::new("outcome_unknown", reason.clone()));
                }
            } else if c
                .attempts
                .values()
                .any(|a| checkpoint::failed_attempt_needs_repair(&c, a))
            {
                let previous = prior_failure.as_ref().filter(|failure| {
                    prior_status == RunStatus::Blocked
                        && unchanged_attempts
                        && failure.kind != "outcome_unknown"
                });
                if let Some(previous) = previous {
                    reason = previous.message.clone();
                    error = Some(previous.clone());
                } else {
                    let failed = c
                        .attempts
                        .values()
                        .filter(|a| checkpoint::failed_attempt_needs_repair(&c, a))
                        .map(|a| format!("step {} (attempt {})", a.step_id, a.attempt_id))
                        .collect::<Vec<_>>()
                        .join(", ");
                    reason = format!(
                        "saved failed execution requires repair before a new attempt: {failed}"
                    );
                    error = Some(Failure::new("repair_required", reason.clone()));
                }
            } else {
                // Zero awaits Rust completion, not a 300s running-response loop.
                match p.drive() {
                    Ok(()) => reason = "authority boundary reached".into(),
                    Err(e) => {
                        reason = e.message.clone();
                        error = Some(e);
                    }
                }
            }
            boundary_pipeline = Some(p);
        }
        if let Some(p) = boundary_pipeline {
            let had_error = error.is_some();
            let (current, current_failure) = p.finish_boundary(error)?;
            c = current;
            let current_failure = if current_failure.is_none()
                && matches!(c.status, RunStatus::Blocked | RunStatus::OutcomeUnknown)
            {
                Some(saved_failure(&store, &c)?.unwrap_or_else(unknown_failure))
            } else {
                current_failure
            };
            if let Some(failure) = &current_failure {
                reason = failure.message.clone();
            } else if had_error {
                reason = "checkpoint advanced before the boundary; current state loaded".into();
            }
            error = current_failure;
        } else {
            c = checkpoint::load_run(&store, &run)?;
            if matches!(c.status, RunStatus::Blocked | RunStatus::OutcomeUnknown) {
                let failure = saved_failure(&store, &c)?.unwrap_or_else(unknown_failure);
                reason = failure.message.clone();
                error = Some(failure);
            }
        }
        let status = c.status.clone();
        if execute {
            crate::orch::project_checkpoint(&store, &checkpoint::load_run(&store, &run)?)?;
        }
        let next_action=match status {RunStatus::NeedsIntentReview=>Some("Core pass/reject using exact research digest.".into()),RunStatus::ReadyToCommit=>Some("Core may review and submit/complete locally. Commit, remote push and installation are external authority, not mandatory release gates.".into()),RunStatus::OutcomeUnknown|RunStatus::Blocked=>Some("Inspect saved artifacts and reconcile terminal/repair evidence. No automatic host wake or blind write retry.".into()),_=>None};
        let mut refs = vec![c.intent];
        refs.extend(c.research.clone());
        refs.extend(c.plan);
        let result = BoundaryResult {
            run_id: run,
            phase: c.phase,
            status,
            short_reason: reason,
            error,
            research: c.research,
            artifact_refs: refs,
            next_action,
        };
        if !matches!(
            request.action,
            DispatchAction::Status { .. } | DispatchAction::Resume { .. }
        ) {
            std::fs::create_dir_all(request_path.parent().unwrap()).map_err(io)?;
            // Older dev.9 readers reject unknown result fields. Store the new
            // diagnostic alongside the result, not inside its legacy shape.
            let mut compatible_result = serde_json::to_value(&result).map_err(io)?;
            compatible_result
                .as_object_mut()
                .ok_or_else(|| fail("invalid boundary result"))?
                .remove("error");
            crate::base::atomic_write(
                &request_path,
                &serde_json::to_vec(&json!({"input":digest(&input),"result":compatible_result,"boundary_error":result.error})).map_err(io)?,
            )
            .map_err(io)?;
        }
        if let Some(observer) = observer {
            observer.on_boundary(&result);
        }
        Ok(result)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    #[serde(default)]
    independent_evaluation: Option<String>,
    #[serde(default)]
    assembly_scope: Option<Vec<String>>,
    units: Vec<Unit>,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningRetryReceipt {
    epoch: usize,
    reason: String,
    attempt_ids: Vec<String>,
}

/// Authenticate the recorded recovery decision before exposing it to roles.
/// Retired attempt pointers bind the latest receipt to this checkpoint; older
/// retry receipts remain immutable evidence when a second retry supersedes them.
fn planning_recovery(store: &Path, c: &Checkpoint) -> Res<(usize, String)> {
    let epoch = checkpoint::planning_epoch(c);
    let retired: Vec<_> = c
        .receipts
        .iter()
        .filter(|(key, _)| key.starts_with("planning-retired-"))
        .collect();
    if epoch == 0 {
        if !retired.is_empty() {
            return Err(fail("planning retry receipt missing for retired attempts"));
        }
        return Ok((0, String::new()));
    }
    if epoch > 2 {
        return Err(fail("invalid planning retry epoch"));
    }
    let run_root = store
        .join("runs")
        .join(&c.run_id)
        .canonicalize()
        .map_err(io)?;
    let mut latest = None;
    let mut reasons = Vec::with_capacity(epoch);
    for revision in 1..=epoch {
        let reference = c
            .receipts
            .get(&format!("planning-retry-{revision}"))
            .ok_or_else(|| fail("planning retry receipt missing"))?;
        let expected = run_root.join(format!("planning-retry-{revision}.json"));
        let resolved = reference.path.canonicalize().map_err(io)?;
        if resolved.parent() != Some(run_root.as_path())
            || resolved != expected.canonicalize().map_err(io)?
        {
            return Err(fail("planning retry receipt is outside its run"));
        }
        let bytes = std::fs::read(&resolved).map_err(io)?;
        if digest(&bytes) != reference.digest {
            return Err(fail("planning retry receipt digest mismatch"));
        }
        let receipt: PlanningRetryReceipt = serde_json::from_slice(&bytes).map_err(io)?;
        if receipt.epoch != revision
            || receipt.reason.trim().is_empty()
            || receipt.attempt_ids.is_empty()
        {
            return Err(fail(
                "planning retry receipt has an invalid epoch or reason",
            ));
        }
        reasons.push(format!("Epoch {revision}: {}", receipt.reason));
        if revision == epoch {
            latest = Some((reference, receipt));
        }
    }
    let (reference, receipt) = latest.ok_or_else(|| fail("planning retry receipt missing"))?;
    let ids: std::collections::BTreeSet<_> = receipt.attempt_ids.iter().collect();
    if ids.len() != receipt.attempt_ids.len()
        || ids.iter().any(|id| {
            !c.attempts.contains_key(*id)
                || c.receipts.get(&format!("planning-retired-{id}")) != Some(reference)
        })
        || retired.len() != ids.len()
        || retired.iter().any(|(key, r)| {
            *r != reference
                || !receipt
                    .attempt_ids
                    .iter()
                    .any(|id| *key == &format!("planning-retired-{id}"))
        })
    {
        return Err(fail(
            "planning retry receipt does not match retired attempts",
        ));
    }
    Ok((epoch, format!(
        "\nCore-approved planning recoveries, in order:\n{}\nApply all corrections to the unfrozen prerequisites. Later reasons supplement earlier ones, or explicitly override them where stated. Preserve the approved user intent, behavior and evaluation standards; do not expose held-out cases or change frozen checks.",
        reasons.join("\n")
    )))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unit {
    #[serde(default)]
    kind: Option<String>,
    id: String,
    brief: String,
    scope: Vec<String>,
    depends_on: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    id: String,
    program: String,
    args: Vec<String>,
    env: Vec<EnvironmentValue>,
    timeout_s: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentValue {
    name: String,
    value: String,
}

#[cfg(windows)]
fn same_environment_name(a: &str, b: &str) -> Res<bool> {
    #[link(name = "kernel32")]
    extern "system" {
        fn CompareStringOrdinal(
            first: *const u16,
            first_length: i32,
            second: *const u16,
            second_length: i32,
            ignore_case: i32,
        ) -> i32;
    }
    let first: Vec<_> = a.encode_utf16().collect();
    let second: Vec<_> = b.encode_utf16().collect();
    let first_length = i32::try_from(first.len()).map_err(io)?;
    let second_length = i32::try_from(second.len()).map_err(io)?;
    // The Windows ordinal comparison uses the OS Unicode uppercase table,
    // matching the case-insensitive names in a Windows process environment.
    match unsafe {
        CompareStringOrdinal(
            first.as_ptr(),
            first_length,
            second.as_ptr(),
            second_length,
            1,
        )
    } {
        2 => Ok(true),
        1 | 3 => Ok(false),
        _ => Err(io(std::io::Error::last_os_error())),
    }
}

#[cfg(not(windows))]
fn same_environment_name(a: &str, b: &str) -> Res<bool> {
    Ok(a == b)
}

/// Frozen commands execute environment entries as a map, not an array.
fn check_environment(entries: &[EnvironmentValue]) -> Res<BTreeMap<String, String>> {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    for entry in entries {
        if entry.name.is_empty() {
            return Err(fail("empty or duplicate check environment name"));
        }
        for name in env.keys() {
            if same_environment_name(name, &entry.name)? {
                return Err(fail("empty or duplicate check environment name"));
            }
        }
        env.insert(entry.name.clone(), entry.value.clone());
    }
    Ok(env)
}

fn same_environment(
    first: &BTreeMap<String, String>,
    second: &BTreeMap<String, String>,
) -> Res<bool> {
    if first.len() != second.len() {
        return Ok(false);
    }
    for (name, value) in first {
        let mut match_value = None;
        for (other_name, other_value) in second {
            if same_environment_name(name, other_name)? {
                match_value = Some(other_value);
                break;
            }
        }
        if match_value != Some(value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn add_environment_default(
    env: &mut BTreeMap<String, String>,
    name: &str,
    value: String,
) -> Res<()> {
    for existing in env.keys() {
        if same_environment_name(existing, name)? {
            return Ok(());
        }
    }
    env.insert(name.to_owned(), value);
    Ok(())
}

#[cfg(test)]
mod environment_tests {
    use super::*;

    #[test]
    fn environment_names_use_platform_case_rules() {
        let entries = [
            EnvironmentValue {
                name: "AXIOM_CASE".into(),
                value: "x".into(),
            },
            EnvironmentValue {
                name: "axiom_case".into(),
                value: "x".into(),
            },
        ];
        let first = BTreeMap::from([("AXIOM_ÉTAT".into(), "x".into())]);
        let second = BTreeMap::from([("axiom_état".into(), "x".into())]);
        if cfg!(windows) {
            assert!(check_environment(&entries).is_err());
            assert!(same_environment(&first, &second).unwrap());
        } else {
            assert_eq!(check_environment(&entries).unwrap().len(), 2);
            assert!(!same_environment(&first, &second).unwrap());
        }
        let different_value = BTreeMap::from([("AXIOM_ÉTAT".into(), "different".into())]);
        assert!(!same_environment(&first, &different_value).unwrap());

        let mut defaults = BTreeMap::from([("cargo_target_dir".into(), "frozen".into())]);
        add_environment_default(&mut defaults, "CARGO_TARGET_DIR", "default".into()).unwrap();
        if cfg!(windows) {
            assert_eq!(defaults.len(), 1);
            assert_eq!(defaults["cargo_target_dir"], "frozen");
        } else {
            assert_eq!(defaults.len(), 2);
            assert_eq!(defaults["CARGO_TARGET_DIR"], "default");
        }
    }

    #[cfg(windows)]
    #[test]
    fn unicode_case_comparison_agrees_with_windows_environment_lookup() {
        let upper = "AXIOM_TEST_ÅNGSTRÖM_CB52";
        let lower = "axiom_test_ångström_cb52";
        assert!(same_environment_name(upper, lower).unwrap());
        // Use a child process, avoiding any mutation of this test process's
        // shared environment while other tests run.
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile", "-NonInteractive", "-Command",
                "if ([Environment]::GetEnvironmentVariable('axiom_test_ångström_cb52') -cne 'unicode-value') { exit 1 }",
            ])
            .env(upper, "unicode-value")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

// Callbacks never retain Bridge or root chat. AppServer unregisters each handler.
struct Pipeline {
    server: Arc<AppServer>,
    store: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    project: String,
    run: String,
    cancel: Arc<AtomicBool>,
    grace: u64,
    mutations: Mutex<()>,
    authority: Mutex<checkpoint::DispatchBoundaryAuthority>,
    sealed: AtomicBool,
    stopped: AtomicBool,
    generation: AtomicU64,
}
impl Pipeline {
    fn planning_step(&self, name: &str) -> Res<String> {
        let epoch = checkpoint::planning_epoch(&self.load()?);
        Ok(if epoch == 0 {
            name.into()
        } else {
            format!("{name}-{epoch}")
        })
    }
    fn load(&self) -> Res<Checkpoint> {
        checkpoint::load_run(&self.store, &self.run)
    }
    fn edit<T>(&self, f: impl FnOnce(&Checkpoint) -> Res<T>) -> Res<T> {
        let _guard = self.mutations.lock().map_err(io)?;
        if self.sealed.load(Ordering::Acquire) {
            return Err(Failure::new(
                "outcome_unknown",
                "the dispatch boundary closed before this callback completed; no late checkpoint mutation was made",
            ));
        }
        f(&self.load()?)
    }
    fn finish_boundary(&self, failure: Option<Failure>) -> Res<(Checkpoint, Option<Failure>)> {
        let _guard = self.mutations.lock().map_err(io)?;
        // A disconnected callback may still be running. Once the boundary is
        // published it must not mutate the checkpoint behind a new bridge.
        self.sealed.store(true, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
        let observed = self.load()?;
        let authority = self.authority.lock().map_err(io)?;
        checkpoint::finish_dispatch_boundary(&self.store, &self.run, &observed, failure, &authority)
    }
    fn claim(&self, spec: &StepSpec) -> Res<checkpoint::ClaimResult> {
        self.edit(|c| {
            let mut authority = self.authority.lock().map_err(io)?;
            let claim =
                checkpoint::claim_step(&self.store, c.revision, spec, &spec.brief_ref.digest)?;
            authority
                .claimed_prepared
                .insert(claim.attempt.attempt_id.clone());
            Ok(claim)
        })
    }
    fn artifacts(&self) -> PathBuf {
        self.store.join("runs").join(&self.run).join("artifacts")
    }
    fn receipt(&self, kind: &str, r: &ArtifactRef) -> Res<()> {
        self.edit(|c| {
            checkpoint::record_receipt(&self.store, &self.run, c.revision, kind, r).map(|_| ())
        })
    }
    fn phase(&self, phase: Phase) -> Res<()> {
        self.edit(|c| {
            checkpoint::advance_phase(&self.store, &self.run, c.revision, phase).map(|_| ())
        })
    }
    fn spec(
        &self,
        id: &str,
        purpose: Purpose,
        brief: &str,
        scope: Vec<String>,
        checks: Vec<String>,
    ) -> Res<StepSpec> {
        Ok(StepSpec {
            run_id: self.run.clone(),
            step_id: id.into(),
            purpose,
            brief_ref: artifact(&self.artifacts(), brief.as_bytes())?,
            scope,
            dependency_ids: vec![],
            check_ids: checks,
        })
    }
    fn intent(&self) -> Res<String> {
        std::fs::read_to_string(self.load()?.intent.path).map_err(io)
    }
    fn output(r: &Value) -> Res<Value> {
        if !r["structured_output"].is_null() {
            return Ok(r["structured_output"].clone());
        }
        serde_json::from_str(
            r["final_message"]
                .as_str()
                .ok_or_else(|| fail("missing role result"))?,
        )
        .map_err(io)
    }

    fn prepare_runner(
        &self,
        identity: &crate::orch::runner::RunnerIdentity,
        source: &[u8],
    ) -> Res<Value> {
        use super::prepared_runner::{self, PreflightOutcome};
        use crate::orch::runner::{self, RunnerClaim, RunnerPhase, RunnerReceipt, RunnerStatus};
        let disposition = self
            .edit(|c| {
                checkpoint::claim_runner(&self.store, &self.run, c.revision, identity, source)
            })?
            .value;
        let claim_bytes = match disposition {
            checkpoint::RunnerClaimDisposition::Replayed(evidence) => {
                return runner_tool_reply(&evidence.receipt);
            }
            checkpoint::RunnerClaimDisposition::New(bytes) => bytes,
        };
        let claim: RunnerClaim = serde_json::from_slice(&claim_bytes).map_err(io)?;
        let scratch = scratch_path(
            &self.store,
            &self.run,
            &identity.attempt_id,
            Some("runner-preflight"),
        )?;
        let outcome = prepared_runner::preflight(&self.server, &self.repo, &scratch, &claim_bytes, identity, source)
            .map_err(|e| Failure::new("outcome_unknown", format!("runner preparation did not record a terminal outcome ({}); its claim is retained and cannot be automatically retried", e.kind)))?;
        let mut receipt = RunnerReceipt {
            version: runner::RUNNER_VERSION,
            identity: identity.clone(),
            claim_digest: digest(&claim_bytes),
            source: claim.source,
            binary: None,
            compiler: None,
            probe_stdout: None,
            status: RunnerStatus::Unavailable,
            phase: RunnerPhase::Resolve,
            exit_code: None,
            timed_out: false,
            policy_applied: false,
            stdout: None,
            stderr: None,
        };
        match outcome {
            PreflightOutcome::Unavailable => {}
            PreflightOutcome::Terminal {
                phase,
                status,
                compiler,
                probe_command,
                command,
                binary,
            } => {
                receipt.phase = phase;
                receipt.status = status;
                receipt.exit_code = command.exit_code;
                receipt.timed_out = command.timed_out;
                receipt.policy_applied = command.policy_applied;
                let save = |label: &str, bytes: &[u8]| {
                    checkpoint::save_runner_bytes(
                        &self.store,
                        &self.run,
                        identity.epoch,
                        label,
                        bytes,
                    )
                };
                receipt.stdout = Some(save(
                    "stdout",
                    &std::fs::read(&command.stdout_path).map_err(io)?,
                )?);
                receipt.stderr = Some(save(
                    "stderr",
                    &std::fs::read(&command.stderr_path).map_err(io)?,
                )?);
                if let (Some(compiler), Some(probe)) = (&compiler, probe_command) {
                    let probe_stdout = std::fs::read(&probe.stdout_path).map_err(io)?;
                    if probe_stdout != compiler.version_verbose.as_bytes() {
                        return Err(Failure::new(
                            "outcome_unknown",
                            "compiler probe output changed before evidence was stored",
                        ));
                    }
                    receipt.probe_stdout = Some(save("probe-stdout", &probe_stdout)?);
                    save(
                        "probe-stderr",
                        &std::fs::read(probe.stderr_path).map_err(io)?,
                    )?;
                }
                receipt.compiler = compiler;
                if let Some(binary) = binary {
                    receipt.binary = Some(save("binary", &binary)?);
                }
            }
        }
        self.edit(|c| {
            checkpoint::complete_runner(&self.store, &self.run, c.revision, &receipt).map(|_| ())
        })?;
        runner_tool_reply(&receipt)
    }
    fn research(self: &Arc<Self>) -> Res<()> {
        let c = self.load()?;
        if c.research.is_some() {
            return Ok(());
        }
        let revision = c
            .receipts
            .keys()
            .filter(|k| k.starts_with("intent-rejection-"))
            .count();
        let feedback = if revision == 0 {
            String::new()
        } else {
            format!("\nRevise the previous research {:?} using the core's rejection {:?}. Preserve relevant findings; resolve the stated intent conflict. No engineering.", c.receipts.get(&format!("rejected-research-{revision}")), c.receipts.get(&format!("intent-rejection-{revision}")))
        };
        let spec=self.spec(&format!("research-{revision}"),Purpose::Research,&format!("Independently research intent, constraints, alternatives, risks, implementation scope and frozen evaluation recommendations. Return JSON {{summary, findings}}. Intent: {}{}",self.intent()?,feedback),vec![],vec![])?;
        let (_,r,_)=self.role(&spec,vec![],Some(json!({"type":"object","required":["summary","findings"],"properties":{"summary":{"type":"string"},"findings":{"type":"array","items":{"type":"string"}}},"additionalProperties":false})))?;
        self.receipt("research", &r)
    }
    fn plan(self: &Arc<Self>) -> Res<()> {
        let checkpoint = self.load()?;
        if checkpoint.plan.is_some() {
            return Ok(());
        }
        let (epoch, recovery) = planning_recovery(&self.store, &checkpoint)?;
        let research = checkpoint
            .research
            .ok_or_else(|| fail("research missing"))?;
        let planner_brief=format!("Fresh independent plan of approved intent: {}\nRead research {:?}. Return JSON units:[{{id,brief,scope:[relative paths],depends_on:[unit IDs]}}], checks:[{{id,program,args,env:[{{name,value}}],timeout_s}}]. Checks freeze before engineering; use {{repo}} for product paths and {{scratch}} for writable artifacts. Commands have scratch cwd and cannot write product. Rust separately assigns every writing unit an absolute AXIOM_SCRATCH and CARGO_TARGET_DIR through its environment. Unit briefs must use those assigned paths; never request a relative scratch/ directory in the repository. No later model-supplied commands. Include substantive project checks. Units have nonempty disjoint scopes and acyclic dependencies. Rust adds mandatory SOL assembly. Optional assembly_scope lists exact integration/delivery paths; otherwise SOL uses the engineering scope union. It is required when content artifacts need additional product delivery paths. Optional unit kind=content uses Luna for non-engineering text/bulk artifacts with empty scope, saved as a report for assembly; otherwise kind=engineer uses Terra with nonempty scope. This run is one current iteration. Freeze only checks of its deliverable. Git commits, push, installation and later rounds are post-verification core duties: do not require them in checks or acceptance. No remote/package release required. Also return independent_evaluation: a concise brief of WHAT behavior held-out/independent acceptance must cover when the intent requires it, otherwise null. Actual runtime capability, do not promise more: Rust then invokes a separate read-only ASTRA HIGH tester that returns checks with a typed ready/failed status. Read-only inspection of relevant public files and APIs via native exec_command is allowed. The only compilation tool optionally compiles and links one parameterized Rust runner in managed scratch without running it; do not execute product, tests, cases, compilers or arbitrary commands through inspection. Rust persists and SHA-256-freezes accepted commands, executes every frozen check in the baseline before engineering (product read-only, per-check scratch cwd), and starts writers only after a typed baseline classification says the acceptance infrastructure is ready. Isolation is contextual: writer and assembler briefs never receive private checks or the tester report, but there is no OS-level guarantee that writers cannot read the run store. Therefore never require OS-unreadable private areas, tester-run sealing or trial runs, and never make units wait for, request or verify evaluation-preparation or baseline reports: Rust enforces that ordering before any unit starts. Missing independent runner/sample files are not a blocker. Keep public regression checks and private evaluation preparation distinct.",self.intent()?,research.path);
        let planner_brief = format!("{planner_brief}\nPublic check IDs must not begin independent-; this prefix is reserved for independently prepared private checks (for example independent-h). The independent_evaluation brief specifies the behavior to cover, not incompatible private IDs. If an explicit user requirement truly conflicts with this contract, return no implementable units instead of silently changing the requirement.{recovery}");
        let spec = self.spec(
            &self.planning_step("planner")?,
            Purpose::Planner,
            &planner_brief,
            vec![],
            vec![],
        )?;
        let schema = json!({"type":"object","required":["assembly_scope","units","checks"],"additionalProperties":false,"properties":{"assembly_scope":{"type":["array","null"],"minItems":1,"items":{"type":"string"}},"units":{"type":"array","items":{"type":"object","required":["kind","id","brief","scope","depends_on"],"additionalProperties":false,"properties":{"kind":{"type":["string","null"],"enum":["engineer","content",null]},"id":{"type":"string"},"brief":{"type":"string"},"scope":{"type":"array","items":{"type":"string"}},"depends_on":{"type":"array","items":{"type":"string"}}}}},"checks":{"type":"array","minItems":1,"items":{"type":"object","required":["id","program","args","env","timeout_s"],"additionalProperties":false,"properties":{"id":{"type":"string"},"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"env":{"type":"array","items":{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string"},"value":{"type":"string"}},"required":["name","value"]}},"timeout_s":{"type":"integer","minimum":1,"maximum":1800}}}}}});
        let mut schema = schema;
        schema["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("independent_evaluation"));
        schema["properties"]["independent_evaluation"] = json!({"type":["string","null"]});
        let check_schema = schema["properties"]["checks"].clone();
        let (report, _, _) = self.role(&spec, vec![], Some(schema))?;
        let value = Self::output(&report)?;
        let plan: Plan = serde_json::from_value(value.clone()).map_err(io)?;
        if plan.checks.is_empty() || plan.units.is_empty() {
            return Err(fail(
                "plan must provide implementable units and public regression checks",
            ));
        }
        if plan
            .checks
            .iter()
            .any(|check| check.id.starts_with("independent-"))
        {
            return Err(fail("public check ID uses reserved independent- prefix"));
        }
        for check in &plan.checks {
            check_environment(&check.env)?;
            if crate::orch::runner::parse_sentinel(&check.program)?.is_some() {
                return Err(Failure::new(
                    "evaluation_unprepared",
                    "prepared runner programs are reserved for independent private checks",
                ));
            }
        }
        let r = artifact(&self.artifacts(), &serde_json::to_vec(&value).map_err(io)?)?;
        let mut steps = vec![];
        for unit in plan.units {
            let content = unit.kind.as_deref() == Some("content");
            if unit
                .kind
                .as_deref()
                .is_some_and(|k| !["content", "engineer"].contains(&k))
            {
                return Err(fail("unknown planned unit kind"));
            }
            if (unit.scope.is_empty() != content)
                || unit.brief.trim().is_empty()
                || [
                    "baseline",
                    "research",
                    "planner",
                    "assembly",
                    "verification",
                    "memory",
                ]
                .contains(&unit.id.as_str())
                || unit.id.starts_with("coord-")
                || unit.id.starts_with("evaluation-prep-")
            {
                return Err(fail("invalid engineering unit"));
            }
            if steps.iter().any(|s: &StepSpec| {
                s.scope.iter().any(|a| {
                    unit.scope.iter().any(|b| {
                        crate::orch::covers(&[a.clone()], b) || crate::orch::covers(&[b.clone()], a)
                    })
                })
            }) {
                return Err(fail("engineering scopes overlap"));
            }
            let memory = if self.home.join("memory/memory.db").exists() {
                let db = crate::memory::Memory::open(&self.home)?;
                let found = db.search_reusable(&unit.brief, &self.project, &self.repo, 3)?;
                let selected: Vec<_> = found["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(
                        |row| json!({"id":row["id"],"title":row["title"],"summary":row["summary"]}),
                    )
                    .collect();
                serde_json::to_string(&selected).map_err(io)?
            } else {
                "[]".into()
            };
            let mut step = self.spec(
                &unit.id,
                if content {
                    Purpose::Content
                } else {
                    Purpose::Engineer
                },
                &format!(
                    "{}\nREUSE (current repository summaries only): {}",
                    unit.brief, memory
                ),
                unit.scope,
                vec![],
            )?;
            step.dependency_ids = unit.depends_on;
            steps.push(step);
        }
        let mut visited = std::collections::BTreeSet::new();
        loop {
            let before = visited.len();
            for s in &steps {
                if s.dependency_ids.iter().all(|d| visited.contains(d)) {
                    visited.insert(s.step_id.clone());
                }
            }
            if visited.len() == steps.len() {
                break;
            }
            if before == visited.len() {
                return Err(fail("cyclic or missing engineering dependency"));
            }
        }
        let mut check_specs = plan.checks;
        if let Some(brief) = plan.independent_evaluation {
            if brief.trim().is_empty() {
                return Err(fail("independent evaluation brief is empty"));
            }
            let (prep_epoch, prep_recovery) = planning_recovery(&self.store, &self.load()?)?;
            if prep_epoch != epoch || prep_recovery != recovery {
                return Err(fail("planning recovery changed during preparation"));
            }
            let prep_brief = format!("PREPARE INDEPENDENT EVALUATION before engineering. You are a separate ASTRA HIGH tester, not a planner or engineer. Approved public plan: {:?}. Evaluation requirements: {}. Intent: {}. Your real capability: you are read-only and return check definitions. You may use native exec_command only to inspect relevant public files and APIs; do not execute product, tests, cases or compilers with it. Optionally call prepare_rust_runner once with source_utf8 for a parameterized Rust program before selecting held-out cases. The managed tool only compiles and links source in scratch with fixed rustc flags and no network; it does not execute the runner, product, diagnostic cases or held-out cases. It is not an arbitrary command tool. On success use its exact program sentinel as the private check program and pass {{repo}} and the later held-out cases as arguments; on failure return status=failed, not an alternate check. Source is limited to 256 KiB and the compiled binary to 64 MiB. Rust compiler macros may read host files: this is not OS read isolation. Compilation does not prove functional correctness or that cases are semantically independent. Rust persists and SHA-256-freezes accepted commands, runs each in the baseline before engineering with the product read-only and a fresh per-check scratch cwd, and withholds your report and commands from writer briefs (contextual isolation, not an OS guarantee). Create compact, self-contained executable checks with NEW held-out cases from the approved behavior and evaluation method. Existing runners or sample files are NOT prerequisites: return inline scripts/arguments that construct fixtures in {{scratch}} and inspect the product at {{repo}}. Every check must reference {{repo}} and decide pass/fail from actual product behavior. Do not modify product or run development. Do not merely invoke existing public tests. Controlled protocol fixtures may exercise real Rust validators for functional acceptance, labelled synthetic; they do not prove live models, providers or OS isolation. Live validation is separate. Each check id must start independent-. Return JSON {{status,summary,exposure,checks}}: status=ready only when the checks are genuine held-out acceptance of the product; status=failed with the concrete reason and checks=[] when you cannot produce them. Never return placeholder, fixed-exit or self-declared-failure checks. exposure states which cases stay private and the limits of this isolation. Use the same program/args/env/timeout_s check format and actual available local executables; commands may write only scratch. Do not depend on any uncreated runner, later writer-owned test or mutable external data. Keep each inline command compact. Missing product behavior may fail the baseline; missing evaluation infrastructure must not be assumed away.",r.path,brief,self.intent()?);
            let prep_brief = format!("{prep_brief}{prep_recovery}");
            let prep = self.spec(
                &format!("evaluation-prep-{epoch}"),
                Purpose::Verifier,
                &prep_brief,
                vec![],
                vec![],
            )?;
            let mut prep_checks = check_schema.clone();
            prep_checks["minItems"] = json!(0);
            let (report, receipt, _) = self.role(&prep, vec![tool("prepare_rust_runner", json!({"source_utf8":{"type":"string","maxLength":262144}}))], Some(json!({"type":"object","required":["status","summary","exposure","checks"],"additionalProperties":false,"properties":{"status":{"type":"string","enum":["ready","failed"]},"summary":{"type":"string"},"exposure":{"type":"string"},"checks":prep_checks}})))?;
            let prep_checkpoint = self.load()?;
            if prep_checkpoint
                .receipts
                .contains_key(&format!("runner-claim-{epoch}"))
            {
                let evidence =
                    checkpoint::runner_evidence(&self.store, &prep_checkpoint, epoch, false)?;
                if evidence.receipt.status != crate::orch::runner::RunnerStatus::Ready {
                    return Err(Failure::new("evaluation_unprepared", "The managed runner did not compile and link; no fallback checks or writers may run. Its evidence was retained."));
                }
            }
            let value = Self::output(&report)?;
            if value["status"] != "ready" {
                return Err(Failure::new(
                    "evaluation_unprepared",
                    format!(
                        "independent evaluation preparation failed before engineering: {}. No checks were installed and no writer started; research and the failure evidence are preserved, so the corrected prerequisite may be retried with recovery retry_planning.",
                        value["summary"].as_str().unwrap_or("no reason given")
                    ),
                ));
            }
            if value["exposure"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(Failure::new(
                    "evaluation_unprepared",
                    "independent evaluation preparation reported ready without stating the exposure boundary; which cases stay private is part of the contract. No checks were installed and no writer started.".to_owned(),
                ));
            }
            let independent: Vec<Check> =
                serde_json::from_value(value["checks"].clone()).map_err(io)?;
            if independent.is_empty()
                || independent
                    .iter()
                    .any(|c| !c.id.starts_with("independent-"))
            {
                return Err(fail(
                    "independent evaluator must supply named executable checks",
                ));
            }
            // A command that never reads the product cannot be its acceptance;
            // this rejects fixed-exit placeholders even when status claims ready.
            if let Some(c) = independent.iter().find(|c| {
                !std::iter::once(&c.program)
                    .chain(&c.args)
                    .chain(c.env.iter().map(|e| &e.value))
                    .any(|s| s.contains("{repo}"))
            }) {
                return Err(Failure::new(
                    "evaluation_unprepared",
                    format!("independent check {} does not inspect the product ({{repo}}); placeholder commands are not acceptance. No checks were installed and no writer started.", c.id),
                ));
            }
            // Relabelling a public regression check is not held-out acceptance.
            for private in &independent {
                let private_env = check_environment(&private.env)?;
                for public in &check_specs {
                    if public.program == private.program
                        && public.args == private.args
                        && same_environment(&check_environment(&public.env)?, &private_env)?
                    {
                        return Err(Failure::new(
                            "evaluation_unprepared",
                            format!("independent check {} repeats a public regression command; public checks cannot be relabelled as held-out acceptance. No checks were installed and no writer started.", private.id),
                        ));
                    }
                }
            }
            if independent
                .iter()
                .any(|i| check_specs.iter().any(|p| p.id == i.id))
            {
                return Err(fail("independent check ID collides with a public check"));
            }
            self.receipt(&format!("evaluation-{epoch}"), &receipt)?;
            check_specs.extend(independent);
        }
        let mut checks = check_specs
            .into_iter()
            .map(|c| {
                crate::base::require_slug("check_id", &c.id)?;
                if c.timeout_s == 0 || c.timeout_s > 1800 {
                    return Err(fail("check budget must be finite and <=1800s"));
                }
                let env = check_environment(&c.env)?;
                let mut f = FrozenCheck {
                    check_id: c.id,
                    program: c.program,
                    args: c.args,
                    env,
                    timeout_s: c.timeout_s,
                    snapshot: Snapshot {
                        source_digest: "pending".into(),
                        plan_digest: r.digest.clone(),
                        checks_digest: "pending".into(),
                    },
                    spec_digest: String::new(),
                };
                f.spec_digest = checkpoint::check_spec_digest(&f)?;
                Ok(f)
            })
            .collect::<Res<Vec<_>>>()?;
        let snapshot = checkpoint::frozen_snapshot(&self.repo, &r, &checks)?;
        for c in &mut checks {
            c.snapshot = snapshot.clone();
        }
        let ids = checks
            .iter()
            .map(|c| c.check_id.clone())
            .collect::<Vec<_>>();
        let assembly_scope = plan.assembly_scope.unwrap_or_else(|| {
            steps
                .iter()
                .filter(|s| s.purpose == Purpose::Engineer)
                .flat_map(|s| s.scope.clone())
                .collect()
        });
        if assembly_scope.is_empty() {
            return Err(fail("plan must scope mandatory assembly writes"));
        }
        steps.push(self.spec("assembly",Purpose::Assembler,&format!("Integrate all units for approved intent: {}. Read only the provided engineering/content result references. Preserve other owners. Run appropriate public checks; reports alone are not success. Do not inspect private evaluation artifacts. No commit, push or install.",self.intent()?),assembly_scope,ids.clone())?);
        steps.push(self.spec("baseline",Purpose::Verifier,"Execute frozen baseline checks before engineering. Invoke verify_run for EVERY ID in VERIFY, then classify each actual result: passed (exit 0); product_missing (fails only because the deliverable does not exist yet); product_defect (existing product behaves wrongly); infrastructure_failure (the check itself cannot run or cannot meaningfully judge the product: broken command, missing executable, timeout, placeholder or self-declared preparation failure). Product failures are expected pre-change observations, not permission to change acceptance criteria. Return JSON {infrastructure_ready:boolean, summary:string, observations:[{check_id, classification}]} covering every ID. Never quote private command bodies, held-out inputs or expected values.",vec![],ids.clone())?);
        steps.push(self.spec("verification",Purpose::Verifier,"Independently inspect assembled implementation against approved intent/plan. Invoke verify_run for EVERY frozen check ID in VERIFY. Rust executes these commands; do not become a second core. Return JSON {pass:boolean,summary:string,defect_owner}. defect_owner=none when pass; assembly when SOL can repair the defect inside the unchanged assembly scope and frozen checks; upstream when the cause is the plan, the evaluation preparation, a missing prerequisite or capability that assembly cannot legally change. Reject real defects in this run's current deliverable. Commit, push, installation and subsequent rounds happen after ready_to_commit; do not reject their absence before that boundary. Completion alone is not a pass. Never reveal held-out inputs, expected values, private command bodies or private output paths in feedback to writers; report actionable defect categories without exposing those cases.",vec![],ids)?);
        steps.push(self.spec("memory",Purpose::Memory,"Record only independently verified outcomes. Invoke record_memory with the provided review evidence digest and concise reusable summary. Do not claim installation, release, push or independent live acceptance.",vec![],vec![])?);
        self.edit(|c| {
            checkpoint::install_plan(&self.store, &self.run, c.revision, &r, &steps, &checks)
                .map(|_| ())
        })
    }
    fn ready_units(&self) -> Res<Vec<StepSpec>> {
        let c = self.load()?;
        Ok(c.steps
            .values()
            .filter(|s| {
                matches!(s.purpose, Purpose::Engineer | Purpose::Content)
                    && !c.attempts.values().any(|a| {
                        a.step_id == s.step_id
                            && matches!(
                                a.outcome,
                                Some(AttemptOutcome::Completed | AttemptOutcome::Blocked)
                            )
                    })
                    && s.dependency_ids.iter().all(|d| {
                        c.attempts.values().any(|a| {
                            &a.step_id == d && a.outcome == Some(AttemptOutcome::Completed)
                        })
                    })
            })
            .cloned()
            .collect())
    }
    fn batch(self: &Arc<Self>, ids: &[String], caller: &checkpoint::ScopeBatchCaller) -> Res<()> {
        if self.load()?.phase != Phase::Engineering {
            return Err(fail("batch outside engineering phase"));
        }
        let ready = self.ready_units()?;
        if ids.is_empty()
            || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
            || ids.iter().any(|id| !ready.iter().any(|s| &s.step_id == id))
        {
            return Err(fail("unknown, unready or duplicate batch unit"));
        }
        let batch: Vec<_> = ready
            .into_iter()
            .filter(|s| ids.contains(&s.step_id))
            .collect();
        let members = batch
            .iter()
            .map(|s| {
                let claim = self.claim(s)?;
                if claim.attempt.state != AttemptState::Prepared {
                    return Err(fail("batch attempt requires reconciliation"));
                }
                Ok((s.step_id.clone(), claim.attempt.attempt_id))
            })
            .collect::<Res<Vec<_>>>()?;
        let batch_ref = self
            .edit(|c| {
                checkpoint::register_scope_batch(
                    &self.store,
                    &self.run,
                    c.revision,
                    &members,
                    caller,
                )
            })?
            .value;
        std::thread::scope(|scope| {
            let tasks = batch
                .iter()
                .map(|s| {
                    let batch_ref = &batch_ref;
                    scope.spawn(move || {
                        let result = self
                            .role_with_peers(s, vec![], Some(writer_schema()), Some(batch_ref))
                            .map(|_| ());
                        match result {
                            // A typed refusal is a known business result: record it,
                            // let peers finish, and let drive stop dependents.
                            Err(e) if e.kind == "work_blocked" => Ok(()),
                            Err(e) => {
                                self.cancel.store(true, Ordering::Release);
                                Err(e)
                            }
                            ok => ok,
                        }
                    })
                })
                .collect::<Vec<_>>();
            let results = tasks
                .into_iter()
                .map(|t| {
                    t.join()
                        .map_err(|_| fail("engineering thread panicked"))
                        .and_then(|r| r)
                })
                .collect::<Vec<_>>();
            for result in results {
                result?;
            }
            Ok(())
        })
    }
    fn check(&self, id: &str, executor: ExecutorIdentity) -> Res<Value> {
        let c = self.load()?;
        let baseline = c.phase == Phase::Planning
            && c.attempts
                .get(&executor.attempt_id)
                .is_some_and(|a| a.step_id == "baseline");
        if c.phase != Phase::Verifying && !baseline {
            return Err(fail("check callback outside verification"));
        }
        let f = c
            .checks
            .get(id)
            .ok_or_else(|| fail("unknown frozen check ID"))?
            .clone();
        if let Some(r) = c.receipts.get(&format!(
            "{}-{id}",
            if baseline { "baseline" } else { "check" }
        )) {
            let result: CheckResult =
                serde_json::from_slice(&std::fs::read(&r.path).map_err(io)?).map_err(io)?;
            if result.executor.attempt_id != executor.attempt_id {
                return Err(fail("check belongs to another execution"));
            }
            return serde_json::to_value(result).map_err(io);
        }
        let scratch = scratch_path(&self.store, &self.run, &executor.attempt_id, Some(id))?;
        let expand = |s: &str| {
            s.replace("{repo}", &command_path(&self.repo))
                .replace("{scratch}", &command_path(&scratch))
        };
        // Existing checkpoints can contain a pair of names that were treated
        // as distinct by an older Windows version; do not run such an
        // ambiguous frozen command or silently let the OS choose a value.
        let frozen_env = f
            .env
            .iter()
            .map(|(name, value)| EnvironmentValue {
                name: name.clone(),
                value: expand(value),
            })
            .collect::<Vec<_>>();
        let mut env = check_environment(&frozen_env)?;
        for (key, dir) in [
            ("CARGO_TARGET_DIR", "target"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
            ("TMPDIR", "tmp"),
        ] {
            let p = scratch.join(dir);
            std::fs::create_dir_all(&p).map_err(io)?;
            // Defaults must not silently change the frozen command contract.
            add_environment_default(&mut env, key, p.to_string_lossy().into_owned())?;
        }
        let program = if let Some(sentinel) = crate::orch::runner::parse_sentinel(&f.program)? {
            let plan = c.plan.as_ref().ok_or_else(|| fail("runner plan missing"))?;
            checkpoint::validate_runner_checks(&self.store, &c, plan, &c.checks)?;
            let evidence = checkpoint::runner_evidence(
                &self.store,
                &c,
                checkpoint::planning_epoch(&c),
                false,
            )?;
            let probe =
                match super::prepared_runner::probe_host(&self.server, &self.repo, &scratch)? {
                    super::prepared_runner::ProbeOutcome::Ready(probe) => probe,
                    _ => {
                        return Err(Failure::new(
                            "evaluation_unprepared",
                            "the prepared runner host probe failed",
                        ))
                    }
                };
            let executable = super::prepared_runner::materialize(
                &self.repo,
                &scratch,
                &evidence.claim_bytes,
                &evidence.receipt,
                &evidence.receipt.identity,
                &sentinel,
                &evidence.source,
                evidence
                    .binary
                    .as_deref()
                    .ok_or_else(|| fail("prepared runner binary missing"))?,
                evidence
                    .probe_stdout
                    .as_deref()
                    .ok_or_else(|| fail("prepared runner probe missing"))?,
                &probe,
            )?;
            command_path(&executable)
        } else {
            expand(&f.program)
        };
        let result = self.server.exec_check(&codex::CommandSpec {
            program,
            args: f.args.iter().map(|s| expand(s)).collect(),
            cwd: scratch.clone(),
            env,
            read_only_roots: vec![self.repo.clone()],
            writable_roots: vec![scratch.clone()],
            output_dir: scratch,
            timeout_s: f.timeout_s,
        })?;
        let result = CheckResult {
            check_id: id.into(),
            snapshot: f.snapshot,
            spec_digest: f.spec_digest,
            exit_code: if result.timed_out {
                None
            } else {
                result.exit_code
            },
            policy_applied: result.policy_applied,
            stdout: reference(&result.stdout_path)?,
            stderr: reference(&result.stderr_path)?,
            executor: ExecutorIdentity {
                policy: ExecutionPolicy::ScratchCheck,
                ..executor
            },
        };
        if baseline {
            self.edit(|c| {
                checkpoint::record_baseline_check(&self.store, &self.run, c.revision, &result)
                    .map(|_| ())
            })?;
        } else {
            self.edit(|c| {
                checkpoint::record_check(&self.store, &self.run, c.revision, &result).map(|_| ())
            })?;
        }
        Ok(serde_json::to_value(result).map_err(io)?)
    }
    fn coordinator(self: &Arc<Self>, id: &str, brief: &str, tools: Vec<DynamicTool>) -> Res<()> {
        let s = self.spec(id, Purpose::Dispatcher, brief, vec![], vec![])?;
        self.role(&s, tools, None).map(|_| ())
    }
    /// First listed step whose only terminal work result is a typed refusal.
    fn blocked_step(c: &Checkpoint, steps: &[&str]) -> Option<Failure> {
        steps.iter().find_map(|id| {
            let attempts = c.attempts.values().filter(|a| a.step_id == *id);
            if attempts
                .clone()
                .any(|a| a.outcome == Some(AttemptOutcome::Completed))
            {
                return None;
            }
            let a = attempts
                .filter(|a| a.outcome == Some(AttemptOutcome::Blocked))
                .last()?;
            let reason = a
                .raw_result
                .as_ref()
                .and_then(|r| std::fs::read(&r.path).ok())
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| work_blocked(&v))
                .unwrap_or_else(|| "saved report unavailable".into());
            Some(blocked_failure(id, &reason))
        })
    }
    /// Separate acceptance-infrastructure readiness from expected pre-change
    /// product failures, cross-checked against the executed baseline receipts.
    fn baseline_gate(&self, _report: &Value, r: &ArtifactRef) -> Res<()> {
        self.edit(|c| {
            checkpoint::record_baseline_gate(&self.store, &self.run, c.revision, r).map(|_| ())
        })
    }
    fn drive(self: &Arc<Self>) -> Res<()> {
        loop {
            let c = self.load()?;
            self.authority.lock().map_err(io)?.phase = Some(c.phase.clone());
            match c.phase {
                Phase::Research => {
                    let revision = c
                        .receipts
                        .keys()
                        .filter(|k| k.starts_with("intent-rejection-"))
                        .count();
                    self.coordinator(&format!("coord-research-{revision}"),"Read-only dispatcher. Call run_research once for independent Astra MAX research. Do not implement, approve or continue beyond research. External core decides pass/reject.",vec![tool("run_research",json!({}))])?;
                    self.phase(Phase::IntentReview)?;
                }
                Phase::Planning => {
                    self.coordinator(&self.planning_step("coord-plan")?,"Read-only dispatcher. Approved research exists. Call run_plan once for a fresh independent Astra HIGH plan. Do not engineer or become a core.",vec![tool("run_plan",json!({}))])?;
                    let c = self.load()?;
                    if !c.steps.contains_key("baseline") {
                        return Err(Failure::new(
                            "plan_not_installed",
                            "planning returned no installable plan, so Rust installed no baseline or engineering steps. No check ran and no writer started; read the coordinator report for the model's stated reason, fix it, and start a new run.",
                        ));
                    }
                    let (report, r, _) = self.role(
                        c.steps.get("baseline").ok_or_else(|| fail("missing baseline step"))?,
                        vec![tool("verify_run", json!({"check_id":{"type":"string"}}))],
                        Some(json!({"type":"object","required":["infrastructure_ready","summary","observations"],"additionalProperties":false,"properties":{
                            "infrastructure_ready":{"type":"boolean"},"summary":{"type":"string"},
                            "observations":{"type":"array","items":{"type":"object","required":["check_id","classification"],"additionalProperties":false,"properties":{
                                "check_id":{"type":"string"},
                                "classification":{"type":"string","enum":["passed","product_missing","product_defect","infrastructure_failure"]}}}}}})),
                    )?;
                    let c = self.load()?;
                    if c.checks
                        .keys()
                        .any(|id| !c.receipts.contains_key(&format!("baseline-{id}")))
                    {
                        return Err(fail("baseline omitted frozen checks"));
                    }
                    self.baseline_gate(&report, &r)?;
                    self.phase(Phase::Engineering)?;
                }
                Phase::Engineering => {
                    let units = c
                        .steps
                        .values()
                        .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Content))
                        .map(|s| s.step_id.as_str())
                        .collect::<Vec<_>>();
                    if let Some(e) = Self::blocked_step(&c, &units) {
                        return Err(e);
                    }
                    let ready = self.ready_units()?;
                    if ready.is_empty() {
                        if c.steps
                            .values()
                            .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Content))
                            .all(|s| {
                                c.attempts.values().any(|a| {
                                    a.step_id == s.step_id
                                        && a.outcome == Some(AttemptOutcome::Completed)
                                })
                            })
                        {
                            self.phase(Phase::Assembling)?;
                            continue;
                        }
                        return Err(fail("engineering dependencies stalled"));
                    }
                    let ids = ready.iter().map(|s| s.step_id.clone()).collect::<Vec<_>>();
                    let id = format!(
                        "coord-batch-{}",
                        &digest(&serde_json::to_vec(&ids).map_err(io)?)[..16]
                    );
                    self.coordinator(&id,&format!("Read-only dispatcher. Call dispatch_batch with ALL independent ready unit_ids in one call so Rust executes concurrently: {}. Do not engineer yourself.",json!(ids)),vec![tool("dispatch_batch",json!({"unit_ids":{"type":"array","items":{"type":"string"}}}))])?;
                    let next = self.ready_units()?;
                    if ids.iter().any(|id| next.iter().any(|s| &s.step_id == id)) {
                        return Err(fail("dispatcher omitted ready units"));
                    }
                }
                Phase::Assembling => {
                    let n = c
                        .receipts
                        .keys()
                        .filter(|k| k.starts_with("rework-"))
                        .count();
                    let step = if n == 0 {
                        "assembly".into()
                    } else {
                        format!("assembly-rework-{n}")
                    };
                    if let Some(e) = Self::blocked_step(&c, &[step.as_str()]) {
                        return Err(e);
                    }
                    let (_, r, _) = self.role(
                        c.steps
                            .get(&step)
                            .ok_or_else(|| fail("mandatory assembly missing"))?,
                        vec![],
                        Some(writer_schema()),
                    )?;
                    self.receipt("assembly", &r)?;
                    self.phase(Phase::Verifying)?;
                }
                Phase::Verifying => {
                    let n = c
                        .receipts
                        .keys()
                        .filter(|k| k.starts_with("rework-"))
                        .count();
                    let step = if n == 0 {
                        "verification".into()
                    } else {
                        format!("verification-rework-{n}")
                    };
                    if !c.attempts.values().any(|a| a.step_id == step) {
                        self.edit(|c| {
                            checkpoint::freeze_verification(&self.store, &self.run, c.revision)
                                .map(|_| ())
                        })?;
                    }
                    let (report,r,executor)=self.role(c.steps.get(&step).ok_or_else(||fail("missing verifier step"))?,vec![tool("verify_run",json!({"check_id":{"type":"string"}}))],Some(json!({"type":"object","required":["pass","summary","defect_owner"],"properties":{"pass":{"type":"boolean"},"summary":{"type":"string"},"defect_owner":{"type":"string","enum":["none","assembly","upstream"]}},"additionalProperties":false})))?;
                    let c = self.load()?;
                    if c.checks
                        .keys()
                        .any(|id| !c.receipts.contains_key(&format!("check-{id}")))
                    {
                        return Err(fail("independent verifier omitted frozen checks"));
                    }
                    let pass = Self::output(&report)?["pass"] == true
                        && c.checks
                            .keys()
                            .all(|id| checkpoint::successful_check(&c, id));
                    let snapshot = c
                        .checks
                        .values()
                        .next()
                        .ok_or_else(|| fail("missing frozen checks"))?
                        .snapshot
                        .clone();
                    self.edit(|c| {
                        checkpoint::record_independent_review(
                            &self.store,
                            &self.run,
                            c.revision,
                            &IndependentReview {
                                review_id: executor.attempt_id.clone(),
                                snapshot,
                                executor,
                                check_ids: c.checks.keys().cloned().collect(),
                                pass,
                                report: r,
                            },
                        )
                        .map(|_| ())
                    })?;
                    if !pass {
                        // The verifier has seen held-out data. Its negative
                        // review remains evidence, but cannot trigger another
                        // writer or reuse that evaluation as independent.
                        checkpoint::public_rework_checks(&self.store, &self.load()?)?;
                        let out = Self::output(&report)?;
                        let summary = out["summary"].as_str().unwrap_or("");
                        // Recovery follows the failure's origin: SOL may only
                        // change assembly scope, so upstream causes must not be
                        // retried as rework, and a rejected deliverable needs a
                        // repairable owner.
                        match out["defect_owner"].as_str() {
                            Some("upstream") => {
                                return Err(Failure::new(
                                    "upstream_defect",
                                    format!("independent verifier attributed the failure to an upstream cause (the plan, the evaluation preparation or a missing prerequisite), which SOL assembly cannot repair inside the unchanged scope and frozen checks; no rework was scheduled. {summary}. The frozen plan, checks and reports stay as evidence: fix the stated cause and start a new run."),
                                ));
                            }
                            Some("assembly") => {}
                            _ => {
                                return Err(Failure::new(
                                    "verification_attribution_invalid",
                                    format!("independent verifier returned pass=false with defect_owner=none, so no legal repair target was named; no rework was scheduled and nothing was retried. {summary}"),
                                ));
                            }
                        }
                        self.edit(|c| {
                            checkpoint::prepare_rework(&self.store, &self.run, c.revision)
                                .map(|_| ())
                        })?;
                        continue;
                    }
                    self.phase(Phase::Memory)?;
                }
                Phase::Memory => {
                    let review = checkpoint::verified_review(&c)
                        .ok_or_else(|| fail("current verified review missing"))?;
                    self.role(c.steps.get("memory").ok_or_else(||fail("missing memory step"))?,vec![tool("record_memory",json!({"evidence_id":{"type":"string","enum":[review.digest]},"content":{"type":"string"}}))],None)?;
                    self.phase(Phase::ReadyToCommit)?;
                }
                _ => return Ok(()),
            }
        }
    }
    fn role(
        self: &Arc<Self>,
        spec: &StepSpec,
        tools: Vec<DynamicTool>,
        schema: Option<Value>,
    ) -> Res<(Value, ArtifactRef, ExecutorIdentity)> {
        self.role_with_peers(spec, tools, schema, None)
    }
    fn role_with_peers(
        self: &Arc<Self>,
        spec: &StepSpec,
        tools: Vec<DynamicTool>,
        schema: Option<Value>,
        batch: Option<&ArtifactRef>,
    ) -> Res<(Value, ArtifactRef, ExecutorIdentity)> {
        if self.cancel.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
            return Err(Failure::new("cancelled", "dispatch cancelled"));
        }
        let claim = self.claim(spec)?;
        if claim.disposition == ClaimDisposition::ExistingActive
            && claim.attempt.state != AttemptState::Prepared
        {
            return Err(Failure::new(
                "outcome_unknown",
                "existing attempt requires reconciliation, not duplication",
            ));
        }
        let writing = matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler);
        let (model_role, effort) = match spec.purpose {
            Purpose::Research => ("astra", "max"),
            Purpose::Planner | Purpose::Verifier => ("astra", "high"),
            Purpose::Assembler => ("sol", "high"),
            Purpose::Content => ("luna", "medium"),
            _ => ("terra", "medium"),
        };
        let identity = |thread_id: String, model: String| ExecutorIdentity {
            attempt_id: claim.attempt.attempt_id.clone(),
            thread_id,
            model,
            purpose: spec.purpose.clone(),
            policy: if writing {
                ExecutionPolicy::WorkspaceWrite
            } else {
                ExecutionPolicy::ProductReadOnly
            },
        };
        if claim.disposition == ClaimDisposition::ExistingTerminal {
            let (report, r, model) = checkpoint::terminal_attempt_report(
                &self.store,
                &self.load()?,
                &claim.attempt.attempt_id,
            )?;
            return Ok((report, r, identity(model.thread_id, model.configured_model)));
        }
        let model = self.server.resolve_model(model_role)?;
        let generation = self.server.generation();
        let expected = self
            .generation
            .compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
            .unwrap_or_else(|old| old);
        if expected != 0 && expected != generation {
            self.stopped.store(true, Ordering::Release);
            return Err(Failure::new(
                "outcome_unknown",
                "run connection generation changed; reconcile before further work",
            ));
        }
        let before = if writing {
            Some(crate::orch::lifecycle::contents(&self.repo)?)
        } else {
            None
        };
        let bound = Arc::new(Bound {
            pipeline: self.clone(),
            purpose: spec.purpose.clone(),
            attempt: claim.attempt.attempt_id.clone(),
            execution: Mutex::new(None),
            tools: tools.iter().map(|t| t.name.clone()).collect(),
            model: model.clone(),
            effort: effort.into(),
            operation: Mutex::new(()),
        });
        let brief = std::fs::read_to_string(&spec.brief_ref.path).map_err(io)?;
        let c = self.load()?;
        let scratch = if writing {
            let path = scratch_path(&self.store, &self.run, &claim.attempt.attempt_id, None)?;
            Some(PathBuf::from(command_path(&path)))
        } else {
            None
        };
        let writer_reports = c
            .attempts
            .values()
            .filter(|a| {
                a.outcome == Some(AttemptOutcome::Completed)
                    && c.steps.get(&a.step_id).is_some_and(|s| {
                        matches!(s.purpose, Purpose::Engineer | Purpose::Content)
                            && (spec.purpose == Purpose::Assembler
                                || spec.dependency_ids.contains(&a.step_id))
                    })
            })
            .filter_map(|a| a.raw_result.clone())
            .collect::<Vec<_>>();
        let context = if spec.purpose == Purpose::Verifier {
            let intent = if spec.step_id.starts_with("evaluation-prep-") {
                String::new()
            } else {
                format!("\nIntent: {}", self.intent()?)
            };
            format!(
                "{intent}\nResearch: {:?}\nPlan: {:?}\nReview evidence: {:?}\nPrivate evaluation (verifier only): {:?}",
                c.research,
                c.plan,
                c.receipts
                    .iter()
                    .filter(|(k, _)| k.starts_with("review-"))
                    .collect::<Vec<_>>(),
                c.receipts.get(&format!("evaluation-{}", checkpoint::planning_epoch(&c)))
            )
        } else if spec.purpose == Purpose::Memory {
            let review = checkpoint::verified_review(&c)
                .ok_or_else(|| fail("current verified review missing"))?;
            format!(
                "\nIntent: {}\nResearch: {:?}\nPlan: {:?}\nVerified review evidence: {:?}",
                self.intent()?,
                c.research,
                c.plan,
                review
            )
        } else if matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler) {
            // Non-leaking prerequisite proof: counts and Rust-enforced facts only.
            let public = checkpoint::public_check_ids(&self.store, &c)?;
            let private = c.checks.len() - public.len();
            let gate = if c.receipts.contains_key(checkpoint::BASELINE_GATE) {
                format!("Rust already verified the prerequisites before any writer started: {} frozen checks (of which {private} are private held-out checks whose contents are withheld from writers by design) executed in the baseline and the acceptance infrastructure was classified ready; pre-change product failures are expected. Engineering is authorized; do not wait for, request or search for evaluation-preparation or baseline reports.", c.checks.len())
            } else {
                "Rust baseline gate receipt is missing; report blocked rather than guessing.".into()
            };
            format!("\nApproved plan with frozen check definitions: {:?}. Read this artifact when needed; do not search sibling directories, other runs or independent acceptance data for missing instructions.\nPrerequisites: {gate}\nCompleted prerequisite unit reports: {:?}.\nRust assigned this absolute scratch directory: {}. AXIOM_SCRATCH, CARGO_TARGET_DIR and temporary-directory environment variables are already set for your shell. All build/test output goes there, never to a relative scratch/ directory under the repository. This concrete runtime location supersedes generic scratch wording in a unit brief.\nReturn JSON {{status,summary,artifact:null}}: status=completed only when this unit's deliverable is in place (or verifiably already satisfied, with that evidence); otherwise status=blocked with the concrete missing prerequisite. A blocked result stops dependents.", c.plan, writer_reports, scratch.as_ref().map(|p| command_path(p)).unwrap_or_default())
        } else if spec.purpose == Purpose::Content {
            "\nReturn JSON {status,summary,artifact}: put the full requested artifact text in artifact with status=completed, or status=blocked with the concrete reason and artifact=null.".into()
        } else {
            String::new()
        };
        let profile = match spec.purpose {
            Purpose::Dispatcher => include_str!("../../engine/agents/dispatcher.md"),
            Purpose::Research => include_str!("../../engine/agents/astra.md"),
            Purpose::Planner => include_str!("../../engine/agents/architect.md"),
            Purpose::Engineer => include_str!("../../engine/agents/engineer.md"),
            Purpose::Assembler => include_str!("../../engine/agents/assembler.md"),
            Purpose::Verifier => include_str!("../../engine/agents/reviewer.md"),
            Purpose::Memory => include_str!("../../engine/agents/memory-keeper.md"),
            Purpose::Content => include_str!("../../engine/agents/artifact.md"),
        };
        let req = RunRequest { brief:format!("ROLE {:?}\nGOAL\n{}{}\nSCOPE\n{:?}\nVERIFY\n{:?}",spec.purpose,brief,context,spec.scope,spec.check_ids), cwd:self.repo.to_string_lossy().into_owned(),model:model.clone(),sandbox:if writing {"workspace-write"} else {"read-only"}.into(),effort:Some(effort.into()),output_schema:schema,
            developer_instructions:Some(format!("{}\nSingle-purpose lean leaf, not a core. Use only the brief and registered tools. Batch independent reads and reuse supplied evidence. On Windows, for non-ASCII UTF-8 reads use Get-Content -LiteralPath <file> -Raw -Encoding UTF8 | ConvertTo-Json -EscapeHandling EscapeNonAscii, or Python json.dumps with ensure_ascii=True. Do not set Console encoding in a constrained shell; if output contains U+FFFD, reread losslessly before judging the file. Test unresolved failures, then stop when the fixed checks and requested boundary are satisfied. No native collaboration, other agents, GOAL continuation, commits, installation or push. Preserve other owners. Engineering writes only within SCOPE; other roles must not modify product. Return requested structured result, explicitly report failure. Model completion is not verification.",profile)), timeout_s:1800,lean:true,keep_plugins:vec![],web_search:spec.purpose==Purpose::Research };
        let result = self.server.run_with_options(
            &req,
            RunOptions {
                attempt_id: claim.attempt.attempt_id.clone(),
                thread: ThreadMode::FreshPersistent,
                tools,
                handler: Some(bound.clone()),
                observer: Some(bound),
                cancel: self.cancel.clone(),
                cancel_grace_ms: self.grace,
                skill_paths: vec![],
                scratch: scratch.clone(),
            },
        );
        let mut report = match result {
            Ok(r) if matches!(r.terminal, TransportTerminal::OutcomeUnknown) => {
                let last_status = r.report["status"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("unavailable");
                let detail = r.report["error"]
                    .as_object()
                    .and_then(|e| e["message"].as_str().map(|m| format!(": {m}")))
                    .unwrap_or_default();
                return Err(Failure::new("outcome_unknown", format!(
                    "execution terminal could not be established for step {} on thread {} (last status {last_status}){detail}",
                    spec.step_id, r.execution.thread_id)));
            }
            Ok(r) => r,
            Err(e) => return Err(e),
        };
        let failure = if matches!(report.terminal, TransportTerminal::Completed) {
            None
        } else {
            Some(cause_failure(&report.report).unwrap_or_else(|| {
                fail(format!(
                    "{:?} failed: {}",
                    spec.purpose, report.report["error"]
                ))
            }))
        };
        let blocked = if failure.is_none() && writes_work(&spec.purpose) {
            work_blocked(&report.report)
        } else {
            None
        };
        let outcome = match report.terminal {
            TransportTerminal::Completed if blocked.is_some() => AttemptOutcome::Blocked,
            TransportTerminal::Completed => AttemptOutcome::Completed,
            TransportTerminal::Failed => AttemptOutcome::Failed,
            TransportTerminal::Cancelled => AttemptOutcome::Cancelled,
            TransportTerminal::OutcomeUnknown => unreachable!(),
        };
        let saved = if let Some(before) = before {
            // The original role alone holds the pre-dispatch map. Persist the
            // complete before/after audit before making any writer terminal,
            // including known failure or cancellation.
            let after = crate::orch::lifecycle::contents(&self.repo)?;
            report.report["scope_checked"] = json!(true);
            report.report["scope_snapshot"] = json!(crate::orch::snapshot(&self.repo, &spec.scope));
            let (saved, payload, outside) = self.edit(|c| {
                checkpoint::commit_scoped_result(
                    &self.store,
                    &self.run,
                    c.revision,
                    &claim.attempt.attempt_id,
                    outcome.clone(),
                    &report.report,
                    batch,
                    scratch
                        .as_ref()
                        .ok_or_else(|| fail("writer scratch missing"))?,
                    &before,
                    &after,
                )
            })?;
            report.report = payload;
            if outside {
                return Err(fail("worker changed paths outside assigned scope"));
            }
            saved.value
        } else {
            self.edit(|c| {
                checkpoint::commit_result(
                    &self.store,
                    &self.run,
                    c.revision,
                    &claim.attempt.attempt_id,
                    outcome.clone(),
                    &report.report,
                )
            })?
            .value
        };
        if let Some(failure) = failure {
            return Err(failure);
        }
        if let Some(reason) = blocked {
            return Err(blocked_failure(&spec.step_id, &reason));
        }
        let evidence =
            checkpoint::attempt_model(&self.store, &self.load()?, &claim.attempt.attempt_id)?;
        Ok((
            report.report,
            saved,
            identity(evidence.thread_id, evidence.configured_model),
        ))
    }
}

#[cfg(test)]
mod scratch_scope_tests {
    use super::*;

    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let result = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn scratch_paths_are_short_and_isolated_by_full_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp
            .path()
            .join("orchestrate")
            .join("sumints-acceptance-dev9-20261101");
        std::fs::create_dir_all(&store).unwrap();
        let run = "run-3ec46c705c1083c78121a1e4";
        let attempt = "d8360f2dc4281fefc91271cc719cd632";
        let id = "independent-sumints-contract-debug-release";
        let assigned = scratch_path(&store, run, attempt, Some(id)).unwrap();
        assert_eq!(
            scratch_path(&store, run, attempt, Some(id)).unwrap(),
            assigned
        );
        assert_eq!(assigned.parent().unwrap(), store.join("s"));
        let other_run = scratch_path(&store, "run-other", attempt, Some(id)).unwrap();
        let other_attempt = scratch_path(&store, run, "other-attempt", Some(id)).unwrap();
        let other_check = scratch_path(&store, run, attempt, Some("another-check")).unwrap();
        let writer = scratch_path(&store, run, attempt, None).unwrap();
        let distinct = [&assigned, &other_run, &other_attempt, &other_check, &writer]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(distinct.len(), 5);
        assert!(!scratch_change(
            &assigned,
            &other_check.join("out").to_string_lossy()
        ));
        let legacy = store
            .join("runs")
            .join(run)
            .join("scratch")
            .join(attempt)
            .join(id);
        assert!(assigned.to_string_lossy().len() + 40 <= legacy.to_string_lossy().len());
        assert_eq!(assigned.file_name().unwrap().len(), 64);
    }

    #[test]
    fn scratch_creation_survives_interruption_and_concurrent_retries() {
        use std::sync::{Arc, Barrier};
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(tmp.path().join("store"));
        std::fs::create_dir_all(store.join("s")).unwrap();
        // An interruption before allocating the identity leaves only the root.
        let expected = scratch_path(&store, "run", "attempt", Some("check")).unwrap();
        std::fs::write(expected.join("preserved"), b"before interruption").unwrap();
        // An interruption after allocating it retains its existing contents.
        assert_eq!(
            scratch_path(&store, "run", "attempt", Some("check")).unwrap(),
            expected
        );
        assert_eq!(
            std::fs::read(expected.join("preserved")).unwrap(),
            b"before interruption"
        );

        let count = 16;
        let barrier = Arc::new(Barrier::new(count));
        let tasks = (0..count)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    scratch_path(&store, "run", "new-attempt", Some("concurrent-check"))
                })
            })
            .collect::<Vec<_>>();
        let assigned = tasks
            .into_iter()
            .map(|t| t.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert!(assigned.iter().all(|p| p == &assigned[0]));
    }

    #[test]
    fn legacy_scratch_is_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let legacy = store.join("runs/run-old/scratch/attempt-old/work");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("existing"), "preserved").unwrap();
        assert_eq!(
            scratch_path(&store, "run-old", "attempt-old", None).unwrap(),
            legacy
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("existing")).unwrap(),
            "preserved"
        );
        let check = store.join("runs/run-old/scratch/attempt-old/public-check");
        std::fs::create_dir_all(&check).unwrap();
        assert_eq!(
            scratch_path(&store, "run-old", "attempt-old", Some("public-check")).unwrap(),
            check
        );
    }

    #[cfg(windows)]
    #[test]
    fn existing_uppercase_legacy_names_are_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let legacy = store.join("RUNS/RUN-OLD/SCRATCH/ATTEMPT-OLD/PUBLIC-CHECK");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("preserved"), "yes").unwrap();
        let assigned =
            scratch_path(&store, "run-old", "attempt-old", Some("public-check")).unwrap();
        assert_eq!(
            assigned.canonicalize().unwrap(),
            legacy.canonicalize().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(assigned.join("preserved")).unwrap(),
            "yes"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn legacy_scratch_rejects_links_to_another_attempt_in_the_same_store() {
        fn link(link: &Path, target: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, link).unwrap();
            #[cfg(windows)]
            junction(link, target);
        }
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let other_attempt = store.join("runs/run-b/scratch/attempt-b");
        let other_check = other_attempt.join("check");
        std::fs::create_dir_all(&other_check).unwrap();
        std::fs::write(other_check.join("preserved"), "yes").unwrap();

        let leaf_parent = store.join("runs/run-a/scratch/attempt-a");
        std::fs::create_dir_all(&leaf_parent).unwrap();
        link(&leaf_parent.join("check"), &other_check);
        assert!(scratch_path(&store, "run-a", "attempt-a", Some("check")).is_err());

        let ancestor_parent = store.join("runs/run-c/scratch");
        std::fs::create_dir_all(&ancestor_parent).unwrap();
        link(&ancestor_parent.join("attempt-c"), &other_attempt);
        assert!(scratch_path(&store, "run-c", "attempt-c", Some("check")).is_err());
        assert_eq!(
            std::fs::read_to_string(other_check.join("preserved")).unwrap(),
            "yes"
        );
    }

    #[cfg(windows)]
    #[test]
    fn existing_uppercase_scratch_names_are_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let identity = serde_json::to_vec(&("run", "attempt", Some("check"))).unwrap();
        let existing = store.join("S").join(digest(&identity).to_ascii_uppercase());
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::write(existing.join("preserved"), "yes").unwrap();
        let assigned = scratch_path(&store, "run", "attempt", Some("check")).unwrap();
        assert_eq!(
            assigned.canonicalize().unwrap(),
            existing.canonicalize().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(assigned.join("preserved")).unwrap(),
            "yes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn scratch_root_cannot_escape_store_via_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&store).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, store.join("s")).unwrap();
        assert!(scratch_path(&store, "run", "attempt", Some("check")).is_err());
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn scratch_junctions_cannot_escape_the_store() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        let root_store = tmp.path().join("root-store");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&root_store).unwrap();
        junction(&root_store.join("s"), &outside);
        assert!(scratch_path(&root_store, "run", "attempt", Some("check")).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);

        let slot_store = tmp.path().join("slot-store");
        std::fs::create_dir_all(slot_store.join("s")).unwrap();
        let identity = serde_json::to_vec(&("run", "attempt", Some("check"))).unwrap();
        junction(&slot_store.join("s").join(digest(&identity)), &outside);
        assert!(scratch_path(&slot_store, "run", "attempt", Some("check")).is_err());
        assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn assigned_scratch_supports_an_actual_msvc_cargo_link() {
        let rustc = std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .unwrap();
        assert!(rustc.status.success());
        if !String::from_utf8_lossy(&rustc.stdout).contains("-pc-windows-msvc") {
            eprintln!("MSVC linker regression skipped: Rust host is not MSVC");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let scratch = scratch_path(
            tmp.path(),
            "run-3ec46c705c1083c78121a1e4",
            "d8360f2dc4281fefc91271cc719cd632",
            Some("independent-sumints-contract-debug-release"),
        )
        .unwrap();
        assert_eq!(scratch.file_name().unwrap().to_str().unwrap().len(), 64);
        let harness = scratch.join("h");
        let source = harness.join("c/src");
        let target = tmp.path().join("t");
        assert!(scratch.starts_with(tmp.path()));
        assert!(source.starts_with(tmp.path()) && target.starts_with(tmp.path()));
        std::fs::create_dir_all(&source).unwrap();
        let manifest = source.parent().unwrap().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname='independent_control'\nversion='0.0.0'\nedition='2021'\n",
        )
        .unwrap();
        let mut code = String::new();
        for n in 0..64 {
            code.push_str(&format!(
                "mod m{n} {{ #[inline(never)] pub fn f(x:i64)->i64 {{ x.wrapping_add({n}) }} }}\n"
            ));
        }
        code.push_str("fn main() { let mut x=std::env::args().count() as i64;\n");
        for n in 0..64 {
            code.push_str(&format!("x=m{n}::f(x);\n"));
        }
        code.push_str("println!(\"{}\",x); }\n");
        std::fs::write(source.join("main.rs"), code).unwrap();
        let cargo = option_env!("CARGO").unwrap_or("cargo");
        let lock = std::process::Command::new(cargo)
            .args(["generate-lockfile", "--offline", "--manifest-path"])
            .arg(&manifest)
            .output()
            .unwrap();
        assert!(
            lock.status.success(),
            "{}",
            String::from_utf8_lossy(&lock.stderr)
        );
        let build = std::process::Command::new(cargo)
            .args(["build", "--frozen", "--offline", "--manifest-path"])
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "target {}: {}",
            target.display(),
            String::from_utf8_lossy(&build.stderr)
        );
        let binary = target.join("debug/independent_control.exe");
        assert!(binary.is_file());
        let bytes = std::fs::read(&binary).unwrap();
        assert!(
            bytes.starts_with(b"MZ"),
            "invalid PE binary: {}",
            binary.display()
        );
    }

    #[test]
    fn only_the_assigned_absolute_scratch_tree_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let scratch = tmp.path().join("assigned");
        let other = tmp.path().join("another-attempt");
        std::fs::create_dir(&scratch).unwrap();
        std::fs::create_dir(&other).unwrap();
        let file = scratch.join("checks.py");
        std::fs::write(&file, "pass").unwrap();
        assert!(scratch_change(&scratch, &file.to_string_lossy()));
        std::fs::remove_file(&file).unwrap();
        assert!(scratch_change(&scratch, &file.to_string_lossy()));
        assert!(!scratch_change(
            &scratch,
            &other.join("checks.py").to_string_lossy()
        ));
        assert!(!scratch_change(
            &scratch,
            &scratch
                .join("../another-attempt/checks.py")
                .to_string_lossy()
        ));
        assert!(!scratch_change(&scratch, "checks.py"));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&other, scratch.join("escape")).unwrap();
            assert!(!scratch_change(
                &scratch,
                &scratch.join("escape/checks.py").to_string_lossy()
            ));
        }
    }
}

#[cfg(test)]
mod work_result_tests {
    use super::*;
    #[test]
    fn typed_work_status_decides_success_without_requiring_a_change() {
        let completed = json!({"structured_output":{"status":"completed","summary":"already satisfied","artifact":null},"final_message":null});
        assert!(
            work_blocked(&completed).is_none(),
            "an already-satisfied unit must not be forced to manufacture changes"
        );
        let blocked = json!({"structured_output":{"status":"blocked","summary":"prerequisite missing","artifact":null},"final_message":null});
        assert_eq!(
            work_blocked(&blocked).as_deref(),
            Some("prerequisite missing")
        );
        // A normally terminating turn without the typed result is not success.
        let missing = json!({"structured_output":null,"final_message":null});
        assert_eq!(
            work_blocked(&missing).as_deref(),
            Some("missing machine-readable work status")
        );
        // A replayed history turn carries the typed result as final text.
        let replay = json!({"structured_output":null,
            "final_message":"{\"status\":\"blocked\",\"summary\":\"no runner\",\"artifact\":null}"});
        assert_eq!(work_blocked(&replay).as_deref(), Some("no runner"));
    }

    #[test]
    fn transport_wrapper_does_not_hide_the_typed_failure() {
        let report = json!({"error":{"kind":"tool_handler_failed","message":"wrapper","causes":[
            {"id":"tool-1","error":{"code":-32603,"message":"wrapper","data":{"kind":"evaluation_unprepared","message":"preparation failed"}}}]}});
        let failure = cause_failure(&report).expect("typed cause");
        assert_eq!(failure.kind, "evaluation_unprepared");
        assert_eq!(failure.message, "preparation failed");
        assert!(cause_failure(&json!({"error":null})).is_none());
        assert!(cause_failure(&json!({})).is_none());
    }

    #[test]
    fn terminal_role_replay_uses_original_receipt_with_transport_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let store = tmp.path().join("store");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&store).unwrap();
        let brief = artifact(&store, b"research request").unwrap();
        let c = checkpoint::create_run(
            &store,
            &checkpoint::RunSeed {
                run_id: "run".into(),
                repo: checkpoint::RepoIdentity {
                    canonical_root: repo.canonicalize().unwrap(),
                    identity_digest: "repo".into(),
                },
                intent: brief.clone(),
            },
        )
        .unwrap();
        let spec = StepSpec {
            run_id: "run".into(),
            step_id: "research".into(),
            purpose: Purpose::Research,
            brief_ref: brief,
            scope: vec![],
            dependency_ids: vec![],
            check_ids: vec![],
        };
        let claim =
            checkpoint::claim_step(&store, c.revision, &spec, &spec.brief_ref.digest).unwrap();
        let rev = checkpoint::record_attempt_dispatch(
            &store,
            "run",
            claim.revision,
            &AttemptEvent {
                event_id: "dispatch".into(),
                attempt_id: claim.attempt.attempt_id.clone(),
                sequence: 1,
                kind: AttemptEventKind::Dispatching {
                    generation: 17,
                    thread_id: "original-thread".into(),
                },
            },
            "original-astra",
            "original-astra",
            Some("max"),
        )
        .unwrap();
        let rev = checkpoint::record_attempt_event(
            &store,
            "run",
            rev,
            &AttemptEvent {
                event_id: "running".into(),
                attempt_id: claim.attempt.attempt_id.clone(),
                sequence: 2,
                kind: AttemptEventKind::Running {
                    turn_id: "original-turn".into(),
                },
            },
        )
        .unwrap();
        let report = json!({"status":"completed","thread_id":"original-thread","turn_id":"original-turn",
            "model":null,"model_source":"thread/read","final_message":"saved result"});
        let saved = checkpoint::commit_result(
            &store,
            "run",
            rev,
            &claim.attempt.attempt_id,
            AttemptOutcome::Completed,
            &report,
        )
        .unwrap();
        let pipeline = Arc::new(Pipeline {
            server: Arc::new(AppServer::new(vec![
                "axiom-model-server-intentionally-unavailable".into(),
            ])),
            store: store.clone(),
            home: tmp.path().into(),
            repo,
            project: "project".into(),
            run: "run".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            grace: 10,
            mutations: Mutex::new(()),
            authority: Mutex::new(checkpoint::DispatchBoundaryAuthority::default()),
            sealed: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        });
        let (replay, reference, executor) = pipeline.role(&spec, vec![], None).unwrap();
        assert_eq!(replay, report);
        assert_eq!(reference, saved.value);
        assert_eq!(executor.model, "original-astra");
        assert_eq!(executor.thread_id, "original-thread");
        assert_eq!(pipeline.generation.load(Ordering::Acquire), 0);
        std::fs::write(&reference.path, b"report changed").unwrap();
        assert!(pipeline.role(&spec, vec![], None).is_err());
    }
}

#[cfg(test)]
mod boundary_concurrency_tests {
    use super::*;
    use std::sync::{mpsc, Barrier};

    struct Fixture {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        repo: PathBuf,
        store: PathBuf,
        pipeline: Arc<Pipeline>,
        attempt: String,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let home = tmp.path().join("home");
        let store = home.join("orchestrate/demo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&store).unwrap();
        let intent = artifact(&store.join("runs/run/artifacts"), b"intent").unwrap();
        let c = checkpoint::create_run(
            &store,
            &checkpoint::RunSeed {
                run_id: "run".into(),
                repo: checkpoint::RepoIdentity {
                    canonical_root: repo.canonicalize().unwrap(),
                    identity_digest: "repo".into(),
                },
                intent: intent.clone(),
            },
        )
        .unwrap();
        let spec = StepSpec {
            run_id: "run".into(),
            step_id: "research-0".into(),
            purpose: Purpose::Research,
            brief_ref: intent,
            scope: vec![],
            dependency_ids: vec![],
            check_ids: vec![],
        };
        let claim =
            checkpoint::claim_step(&store, c.revision, &spec, &spec.brief_ref.digest).unwrap();
        let attempt = claim.attempt.attempt_id;
        let rev = checkpoint::record_attempt_dispatch(
            &store,
            "run",
            claim.revision,
            &AttemptEvent {
                event_id: "dispatch-test".into(),
                attempt_id: attempt.clone(),
                sequence: 1,
                kind: AttemptEventKind::Dispatching {
                    generation: 17,
                    thread_id: "original-thread".into(),
                },
            },
            "test-model",
            "test-model",
            Some("high"),
        )
        .unwrap();
        checkpoint::record_attempt_event(
            &store,
            "run",
            rev,
            &AttemptEvent {
                event_id: "running-test".into(),
                attempt_id: attempt.clone(),
                sequence: 2,
                kind: AttemptEventKind::Running {
                    turn_id: "original-turn".into(),
                },
            },
        )
        .unwrap();
        let current = checkpoint::load_run(&store, "run").unwrap();
        let mut authority = checkpoint::DispatchBoundaryAuthority {
            phase: Some(Phase::Research),
            original_attempts: current.attempts.clone(),
            ..Default::default()
        };
        authority
            .executions
            .insert(attempt.clone(), current.attempts[&attempt].clone());
        let pipeline = Arc::new(Pipeline {
            server: Arc::new(AppServer::new(vec!["axiom-unavailable-test-server".into()])),
            store: store.clone(),
            home: home.clone(),
            repo: repo.clone(),
            project: "demo".into(),
            run: "run".into(),
            cancel: Arc::new(AtomicBool::new(false)),
            grace: 10,
            mutations: Mutex::new(()),
            authority: Mutex::new(authority),
            sealed: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            generation: AtomicU64::new(17),
        });
        Fixture {
            _tmp: tmp,
            home,
            repo,
            store,
            pipeline,
            attempt,
        }
    }

    fn lost_event(attempt: &str) -> AttemptEvent {
        AttemptEvent {
            event_id: format!("{attempt}-3"),
            attempt_id: attempt.into(),
            sequence: 3,
            kind: AttemptEventKind::OutcomeUnknown {
                reason: "connection lost".into(),
            },
        }
    }

    fn start_foreign_attempt(f: &Fixture) -> String {
        let c = checkpoint::load_run(&f.store, "run").unwrap();
        let spec = StepSpec {
            run_id: "run".into(),
            step_id: "research-foreign".into(),
            purpose: Purpose::Research,
            brief_ref: c.intent.clone(),
            scope: vec![],
            dependency_ids: vec![],
            check_ids: vec![],
        };
        let claim =
            checkpoint::claim_step(&f.store, c.revision, &spec, &spec.brief_ref.digest).unwrap();
        let id = claim.attempt.attempt_id;
        let rev = checkpoint::record_attempt_dispatch(
            &f.store,
            "run",
            claim.revision,
            &AttemptEvent {
                event_id: "foreign-dispatch".into(),
                attempt_id: id.clone(),
                sequence: 1,
                kind: AttemptEventKind::Dispatching {
                    generation: 19,
                    thread_id: "foreign-thread".into(),
                },
            },
            "foreign-model",
            "foreign-model",
            Some("high"),
        )
        .unwrap();
        checkpoint::record_attempt_event(
            &f.store,
            "run",
            rev,
            &AttemptEvent {
                event_id: "foreign-running".into(),
                attempt_id: id.clone(),
                sequence: 2,
                kind: AttemptEventKind::Running {
                    turn_id: "foreign-turn".into(),
                },
            },
        )
        .unwrap();
        id
    }

    #[test]
    fn stale_snapshot_is_reclassified_under_store_lock() {
        let f = fixture();
        let observed = checkpoint::load_run(&f.store, "run").unwrap();
        let gate = Arc::new(Barrier::new(2));
        let event_gate = gate.clone();
        let event_store = f.store.clone();
        let event = lost_event(&f.attempt);
        let expected_revision = observed.revision;
        let worker = std::thread::spawn(move || {
            event_gate.wait();
            checkpoint::record_attempt_event(&event_store, "run", expected_revision, &event)
                .unwrap()
        });
        gate.wait();
        let event_revision = worker.join().unwrap();
        // Reproduce the original interleaving deterministically: publishing an
        // artifact based on the old snapshot must conflict after the event.
        let stale = artifact(
            &f.store.join("runs/run/artifacts"),
            &serde_json::to_vec(&json!({"revision":observed.revision + 1,"phase":observed.phase,
                "status":"outcome_unknown","error":{"kind":"old_error","message":"obsolete result"}})).unwrap(),
        ).unwrap();
        let conflict = checkpoint::record_dispatch_failure_with_receipt(
            &f.store,
            "run",
            observed.revision,
            true,
            &stale,
        )
        .unwrap_err();
        assert_eq!(conflict.kind, "revision_conflict");
        assert_eq!(
            checkpoint::load_run(&f.store, "run").unwrap().revision,
            event_revision
        );
        let (current, failure) = checkpoint::finish_dispatch_boundary(
            &f.store,
            "run",
            &observed,
            Some(Failure::new("old_error", "obsolete result")),
            &f.pipeline.authority.lock().unwrap(),
        )
        .unwrap();
        assert_eq!(current.revision, event_revision);
        assert_eq!(current.status, RunStatus::OutcomeUnknown);
        assert_eq!(
            current.attempts[&f.attempt].state,
            AttemptState::OutcomeUnknown
        );
        assert!(failure.is_none());
        assert!(current.receipts.get(checkpoint::DISPATCH_FAILURE).is_none());
    }

    #[test]
    fn in_flight_event_finishes_before_the_same_pipeline_boundary() {
        let f = fixture();
        let initial = checkpoint::load_run(&f.store, "run").unwrap();
        let p = f.pipeline.clone();
        let event = lost_event(&f.attempt);
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let callback = std::thread::spawn(move || {
            p.edit(|c| {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                checkpoint::record_attempt_event(&p.store, "run", c.revision, &event)
            })
        });
        entered_rx.recv().unwrap();
        assert!(f.pipeline.mutations.try_lock().is_err());
        let boundary_pipeline = f.pipeline.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let boundary = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            boundary_pipeline
                .finish_boundary(Some(Failure::new("outcome_unknown", "connection lost")))
        });
        started_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        let event_revision = callback.join().unwrap().unwrap();
        let (current, failure) = boundary.join().unwrap().unwrap();
        assert_eq!(event_revision, initial.revision + 1);
        assert_eq!(current.revision, event_revision + 1);
        assert_eq!(
            current.attempts[&f.attempt].state,
            AttemptState::OutcomeUnknown
        );
        assert_eq!(failure.unwrap().kind, "outcome_unknown");
        assert!(current
            .receipts
            .contains_key(&format!("event-{}-3", f.attempt)));
        assert_eq!(checkpoint::load_run(&f.store, "run").unwrap(), current);
    }

    #[test]
    fn old_callback_cannot_mutate_after_a_new_bridge_resumes() {
        let f = fixture();
        let key = codex::ExecutionKey {
            generation: 17,
            thread_id: "original-thread".into(),
            turn_id: Some("original-turn".into()),
        };
        let bound = Bound {
            pipeline: f.pipeline.clone(),
            purpose: Purpose::Research,
            attempt: f.attempt.clone(),
            execution: Mutex::new(Some(key.clone())),
            tools: vec![],
            model: "test-model".into(),
            effort: "high".into(),
            operation: Mutex::new(()),
        };
        let event = RunEvent {
            attempt_id: f.attempt.clone(),
            sequence: 3,
            execution: key,
            requested_model: "test-model".into(),
            configured_model: "test-model".into(),
            requested_effort: Some("high".into()),
            kind: RunEventKind::ConnectionLost {
                reason: "connection lost".into(),
            },
        };
        let (started_tx, started_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let old_callback = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            bound.on_event(&event)
        });
        started_rx.recv().unwrap();
        let (first, _) = f
            .pipeline
            .finish_boundary(Some(Failure::new(
                "outcome_unknown",
                "original unresolved attempt",
            )))
            .unwrap();
        assert_eq!(first.status, RunStatus::OutcomeUnknown);
        assert_eq!(
            first.attempts[&f.attempt].state,
            AttemptState::OutcomeUnknown
        );
        assert_eq!(first.attempts[&f.attempt].last_event_sequence, 3);
        let event_key = format!("event-boundary-{}-{}", f.attempt, first.revision);
        let recorded: AttemptEvent =
            serde_json::from_slice(&std::fs::read(&first.receipts[&event_key].path).unwrap())
                .unwrap();
        assert_eq!(recorded.attempt_id, f.attempt);
        assert_eq!(recorded.sequence, 3);
        assert!(matches!(
            recorded.kind,
            AttemptEventKind::OutcomeUnknown { .. }
        ));

        let new_bridge = Bridge::new(crate::bridge::Config {
            data_home: Some(f.home.clone()),
            codex: Ok(vec!["axiom-unavailable-test-server".into()]),
            agy: Err(Failure::new("missing", "unused")),
            wsl: None,
            on_windows: cfg!(windows),
        });
        let resumed = new_bridge
            .dispatch(
                &RootContext {
                    repo_root: f.repo.clone(),
                    project: "demo".into(),
                },
                &DispatchRequest {
                    request_id: "resume-new-bridge".into(),
                    action: DispatchAction::Resume {
                        run_id: "run".into(),
                        recovery: None,
                    },
                },
                &DispatchOptions::default(),
                None,
            )
            .unwrap();
        assert_eq!(resumed.status, RunStatus::OutcomeUnknown);
        let before_release = checkpoint::load_run(&f.store, "run").unwrap();
        release_tx.send(()).unwrap();
        let late = old_callback.join().unwrap().unwrap_err();
        assert_eq!(late.kind, "outcome_unknown");
        assert!(late.message.contains("boundary closed"));
        let after_release = checkpoint::load_run(&f.store, "run").unwrap();
        assert_eq!(after_release, before_release);
        assert_eq!(
            after_release.attempts[&f.attempt].state,
            AttemptState::OutcomeUnknown
        );
        assert!(!after_release
            .receipts
            .contains_key(&format!("event-{}-3", f.attempt)));
        // Explicit stop and repair evidence can still resolve this unknown
        // attempt; sealing did not leave it permanently Running.
        let stop = artifact(
            &f.store.join("runs/run/artifacts"),
            &serde_json::to_vec(&json!({"attempt_id":f.attempt,"terminal":true,
                "thread_id":"original-thread","turn_id":"original-turn","generation":17}))
            .unwrap(),
        )
        .unwrap();
        let repair = artifact(
            &f.store.join("runs/run/artifacts"),
            &serde_json::to_vec(&json!({"attempt_id":f.attempt,"repaired":true})).unwrap(),
        )
        .unwrap();
        checkpoint::resolve_attempt(
            &f.store,
            "run",
            after_release.revision,
            &checkpoint::RecoveryDecision::RetryAfterRepair {
                attempt_id: f.attempt.clone(),
                stopped_evidence: stop,
                repair_evidence: repair,
                reason: "explicit stop and repair evidence".into(),
            },
        )
        .unwrap();
        let repaired = checkpoint::load_run(&f.store, "run").unwrap();
        assert_eq!(repaired.attempts[&f.attempt].state, AttemptState::Terminal);
        assert_eq!(
            repaired.attempts[&f.attempt].outcome,
            Some(AttemptOutcome::Cancelled)
        );
    }

    #[test]
    fn terminal_progress_is_not_overwritten_by_a_stale_failure() {
        let f = fixture();
        let observed = checkpoint::load_run(&f.store, "run").unwrap();
        let report = json!({"status":"completed","thread_id":"original-thread","turn_id":"original-turn","model":"test-model"});
        let rev = checkpoint::commit_result(
            &f.store,
            "run",
            observed.revision,
            &f.attempt,
            AttemptOutcome::Completed,
            &report,
        )
        .unwrap()
        .revision;
        let cancelled =
            checkpoint::cancel_run(&f.store, "run", rev, "cancel after completion").unwrap();
        let (current, failure) = checkpoint::finish_dispatch_boundary(
            &f.store,
            "run",
            &observed,
            Some(Failure::new("outcome_unknown", "obsolete")),
            &f.pipeline.authority.lock().unwrap(),
        )
        .unwrap();
        assert_eq!(current.status, RunStatus::Cancelled);
        assert_eq!(current.revision, cancelled);
        assert!(failure.is_none());
    }

    #[test]
    fn newer_nonterminal_phase_is_not_overwritten_by_a_stale_failure() {
        let f = fixture();
        let observed = checkpoint::load_run(&f.store, "run").unwrap();
        let report = json!({"status":"completed","thread_id":"original-thread","turn_id":"original-turn","model":"test-model"});
        let result = checkpoint::commit_result(
            &f.store,
            "run",
            observed.revision,
            &f.attempt,
            AttemptOutcome::Completed,
            &report,
        )
        .unwrap();
        let rev =
            checkpoint::record_receipt(&f.store, "run", result.revision, "research", &result.value)
                .unwrap();
        let rev = checkpoint::advance_phase(&f.store, "run", rev, Phase::IntentReview).unwrap();
        let rev = checkpoint::record_core_decision(
            &f.store,
            "run",
            rev,
            &CoreDecision {
                research_digest: result.value.digest,
                pass: true,
                reason: "approved".into(),
            },
        )
        .unwrap();
        checkpoint::advance_phase(&f.store, "run", rev, Phase::Planning).unwrap();
        let newer = checkpoint::load_run(&f.store, "run").unwrap();
        let (current, failure) = f
            .pipeline
            .finish_boundary(Some(Failure::new(
                "outcome_unknown",
                "old research failure",
            )))
            .unwrap();
        assert_eq!(current, newer);
        assert_eq!(current.phase, Phase::Planning);
        assert_eq!(current.status, RunStatus::Running);
        assert!(failure.is_none());
    }

    #[test]
    fn confirmed_failure_replaces_stale_unknown_diagnostic() {
        let f = fixture();
        let observed = checkpoint::load_run(&f.store, "run").unwrap();
        let report = json!({"status":"failed","thread_id":"original-thread","turn_id":"original-turn","model":"test-model"});
        let failed = checkpoint::commit_result(
            &f.store,
            "run",
            observed.revision,
            &f.attempt,
            AttemptOutcome::Failed,
            &report,
        )
        .unwrap()
        .revision;
        let (current, failure) = checkpoint::finish_dispatch_boundary(
            &f.store,
            "run",
            &observed,
            Some(Failure::new("outcome_unknown", "obsolete")),
            &f.pipeline.authority.lock().unwrap(),
        )
        .unwrap();
        assert_eq!(current.revision, failed + 1);
        assert_eq!(current.status, RunStatus::Blocked);
        let failure = failure.unwrap();
        assert_eq!(failure.kind, "repair_required");
        assert!(failure.message.contains(&f.attempt));
        assert!(!failure.message.contains("obsolete"));
    }

    #[test]
    fn newer_bridge_running_attempt_is_not_downgraded() {
        let f = fixture();
        let old = checkpoint::load_run(&f.store, "run").unwrap();
        let report = json!({"status":"completed","thread_id":"original-thread","turn_id":"original-turn","model":"test-model"});
        checkpoint::commit_result(
            &f.store,
            "run",
            old.revision,
            &f.attempt,
            AttemptOutcome::Completed,
            &report,
        )
        .unwrap();
        let foreign = start_foreign_attempt(&f);
        let newer = checkpoint::load_run(&f.store, "run").unwrap();
        let (current, failure) = f
            .pipeline
            .finish_boundary(Some(Failure::new(
                "outcome_unknown",
                "obsolete old pipeline",
            )))
            .unwrap();
        assert_eq!(current, newer);
        assert_eq!(current.status, RunStatus::Running);
        assert_eq!(current.attempts[&foreign].state, AttemptState::Running);
        assert!(failure.is_none());
    }

    #[test]
    fn completed_foreign_progress_in_same_phase_is_not_downgraded() {
        let f = fixture();
        let old = checkpoint::load_run(&f.store, "run").unwrap();
        let report = json!({"status":"completed","thread_id":"original-thread","turn_id":"original-turn","model":"test-model"});
        checkpoint::commit_result(
            &f.store,
            "run",
            old.revision,
            &f.attempt,
            AttemptOutcome::Completed,
            &report,
        )
        .unwrap();
        let foreign = start_foreign_attempt(&f);
        let live = checkpoint::load_run(&f.store, "run").unwrap();
        let foreign_report = json!({"status":"completed","thread_id":"foreign-thread","turn_id":"foreign-turn","model":"foreign-model"});
        checkpoint::commit_result(
            &f.store,
            "run",
            live.revision,
            &foreign,
            AttemptOutcome::Completed,
            &foreign_report,
        )
        .unwrap();
        let newer = checkpoint::load_run(&f.store, "run").unwrap();
        let (current, failure) = f
            .pipeline
            .finish_boundary(Some(Failure::new(
                "outcome_unknown",
                "obsolete old pipeline",
            )))
            .unwrap();
        assert_eq!(current, newer);
        assert_eq!(current.phase, Phase::Research);
        assert_eq!(current.status, RunStatus::Running);
        assert_eq!(current.attempts[&foreign].state, AttemptState::Terminal);
        assert!(failure.is_none());
    }

    #[test]
    fn changed_checkpoint_marks_only_the_owned_execution_unknown() {
        let f = fixture();
        let observed = checkpoint::load_run(&f.store, "run").unwrap();
        let foreign = start_foreign_attempt(&f);
        let newer = checkpoint::load_run(&f.store, "run").unwrap();
        let (current, failure) = checkpoint::finish_dispatch_boundary(
            &f.store,
            "run",
            &observed,
            Some(Failure::new("outcome_unknown", "obsolete snapshot")),
            &f.pipeline.authority.lock().unwrap(),
        )
        .unwrap();
        assert_eq!(current.revision, newer.revision + 1);
        assert_eq!(
            current.attempts[&f.attempt].state,
            AttemptState::OutcomeUnknown
        );
        assert_eq!(current.attempts[&foreign], newer.attempts[&foreign]);
        assert_eq!(current.attempts[&foreign].state, AttemptState::Running);
        assert_eq!(failure.unwrap().kind, "outcome_unknown");
        assert!(!current
            .receipts
            .keys()
            .any(|k| k.starts_with(&format!("event-boundary-{foreign}-"))));
    }

    #[test]
    fn stale_or_different_execution_identity_cannot_be_closed() {
        for field in ["attempt", "step", "input", "generation", "thread", "turn"] {
            let f = fixture();
            let before = checkpoint::load_run(&f.store, "run").unwrap();
            {
                let mut authority = f.pipeline.authority.lock().unwrap();
                let owner = authority.executions.get_mut(&f.attempt).unwrap();
                match field {
                    "attempt" => owner.attempt_id.push('x'),
                    "step" => owner.step_id.push('x'),
                    "input" => owner.input_digest.push('x'),
                    "generation" => owner.generation = Some(99),
                    "thread" => owner.thread_id = Some("other-thread".into()),
                    "turn" => owner.turn_id = Some("other-turn".into()),
                    _ => unreachable!(),
                }
            }
            let (after, failure) = f
                .pipeline
                .finish_boundary(Some(Failure::new("outcome_unknown", "old")))
                .unwrap();
            assert_eq!(
                after, before,
                "mismatched {field} must not be marked unknown"
            );
            assert!(failure.is_none(), "mismatched {field}");
        }
    }
}

#[cfg(all(test, windows))]
mod command_path_tests {
    use super::*;
    #[test]
    fn canonical_repo_placeholder_is_readable_by_powershell_select_string() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("scope with spaces");
        std::fs::create_dir(&repo).unwrap();
        std::fs::write(repo.join("Cargo.lock"), "needle\n").unwrap();
        let canonical = repo.canonicalize().unwrap();
        let literal = format!("{}/Cargo.lock", command_path(&canonical)).replace('\'', "''");
        let result = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &format!("$ErrorActionPreference='Stop'; if (-not (Select-String -LiteralPath '{literal}' -Pattern '^needle$' -Quiet)) {{exit 1}}")])
            .output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
