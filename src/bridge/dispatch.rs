//! Public boundary for the checkpoint-backed dispatcher.
//!
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

use super::Bridge;
use crate::base::{Failure, Res};
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
fn relative_change(repo: &Path, name: &str) -> Option<String> {
    let path = Path::new(name);
    let relative = if path.is_absolute() {
        // Codex can report ordinary Windows paths while canonical roots use
        // the verbatim prefix. Normalize both, including deleted paths.
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let clean = |p: &Path| {
            let text = p.to_string_lossy().replace('\\', "/");
            PathBuf::from(text.strip_prefix("//?/").unwrap_or(&text))
        };
        clean(&path).strip_prefix(clean(repo)).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    if relative.components().any(|c| {
        !matches!(
            c,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return None;
    }
    Some(relative.to_string_lossy().replace('\\', "/"))
}
fn scratch_change(scratch: &Path, name: &str) -> bool {
    let path = Path::new(name);
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let Ok(root) = scratch.canonicalize() else {
        return false;
    };
    // Resolve links. For deleted artifacts, resolve the closest surviving parent.
    path.ancestors()
        .find_map(|p| p.canonicalize().ok())
        .is_some_and(|p| p.starts_with(root))
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
struct Bound {
    pipeline: Arc<Pipeline>,
    purpose: Purpose,
    attempt: String,
    execution: Mutex<Option<codex::ExecutionKey>>,
    tools: Vec<String>,
    model: String,
    operation: Mutex<()>,
}
impl RunObserver for Bound {
    fn on_event(&self, event: &RunEvent) -> Res<()> {
        if event.attempt_id != self.attempt {
            return Err(fail("observer attempt mismatch"));
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
                checkpoint::record_attempt_event(
                    &self.pipeline.store,
                    &self.pipeline.run,
                    c.revision,
                    &AttemptEvent {
                        event_id: format!("{}-{}", self.attempt, event.sequence),
                        attempt_id: self.attempt.clone(),
                        sequence: event.sequence,
                        kind,
                    },
                )
                .map(|_| ())
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
                p.batch(&args.unit_ids)?;
                json!({"completed":args.unit_ids})
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
            (Purpose::Memory, "record_memory") if c.phase == Phase::Memory => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Args {
                    evidence_id: String,
                    content: String,
                }
                let args: Args = serde_json::from_value(call.arguments).map_err(io)?;
                if !c
                    .receipts
                    .iter()
                    .any(|(k, r)| k.starts_with("review-") && r.digest == args.evidence_id)
                {
                    return Err(fail("memory evidence digest mismatch"));
                }
                let result = crate::memory::Memory::open(&p.home)?.record_run(
                    &p.project,
                    &p.run,
                    &args.content,
                )?;
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
                return serde_json::from_value(old["result"].clone()).map_err(io);
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
                if c.attempts
                    .values()
                    .any(|a| a.state != AttemptState::Terminal)
                {
                    return Err(Failure::new(
                        "outcome_unknown",
                        "reconcile active attempts before cancellation",
                    ));
                }
                checkpoint::advance_phase(&store, &run, c.revision, Phase::Cancelled)?;
            }
            DispatchAction::Resume { recovery, .. } => {
                if let Some(d) = recovery {
                    checkpoint::resolve_attempt(&store, &run, c.revision, d)?;
                    c = checkpoint::load_run(&store, &run)?;
                }
                checkpoint::reconcile_run(&store, &run, c.revision, &[])?;
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
        if execute {
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
                    if let Some(thread) = &a.thread_id {
                        if let Ok(codex::AttemptRead::Terminal(report)) =
                            p.server.read_attempt(thread, a.turn_id.as_deref())
                        {
                            let outcome = match report.terminal {
                                TransportTerminal::Completed => AttemptOutcome::Completed,
                                TransportTerminal::Failed => AttemptOutcome::Failed,
                                TransportTerminal::Cancelled => AttemptOutcome::Cancelled,
                                _ => continue,
                            };
                            p.edit(|c| {
                                checkpoint::commit_result(
                                    &p.store,
                                    &p.run,
                                    c.revision,
                                    &a.attempt_id,
                                    outcome,
                                    &report.report,
                                )
                                .map(|_| ())
                            })?;
                        }
                    }
                }
            }
            let c = p.load()?;
            if c.attempts
                .values()
                .any(|a| a.state != AttemptState::Terminal && a.state != AttemptState::Prepared)
            {
                reason = "unknown execution retained; no writing attempt redispatched".into();
            } else if c
                .attempts
                .values()
                .any(|a| checkpoint::failed_attempt_needs_repair(&c, a))
            {
                reason = "saved failed execution requires repair before a new attempt".into();
            } else {
                // Zero awaits Rust completion, not a 300s running-response loop.
                match p.drive() {
                    Ok(()) => reason = "authority boundary reached".into(),
                    Err(e) => reason = e.message,
                }
            }
        }
        c = checkpoint::load_run(&store, &run)?;
        let active = c
            .attempts
            .values()
            .any(|a| !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared));
        let status = if execute && active {
            RunStatus::OutcomeUnknown
        } else if execute
            && !matches!(
                c.phase,
                Phase::IntentReview | Phase::ReadyToCommit | Phase::Released | Phase::Cancelled
            )
        {
            RunStatus::Blocked
        } else {
            c.status.clone()
        };
        if execute && c.status != status {
            checkpoint::record_dispatch_failure(
                &store,
                &run,
                c.revision,
                status == RunStatus::OutcomeUnknown,
            )?;
        }
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
            research: c.research,
            artifact_refs: refs,
            next_action,
        };
        if !matches!(
            request.action,
            DispatchAction::Status { .. } | DispatchAction::Resume { .. }
        ) {
            std::fs::create_dir_all(request_path.parent().unwrap()).map_err(io)?;
            crate::base::atomic_write(
                &request_path,
                &serde_json::to_vec(&json!({"input":digest(&input),"result":result}))
                    .map_err(io)?,
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
        f(&self.load()?)
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
        if self.load()?.plan.is_some() {
            return Ok(());
        }
        let research = self
            .load()?
            .research
            .ok_or_else(|| fail("research missing"))?;
        let spec=self.spec(&self.planning_step("planner")?,Purpose::Planner,&format!("Fresh independent plan of approved intent: {}\nRead research {:?}. Return JSON units:[{{id,brief,scope:[relative paths],depends_on:[unit IDs]}}], checks:[{{id,program,args,env:[{{name,value}}],timeout_s}}]. Checks freeze before engineering; use {{repo}} for product paths and {{scratch}} for writable artifacts. Commands have scratch cwd and cannot write product. Rust separately assigns every writing unit an absolute AXIOM_SCRATCH and CARGO_TARGET_DIR through its environment. Unit briefs must use those assigned paths; never request a relative scratch/ directory in the repository. No later model-supplied commands. Include substantive project checks. Units have nonempty disjoint scopes and acyclic dependencies. Rust adds mandatory SOL assembly. Optional assembly_scope lists exact integration/delivery paths; otherwise SOL uses the engineering scope union. It is required when content artifacts need additional product delivery paths. Optional unit kind=content uses Luna for non-engineering text/bulk artifacts with empty scope, saved as a report for assembly; otherwise kind=engineer uses Terra with nonempty scope. This run is one current iteration. Freeze only checks of its deliverable. Git commits, push, installation and later rounds are post-verification core duties: do not require them in checks or acceptance. No remote/package release required. Also return independent_evaluation: a concise preparation brief when the intent requires held-out/independent acceptance, otherwise null. Rust will then invoke a separate read-only ASTRA HIGH tester to create and seal self-contained private check commands BEFORE baseline and engineering. Missing independent runner/sample files are not a blocker: requesting their preparation is your responsibility. Keep public regression checks and private evaluation preparation distinct.",self.intent()?,research.path),vec![],vec![])?;
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
                let found = db.search(&unit.brief, Some(&self.project), 3)?;
                let mut selected = vec![];
                for row in found["entries"].as_array().into_iter().flatten() {
                    if row["stale"] == false {
                        if let Some(id) = row["id"].as_i64() {
                            let entry = db.get(id)?;
                            if entry["entry"]["project_dir"].as_str() == self.repo.to_str() {
                                selected
                                    .push(json!({"title":row["title"],"summary":row["summary"]}));
                            }
                        }
                    }
                }
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
            let epoch = checkpoint::planning_epoch(&self.load()?);
            let prep = self.spec(&format!("evaluation-prep-{epoch}"), Purpose::Verifier,
                &format!("PREPARE INDEPENDENT EVALUATION before engineering. You are a separate ASTRA HIGH tester, not a planner or engineer. Approved public plan: {:?}. Evaluation requirements: {}. Intent: {}. Create compact, self-contained executable checks with NEW held-out cases from the approved behavior and evaluation method. Existing runners or sample files are NOT prerequisites: return inline scripts/arguments that construct fixtures in {{scratch}} and inspect the product at {{repo}}. The Rust executor will persist and hash these commands and inputs before baseline or engineering. Do not modify product or run development. Do not merely invoke existing public tests or pretend a fake role is live acceptance. Each check id must start independent-. Return JSON {{summary,checks}} using the same program/args/env/timeout_s check format. Use actual available local executables; commands run with scratch cwd and may write only scratch. Do not depend on any uncreated runner, later writer-owned test or mutable external data. Keep each inline command compact. Missing product behavior may fail the baseline; missing evaluation infrastructure must not be assumed away.",r.path,brief,self.intent()?),vec![],vec![])?;
            let (report, receipt, _) = self.role(&prep, vec![], Some(json!({"type":"object","required":["summary","checks"],"additionalProperties":false,"properties":{"summary":{"type":"string"},"checks":check_schema}})))?;
            let value = Self::output(&report)?;
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
                let mut env = BTreeMap::new();
                for entry in c.env {
                    if entry.name.is_empty() || env.insert(entry.name, entry.value).is_some() {
                        return Err(fail("empty or duplicate check environment name"));
                    }
                }
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
        steps.push(self.spec("baseline",Purpose::Verifier,"Execute frozen baseline checks before engineering. Invoke verify_run for EVERY ID in VERIFY. Failures are baseline observations, not permission to change acceptance criteria. Return observations.",vec![],ids.clone())?);
        steps.push(self.spec("verification",Purpose::Verifier,"Independently inspect assembled implementation against approved intent/plan. Invoke verify_run for EVERY frozen check ID in VERIFY. Rust executes these commands; do not become a second core. Return JSON {pass:boolean,summary:string}. Reject real defects in this run's current deliverable. Commit, push, installation and subsequent rounds happen after ready_to_commit; do not reject their absence before that boundary. Completion alone is not a pass. Never reveal held-out inputs, expected values, private command bodies or private output paths in feedback to writers; report actionable defect categories without exposing those cases.",vec![],ids)?);
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
                        a.step_id == s.step_id && a.outcome == Some(AttemptOutcome::Completed)
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
    fn batch(self: &Arc<Self>, ids: &[String]) -> Res<()> {
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
        std::thread::scope(|scope| {
            let tasks = ready
                .into_iter()
                .filter(|s| ids.contains(&s.step_id))
                .map(|s| {
                    scope.spawn(move || {
                        let result = self.role(&s, vec![], None).map(|_| ());
                        if result.is_err() {
                            self.cancel.store(true, Ordering::Release);
                        }
                        result
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
        let scratch = self
            .store
            .join("runs")
            .join(&self.run)
            .join("scratch")
            .join(&executor.attempt_id)
            .join(id);
        std::fs::create_dir_all(&scratch).map_err(io)?;
        let expand = |s: &str| {
            s.replace("{repo}", &command_path(&self.repo))
                .replace("{scratch}", &command_path(&scratch))
        };
        let mut env = f
            .env
            .iter()
            .map(|(k, v)| (k.clone(), expand(v)))
            .collect::<BTreeMap<_, _>>();
        for (key, dir) in [
            ("CARGO_TARGET_DIR", "target"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
            ("TMPDIR", "tmp"),
        ] {
            let p = scratch.join(dir);
            std::fs::create_dir_all(&p).map_err(io)?;
            // Defaults must not silently change the frozen command contract.
            env.entry(key.into())
                .or_insert_with(|| p.to_string_lossy().into_owned());
        }
        let result = self.server.exec_check(&codex::CommandSpec {
            program: expand(&f.program),
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
            if !result.policy_applied {
                return Err(fail("baseline execution policy missing"));
            }
            let r = artifact(&self.artifacts(), &serde_json::to_vec(&result).map_err(io)?)?;
            self.receipt(&format!("baseline-{id}"), &r)?;
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
    fn drive(self: &Arc<Self>) -> Res<()> {
        loop {
            let c = self.load()?;
            match c.phase {
                Phase::Research => {
                    let revision = c
                        .receipts
                        .keys()
                        .filter(|k| k.starts_with("intent-rejection-"))
                        .count();
                    self.coordinator(&format!("coord-research-{revision}"),&format!("Read-only dispatcher. Call run_research once for independent Astra MAX research of intent: {}. Do not implement, approve or continue beyond research. External core decides pass/reject.",self.intent()?),vec![tool("run_research",json!({}))])?;
                    self.phase(Phase::IntentReview)?;
                }
                Phase::Planning => {
                    self.coordinator(&self.planning_step("coord-plan")?,"Read-only dispatcher. Approved research exists. Call run_plan once for a fresh independent Astra HIGH plan. Do not engineer or become a core.",vec![tool("run_plan",json!({}))])?;
                    let c = self.load()?;
                    self.role(
                        c.steps
                            .get("baseline")
                            .ok_or_else(|| fail("planner did not install baseline step"))?,
                        vec![tool("verify_run", json!({"check_id":{"type":"string"}}))],
                        None,
                    )?;
                    let c = self.load()?;
                    if c.checks
                        .keys()
                        .any(|id| !c.receipts.contains_key(&format!("baseline-{id}")))
                    {
                        return Err(fail("baseline omitted frozen checks"));
                    }
                    self.phase(Phase::Engineering)?;
                }
                Phase::Engineering => {
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
                    let (_, r, _) = self.role(
                        c.steps
                            .get(&step)
                            .ok_or_else(|| fail("mandatory assembly missing"))?,
                        vec![],
                        None,
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
                    let (report,r,executor)=self.role(c.steps.get(&step).ok_or_else(||fail("missing verifier step"))?,vec![tool("verify_run",json!({"check_id":{"type":"string"}}))],Some(json!({"type":"object","required":["pass","summary"],"properties":{"pass":{"type":"boolean"},"summary":{"type":"string"}},"additionalProperties":false})))?;
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
                        self.edit(|c| {
                            checkpoint::prepare_rework(&self.store, &self.run, c.revision)
                                .map(|_| ())
                        })?;
                        continue;
                    }
                    self.phase(Phase::Memory)?;
                }
                Phase::Memory => {
                    let review = c
                        .receipts
                        .iter()
                        .find(|(k, _)| k.starts_with("review-"))
                        .ok_or_else(|| fail("verified evidence missing"))?
                        .1;
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
        if self.cancel.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
            return Err(Failure::new("cancelled", "dispatch cancelled"));
        }
        let claim = self.edit(|c| {
            checkpoint::claim_step(&self.store, c.revision, spec, &spec.brief_ref.digest)
        })?;
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
        let identity = |thread_id: String| ExecutorIdentity {
            attempt_id: claim.attempt.attempt_id.clone(),
            thread_id,
            model: model.clone(),
            purpose: spec.purpose.clone(),
            policy: if writing {
                ExecutionPolicy::WorkspaceWrite
            } else {
                ExecutionPolicy::ProductReadOnly
            },
        };
        if claim.disposition == ClaimDisposition::ExistingTerminal {
            let r = claim
                .attempt
                .raw_result
                .clone()
                .ok_or_else(|| fail("terminal attempt lacks saved result"))?;
            let report =
                serde_json::from_slice(&std::fs::read(&r.path).map_err(io)?).map_err(io)?;
            return Ok((
                report,
                r,
                identity(claim.attempt.thread_id.clone().unwrap_or_default()),
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
            operation: Mutex::new(()),
        });
        let brief = std::fs::read_to_string(&spec.brief_ref.path).map_err(io)?;
        let c = self.load()?;
        let scratch = if writing {
            let path = self
                .store
                .join("runs")
                .join(&self.run)
                .join("scratch")
                .join(&claim.attempt.attempt_id)
                .join("work");
            std::fs::create_dir_all(&path).map_err(io)?;
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
        let context = if matches!(spec.purpose, Purpose::Verifier | Purpose::Memory) {
            format!(
                "\nIntent: {}\nResearch: {:?}\nPlan: {:?}\nReview evidence: {:?}\nPrivate evaluation (verifier only): {:?}",
                self.intent()?,
                c.research,
                c.plan,
                c.receipts
                    .iter()
                    .filter(|(k, _)| k.starts_with("review-"))
                    .collect::<Vec<_>>(),
                if spec.purpose == Purpose::Verifier { c.receipts.get(&format!("evaluation-{}", checkpoint::planning_epoch(&c))) } else { None }
            )
        } else if matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler) {
            format!("\nApproved plan with frozen check definitions: {:?}. Read this artifact when needed; do not search sibling directories, other runs or independent acceptance data for missing instructions.\nCompleted prerequisite unit reports: {:?}.\nRust assigned this absolute scratch directory: {}. AXIOM_SCRATCH, CARGO_TARGET_DIR and temporary-directory environment variables are already set for your shell. All build/test output goes there, never to a relative scratch/ directory under the repository. This concrete runtime location supersedes generic scratch wording in a unit brief.", c.plan, writer_reports, scratch.as_ref().map(|p| command_path(p)).unwrap_or_default())
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
            developer_instructions:Some(format!("{}\nSingle-purpose lean leaf, not a core. Use only the brief and registered tools. No native collaboration, other agents, GOAL continuation, commits, installation or push. Preserve other owners. Engineering writes only within SCOPE; other roles must not modify product. Return requested structured result, explicitly report failure. Model completion is not verification.",profile)), timeout_s:1800,lean:true,keep_plugins:vec![],web_search:spec.purpose==Purpose::Research };
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
            Ok(r) if matches!(r.terminal, TransportTerminal::Completed) => r,
            Ok(r) => {
                if matches!(r.terminal, TransportTerminal::OutcomeUnknown) {
                    return Err(Failure::new(
                        "outcome_unknown",
                        "execution terminal could not be established",
                    ));
                }
                self.edit(|c| {
                    checkpoint::commit_result(
                        &self.store,
                        &self.run,
                        c.revision,
                        &claim.attempt.attempt_id,
                        if matches!(r.terminal, TransportTerminal::Cancelled) {
                            AttemptOutcome::Cancelled
                        } else {
                            AttemptOutcome::Failed
                        },
                        &r.report,
                    )
                })?;
                return Err(fail(format!(
                    "{:?} failed: {}",
                    spec.purpose, r.report["error"]
                )));
            }
            Err(e) => return Err(e),
        };
        if let Some(before) = before {
            let after = crate::orch::lifecycle::contents(&self.repo)?;
            let c = self.load()?;
            let peers: Vec<String> = if spec.purpose == Purpose::Engineer {
                c.steps
                    .values()
                    .filter(|s| s.purpose == Purpose::Engineer && s.step_id != spec.step_id)
                    .flat_map(|s| s.scope.clone())
                    .collect()
            } else {
                vec![]
            };
            let outside = before.keys().chain(after.keys()).any(|p| {
                before.get(p) != after.get(p)
                    && !crate::orch::covers(&spec.scope, p)
                    && !crate::orch::covers(&peers, p)
            });
            let reported_outside = report.report["file_changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["path"].as_str())
                .any(|p| {
                    !scratch.as_ref().is_some_and(|root| scratch_change(root, p))
                        && relative_change(&self.repo, p)
                            .is_none_or(|p| !crate::orch::covers(&spec.scope, &p))
                });
            if outside || reported_outside {
                report.report["error"] = json!({"kind":"scope_violation","message":"worker changed paths outside assigned product scope; use the assigned external scratch directory for build output"});
                self.edit(|c| {
                    checkpoint::commit_result(
                        &self.store,
                        &self.run,
                        c.revision,
                        &claim.attempt.attempt_id,
                        AttemptOutcome::Failed,
                        &report.report,
                    )
                })?;
                return Err(fail("worker changed paths outside assigned scope"));
            }
            report.report["scope_checked"] = json!(true);
            report.report["scope_snapshot"] = json!(crate::orch::snapshot(&self.repo, &spec.scope));
        }
        let saved = self
            .edit(|c| {
                checkpoint::commit_result(
                    &self.store,
                    &self.run,
                    c.revision,
                    &claim.attempt.attempt_id,
                    AttemptOutcome::Completed,
                    &report.report,
                )
            })?
            .value;
        Ok((report.report, saved, identity(report.execution.thread_id)))
    }
}

#[cfg(test)]
mod scratch_scope_tests {
    use super::*;

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
