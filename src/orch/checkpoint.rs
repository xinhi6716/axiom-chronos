use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

macro_rules! dto { ($($body:item)*) => {$($body)*}; }
macro_rules! wire_struct { ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => { #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)] #[serde(deny_unknown_fields)] pub struct $name { $(pub $field: $ty),* } }; }
macro_rules! wire_enum { ($name:ident { $($variant:ident),* $(,)? }) => { #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)] #[serde(rename_all="snake_case")] pub enum $name { $($variant),* } }; }
dto! {
wire_struct!(ArtifactRef { path: PathBuf, digest: String });
wire_struct!(RepoIdentity { canonical_root: PathBuf, identity_digest: String });
wire_struct!(Snapshot { source_digest: String, plan_digest: String, checks_digest: String });
wire_enum!(Purpose { Dispatcher, Research, Planner, Engineer, Assembler, Verifier, Memory, Content });
wire_enum!(Phase { Research, IntentReview, Planning, Engineering, Assembling, Verifying, Memory, ReadyToCommit, Released, Cancelled });
wire_enum!(RunStatus { NeedsIntentReview, Running, Blocked, OutcomeUnknown, ReadyToCommit, Released, Cancelled });
wire_enum!(ExecutionPolicy { ProductReadOnly, WorkspaceWrite, ScratchCheck });
wire_struct!(StepSpec { run_id: String, step_id: String, purpose: Purpose, brief_ref: ArtifactRef, scope: Vec<String>, dependency_ids: Vec<String>, check_ids: Vec<String> });
wire_enum!(AttemptState { Prepared, Dispatching, Running, Terminal, OutcomeUnknown });
// Blocked: the turn ended normally but the leaf reported, in its typed work
// result, that the business goal was not achieved. It never satisfies a
// dependency and needs no transport repair.
wire_enum!(AttemptOutcome { Completed, Failed, Cancelled, Blocked });
wire_struct!(Attempt { attempt_id: String, step_id: String, input_digest: String, state: AttemptState, generation: Option<u64>, thread_id: Option<String>, turn_id: Option<String>, raw_result: Option<ArtifactRef>, outcome: Option<AttemptOutcome>, last_event_sequence: u64 });
wire_struct!(CoreDecision { research_digest: String, pass: bool, reason: String });
wire_struct!(FrozenCheck { check_id: String, program: String, args: Vec<String>, env: BTreeMap<String,String>, timeout_s: u64, snapshot: Snapshot, spec_digest: String });
wire_struct!(ExecutorIdentity { attempt_id: String, thread_id: String, model: String, purpose: Purpose, policy: ExecutionPolicy });
wire_struct!(CheckResult { check_id: String, snapshot: Snapshot, spec_digest: String, exit_code: Option<i32>, policy_applied: bool, stdout: ArtifactRef, stderr: ArtifactRef, executor: ExecutorIdentity });
wire_struct!(IndependentReview { review_id: String, snapshot: Snapshot, executor: ExecutorIdentity, check_ids: Vec<String>, pass: bool, report: ArtifactRef });
wire_struct!(ScopeReservation { attempt_id: String, paths: Vec<String> });
wire_struct!(EvidenceProjection { evidence_id: String, attempt_id: String, report: ArtifactRef, projected: bool });
wire_struct!(Checkpoint { schema_version: u32, revision: u64, run_id: String, repo: RepoIdentity, intent: ArtifactRef, research: Option<ArtifactRef>, plan: Option<ArtifactRef>, phase: Phase, status: RunStatus, core_decision: Option<CoreDecision>, steps: BTreeMap<String,StepSpec>, checks: BTreeMap<String,FrozenCheck>, attempts: BTreeMap<String,Attempt>, reservations: Vec<ScopeReservation>, pending_evidence: Vec<EvidenceProjection>, receipts: BTreeMap<String,ArtifactRef> });
wire_struct!(RunSeed { run_id: String, repo: RepoIdentity, intent: ArtifactRef });
wire_struct!(RecoveryObservation { attempt_id: String, live_in_process: bool, terminal_report: Option<ArtifactRef>, stopped_evidence: Option<ArtifactRef> });
wire_struct!(ScopeBatch { members: BTreeMap<String, String> });
wire_struct!(ScopeBatchCaller { attempt_id: String, generation: u64, thread_id: String, turn_id: String, call_id: String });
wire_struct!(ScopeBatchDispatch { run_id: String, input_digest: String, caller: ScopeBatchCaller, batch: ArtifactRef });
wire_struct!(ScopeAudit { attempt_id: String, step_id: String, generation: Option<u64>, thread_id: Option<String>, turn_id: Option<String>, scope: Vec<String>, batch: Option<ArtifactRef>, peer_attempts: Vec<String>, scratch: PathBuf, before: BTreeMap<String, String>, after: BTreeMap<String, String>, report_digest: String, outcome: AttemptOutcome });
}
#[derive(Debug, Clone)]
pub struct Mutation<T> {
    pub revision: u64,
    pub value: T,
}
wire_enum!(ClaimDisposition {
    New,
    ExistingActive,
    ExistingTerminal
});
#[derive(Debug, Clone)]
pub struct ClaimResult {
    pub revision: u64,
    pub disposition: ClaimDisposition,
    pub attempt: Attempt,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AttemptEventKind {
    Dispatching { generation: u64, thread_id: String },
    Running { turn_id: String },
    OutcomeUnknown { reason: String },
}
wire_struct!(AttemptEvent {
    event_id: String,
    attempt_id: String,
    sequence: u64,
    kind: AttemptEventKind
});
// Durable evidence of the request and thread configuration established before
// dispatch. Neither model field claims to be per-turn provider telemetry.
wire_struct!(AttemptModelReceipt {
    run_id: String,
    step_id: String,
    attempt_id: String,
    input_digest: String,
    generation: u64,
    thread_id: String,
    dispatch_event_id: String,
    dispatch_sequence: u64,
    requested_model: String,
    configured_model: String,
    requested_effort: Option<String>
});
wire_struct!(PreparedBatchRetry {
    coordinator: Attempt,
    batch: ArtifactRef,
    members: BTreeMap<String, Attempt>
});
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryAction {
    DispatchPrepared { attempt_id: String },
    JoinLocal { attempt_id: String },
    ReplayResult { attempt_id: String },
    ReadHistory { attempt_id: String },
    NeedsDecision { attempt_id: String, reason: String },
}
#[derive(Debug, Clone)]
pub struct ReconcileResult {
    pub revision: u64,
    pub actions: Vec<RecoveryAction>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryDecision {
    RetryPlanning {
        reason: String,
    },
    AcceptTerminal {
        attempt_id: String,
        report: ArtifactRef,
        reason: String,
    },
    RetryAfterRepair {
        attempt_id: String,
        stopped_evidence: ArtifactRef,
        repair_evidence: ArtifactRef,
        reason: String,
    },
}

/// Receipt for the typed baseline classification that authorizes engineering.
pub const BASELINE_GATE: &str = "gate-baseline";
/// Pointer to the diagnostic for the current blocked dispatch boundary.
pub const DISPATCH_FAILURE: &str = "dispatch-failure";

fn fail(s: impl Into<String>) -> Failure {
    Failure::new("checkpoint", s)
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn nonempty(s: &str) -> Res<()> {
    if s.trim().is_empty() {
        Err(fail("empty identifier or digest"))
    } else {
        Ok(())
    }
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn run_dir(store: &Path, id: &str) -> Res<PathBuf> {
    base::require_slug("run_id", id)?;
    Ok(store.join("runs").join(id))
}
#[cfg(test)]
thread_local! {
    // Allows a deterministic file replacement after verification to test that
    // callers consume exactly the bytes whose digest was checked.
    static AFTER_ARTIFACT_VERIFICATION: std::cell::RefCell<Option<Box<dyn FnMut(&Path)>>> =
        std::cell::RefCell::new(None);
}

fn verified_bytes(a: &ArtifactRef, path: &Path) -> Res<Vec<u8>> {
    nonempty(&a.digest)?;
    let bytes = std::fs::read(path).map_err(io)?;
    if digest(&bytes) != a.digest {
        return Err(fail("artifact digest mismatch"));
    }
    #[cfg(test)]
    AFTER_ARTIFACT_VERIFICATION.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(path);
        }
    });
    Ok(bytes)
}
fn artifact_bytes(a: &ArtifactRef) -> Res<Vec<u8>> {
    verified_bytes(a, &a.path)
}
fn artifact(a: &ArtifactRef) -> Res<()> {
    artifact_bytes(a).map(|_| ())
}
fn bounded_artifact_bytes(store: &Path, repo: &Path, value: &ArtifactRef) -> Res<Vec<u8>> {
    let path = std::fs::canonicalize(&value.path).map_err(io)?;
    let root = std::fs::canonicalize(store).map_err(io)?;
    if !path.starts_with(root) && !path.starts_with(repo) {
        return Err(fail("artifact outside repository and store"));
    }
    verified_bytes(value, &path)
}
fn bounded_artifact(store: &Path, repo: &Path, value: &ArtifactRef) -> Res<()> {
    bounded_artifact_bytes(store, repo, value).map(|_| ())
}
fn snapshot_ok(s: &Snapshot) -> Res<()> {
    nonempty(&s.source_digest)?;
    nonempty(&s.plan_digest)?;
    nonempty(&s.checks_digest)
}

/// Old `tree:` snapshots did not cover ignored inputs. They remain historical
/// evidence, but cannot authorize any work or acceptance under the v2 policy.
pub(crate) fn current_source_digest(source: &str) -> bool {
    let Some(rest) = source.strip_prefix("tree-v2:") else {
        return false;
    };
    let Some((tree, manifest)) = rest.split_once(':') else {
        return false;
    };
    let lowercase_hex = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    matches!(tree.len(), 40 | 64)
        && manifest.len() == 64
        && lowercase_hex(tree)
        && lowercase_hex(manifest)
}

fn legacy_snapshot() -> Failure {
    Failure::new(
        "legacy_snapshot",
        "the frozen source evidence does not cover ignored files; the historical run and receipts remain unchanged. Start a new run to establish a complete baseline",
    )
}

/// Check only the recorded source format, not the current filesystem; suitable
/// for exposing a historical status without changing the checkpoint.
pub(crate) fn current_frozen_sources(c: &Checkpoint) -> Res<()> {
    if c.checks.is_empty() {
        return if c.plan.is_some() {
            Err(fail("installed plan contains no frozen checks"))
        } else {
            Ok(())
        };
    }
    for (id, check) in &c.checks {
        if !current_source_digest(&check.snapshot.source_digest) {
            return Err(legacy_snapshot());
        }
        // Verification may have frozen a newer source, so the baseline's source
        // is independently retained and may legitimately differ from it.
        if let Some(reference) = c.receipts.get(&format!("baseline-{id}")) {
            let store = check_evidence_store(c, reference)
                .ok_or_else(|| fail("baseline receipt is outside its run evidence"))?;
            let result: CheckResult = serde_json::from_slice(&bounded_artifact_bytes(
                &store,
                &c.repo.canonical_root,
                reference,
            )?)
            .map_err(io)?;
            if !current_source_digest(&result.snapshot.source_digest) {
                return Err(legacy_snapshot());
            }
        }
    }
    Ok(())
}
/// Digest the executable specification independently of its enclosing snapshot.
pub fn check_spec_digest(check: &FrozenCheck) -> Res<String> {
    let bytes = serde_json::to_vec(&(
        &check.check_id,
        &check.program,
        &check.args,
        &check.env,
        check.timeout_s,
    ))
    .map_err(io)?;
    Ok(digest(&bytes))
}
/// Snapshot the exact worktree contents, including staged and previously dirty files.
pub fn frozen_snapshot(repo: &Path, plan: &ArtifactRef, checks: &[FrozenCheck]) -> Res<Snapshot> {
    artifact(plan)?;
    let source = super::snapshot(repo, &[".".into()]);
    if !current_source_digest(&source) {
        return Err(fail("cannot create a complete v2 source snapshot"));
    }
    let mut specs = checks
        .iter()
        .map(|c| Ok((c.check_id.clone(), check_spec_digest(c)?)))
        .collect::<Res<Vec<_>>>()?;
    specs.sort();
    Ok(Snapshot {
        source_digest: source,
        plan_digest: plan.digest.clone(),
        checks_digest: digest(&serde_json::to_vec(&specs).map_err(io)?),
    })
}
fn save(store: &Path, c: &Checkpoint) -> Res<()> {
    let p = run_dir(store, &c.run_id)?.join("checkpoint.json");
    base::atomic_write(&p, &serde_json::to_vec_pretty(c).map_err(io)?).map_err(io)
}
pub fn load_run(store: &Path, run_id: &str) -> Res<Checkpoint> {
    let bytes = std::fs::read(run_dir(store, run_id)?.join("checkpoint.json")).map_err(io)?;
    let c: Checkpoint = serde_json::from_slice(&bytes).map_err(io)?;
    if c.schema_version != 1 || c.run_id != run_id {
        return Err(fail("invalid checkpoint identity or schema"));
    }
    Ok(c)
}
fn mutate<T>(
    store: &Path,
    run_id: &str,
    revision: u64,
    f: impl FnOnce(&mut Checkpoint) -> Res<T>,
) -> Res<Mutation<T>> {
    super::with_lock(store, false, || {
        let mut c = load_run(store, run_id)?;
        if c.revision != revision {
            return Err(Failure::new(
                "revision_conflict",
                "checkpoint revision changed",
            ));
        }
        let value = f(&mut c)?;
        c.revision = c
            .revision
            .checked_add(1)
            .ok_or_else(|| fail("revision overflow"))?;
        save(store, &c)?;
        Ok(Mutation {
            revision: c.revision,
            value,
        })
    })
}
pub fn create_run(store: &Path, seed: &RunSeed) -> Res<Checkpoint> {
    let dir = run_dir(store, &seed.run_id)?;
    nonempty(&seed.repo.identity_digest)?;
    artifact(&seed.intent)?;
    let root = std::fs::canonicalize(&seed.repo.canonical_root).map_err(io)?;
    if root != seed.repo.canonical_root || !root.is_dir() {
        return Err(fail("repository must be canonical"));
    }
    std::fs::create_dir_all(store).map_err(io)?;
    super::with_lock(store, false, || {
        if dir.join("checkpoint.json").exists() {
            let c = load_run(store, &seed.run_id)?;
            if c.repo != seed.repo || c.intent != seed.intent {
                return Err(fail("run id reused with different input"));
            }
            return Ok(c);
        }
        std::fs::create_dir_all(&dir).map_err(io)?;
        let c = Checkpoint {
            schema_version: 1,
            revision: 0,
            run_id: seed.run_id.clone(),
            repo: seed.repo.clone(),
            intent: seed.intent.clone(),
            research: None,
            plan: None,
            phase: Phase::Research,
            status: RunStatus::Running,
            core_decision: None,
            steps: BTreeMap::new(),
            checks: BTreeMap::new(),
            attempts: BTreeMap::new(),
            reservations: vec![],
            pending_evidence: vec![],
            receipts: BTreeMap::new(),
        };
        save(store, &c)?;
        Ok(c)
    })
}
pub fn record_core_decision(store: &Path, id: &str, rev: u64, d: &CoreDecision) -> Res<u64> {
    nonempty(&d.reason)?;
    nonempty(&d.research_digest)?;
    Ok(mutate(store, id, rev, |c| {
        if c.research.as_ref().map(|r| &r.digest) != Some(&d.research_digest) {
            return Err(fail("research digest mismatch"));
        }
        if !d.pass {
            if c.phase != Phase::IntentReview
                || c.plan.is_some()
                || c.attempts
                    .values()
                    .any(|a| a.state != AttemptState::Terminal)
            {
                return Err(fail("research revision requires an idle intent review"));
            }
            let revision = c
                .receipts
                .keys()
                .filter(|k| k.starts_with("intent-rejection-"))
                .count()
                + 1;
            let rejected = save_evidence(store, id, "intent", &format!("rejection-{revision}"), d)?;
            c.receipts
                .insert(format!("intent-rejection-{revision}"), rejected);
            c.receipts.insert(
                format!("rejected-research-{revision}"),
                c.research.take().unwrap(),
            );
            c.receipts.remove("research");
            c.core_decision = None;
            c.phase = Phase::Research;
            c.status = RunStatus::Running;
            return Ok(());
        }
        if let Some(old) = &c.core_decision {
            if old != d {
                return Err(fail("core decision is immutable; start a new research run"));
            }
        }
        c.core_decision = Some(d.clone());
        c.status = if d.pass {
            RunStatus::Running
        } else {
            RunStatus::Blocked
        };
        Ok(())
    })?
    .revision)
}
pub fn record_dispatch_failure(store: &Path, id: &str, rev: u64, unknown: bool) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        c.status = if unknown {
            RunStatus::OutcomeUnknown
        } else {
            RunStatus::Blocked
        };
        Ok(())
    })?
    .revision)
}

/// Persist the failure and its status in one checkpoint mutation. The caller
/// has already created an immutable artifact describing the next revision.
pub fn record_dispatch_failure_with_receipt(
    store: &Path,
    id: &str,
    rev: u64,
    unknown: bool,
    receipt: &ArtifactRef,
) -> Res<u64> {
    artifact(receipt)?;
    Ok(mutate(store, id, rev, |c| {
        bounded_artifact(store, &c.repo.canonical_root, receipt)?;
        let run_artifacts = run_dir(store, id)?
            .join("artifacts")
            .canonicalize()
            .map_err(io)?;
        if !receipt
            .path
            .canonicalize()
            .map_err(io)?
            .starts_with(run_artifacts)
        {
            return Err(fail("dispatch failure artifact outside its run"));
        }
        c.status = if unknown {
            RunStatus::OutcomeUnknown
        } else {
            RunStatus::Blocked
        };
        c.receipts.insert(DISPATCH_FAILURE.into(), receipt.clone());
        Ok(())
    })?
    .revision)
}

#[derive(Default)]
pub(crate) struct DispatchBoundaryAuthority {
    pub phase: Option<Phase>,
    pub original_attempts: BTreeMap<String, Attempt>,
    pub claimed_prepared: BTreeSet<String>,
    pub executions: BTreeMap<String, Attempt>,
}

fn same_execution(left: &Attempt, right: &Attempt) -> bool {
    left.attempt_id == right.attempt_id
        && left.step_id == right.step_id
        && left.input_digest == right.input_digest
        && left.generation == right.generation
        && left.thread_id == right.thread_id
        && left.turn_id == right.turn_id
}

/// Resolve and persist a dispatch boundary from the latest checkpoint while
/// holding the store lock. Only this pipeline's bound executions may acquire
/// synthetic unknown events; newer foreign work is never relabelled.
pub(crate) fn finish_dispatch_boundary(
    store: &Path,
    id: &str,
    observed: &Checkpoint,
    failure: Option<Failure>,
    authority: &DispatchBoundaryAuthority,
) -> Res<(Checkpoint, Option<Failure>)> {
    if observed.run_id != id {
        return Err(fail("invalid dispatch boundary identity"));
    }
    super::with_lock(store, false, || {
        let mut c = load_run(store, id)?;
        let owned_pending: Vec<_> = c
            .attempts
            .values()
            .filter(|a| {
                matches!(a.state, AttemptState::Dispatching | AttemptState::Running)
                    && authority
                        .executions
                        .get(&a.attempt_id)
                        .is_some_and(|owner| same_execution(owner, a))
            })
            .map(|a| a.attempt_id.clone())
            .collect();
        let foreign_progress = c.attempts.values().any(|a| {
            !authority
                .executions
                .get(&a.attempt_id)
                .is_some_and(|owner| same_execution(owner, a))
                && !(a.state == AttemptState::Prepared
                    && authority.claimed_prepared.contains(&a.attempt_id))
                && authority.original_attempts.get(&a.attempt_id) != Some(a)
        });
        let foreign_live = c.attempts.values().any(|a| {
            matches!(a.state, AttemptState::Dispatching | AttemptState::Running)
                && !authority
                    .executions
                    .get(&a.attempt_id)
                    .is_some_and(|owner| same_execution(owner, a))
        });
        let phase_changed = authority
            .phase
            .as_ref()
            .is_some_and(|phase| *phase != c.phase);
        let changed = c.revision != observed.revision || phase_changed || foreign_progress;
        let owned_failed = c.attempts.values().any(|a| {
            failed_attempt_needs_repair(&c, a)
                && authority
                    .executions
                    .get(&a.attempt_id)
                    .is_some_and(|owner| same_execution(owner, a))
        });
        // A different execution or a later phase has advanced without any
        // still-running execution owned by us. Do not publish an obsolete
        // failure or change the newer pipeline's status.
        if owned_pending.is_empty()
            && (foreign_live || (changed && !(owned_failed && !foreign_progress && !phase_changed)))
        {
            return Ok((c, None));
        }
        let active = c
            .attempts
            .values()
            .any(|a| !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared));
        let status = if active {
            RunStatus::OutcomeUnknown
        } else if !matches!(
            c.phase,
            Phase::IntentReview | Phase::ReadyToCommit | Phase::Released | Phase::Cancelled
        ) {
            RunStatus::Blocked
        } else {
            c.status.clone()
        };
        if !matches!(status, RunStatus::Blocked | RunStatus::OutcomeUnknown) {
            return Ok((c, None));
        }

        let next_revision = c
            .revision
            .checked_add(1)
            .ok_or_else(|| fail("revision overflow"))?;
        // Closing the dispatcher also closes its callback lease. Record an
        // explicit unknown event for every still-dispatched attempt in this
        // same checkpoint commit, so a late callback cannot leave it stuck in
        // Running and the normal stop/repair path remains available.
        let pending = c
            .attempts
            .values()
            .filter(|a| owned_pending.contains(&a.attempt_id))
            .map(|a| {
                let sequence = a
                    .last_event_sequence
                    .checked_add(1)
                    .ok_or_else(|| fail("attempt event sequence overflow"))?;
                Ok(AttemptEvent {
                    event_id: format!("boundary-{}-{next_revision}", a.attempt_id),
                    attempt_id: a.attempt_id.clone(),
                    sequence,
                    kind: AttemptEventKind::OutcomeUnknown {
                        reason: "dispatcher boundary closed before terminal evidence was recorded"
                            .into(),
                    },
                })
            })
            .collect::<Res<Vec<_>>>()?;
        for event in &pending {
            apply_attempt_event(store, id, &mut c, event)?;
        }
        let no_longer_unknown = !active
            && failure
                .as_ref()
                .is_some_and(|error| error.kind == "outcome_unknown");
        let failure = if changed || no_longer_unknown {
            if active {
                let pending = c
                    .attempts
                    .values()
                    .filter(|a| !matches!(a.state, AttemptState::Terminal | AttemptState::Prepared))
                    .map(|a| {
                        format!(
                            "step {} (attempt {}, thread {}, turn {}, status {:?})",
                            a.step_id,
                            a.attempt_id,
                            a.thread_id.as_deref().unwrap_or("unknown"),
                            a.turn_id.as_deref().unwrap_or("unknown"),
                            a.state
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Failure::new(
                    "outcome_unknown",
                    format!(
                        "checkpoint advanced before the dispatch boundary; unresolved: {pending}."
                    ),
                )
            } else if c
                .attempts
                .values()
                .any(|a| failed_attempt_needs_repair(&c, a))
            {
                let failed = c
                    .attempts
                    .values()
                    .filter(|a| failed_attempt_needs_repair(&c, a))
                    .map(|a| format!("step {} (attempt {})", a.step_id, a.attempt_id))
                    .collect::<Vec<_>>()
                    .join(", ");
                Failure::new(
                    "repair_required",
                    format!(
                        "saved failed execution requires repair before a new attempt: {failed}"
                    ),
                )
            } else {
                Failure::new("unknown", "Checkpoint state changed before the dispatch boundary; inspect the current reports and receipts.")
            }
        } else {
            failure.unwrap_or_else(|| {
                Failure::new(
                    "incomplete_run",
                    "The run stopped before an authority boundary; inspect its saved evidence.",
                )
            })
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "revision":next_revision,"phase":c.phase,"status":status,"error":failure
        }))
        .map_err(io)?;
        let path = run_dir(store, id)?
            .join("artifacts")
            .join(format!("{}.json", digest(&bytes)));
        let receipt = immutable(&path, &bytes)?;
        c.status = status;
        c.receipts.insert(DISPATCH_FAILURE.into(), receipt);
        c.revision = next_revision;
        save(store, &c)?;
        Ok((c, Some(failure)))
    })
}
pub fn install_plan(
    store: &Path,
    id: &str,
    rev: u64,
    plan: &ArtifactRef,
    steps: &[StepSpec],
    checks: &[FrozenCheck],
) -> Res<u64> {
    artifact(plan)?;
    Ok(mutate(store, id, rev, |c| {
        bounded_artifact(store, &c.repo.canonical_root, plan)?;
        if !c.core_decision.as_ref().is_some_and(|x| {
            x.pass
                && c.research
                    .as_ref()
                    .is_some_and(|r| r.digest == x.research_digest)
        }) {
            return Err(fail("missing approved research"));
        }
        if c.attempts
            .values()
            .any(|a| a.state != AttemptState::Terminal && c.steps.get(&a.step_id).is_none_or(|s| !matches!(s.purpose, Purpose::Dispatcher | Purpose::Research | Purpose::Planner)))
            || !c.reservations.is_empty()
        {
            return Err(fail(
                "cannot replace plan while attempts are active or unknown",
            ));
        }
        let mut ss = BTreeMap::new();
        for s in steps {
            nonempty(&s.step_id)?;
            base::require_slug("step_id", &s.step_id)?;
            if s.run_id != id || ss.insert(s.step_id.clone(), s.clone()).is_some() {
                return Err(fail("invalid or duplicate step"));
            }
            for scope in &s.scope {
                let path = Path::new(scope);
                if scope.trim().is_empty()
                    || path.is_absolute()
                    || path.components().any(|part| {
                        matches!(
                            part,
                            std::path::Component::ParentDir | std::path::Component::Prefix(_)
                        )
                    })
                {
                    return Err(fail("invalid step scope"));
                }
                let mut existing = c.repo.canonical_root.join(path);
                while !existing.exists() {
                    if !existing.pop() {
                        return Err(fail("invalid step scope"));
                    }
                }
                if !std::fs::canonicalize(existing)
                    .map_err(io)?
                    .starts_with(&c.repo.canonical_root)
                {
                    return Err(fail("step scope escapes repository"));
                }
            }
            bounded_artifact(store, &c.repo.canonical_root, &s.brief_ref)?;
        }
        let mut cc = BTreeMap::new();
        let expected_snapshot = if checks.is_empty() {
            None
        } else {
            Some(frozen_snapshot(&c.repo.canonical_root, plan, checks)?)
        };
        for x in checks {
            nonempty(&x.check_id)?;
            nonempty(&x.program)?;
            nonempty(&x.spec_digest)?;
            snapshot_ok(&x.snapshot)?;
            if x.timeout_s == 0
                || x.spec_digest != check_spec_digest(x)?
                || Some(&x.snapshot) != expected_snapshot.as_ref()
                || cc.insert(x.check_id.clone(), x.clone()).is_some()
            {
                return Err(fail("invalid check"));
            }
        }
        validate_runner_checks(store, c, plan, &cc)?;
        for s in ss.values() {
            if s.dependency_ids
                .iter()
                .any(|d| !ss.contains_key(d) || d == &s.step_id)
                || s.check_ids.iter().any(|x| !cc.contains_key(x))
            {
                return Err(fail("invalid plan references"));
            }
        }
        if c.plan.is_some() && (c.plan.as_ref() != Some(plan) || ss.iter().any(|(id, s)| c.steps.get(id) != Some(s))) {
            return Err(fail("historical attempts require the original plan and steps; start a new run for a changed plan"));
        }
        c.plan = Some(plan.clone());
        for (id, s) in &c.steps {
            if !ss.contains_key(id) && (matches!(s.purpose, Purpose::Dispatcher | Purpose::Research | Purpose::Planner) || s.step_id.starts_with("evaluation-prep-")) {
                ss.insert(id.clone(), s.clone());
            }
        }
        c.steps = ss;
        c.checks = cc;
        c.receipts.retain(|k, _| {
            !k.starts_with("check-")
                && !k.starts_with("review-")
                && k != "assembly"
                && k != "memory"
                && k != "release"
        });
        Ok(())
    })?
    .revision)
}

pub(crate) fn validate_runner_checks(
    store: &Path,
    c: &Checkpoint,
    plan: &ArtifactRef,
    checks: &BTreeMap<String, FrozenCheck>,
) -> Res<()> {
    use super::runner::{self, RunnerStatus};
    let epoch = planning_epoch(c);
    let has_claim = c.receipts.contains_key(&runner_claim_key(epoch));
    let mut special = Vec::new();
    for check in checks.values() {
        if let Some(sentinel) = runner::parse_sentinel(&check.program)? {
            special.push((check, sentinel));
        }
    }
    if !has_claim && special.is_empty() {
        return Ok(());
    }
    if !has_claim || special.is_empty() {
        return Err(Failure::new(
            "evaluation_unprepared",
            "prepared runner claim and private frozen check must both exist",
        ));
    }
    let evidence = runner_evidence(store, c, epoch, false)?;
    if evidence.receipt.status != RunnerStatus::Ready {
        return Err(Failure::new(
            "evaluation_unprepared",
            "runner preparation did not compile and link successfully",
        ));
    }
    let report_ref = c
        .receipts
        .get(&format!("evaluation-{epoch}"))
        .ok_or_else(|| fail("independent runner preparation report missing"))?;
    let attempt = c
        .attempts
        .get(&evidence.receipt.identity.attempt_id)
        .ok_or_else(|| fail("runner attempt missing"))?;
    if attempt.raw_result.as_ref() != Some(report_ref) {
        return Err(fail("runner preparation report identity mismatch"));
    }
    let report: Value = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        report_ref,
    )?)
    .map_err(io)?;
    let output = if report["structured_output"].is_null() {
        serde_json::from_str::<Value>(
            report["final_message"]
                .as_str()
                .ok_or_else(|| fail("runner preparation output missing"))?,
        )
        .map_err(io)?
    } else {
        report["structured_output"].clone()
    };
    if output["status"] != "ready" {
        return Err(fail("runner preparation was not ready"));
    }
    let prepared = output["checks"]
        .as_array()
        .ok_or_else(|| fail("runner preparation checks missing"))?;
    let public_plan: Value = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        plan,
    )?)
    .map_err(io)?;
    let public = public_plan["checks"]
        .as_array()
        .ok_or_else(|| fail("public check list missing"))?;
    for (check, sentinel) in special {
        if !check.check_id.starts_with("independent-")
            || public.iter().any(|entry| entry["id"] == check.check_id)
            || !check.args.iter().any(|arg| arg.contains("{repo}"))
            || sentinel.source_digest != evidence.receipt.source.digest
            || evidence
                .receipt
                .binary
                .as_ref()
                .is_none_or(|binary| sentinel.binary_digest != binary.digest)
        {
            return Err(Failure::new(
                "evaluation_unprepared",
                "prepared runner may only be used by its private frozen check",
            ));
        }
        let matching: Vec<_> = prepared
            .iter()
            .filter(|entry| entry["id"] == check.check_id)
            .collect();
        if matching.len() != 1 {
            return Err(fail("runner check is absent from its preparation report"));
        }
        let entry = matching[0];
        let args: Vec<String> = serde_json::from_value(entry["args"].clone()).map_err(io)?;
        let mut env = BTreeMap::new();
        for item in entry["env"]
            .as_array()
            .ok_or_else(|| fail("runner check environment missing"))?
        {
            let name = item["name"]
                .as_str()
                .ok_or_else(|| fail("runner check environment name missing"))?;
            let value = item["value"]
                .as_str()
                .ok_or_else(|| fail("runner check environment value missing"))?;
            if env.insert(name.to_owned(), value.to_owned()).is_some() {
                return Err(fail("duplicate runner environment"));
            }
        }
        if entry["program"].as_str() != Some(check.program.as_str())
            || args != check.args
            || env != check.env
            || entry["timeout_s"].as_u64() != Some(check.timeout_s)
        {
            return Err(fail(
                "frozen runner check differs from its preparation report",
            ));
        }
    }
    Ok(())
}
fn paths_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| {
        b.iter().any(|y| {
            x == "."
                || y == "."
                || x == y
                || x.starts_with(&format!("{}/", y.trim_end_matches('/')))
                || y.starts_with(&format!("{}/", x.trim_end_matches('/')))
        })
    })
}
pub fn claim_step(store: &Path, rev: u64, spec: &StepSpec, input_digest: &str) -> Res<ClaimResult> {
    nonempty(input_digest)?;
    let m = mutate(store, &spec.run_id, rev, |c| {
        if matches!(
            spec.purpose,
            Purpose::Engineer | Purpose::Assembler | Purpose::Content
        ) && (!approved_research(c) || c.plan.is_none())
        {
            return Err(fail(
                "engineering requires approved research and an installed plan",
            ));
        }
        if matches!(
            spec.purpose,
            Purpose::Engineer | Purpose::Assembler | Purpose::Content
        ) {
            let phase = if spec.purpose == Purpose::Assembler {
                Phase::Assembling
            } else {
                Phase::Engineering
            };
            if c.phase != phase {
                return Err(fail("work claim is outside its authorized phase"));
            }
            baseline_authorized(c)?;
            if matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler)
                && c.attempts
                    .values()
                    .any(|a| a.step_id == spec.step_id && failed_attempt_needs_repair(c, a))
            {
                return Err(fail(
                    "writer requires explicit stop and repair evidence before retry",
                ));
            }
            if spec.purpose == Purpose::Assembler
                && !c
                    .steps
                    .values()
                    .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Content))
                    .all(|s| {
                        c.attempts
                            .values()
                            .any(|a| a.step_id == s.step_id && completed_attempt(c, a))
                    })
            {
                return Err(fail("assembly requires audited engineering results"));
            }
        } else if spec.purpose == Purpose::Verifier && c.plan.is_some() {
            current_frozen_sources(c)?;
            if spec.step_id == "baseline" {
                if c.phase != Phase::Planning {
                    return Err(fail("baseline verifier is outside planning"));
                }
            } else {
                baseline_authorized(c)?;
                if c.phase != Phase::Verifying || spec.step_id != verification_step_id(c) {
                    return Err(fail("verifier is outside the current verification"));
                }
            }
        }
        if !c.steps.contains_key(&spec.step_id) {
            let preplan = spec.scope.is_empty()
                && spec.dependency_ids.is_empty()
                && spec.check_ids.is_empty()
                && match spec.purpose {
                    Purpose::Dispatcher => matches!(
                        c.phase,
                        Phase::Research | Phase::Planning | Phase::Engineering
                    ),
                    Purpose::Research => c.phase == Phase::Research && c.plan.is_none(),
                    Purpose::Planner => {
                        c.phase == Phase::Planning && approved_research(c) && c.plan.is_none()
                    }
                    Purpose::Verifier => {
                        c.phase == Phase::Planning
                            && approved_research(c)
                            && c.plan.is_none()
                            && spec.step_id.starts_with("evaluation-prep-")
                    }
                    _ => false,
                };
            if !preplan {
                return Err(fail(
                    "unplanned step is not a fixed-purpose read-only transition",
                ));
            }
            base::require_slug("step_id", &spec.step_id)?;
            bounded_artifact(store, &c.repo.canonical_root, &spec.brief_ref)?;
            c.steps.insert(spec.step_id.clone(), spec.clone());
        }
        if c.steps.get(&spec.step_id) != Some(spec) {
            return Err(fail("step differs from frozen plan"));
        }
        if let Some(a) = c.attempts.values().find(|a| {
            a.step_id == spec.step_id
                && a.input_digest == input_digest
                && (a.state != AttemptState::Terminal
                    || a.outcome == Some(AttemptOutcome::Completed))
        }) {
            if a.state == AttemptState::Terminal && !completed_attempt(c, a) {
                return Err(fail("completed writer lacks its original scope audit"));
            }
            return Ok((
                if a.state == AttemptState::Terminal {
                    ClaimDisposition::ExistingTerminal
                } else {
                    ClaimDisposition::ExistingActive
                },
                a.clone(),
            ));
        }
        if spec.purpose == Purpose::Verifier
            && c.plan.is_some()
            && c.attempts.values().any(|a| {
                a.step_id == spec.step_id
                    && a.state == AttemptState::Terminal
                    && a.outcome == Some(AttemptOutcome::Completed)
            })
        {
            return Err(fail(
                "completed verifier cannot be replaced by a different input",
            ));
        }
        if c.attempts
            .values()
            .any(|a| a.step_id == spec.step_id && a.state != AttemptState::Terminal)
        {
            return Err(fail("active attempt has different input"));
        }
        for dep in &spec.dependency_ids {
            if !c
                .attempts
                .values()
                .any(|a| &a.step_id == dep && completed_attempt(c, a))
            {
                return Err(fail("dependency lacks completed evidence"));
            }
        }
        if c.reservations
            .iter()
            .any(|r| paths_overlap(&r.paths, &spec.scope))
        {
            return Err(Failure::new("busy", "scope reserved"));
        }
        let a = Attempt {
            attempt_id: base::unique(),
            step_id: spec.step_id.clone(),
            input_digest: input_digest.into(),
            state: AttemptState::Prepared,
            generation: None,
            thread_id: None,
            turn_id: None,
            raw_result: None,
            outcome: None,
            last_event_sequence: 0,
        };
        if matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler) {
            c.reservations.push(ScopeReservation {
                attempt_id: a.attempt_id.clone(),
                paths: spec.scope.clone(),
            });
        }
        c.attempts.insert(a.attempt_id.clone(), a.clone());
        Ok((ClaimDisposition::New, a))
    })?;
    Ok(ClaimResult {
        revision: m.revision,
        disposition: m.value.0,
        attempt: m.value.1,
    })
}
pub fn record_attempt_event(store: &Path, id: &str, rev: u64, event: &AttemptEvent) -> Res<u64> {
    base::require_slug("event_id", &event.event_id)?;
    Ok(mutate(store, id, rev, |c| apply_attempt_event(store, id, c, event))?.revision)
}
fn apply_attempt_event(
    store: &Path,
    id: &str,
    c: &mut Checkpoint,
    event: &AttemptEvent,
) -> Res<()> {
    let key = format!("event-{}", event.event_id);
    if let Some(old) = c.receipts.get(&key) {
        if artifact_bytes(old)? == serde_json::to_vec(event).map_err(io)? {
            return Ok(());
        }
        return Err(fail("event id reused with different content"));
    }
    let a = c
        .attempts
        .get_mut(&event.attempt_id)
        .ok_or_else(|| fail("unknown attempt"))?;
    if event.sequence <= a.last_event_sequence {
        return Err(fail("stale event"));
    }
    match &event.kind {
        AttemptEventKind::Dispatching {
            generation,
            thread_id,
        } if a.state == AttemptState::Prepared => {
            nonempty(thread_id)?;
            a.state = AttemptState::Dispatching;
            a.generation = Some(*generation);
            a.thread_id = Some(thread_id.clone());
        }
        AttemptEventKind::Running { turn_id } if a.state == AttemptState::Dispatching => {
            nonempty(turn_id)?;
            a.state = AttemptState::Running;
            a.turn_id = Some(turn_id.clone());
        }
        AttemptEventKind::OutcomeUnknown { reason }
            if matches!(a.state, AttemptState::Dispatching | AttemptState::Running) =>
        {
            nonempty(reason)?;
            a.state = AttemptState::OutcomeUnknown;
            c.status = RunStatus::OutcomeUnknown;
        }
        _ => return Err(fail("invalid attempt transition")),
    }
    a.last_event_sequence = event.sequence;
    let content = digest(&serde_json::to_vec(event).map_err(io)?);
    let reference = save_evidence(store, id, "event", &content, event)?;
    c.receipts.insert(key, reference);
    Ok(())
}

/// Save dispatch identity and its model evidence in the same checkpoint write.
/// If any evidence or checkpoint write fails, the observer must not start a turn.
pub fn record_attempt_dispatch(
    store: &Path,
    id: &str,
    rev: u64,
    event: &AttemptEvent,
    requested_model: &str,
    configured_model: &str,
    requested_effort: Option<&str>,
) -> Res<u64> {
    base::require_slug("event_id", &event.event_id)?;
    nonempty(requested_model)?;
    if configured_model != requested_model || requested_effort.is_some_and(|s| s.trim().is_empty())
    {
        return Err(Failure::new(
            "model_identity",
            "invalid requested/configured model evidence",
        ));
    }
    let AttemptEventKind::Dispatching {
        generation,
        thread_id,
    } = &event.kind
    else {
        return Err(fail("model receipt requires a dispatching event"));
    };
    Ok(mutate(store, id, rev, |c| {
        let attempt = c
            .attempts
            .get(&event.attempt_id)
            .ok_or_else(|| fail("unknown attempt"))?;
        let receipt = AttemptModelReceipt {
            run_id: id.into(),
            step_id: attempt.step_id.clone(),
            attempt_id: attempt.attempt_id.clone(),
            input_digest: attempt.input_digest.clone(),
            generation: *generation,
            thread_id: thread_id.clone(),
            dispatch_event_id: event.event_id.clone(),
            dispatch_sequence: event.sequence,
            requested_model: requested_model.into(),
            configured_model: configured_model.into(),
            requested_effort: requested_effort.map(str::to_owned),
        };
        let key = format!("attempt-model-{}", attempt.attempt_id);
        if let Some(old) = c.receipts.get(&key) {
            if model_evidence_bytes(store, c, old, &key)?
                != serde_json::to_vec(&receipt).map_err(io)?
            {
                return Err(Failure::new(
                    "model_identity",
                    "attempt model receipt conflict",
                ));
            }
        } else {
            if c.receipts
                .contains_key(&format!("event-{}", event.event_id))
            {
                return Err(Failure::new(
                    "model_identity",
                    "dispatch event lacks its atomic model receipt",
                ));
            }
            let content = digest(&serde_json::to_vec(&receipt).map_err(io)?);
            let reference = save_evidence(store, id, "attempt-model", &content, &receipt)?;
            c.receipts.insert(key, reference);
        }
        apply_attempt_event(store, id, c, event)
    })?
    .revision)
}

fn model_evidence_bytes(
    store: &Path,
    c: &Checkpoint,
    reference: &ArtifactRef,
    name: &str,
) -> Res<Vec<u8>> {
    // Older referenced receipts used the logical key as their filename. New
    // evidence uses its content digest, so an unreferenced file left by a
    // failed checkpoint write cannot pin a Prepared attempt to an old thread.
    let legacy = format!("{name}.json");
    let kind = if name.starts_with("attempt-model-") {
        "attempt-model"
    } else {
        "event"
    };
    let content_name = format!("{kind}-{}.json", reference.digest);
    let filename = reference.path.file_name().and_then(|v| v.to_str());
    if filename != Some(legacy.as_str()) && filename != Some(content_name.as_str()) {
        return Err(Failure::new(
            "model_identity",
            "model evidence filename mismatch",
        ));
    }
    let expected = run_dir(store, &c.run_id)?
        .join("evidence")
        .join(filename.unwrap());
    let expected = expected.canonicalize().map_err(io)?;
    if reference.path.canonicalize().map_err(io)? != expected
        || check_evidence_store(c, reference).is_none()
    {
        return Err(Failure::new(
            "model_identity",
            "model evidence path does not belong to its attempt",
        ));
    }
    bounded_artifact_bytes(store, &c.repo.canonical_root, reference)
}

/// Verify the persisted dispatch configuration without consulting model/list.
pub fn attempt_model(store: &Path, c: &Checkpoint, attempt_id: &str) -> Res<AttemptModelReceipt> {
    let check = || -> Res<AttemptModelReceipt> {
        let a = c
            .attempts
            .get(attempt_id)
            .ok_or_else(|| fail("unknown attempt"))?;
        let key = format!("attempt-model-{attempt_id}");
        let reference = c
            .receipts
            .get(&key)
            .ok_or_else(|| fail("attempt model receipt missing"))?;
        let receipt: AttemptModelReceipt =
            serde_json::from_slice(&model_evidence_bytes(store, c, reference, &key)?)
                .map_err(io)?;
        base::require_slug("dispatch event id", &receipt.dispatch_event_id)?;
        if receipt.run_id != c.run_id
            || receipt.step_id != a.step_id
            || receipt.attempt_id != a.attempt_id
            || receipt.input_digest != a.input_digest
            || Some(receipt.generation) != a.generation
            || Some(&receipt.thread_id) != a.thread_id.as_ref()
            || receipt.requested_model.trim().is_empty()
            || receipt.configured_model != receipt.requested_model
            || receipt.thread_id.trim().is_empty()
            || receipt
                .requested_effort
                .as_deref()
                .is_some_and(|s| s.trim().is_empty())
            || receipt.dispatch_sequence == 0
            || receipt.dispatch_sequence > a.last_event_sequence
            || c.steps.get(&a.step_id).is_none_or(|s| s.run_id != c.run_id)
        {
            return Err(fail("attempt model receipt identity mismatch"));
        }
        let event_key = format!("event-{}", receipt.dispatch_event_id);
        let event_ref = c
            .receipts
            .get(&event_key)
            .ok_or_else(|| fail("model dispatch event missing"))?;
        let event: AttemptEvent =
            serde_json::from_slice(&model_evidence_bytes(store, c, event_ref, &event_key)?)
                .map_err(io)?;
        if event.attempt_id != a.attempt_id
            || event.event_id != receipt.dispatch_event_id
            || event.sequence != receipt.dispatch_sequence
            || event.kind
                != (AttemptEventKind::Dispatching {
                    generation: receipt.generation,
                    thread_id: receipt.thread_id.clone(),
                })
        {
            return Err(fail("model dispatch event mismatch"));
        }
        Ok(receipt)
    };
    check().map_err(|e| Failure::new("model_identity", e.message))
}

/// History's nullable current thread model stays raw. Its independent dispatch
/// receipt authorizes identity; a concrete contradictory model still blocks.
pub fn validate_attempt_report(
    store: &Path,
    c: &Checkpoint,
    attempt_id: &str,
    report: &Value,
) -> Res<AttemptModelReceipt> {
    let receipt = attempt_model(store, c, attempt_id)?;
    let a = &c.attempts[attempt_id];
    let model_matches = match report.get("model") {
        Some(Value::String(model)) => model == &receipt.configured_model,
        None | Some(Value::Null) => report["model_source"] == "thread/read",
        _ => false,
    };
    if !model_matches
        || a.turn_id.as_deref().is_none_or(|s| s.trim().is_empty())
        || report["thread_id"].as_str() != a.thread_id.as_deref()
        || report["turn_id"].as_str() != a.turn_id.as_deref()
        || report
            .get("requested_model")
            .is_some_and(|v| v.as_str() != Some(receipt.requested_model.as_str()))
        || report
            .get("configured_model")
            .is_some_and(|v| v.as_str() != Some(receipt.configured_model.as_str()))
    {
        return Err(Failure::new(
            "model_identity",
            "report differs from its persisted dispatch identity",
        ));
    }
    Ok(receipt)
}

pub fn terminal_attempt_report(
    store: &Path,
    c: &Checkpoint,
    attempt_id: &str,
) -> Res<(Value, ArtifactRef, AttemptModelReceipt)> {
    let a = c
        .attempts
        .get(attempt_id)
        .ok_or_else(|| fail("unknown attempt"))?;
    if a.state != AttemptState::Terminal {
        return Err(fail("attempt is not terminal"));
    }
    let reference = a
        .raw_result
        .as_ref()
        .ok_or_else(|| fail("terminal attempt lacks saved report"))?;
    let report = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        reference,
    )?)
    .map_err(io)?;
    let receipt = validate_attempt_report(store, c, attempt_id, &report)?;
    Ok((report, reference.clone(), receipt))
}
fn immutable(path: &Path, bytes: &[u8]) -> Res<ArtifactRef> {
    let parent = path.parent().ok_or_else(|| fail("invalid evidence path"))?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let temp = parent.join(format!(".evidence-{}.tmp", base::unique()));
    let result = (|| -> Res<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(io)?;
        f.write_all(bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
        drop(f);
        match std::fs::hard_link(&temp, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(path).map_err(io)? != bytes {
                    Err(fail("immutable evidence conflict"))
                } else {
                    Ok(())
                }
            }
            Err(e) => Err(io(e)),
        }
    })();
    let _ = std::fs::remove_file(&temp);
    result?;
    Ok(ArtifactRef {
        path: path.to_path_buf(),
        digest: digest(bytes),
    })
}
fn finish_result(
    store: &Path,
    id: &str,
    c: &mut Checkpoint,
    attempt_id: &str,
    outcome: AttemptOutcome,
    bytes: &[u8],
) -> Res<ArtifactRef> {
    let report: Value = serde_json::from_slice(bytes).map_err(io)?;
    validate_attempt_report(store, c, attempt_id, &report)?;
    if c.attempts
        .get(attempt_id)
        .and_then(|a| c.steps.get(&a.step_id))
        .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))
    {
        validate_scope_audit(store, c, attempt_id, &outcome, bytes)?;
    }
    let a = c
        .attempts
        .get_mut(attempt_id)
        .ok_or_else(|| fail("unknown attempt"))?;
    if a.state == AttemptState::Prepared {
        return Err(fail("prepared attempt was not dispatched"));
    }
    let dir = run_dir(store, id)?.join("reports");
    immutable(
        &dir.join(format!("{attempt_id}.outcome.json")),
        &serde_json::to_vec(&outcome).map_err(io)?,
    )?;
    let raw = immutable(&dir.join(format!("{attempt_id}.json")), bytes)?;
    if a.state == AttemptState::Terminal {
        if a.raw_result.as_ref() != Some(&raw) || a.outcome.as_ref() != Some(&outcome) {
            return Err(fail("terminal result conflict"));
        }
        return Ok(raw);
    }
    let evidence_id = format!("attempt-{attempt_id}");
    let projection = run_dir(store, id)?
        .join("evidence")
        .join(format!("{evidence_id}.json"));
    let projected = immutable(&projection, bytes)?;
    if projected.digest != raw.digest {
        return Err(fail("projection mismatch"));
    }
    a.state = AttemptState::Terminal;
    a.raw_result = Some(raw.clone());
    a.outcome = Some(outcome);
    c.reservations.retain(|r| r.attempt_id != attempt_id);
    c.pending_evidence.push(EvidenceProjection {
        evidence_id,
        attempt_id: attempt_id.into(),
        report: raw.clone(),
        projected: true,
    });
    if c.status == RunStatus::OutcomeUnknown
        && !c
            .attempts
            .values()
            .any(|a| a.state == AttemptState::OutcomeUnknown)
    {
        c.status = RunStatus::Running;
    }
    Ok(raw)
}
pub fn commit_result(
    store: &Path,
    id: &str,
    rev: u64,
    attempt_id: &str,
    outcome: AttemptOutcome,
    report: &Value,
) -> Res<Mutation<ArtifactRef>> {
    base::require_slug("attempt_id", attempt_id)?;
    let bytes = serde_json::to_vec(report).map_err(io)?;
    mutate(store, id, rev, |c| {
        finish_result(store, id, c, attempt_id, outcome, &bytes)
    })
}
fn save_evidence<T: Serialize>(
    store: &Path,
    id: &str,
    kind: &str,
    key: &str,
    value: &T,
) -> Res<ArtifactRef> {
    base::require_slug("evidence_id", key)?;
    let bytes = serde_json::to_vec(value).map_err(io)?;
    immutable(
        &run_dir(store, id)?
            .join("evidence")
            .join(format!("{kind}-{key}.json")),
        &bytes,
    )
}

/// Receipt names are reserved so a generic receipt cannot impersonate a
/// prepared runner claim or result.
fn runner_claim_key(epoch: usize) -> String {
    format!("runner-claim-{epoch}")
}
fn runner_result_key(epoch: usize) -> String {
    format!("runner-result-{epoch}")
}

fn runner_evidence_name(epoch: usize, label: &str) -> Res<String> {
    if !matches!(
        label,
        "source" | "binary" | "stdout" | "stderr" | "probe-stdout" | "probe-stderr"
    ) {
        return Err(fail("invalid runner evidence label"));
    }
    Ok(format!("runner-{epoch}-{label}.bin"))
}

pub(crate) fn save_runner_bytes(
    store: &Path,
    id: &str,
    epoch: usize,
    label: &str,
    bytes: &[u8],
) -> Res<ArtifactRef> {
    immutable(
        &run_dir(store, id)?
            .join("evidence")
            .join(runner_evidence_name(epoch, label)?),
        bytes,
    )
}

fn runner_bytes(store: &Path, c: &Checkpoint, reference: &ArtifactRef, name: &str) -> Res<Vec<u8>> {
    let expected = run_dir(store, &c.run_id)?.join("evidence").join(name);
    let owner = check_evidence_store(c, reference)
        .ok_or_else(|| fail("runner artifact is outside its run evidence"))?;
    if owner.canonicalize().map_err(io)? != store.canonicalize().map_err(io)?
        || reference.path.canonicalize().map_err(io)? != expected.canonicalize().map_err(io)?
    {
        return Err(fail("runner artifact identity mismatch"));
    }
    verified_bytes(reference, &reference.path)
}

fn check_runner_identity(
    c: &Checkpoint,
    identity: &super::runner::RunnerIdentity,
    running: bool,
) -> Res<()> {
    identity.validate()?;
    let attempt = c
        .attempts
        .get(&identity.attempt_id)
        .ok_or_else(|| fail("runner attempt missing"))?;
    let expected_state = if running {
        AttemptState::Running
    } else {
        AttemptState::Terminal
    };
    if identity.run_id != c.run_id
        || identity.epoch != planning_epoch(c)
        || attempt.step_id != format!("evaluation-prep-{}", identity.epoch)
        || c.steps
            .get(&attempt.step_id)
            .is_none_or(|s| s.purpose != Purpose::Verifier)
        || attempt.state != expected_state
        || (!running && attempt.outcome != Some(AttemptOutcome::Completed))
        || attempt.generation != Some(identity.generation)
        || attempt.thread_id.as_deref() != Some(identity.thread_id.as_str())
        || attempt.turn_id.as_deref() != Some(identity.turn_id.as_str())
        || (running && (c.phase != Phase::Planning || c.plan.is_some()))
    {
        return Err(fail(
            "runner is not bound to the current evaluation attempt",
        ));
    }
    Ok(())
}

pub(crate) struct RunnerEvidence {
    pub claim_bytes: Vec<u8>,
    pub receipt: super::runner::RunnerReceipt,
    pub source: Vec<u8>,
    pub binary: Option<Vec<u8>>,
    pub probe_stdout: Option<Vec<u8>>,
}

pub(crate) fn runner_evidence(
    store: &Path,
    c: &Checkpoint,
    epoch: usize,
    running: bool,
) -> Res<RunnerEvidence> {
    use super::runner;
    let claim_ref = c
        .receipts
        .get(&runner_claim_key(epoch))
        .ok_or_else(|| fail("runner claim missing"))?;
    let claim_bytes = runner_bytes(store, c, claim_ref, &format!("runner-claim-{epoch}.json"))?;
    let claim: runner::RunnerClaim = serde_json::from_slice(&claim_bytes).map_err(io)?;
    check_runner_identity(c, &claim.identity, running)?;
    let source = runner_bytes(
        store,
        c,
        &claim.source,
        &runner_evidence_name(epoch, "source")?,
    )?;
    runner::validate_claim(&claim_bytes, &claim.identity, &source)?;
    let result_ref = c.receipts.get(&runner_result_key(epoch)).ok_or_else(|| {
        Failure::new(
            "outcome_unknown",
            "prepared runner claim has no terminal result; do not recompile",
        )
    })?;
    let result_bytes = runner_bytes(store, c, result_ref, &format!("runner-result-{epoch}.json"))?;
    let receipt: runner::RunnerReceipt = serde_json::from_slice(&result_bytes).map_err(io)?;
    let probe_stdout = receipt
        .probe_stdout
        .as_ref()
        .map(|r| runner_bytes(store, c, r, &runner_evidence_name(epoch, "probe-stdout")?))
        .transpose()?;
    runner::validate_receipt(
        &claim_bytes,
        &receipt,
        &claim.identity,
        probe_stdout.as_deref(),
    )?;
    for (label, r) in [("stdout", &receipt.stdout), ("stderr", &receipt.stderr)] {
        if let Some(r) = r {
            runner_bytes(store, c, r, &runner_evidence_name(epoch, label)?)?;
        }
    }
    let binary = receipt
        .binary
        .as_ref()
        .map(|r| runner_bytes(store, c, r, &runner_evidence_name(epoch, "binary")?))
        .transpose()?;
    if receipt.status == runner::RunnerStatus::Ready {
        let binary_bytes = binary
            .as_deref()
            .ok_or_else(|| fail("runner binary missing"))?;
        let sentinel = runner::RunnerSentinel {
            source_digest: receipt.source.digest.clone(),
            binary_digest: runner::sha256(binary_bytes),
        };
        runner::verify_material(
            &receipt,
            &sentinel,
            &source,
            binary_bytes,
            probe_stdout
                .as_deref()
                .ok_or_else(|| fail("runner probe missing"))?,
        )?;
    }
    Ok(RunnerEvidence {
        claim_bytes,
        receipt,
        source,
        binary,
        probe_stdout,
    })
}

pub(crate) enum RunnerClaimDisposition {
    New(Vec<u8>),
    Replayed(RunnerEvidence),
}

/// Atomically publish a single source claim for the current running prep turn.
pub(crate) fn claim_runner(
    store: &Path,
    id: &str,
    rev: u64,
    identity: &super::runner::RunnerIdentity,
    source: &[u8],
) -> Res<Mutation<RunnerClaimDisposition>> {
    use super::runner;
    mutate(store, id, rev, |c| {
        check_runner_identity(c, identity, true)?;
        let key = runner_claim_key(identity.epoch);
        if c.receipts.contains_key(&key) {
            let evidence = runner_evidence(store, c, identity.epoch, true)?;
            if evidence.receipt.identity != *identity || evidence.source != source {
                return Err(Failure::new(
                    "evaluation_unprepared",
                    "the preparation attempt already claimed a different runner",
                ));
            }
            return Ok(RunnerClaimDisposition::Replayed(evidence));
        }
        if source.is_empty()
            || source.len() > runner::MAX_RUNNER_SOURCE_BYTES
            || std::str::from_utf8(source).is_err()
        {
            return Err(Failure::new(
                "evaluation_unprepared",
                "runner source must be nonempty UTF-8 within the size limit",
            ));
        }
        let source_ref = save_runner_bytes(store, id, identity.epoch, "source", source)?;
        let claim = runner::RunnerClaim {
            version: runner::RUNNER_VERSION,
            identity: identity.clone(),
            source: source_ref,
        };
        let bytes = serde_json::to_vec(&claim).map_err(io)?;
        let reference = save_evidence(
            store,
            id,
            "runner-claim",
            &identity.epoch.to_string(),
            &claim,
        )?;
        c.receipts.insert(key, reference);
        Ok(RunnerClaimDisposition::New(bytes))
    })
}

/// The exact claim and all artifacts are re-read and verified before a terminal
/// result is committed. A crash before this point leaves an unknown claim.
pub(crate) fn complete_runner(
    store: &Path,
    id: &str,
    rev: u64,
    receipt: &super::runner::RunnerReceipt,
) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        check_runner_identity(c, &receipt.identity, true)?;
        let epoch = receipt.identity.epoch;
        let key = runner_result_key(epoch);
        if c.receipts.contains_key(&key) {
            return Err(fail("runner result already recorded"));
        }
        let saved = save_evidence(store, id, "runner-result", &epoch.to_string(), receipt)?;
        c.receipts.insert(key, saved);
        runner_evidence(store, c, epoch, true)?;
        Ok(())
    })?
    .revision)
}

pub(crate) fn relative_change(repo: &Path, name: &str) -> Option<String> {
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
pub(crate) fn scratch_change(scratch: &Path, name: &str) -> bool {
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

// The batch is registered before any member is dispatched. Its immutable
// members bind peer scope allowances to the actual scheduled attempts.
pub(crate) fn register_scope_batch(
    store: &Path,
    id: &str,
    rev: u64,
    members: &[(String, String)],
    caller: &ScopeBatchCaller,
) -> Res<Mutation<ArtifactRef>> {
    mutate(store, id, rev, |c| {
        if c.phase != Phase::Engineering || members.is_empty() {
            return Err(fail("invalid scope audit batch"));
        }
        let coordinator = c
            .attempts
            .get(&caller.attempt_id)
            .ok_or_else(|| fail("scope batch caller missing"))?;
        if coordinator.state != AttemptState::Running
            || c.steps
                .get(&coordinator.step_id)
                .is_none_or(|s| s.purpose != Purpose::Dispatcher)
            || coordinator.generation != Some(caller.generation)
            || coordinator.thread_id.as_deref() != Some(caller.thread_id.as_str())
            || coordinator.turn_id.as_deref() != Some(caller.turn_id.as_str())
            || caller.turn_id.trim().is_empty()
            || caller.call_id.trim().is_empty()
        {
            return Err(fail(
                "scope batch is not a callback of the running dispatcher",
            ));
        }
        attempt_model(store, c, &caller.attempt_id)?;
        let input_digest = coordinator.input_digest.clone();
        let mut batch = ScopeBatch {
            members: BTreeMap::new(),
        };
        let mut attempts = BTreeSet::new();
        for (step_id, attempt_id) in members {
            let step = c
                .steps
                .get(step_id)
                .ok_or_else(|| fail("batch step missing"))?;
            let attempt = c
                .attempts
                .get(attempt_id)
                .ok_or_else(|| fail("batch attempt missing"))?;
            if !matches!(step.purpose, Purpose::Engineer | Purpose::Content)
                || attempt.step_id != *step_id
                || attempt.state != AttemptState::Prepared
                || !attempts.insert(attempt_id.clone())
                || batch
                    .members
                    .insert(step_id.clone(), attempt_id.clone())
                    .is_some()
            {
                return Err(fail("scope batch must contain distinct prepared attempts"));
            }
        }
        let key = base::unique();
        let receipt = save_evidence(store, id, "scope-batch", &key, &batch)?;
        c.receipts
            .insert(format!("scope-batch-{key}"), receipt.clone());
        // ScopeBatch's old shape remains valid for scope audits. Only this
        // additional receipt can prove which actual callback registered it.
        let dispatch = ScopeBatchDispatch {
            run_id: id.into(),
            input_digest,
            caller: caller.clone(),
            batch: receipt.clone(),
        };
        let binding = save_evidence(store, id, "scope-dispatch", &key, &dispatch)?;
        c.receipts.insert(format!("scope-dispatch-{key}"), binding);
        Ok(receipt)
    })
}

fn audit_scope_outside(
    store: &Path,
    c: &Checkpoint,
    audit: &ScopeAudit,
    report: &Value,
) -> Res<bool> {
    let attempt = c
        .attempts
        .get(&audit.attempt_id)
        .ok_or_else(|| fail("scope audit attempt missing"))?;
    let step = c
        .steps
        .get(&attempt.step_id)
        .ok_or_else(|| fail("scope audit step missing"))?;
    if !matches!(step.purpose, Purpose::Engineer | Purpose::Assembler)
        || audit.step_id != attempt.step_id
        || audit.scope != step.scope
        || audit.generation != attempt.generation
        || audit.thread_id != attempt.thread_id
        || audit.turn_id != attempt.turn_id
        || attempt.generation.is_none()
        || attempt.thread_id.as_deref().is_none_or(str::is_empty)
        || attempt.turn_id.as_deref().is_none_or(str::is_empty)
        || report["thread_id"].as_str() != attempt.thread_id.as_deref()
        || report["turn_id"].as_str() != attempt.turn_id.as_deref()
    {
        return Err(fail("scope audit attempt identity mismatch"));
    }
    let root = store.canonicalize().map_err(io)?;
    let scratch = audit.scratch.canonicalize().map_err(io)?;
    if !scratch.starts_with(&root) {
        return Err(fail("scope audit scratch outside store"));
    }
    let mut peers = Vec::new();
    if step.purpose == Purpose::Engineer {
        let batch_ref = audit
            .batch
            .as_ref()
            .ok_or_else(|| fail("engineer scope batch missing"))?;
        if !c
            .receipts
            .iter()
            .any(|(key, value)| key.starts_with("scope-batch-") && value == batch_ref)
            || check_evidence_store(c, batch_ref)
                .and_then(|p| p.canonicalize().ok())
                .as_ref()
                != Some(&root)
        {
            return Err(fail("scope batch is not bound to this run"));
        }
        let batch: ScopeBatch = serde_json::from_slice(&bounded_artifact_bytes(
            store,
            &c.repo.canonical_root,
            batch_ref,
        )?)
        .map_err(io)?;
        if batch.members.get(&audit.step_id) != Some(&audit.attempt_id) {
            return Err(fail("scope batch does not contain attempt"));
        }
        let peer_ids: BTreeSet<_> = audit.peer_attempts.iter().collect();
        if peer_ids.len() != audit.peer_attempts.len() {
            return Err(fail("duplicate scope peer"));
        }
        for (peer_step, peer_attempt) in &batch.members {
            if peer_step == &audit.step_id {
                continue;
            }
            let spec = c
                .steps
                .get(peer_step)
                .ok_or_else(|| fail("scope batch peer step missing"))?;
            let other = c
                .attempts
                .get(peer_attempt)
                .ok_or_else(|| fail("scope batch peer attempt missing"))?;
            if other.step_id != *peer_step
                || !matches!(spec.purpose, Purpose::Engineer | Purpose::Content)
            {
                return Err(fail("scope batch peer identity mismatch"));
            }
            // A merely prepared peer could not have made an authorized write.
            if peer_ids.contains(peer_attempt)
                && spec.purpose == Purpose::Engineer
                && other.state != AttemptState::Prepared
            {
                peers.extend(spec.scope.clone());
            } else if peer_ids.contains(peer_attempt) {
                return Err(fail("scope peer did not actually start writing"));
            }
        }
        if peer_ids.iter().any(|id| {
            !batch
                .members
                .iter()
                .any(|(step, attempt)| step != &audit.step_id && attempt == *id)
        }) {
            return Err(fail("scope peer is outside the original batch"));
        }
    } else if audit.batch.is_some() || !audit.peer_attempts.is_empty() {
        return Err(fail("assembler cannot borrow engineer scopes"));
    }
    let outside = audit.before.keys().chain(audit.after.keys()).any(|p| {
        audit.before.get(p) != audit.after.get(p)
            && !super::covers_change(
                &step.scope,
                p,
                audit.before.contains_key(p),
                audit.after.contains_key(p),
            )
            && !super::covers_change(
                &peers,
                p,
                audit.before.contains_key(p),
                audit.after.contains_key(p),
            )
    });
    let reported_outside = report["file_changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["path"].as_str())
        .any(|p| {
            !scratch_change(&audit.scratch, p)
                && relative_change(&c.repo.canonical_root, p)
                    .is_none_or(|p| !super::covers(&step.scope, &p))
        });
    Ok(outside || reported_outside)
}

fn validate_scope_audit(
    store: &Path,
    c: &Checkpoint,
    attempt_id: &str,
    outcome: &AttemptOutcome,
    bytes: &[u8],
) -> Res<()> {
    let mut report: Value = serde_json::from_slice(bytes).map_err(io)?;
    let reference: ArtifactRef = serde_json::from_value(
        report
            .as_object_mut()
            .ok_or_else(|| fail("writer report is not an object"))?
            .remove("scope_audit")
            .ok_or_else(|| {
                fail("writer scope audit missing; preserve reservation and use stop/repair")
            })?,
    )
    .map_err(io)?;
    let expected = run_dir(store, &c.run_id)?
        .join("evidence")
        .join(format!("scope-audit-{attempt_id}.json"));
    if reference.path != expected
        || check_evidence_store(c, &reference).and_then(|p| p.canonicalize().ok())
            != store.canonicalize().ok()
    {
        return Err(fail("writer scope audit is outside its run"));
    }
    let audit: ScopeAudit = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        &reference,
    )?)
    .map_err(io)?;
    if audit.attempt_id != attempt_id
        || &audit.outcome != outcome
        || audit.report_digest != digest(&serde_json::to_vec(&report).map_err(io)?)
    {
        return Err(fail("writer scope audit does not match terminal report"));
    }
    if audit_scope_outside(store, c, &audit, &report)? && *outcome != AttemptOutcome::Failed {
        return Err(fail("writer changed paths outside its authorized batch"));
    }
    Ok(())
}

pub(crate) fn completed_attempt(c: &Checkpoint, attempt: &Attempt) -> bool {
    if attempt.state != AttemptState::Terminal || attempt.outcome != Some(AttemptOutcome::Completed)
    {
        return false;
    }
    let Some(step) = c.steps.get(&attempt.step_id) else {
        return false;
    };
    let Some(model_ref) = c
        .receipts
        .get(&format!("attempt-model-{}", attempt.attempt_id))
    else {
        return false;
    };
    let Some(store) = check_evidence_store(c, model_ref) else {
        return false;
    };
    let Some(raw) = &attempt.raw_result else {
        return false;
    };
    let Ok(bytes) = bounded_artifact_bytes(&store, &c.repo.canonical_root, raw) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    if validate_attempt_report(&store, c, &attempt.attempt_id, &value).is_err() {
        return false;
    }
    if !matches!(step.purpose, Purpose::Engineer | Purpose::Assembler) {
        return true;
    }
    let Ok(reference) = serde_json::from_value::<ArtifactRef>(value["scope_audit"].clone()) else {
        return false;
    };
    let Some(audit_store) = check_evidence_store(c, &reference) else {
        return false;
    };
    if audit_store.canonicalize().ok() != store.canonicalize().ok() {
        return false;
    }
    validate_scope_audit(
        &store,
        c,
        &attempt.attempt_id,
        &AttemptOutcome::Completed,
        &bytes,
    )
    .is_ok()
}

// Only the original role can call this, while its before/after maps are still
// available. The audit is durable before the report and checkpoint commit.
pub(crate) fn commit_scoped_result(
    store: &Path,
    id: &str,
    rev: u64,
    attempt_id: &str,
    mut outcome: AttemptOutcome,
    report: &Value,
    batch: Option<&ArtifactRef>,
    scratch: &Path,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Res<(Mutation<ArtifactRef>, Value, bool)> {
    mutate(store, id, rev, |c| {
        let attempt = c.attempts.get(attempt_id).ok_or_else(|| fail("unknown scope audit attempt"))?;
        if attempt.state != AttemptState::Running { return Err(fail("scope audit requires original running role")); }
        let step = c.steps.get(&attempt.step_id).ok_or_else(|| fail("scope audit step missing"))?;
        let authorized_phase = match step.purpose {
            Purpose::Engineer => Phase::Engineering,
            Purpose::Assembler => Phase::Assembling,
            _ => return Err(fail("scope audit requires a writing role")),
        };
        if c.phase != authorized_phase { return Err(fail("scope audit is outside the writing phase")); }
        let mut payload = report.clone();
        let object = payload.as_object_mut().ok_or_else(|| fail("writer report is not an object"))?;
        if object.contains_key("scope_audit") { return Err(fail("writer cannot supply its scope audit")); }
        let mut audit = ScopeAudit {
            attempt_id: attempt_id.into(), step_id: attempt.step_id.clone(), generation: attempt.generation,
            thread_id: attempt.thread_id.clone(), turn_id: attempt.turn_id.clone(), scope: step.scope.clone(),
            batch: batch.cloned(), peer_attempts: Vec::new(), scratch: scratch.to_path_buf(), before: before.clone(), after: after.clone(),
            report_digest: String::new(), outcome: outcome.clone(),
        };
        if let Some(reference) = batch {
            let members: ScopeBatch = serde_json::from_slice(&bounded_artifact_bytes(store, &c.repo.canonical_root, reference)?).map_err(io)?;
            audit.peer_attempts = members.members.iter().filter_map(|(step, id)| {
                let spec = c.steps.get(step)?;
                let peer = c.attempts.get(id)?;
                (id != attempt_id && spec.purpose == Purpose::Engineer && peer.state != AttemptState::Prepared).then_some(id.clone())
            }).collect();
        }
        let outside = audit_scope_outside(store, c, &audit, &payload)?;
        if outside {
            outcome = AttemptOutcome::Failed;
            payload["error"] = serde_json::json!({"kind":"scope_violation","message":"worker changed paths outside assigned product scope; use the assigned external scratch directory for build output"});
        }
        audit.outcome = outcome.clone();
        audit.report_digest = digest(&serde_json::to_vec(&payload).map_err(io)?);
        let reference = save_evidence(store, id, "scope-audit", attempt_id, &audit)?;
        payload["scope_audit"] = serde_json::to_value(reference).map_err(io)?;
        let bytes = serde_json::to_vec(&payload).map_err(io)?;
        let raw = finish_result(store, id, c, attempt_id, outcome, &bytes)?;
        Ok((raw, payload, outside))
    }).map(|m| (Mutation { revision: m.revision, value: m.value.0 }, m.value.1, m.value.2))
}

/// Record a baseline check only for the live, bound baseline verifier. Generic
/// receipt insertion must not be able to fabricate the baseline authority.
pub fn record_baseline_check(store: &Path, id: &str, rev: u64, result: &CheckResult) -> Res<u64> {
    snapshot_ok(&result.snapshot)?;
    Ok(mutate(store, id, rev, |c| {
        if c.phase != Phase::Planning {
            return Err(fail("baseline check is outside planning"));
        }
        current_frozen_sources(c)?;
        let check = c
            .checks
            .get(&result.check_id)
            .ok_or_else(|| fail("unknown baseline check"))?;
        let attempt = c
            .attempts
            .get(&result.executor.attempt_id)
            .ok_or_else(|| fail("unknown baseline verifier"))?;
        let model = attempt_model(store, c, &attempt.attempt_id)?;
        if result.check_id != check.check_id
            || result.snapshot != check.snapshot
            || result.spec_digest != check.spec_digest
            || check_spec_digest(check)? != check.spec_digest
            || super::snapshot(&c.repo.canonical_root, &[".".into()])
                != result.snapshot.source_digest
            || c.plan.as_ref().map(|p| &p.digest) != Some(&check.snapshot.plan_digest)
            || !result.policy_applied
            || result.executor.purpose != Purpose::Verifier
            || result.executor.policy != ExecutionPolicy::ScratchCheck
            || result.executor.model != model.configured_model
            || result.executor.thread_id.trim().is_empty()
            || attempt.state != AttemptState::Running
            || attempt.step_id != "baseline"
            || attempt.turn_id.as_deref().is_none_or(str::is_empty)
            || attempt.thread_id.as_deref() != Some(&result.executor.thread_id)
            || c.steps.get(&attempt.step_id).map(|s| &s.purpose) != Some(&Purpose::Verifier)
        {
            return Err(fail("baseline execution evidence mismatch"));
        }
        bounded_artifact(store, &c.repo.canonical_root, &result.stdout)?;
        bounded_artifact(store, &c.repo.canonical_root, &result.stderr)?;
        let key = format!("baseline-{}", result.check_id);
        if c.receipts.contains_key(&key) {
            return Err(fail("baseline check already recorded"));
        }
        let evidence_id = digest(
            &serde_json::to_vec(&(&result.check_id, &result.executor.attempt_id)).map_err(io)?,
        );
        let receipt = save_evidence(store, id, "baseline", &evidence_id, result)?;
        c.receipts.insert(key, receipt);
        Ok(())
    })?
    .revision)
}

fn baseline_output(report: &Value) -> Res<Value> {
    if !report["structured_output"].is_null() {
        return Ok(report["structured_output"].clone());
    }
    serde_json::from_str(
        report["final_message"]
            .as_str()
            .ok_or_else(|| fail("missing baseline result"))?,
    )
    .map_err(io)
}

/// Validate the complete, typed baseline evidence against one terminal
/// verifier attempt. This is reused by the phase, claim and release gates.
fn validate_baseline_gate(c: &Checkpoint, report_ref: &ArtifactRef) -> Res<()> {
    current_frozen_sources(c)?;
    if c.checks.is_empty() {
        return Err(fail("baseline requires frozen checks"));
    }
    let plan = c
        .plan
        .as_ref()
        .ok_or_else(|| fail("baseline plan missing"))?;
    artifact(plan)?;
    let mut specs = c
        .checks
        .values()
        .map(|check| Ok((check.check_id.clone(), check_spec_digest(check)?)))
        .collect::<Res<Vec<_>>>()?;
    specs.sort();
    let checks_digest = digest(&serde_json::to_vec(&specs).map_err(io)?);
    let first_id = c
        .checks
        .keys()
        .next()
        .ok_or_else(|| fail("baseline requires frozen checks"))?;
    let first_reference = c
        .receipts
        .get(&format!("baseline-{first_id}"))
        .ok_or_else(|| fail("baseline omitted frozen check"))?;
    let store = check_evidence_store(c, first_reference)
        .ok_or_else(|| fail("baseline receipt is outside its run evidence"))?;
    let attempt = c
        .attempts
        .values()
        .find(|a| {
            a.step_id == "baseline"
                && a.state == AttemptState::Terminal
                && a.outcome == Some(AttemptOutcome::Completed)
                && a.raw_result.as_ref() == Some(report_ref)
                && c.steps.get(&a.step_id).map(|s| &s.purpose) == Some(&Purpose::Verifier)
        })
        .ok_or_else(|| fail("baseline report is not a completed verifier attempt"))?;
    let report: Value = serde_json::from_slice(&bounded_artifact_bytes(
        &store,
        &c.repo.canonical_root,
        report_ref,
    )?)
    .map_err(io)?;
    if report["status"] != "completed"
        || attempt.thread_id.as_deref().is_none_or(str::is_empty)
        || attempt.turn_id.as_deref().is_none_or(str::is_empty)
        || report["thread_id"].as_str() != attempt.thread_id.as_deref()
        || report["turn_id"].as_str() != attempt.turn_id.as_deref()
    {
        return Err(fail("baseline report identity mismatch"));
    }
    let model = validate_attempt_report(&store, c, &attempt.attempt_id, &report)?;
    let output = baseline_output(&report)?;
    let infrastructure_failure = |detail: &str| {
        Failure::new(
            "evaluation_infrastructure",
            format!(
                "acceptance infrastructure not ready before engineering ({detail}): {}. No writer started; the frozen plan and checks stay as evidence. Fix the cause and start a new run.",
                output["summary"].as_str().unwrap_or("")
            ),
        )
    };
    if output["infrastructure_ready"] != true {
        return Err(infrastructure_failure(
            "verifier reported infrastructure not ready",
        ));
    }
    let observations = output["observations"]
        .as_array()
        .ok_or_else(|| fail("baseline classifications missing"))?;
    let mut classes = BTreeMap::new();
    for observation in observations {
        let id = observation["check_id"]
            .as_str()
            .ok_or_else(|| fail("baseline classification ID missing"))?;
        let class = observation["classification"]
            .as_str()
            .ok_or_else(|| fail("baseline classification missing"))?;
        if classes.insert(id, class).is_some() || !c.checks.contains_key(id) {
            return Err(fail("duplicate or unknown baseline classification"));
        }
    }
    let mut baseline_snapshot: Option<Snapshot> = None;
    for (id, check) in &c.checks {
        let reference = c
            .receipts
            .get(&format!("baseline-{id}"))
            .ok_or_else(|| fail("baseline omitted frozen check"))?;
        if check_evidence_store(c, reference).as_deref() != Some(store.as_path()) {
            return Err(fail("baseline receipt is outside its run evidence"));
        }
        let result: CheckResult = serde_json::from_slice(&bounded_artifact_bytes(
            &store,
            &c.repo.canonical_root,
            reference,
        )?)
        .map_err(io)?;
        if result.check_id != *id
            || check.check_id != *id
            || result.spec_digest != check.spec_digest
            || check_spec_digest(check)? != check.spec_digest
            || check.snapshot.plan_digest != plan.digest
            || check.snapshot.checks_digest != checks_digest
            || result.snapshot.plan_digest != check.snapshot.plan_digest
            || result.snapshot.checks_digest != check.snapshot.checks_digest
            || !current_source_digest(&result.snapshot.source_digest)
            || baseline_snapshot
                .as_ref()
                .is_some_and(|s| s != &result.snapshot)
            || result.executor.attempt_id != attempt.attempt_id
            || Some(result.executor.thread_id.as_str()) != attempt.thread_id.as_deref()
            || result.executor.model != model.configured_model
            || result.executor.purpose != Purpose::Verifier
            || result.executor.policy != ExecutionPolicy::ScratchCheck
            || !result.policy_applied
        {
            return Err(fail("baseline check evidence mismatch"));
        }
        bounded_artifact(&store, &c.repo.canonical_root, &result.stdout)?;
        bounded_artifact(&store, &c.repo.canonical_root, &result.stderr)?;
        baseline_snapshot.get_or_insert_with(|| result.snapshot.clone());
        match (classes.get(id.as_str()).copied(), result.exit_code) {
            (Some("passed"), Some(0)) => {}
            (Some("product_missing" | "product_defect"), Some(code)) if code != 0 => {}
            (Some("infrastructure_failure"), _) | (_, None) => {
                return Err(infrastructure_failure(&format!(
                    "{id} infrastructure failure or timeout"
                )));
            }
            _ => {
                return Err(infrastructure_failure(&format!(
                    "{id} classification contradicts its executed check"
                )))
            }
        }
    }
    Ok(())
}

pub(crate) fn baseline_authorized(c: &Checkpoint) -> Res<()> {
    current_frozen_sources(c)?;
    if !approved_research(c) {
        return Err(fail("baseline requires approved research"));
    }
    let report = c
        .receipts
        .get(BASELINE_GATE)
        .ok_or_else(|| fail("baseline gate missing"))?;
    validate_baseline_gate(c, report)
}

pub fn record_baseline_gate(store: &Path, id: &str, rev: u64, report: &ArtifactRef) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        if c.phase != Phase::Planning {
            return Err(fail("baseline gate is outside planning"));
        }
        bounded_artifact(store, &c.repo.canonical_root, report)?;
        validate_baseline_gate(c, report)?;
        if c.receipts
            .get(BASELINE_GATE)
            .is_some_and(|old| old != report)
        {
            return Err(fail("baseline gate receipt conflict"));
        }
        c.receipts.insert(BASELINE_GATE.into(), report.clone());
        Ok(())
    })?
    .revision)
}

/// Check receipts are created by `save_evidence` inside this run's evidence
/// directory. Derive the original store only from that established layout,
/// rejecting links or junctions in the run-owned path before reading outputs.
fn check_evidence_store(c: &Checkpoint, reference: &ArtifactRef) -> Option<PathBuf> {
    fn real_entry(path: &Path, directory: bool) -> bool {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        let is_link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let is_link = {
            use std::os::windows::fs::MetadataExt;
            is_link || metadata.file_attributes() & 0x400 != 0
        };
        !is_link
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
    }
    let evidence = reference.path.parent()?;
    let run = evidence.parent()?;
    let runs = run.parent()?;
    let store = runs.parent()?;
    if evidence.file_name()? != "evidence"
        || run.file_name()? != c.run_id.as_str()
        || runs.file_name()? != "runs"
        || !real_entry(runs, true)
        || !real_entry(run, true)
        || !real_entry(evidence, true)
        || !real_entry(&reference.path, false)
    {
        return None;
    }
    let store_root = store.canonicalize().ok()?;
    let evidence_root = evidence.canonicalize().ok()?;
    let receipt_path = reference.path.canonicalize().ok()?;
    if !evidence_root.starts_with(store_root)
        || receipt_path.parent() != Some(evidence_root.as_path())
    {
        return None;
    }
    Some(store.to_path_buf())
}

fn rework_step_id(c: &Checkpoint, name: &str) -> String {
    let count = c
        .receipts
        .keys()
        .filter(|k| k.starts_with("rework-"))
        .count();
    if count == 0 {
        name.into()
    } else {
        format!("{name}-rework-{count}")
    }
}

fn verification_step_id(c: &Checkpoint) -> String {
    rework_step_id(c, "verification")
}

// A completed verifier is replayable, but a second active/completed attempt
// for the same verification epoch would make its evidence ambiguous.
fn current_verifier_attempt(c: &Checkpoint) -> Option<&Attempt> {
    let step_id = verification_step_id(c);
    let step = c.steps.get(&step_id)?;
    if step.purpose != Purpose::Verifier {
        return None;
    }
    let mut candidates = c.attempts.values().filter(|a| {
        a.step_id == step_id
            && (a.state != AttemptState::Terminal || a.outcome == Some(AttemptOutcome::Completed))
    });
    let candidate = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(candidate)
}

fn verified_check_result(c: &Checkpoint, id: &str) -> Option<CheckResult> {
    let reference = c.receipts.get(&format!("check-{id}"))?;
    let store = check_evidence_store(c, reference)?;
    let bytes = bounded_artifact_bytes(&store, &c.repo.canonical_root, reference).ok()?;
    let result = serde_json::from_slice::<CheckResult>(&bytes).ok()?;
    let model = attempt_model(&store, c, &result.executor.attempt_id).ok()?;
    let valid = c.checks.get(id).is_some_and(|f| {
        result.check_id == id
            && result.snapshot == f.snapshot
            && result.spec_digest == f.spec_digest
            && check_spec_digest(f).ok().as_deref() == Some(f.spec_digest.as_str())
            && c.plan
                .as_ref()
                .is_some_and(|p| p.digest == f.snapshot.plan_digest && artifact(p).is_ok())
            && super::snapshot(&c.repo.canonical_root, &[".".into()]) == f.snapshot.source_digest
            && result.policy_applied
            && result.executor.purpose == Purpose::Verifier
            && result.executor.policy == ExecutionPolicy::ScratchCheck
            && result.executor.model == model.configured_model
            && result.executor.thread_id == model.thread_id
            && bounded_artifact(&store, &c.repo.canonical_root, &result.stdout).is_ok()
            && bounded_artifact(&store, &c.repo.canonical_root, &result.stderr).is_ok()
    });
    valid.then_some(result)
}

pub(crate) fn successful_check(c: &Checkpoint, id: &str) -> bool {
    verified_check_result(c, id).is_some_and(|result| result.exit_code == Some(0))
}

/// A review's decision and attribution are authoritative only when bound to
/// one completed verifier report and all results from that same attempt.
fn validate_review(
    c: &Checkpoint,
    store: &Path,
    review: &IndependentReview,
) -> Res<(bool, String)> {
    let attempt = current_verifier_attempt(c)
        .ok_or_else(|| fail("current verifier is ambiguous or missing"))?;
    let step = c
        .steps
        .get(&attempt.step_id)
        .ok_or_else(|| fail("missing verifier step"))?;
    if attempt.state != AttemptState::Terminal
        || attempt.outcome != Some(AttemptOutcome::Completed)
        || attempt.raw_result.as_ref() != Some(&review.report)
        || review.review_id != attempt.attempt_id
        || review.executor.attempt_id != attempt.attempt_id
        || review.executor.purpose != Purpose::Verifier
        || review.executor.policy != ExecutionPolicy::ProductReadOnly
        || attempt.thread_id.as_deref().is_none_or(str::is_empty)
        || attempt.turn_id.as_deref().is_none_or(str::is_empty)
        || attempt.thread_id.as_deref() != Some(review.executor.thread_id.as_str())
        || review.executor.model.trim().is_empty()
    {
        return Err(fail("review is not the completed current verifier attempt"));
    }
    if c.attempts.values().any(|other| {
        other.attempt_id != attempt.attempt_id
            && other.thread_id.as_ref() == attempt.thread_id.as_ref()
            && c.steps
                .get(&other.step_id)
                .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))
    }) {
        return Err(fail("review thread is not independent"));
    }
    let report: Value = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        &review.report,
    )?)
    .map_err(io)?;
    let model = validate_attempt_report(store, c, &attempt.attempt_id, &report)?;
    if report["status"] != "completed"
        || report["thread_id"].as_str() != attempt.thread_id.as_deref()
        || report["turn_id"].as_str() != attempt.turn_id.as_deref()
        || review.executor.model != model.configured_model
    {
        return Err(fail("review report identity mismatch"));
    }
    let output = baseline_output(&report)?;
    let declared_pass = output["pass"]
        .as_bool()
        .ok_or_else(|| fail("review decision missing"))?;
    if output["summary"].as_str().is_none() {
        return Err(fail("review summary missing"));
    }
    let owner = output["defect_owner"]
        .as_str()
        .ok_or_else(|| fail("review attribution missing"))?;
    if !matches!(owner, "none" | "assembly" | "upstream") || (declared_pass && owner != "none") {
        return Err(fail("review decision and attribution disagree"));
    }
    let ids: BTreeSet<_> = review.check_ids.iter().collect();
    if ids.is_empty()
        || ids.len() != review.check_ids.len()
        || ids != c.checks.keys().collect::<BTreeSet<_>>()
    {
        return Err(fail("review must cover every frozen check exactly once"));
    }
    let store_root = store.canonicalize().map_err(io)?;
    let mut checks_pass = true;
    for id in &review.check_ids {
        let reference = c
            .receipts
            .get(&format!("check-{id}"))
            .ok_or_else(|| fail("review check missing"))?;
        let receipt_store = check_evidence_store(c, reference)
            .ok_or_else(|| fail("review check outside its run"))?;
        if receipt_store.canonicalize().map_err(io)? != store_root || !step.check_ids.contains(id) {
            return Err(fail("review check is not authorized for this verifier"));
        }
        let result =
            verified_check_result(c, id).ok_or_else(|| fail("review check evidence invalid"))?;
        if result.snapshot != review.snapshot
            || result.executor.attempt_id != attempt.attempt_id
            || result.executor.thread_id != review.executor.thread_id
            || result.executor.model != review.executor.model
        {
            return Err(fail(
                "review check belongs to a different verifier or snapshot",
            ));
        }
        checks_pass &= result.exit_code == Some(0);
    }
    if super::snapshot(&c.repo.canonical_root, &[".".into()]) != review.snapshot.source_digest {
        return Err(fail("review source snapshot changed"));
    }
    let effective_pass = declared_pass && checks_pass;
    if review.pass != effective_pass {
        return Err(fail("review decision disagrees with its report and checks"));
    }
    Ok((effective_pass, owner.to_owned()))
}
/// Return one currently valid independent passing review. Callers that bind
/// an action to a review must use this exact reference, not any review receipt.
pub fn verified_review(c: &Checkpoint) -> Option<&ArtifactRef> {
    c.receipts
        .iter()
        .filter(|(k, _)| k.starts_with("review-"))
        .find_map(|(key, r)| {
            let store = check_evidence_store(c, r)?;
            let Ok(bytes) = bounded_artifact_bytes(&store, &c.repo.canonical_root, r) else {
                return None;
            };
            let Ok(review) = serde_json::from_slice::<IndependentReview>(&bytes) else {
                return None;
            };
            (key == &format!("review-{}", review.review_id)
                && validate_review(c, &store, &review).is_ok_and(|(pass, _)| pass))
            .then_some(r)
        })
}
fn valid_review(c: &Checkpoint) -> bool {
    // validate_review requires every frozen check exactly once, validates each
    // result and its current source/logs, and returns pass only if all exited 0.
    verified_review(c).is_some()
}
pub(crate) fn approved_research(c: &Checkpoint) -> bool {
    c.core_decision.as_ref().is_some_and(|d| {
        d.pass
            && !d.reason.trim().is_empty()
            && c.research
                .as_ref()
                .is_some_and(|r| r.digest == d.research_digest && artifact(r).is_ok())
    })
}
pub(crate) fn ready_evidence(c: &Checkpoint) -> bool {
    matches!(c.phase, Phase::ReadyToCommit | Phase::Released)
        && approved_research(c)
        && baseline_authorized(c).is_ok()
        && !c.checks.is_empty()
        && valid_review(c)
        && assembly_evidence(c)
        && c.receipts
            .get("memory")
            .is_some_and(|r| artifact(r).is_ok())
}
pub fn verified_outcome(c: &Checkpoint) -> bool {
    approved_research(c)
        && baseline_authorized(c).is_ok()
        && assembly_evidence(c)
        && !c.checks.is_empty()
        && valid_review(c)
}
/// Keep executable IDs/specifications fixed; bind the final verification to the
/// assembled source, not the pre-engineering baseline snapshot.
pub fn freeze_verification(store: &Path, id: &str, rev: u64) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        if c.phase != Phase::Verifying
            || c.attempts.values().any(|a| {
                a.state != AttemptState::Terminal
                    && c.steps
                        .get(&a.step_id)
                        .is_some_and(|s| s.purpose == Purpose::Verifier)
            })
        {
            return Err(fail(
                "verification freeze requires completed assembly and no active verifier",
            ));
        }
        // A new verification snapshot cannot retrospectively upgrade a v1
        // baseline that never captured the ignored input files.
        baseline_authorized(c)?;
        let checks: Vec<_> = c.checks.values().cloned().collect();
        let snapshot = frozen_snapshot(
            &c.repo.canonical_root,
            c.plan.as_ref().ok_or_else(|| fail("missing plan"))?,
            &checks,
        )?;
        for check in c.checks.values_mut() {
            check.snapshot = snapshot.clone();
        }
        c.receipts
            .retain(|k, _| !k.starts_with("check-") && !k.starts_with("review-"));
        Ok(())
    })?
    .revision)
}
fn assembly_evidence(c: &Checkpoint) -> bool {
    c.receipts.get("assembly").is_some_and(|r| {
        artifact(r).is_ok()
            && c.attempts.values().any(|a| {
                a.step_id == rework_step_id(c, "assembly")
                    && completed_attempt(c, a)
                    && a.raw_result.as_ref() == Some(r)
                    && c.steps
                        .get(&a.step_id)
                        .is_some_and(|s| s.purpose == Purpose::Assembler)
            })
    })
}
pub fn record_check(store: &Path, id: &str, rev: u64, result: &CheckResult) -> Res<u64> {
    snapshot_ok(&result.snapshot)?;
    Ok(mutate(store, id, rev, |c| {
        baseline_authorized(c)?;
        if c.phase != Phase::Verifying {
            return Err(fail("check is outside verification"));
        }
        bounded_artifact(store, &c.repo.canonical_root, &result.stdout)?;
        bounded_artifact(store, &c.repo.canonical_root, &result.stderr)?;
        let f = c
            .checks
            .get(&result.check_id)
            .ok_or_else(|| fail("unknown check"))?;
        if f.snapshot != result.snapshot
            || f.spec_digest != result.spec_digest
            || check_spec_digest(f)? != f.spec_digest
            || super::snapshot(&c.repo.canonical_root, &[".".into()]) != f.snapshot.source_digest
            || c.plan.as_ref().map(|p| &p.digest) != Some(&f.snapshot.plan_digest)
            || !result.policy_applied
            || result.executor.purpose != Purpose::Verifier
            || result.executor.policy != ExecutionPolicy::ScratchCheck
        {
            return Err(fail("check evidence mismatch or isolation missing"));
        }
        let a = current_verifier_attempt(c)
            .ok_or_else(|| fail("current verifier is ambiguous or missing"))?;
        let model = attempt_model(store, c, &a.attempt_id)?;
        if a.attempt_id != result.executor.attempt_id
            || a.state != AttemptState::Running
            || a.turn_id.as_deref().is_none_or(str::is_empty)
            || a.thread_id.as_deref().is_none_or(str::is_empty)
            || a.thread_id.as_deref() != Some(result.executor.thread_id.as_str())
            || !c
                .steps
                .get(&a.step_id)
                .is_some_and(|step| step.check_ids.contains(&result.check_id))
            || result.executor.model != model.configured_model
        {
            return Err(fail(
                "check is not authorized by the current running verifier",
            ));
        }
        let key = digest(
            &serde_json::to_vec(&(&result.check_id, &result.executor.attempt_id)).map_err(io)?,
        );
        let r = save_evidence(store, id, "check", &key, result)?;
        c.receipts.insert(format!("check-{}", result.check_id), r);
        Ok(())
    })?
    .revision)
}
pub fn record_independent_review(
    store: &Path,
    id: &str,
    rev: u64,
    review: &IndependentReview,
) -> Res<u64> {
    snapshot_ok(&review.snapshot)?;
    Ok(mutate(store, id, rev, |c| {
        baseline_authorized(c)?;
        if c.phase != Phase::Verifying {
            return Err(fail("review is outside verification"));
        }
        validate_review(c, store, review)?;
        let r = save_evidence(store, id, "review", &review.review_id, review)?;
        c.receipts.insert(format!("review-{}", review.review_id), r);
        Ok(())
    })?
    .revision)
}

/// Identify public checks from the frozen, hash-verified plan. Older plans
/// could use `independent-` for a public ID, so names alone are insufficient.
/// A frozen check absent from the plan is private only if its preparation
/// receipt identifies exactly the same private IDs; ambiguous data fail closed.
pub fn public_check_ids(store: &Path, c: &Checkpoint) -> Res<BTreeSet<String>> {
    let plan = c.plan.as_ref().ok_or_else(|| fail("frozen plan missing"))?;
    let plan_bytes = bounded_artifact_bytes(store, &c.repo.canonical_root, plan)?;
    let plan_value: Value = serde_json::from_slice(&plan_bytes).map_err(io)?;
    let checks = plan_value["checks"]
        .as_array()
        .ok_or_else(|| fail("frozen plan has no public check list"))?;
    let mut public = BTreeSet::new();
    for check in checks {
        let id = check["id"]
            .as_str()
            .ok_or_else(|| fail("frozen plan has an invalid public check ID"))?;
        if !public.insert(id.to_owned()) || !c.checks.contains_key(id) {
            return Err(fail("frozen public checks do not match the plan"));
        }
    }
    if public.is_empty()
        || c.checks
            .values()
            .any(|check| check.snapshot.plan_digest != plan.digest)
    {
        return Err(fail("frozen public checks do not match the plan"));
    }
    let private: BTreeSet<_> = c
        .checks
        .keys()
        .filter(|id| !public.contains(*id))
        .cloned()
        .collect();
    if private.is_empty() {
        return Ok(public);
    }
    if private.iter().any(|id| !id.starts_with("independent-")) {
        return Err(fail("independent check provenance cannot be verified"));
    }
    let epoch = planning_epoch(c);
    let receipt = c
        .receipts
        .get(&format!("evaluation-{epoch}"))
        .ok_or_else(|| fail("independent preparation receipt missing"))?;
    let receipt_bytes = bounded_artifact_bytes(store, &c.repo.canonical_root, receipt)?;
    if !c.attempts.values().any(|attempt| {
        attempt.step_id == format!("evaluation-prep-{epoch}")
            && attempt.state == AttemptState::Terminal
            && attempt.outcome == Some(AttemptOutcome::Completed)
            && attempt.raw_result.as_ref() == Some(receipt)
    }) {
        return Err(fail("independent preparation identity cannot be verified"));
    }
    let report: Value = serde_json::from_slice(&receipt_bytes).map_err(io)?;
    let output = if report["structured_output"].is_null() {
        serde_json::from_str::<Value>(
            report["final_message"]
                .as_str()
                .ok_or_else(|| fail("independent preparation output missing"))?,
        )
        .map_err(io)?
    } else {
        report["structured_output"].clone()
    };
    if output["status"] != "ready"
        || output["exposure"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(fail(
            "independent preparation provenance cannot be verified",
        ));
    }
    let prepared = output["checks"]
        .as_array()
        .ok_or_else(|| fail("independent preparation checks missing"))?;
    let mut prepared_ids = BTreeSet::new();
    for check in prepared {
        let id = check["id"]
            .as_str()
            .ok_or_else(|| fail("independent preparation check ID missing"))?;
        if !prepared_ids.insert(id.to_owned()) {
            return Err(fail("duplicate independent preparation check ID"));
        }
    }
    if prepared_ids != private {
        return Err(fail(
            "independent preparation checks do not match frozen checks",
        ));
    }
    Ok(public)
}
/// Return a known, independently rejected assembly to SOL without changing
/// scope or frozen commands. Prior failures remain immutable evidence.
pub fn prepare_rework(store: &Path, id: &str, rev: u64) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        baseline_authorized(c)?;
        let count = c.receipts.keys().filter(|k| k.starts_with("rework-")).count();
        if count >= 2 || c.phase != Phase::Verifying || !c.reservations.is_empty()
            || c.attempts.values().any(|a| a.state != AttemptState::Terminal)
            || !c.receipts.iter().filter(|(k, _)| k.starts_with("review-")).any(|(key, r)| {
                bounded_artifact_bytes(store, &c.repo.canonical_root, r).ok()
                    .and_then(|b| serde_json::from_slice::<IndependentReview>(&b).ok())
                    .is_some_and(|r| key == &format!("review-{}", r.review_id)
                        && validate_review(c, store, &r).is_ok_and(|(pass, owner)| !pass && owner == "assembly"))
            })
        {
            return Err(fail("rework requires a known rejected review; at most two repairs"));
        }
        let public = public_check_ids(store, c)?;
        let evidence: BTreeMap<_, _> = c.receipts.iter()
            .filter(|(k, _)| k.strip_prefix("check-").is_some_and(|id| public.contains(id)) || k.starts_with("review-"))
            .map(|(k, v)| (k.clone(), v.clone())).collect();
        let number = count + 1;
        let feedback = save_evidence(store, id, "rework", &number.to_string(), &evidence)?;
        for name in ["assembly", "verification"] {
            let mut step = c.steps.get(name).ok_or_else(|| fail("missing repair step"))?.clone();
            step.step_id = format!("{name}-rework-{number}");
            if name == "assembly" {
                let brief = format!("{}\nRepair the independently rejected result within the unchanged scope. Read failure evidence {:?}. Frozen acceptance commands must remain unchanged.", std::fs::read_to_string(&step.brief_ref.path).map_err(io)?, feedback.path);
                step.brief_ref = immutable(&run_dir(store, id)?.join("artifacts").join(format!("{}.txt", digest(brief.as_bytes()))), brief.as_bytes())?;
            }
            c.steps.insert(step.step_id.clone(), step);
        }
        c.receipts.insert(format!("rework-{number}"), feedback);
        c.receipts.remove("assembly");
        c.receipts.remove("memory");
        c.phase = Phase::Assembling;
        c.status = RunStatus::Running;
        Ok(())
    })?.revision)
}

/// Prepared is the durable proof that BeforeTurn never committed. Cancellation
/// may release only those reservations; Dispatching/Running/unknown still need
/// terminal or explicit stop and repair evidence.
pub fn cancel_run(store: &Path, id: &str, rev: u64, reason: &str) -> Res<u64> {
    nonempty(reason)?;
    Ok(mutate(store, id, rev, |c| {
        let prepared = c
            .attempts
            .values()
            .filter(|a| a.state == AttemptState::Prepared)
            .map(|a| a.attempt_id.clone())
            .collect::<Vec<_>>();
        if c.attempts.values().any(|a| {
            !matches!(a.state, AttemptState::Prepared | AttemptState::Terminal)
                || (a.state == AttemptState::Prepared && !prepared_without_dispatch(c, a))
        }) || c
            .reservations
            .iter()
            .any(|r| !prepared.contains(&r.attempt_id))
        {
            return Err(Failure::new(
                "outcome_unknown",
                "reconcile dispatched attempts before cancellation",
            ));
        }
        for attempt_id in prepared {
            // A pre-dispatch cancellation is not a model terminal report and
            // cannot satisfy a dependency or any completion gate.
            let report = serde_json::json!({"attempt_id":attempt_id,"status":"cancelled",
                "pre_dispatch":true,"reason":reason});
            let content = digest(&serde_json::to_vec(&report).map_err(io)?);
            let reference = save_evidence(store, id, "cancel-prepared", &content, &report)?;
            c.receipts
                .insert(format!("cancel-prepared-{attempt_id}"), reference.clone());
            let a = c
                .attempts
                .get_mut(&attempt_id)
                .ok_or_else(|| fail("unknown prepared attempt"))?;
            a.state = AttemptState::Terminal;
            a.outcome = Some(AttemptOutcome::Cancelled);
            a.raw_result = Some(reference);
        }
        c.reservations.clear();
        c.phase = Phase::Cancelled;
        c.status = RunStatus::Cancelled;
        Ok(())
    })?
    .revision)
}

pub fn advance_phase(store: &Path, id: &str, rev: u64, phase: Phase) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        if phase != Phase::Cancelled && c.plan.is_some() {
            current_frozen_sources(c)?;
        }
        if phase != Phase::Cancelled
            && (phase == Phase::Engineering
                || matches!(
                    c.phase,
                    Phase::Engineering
                        | Phase::Assembling
                        | Phase::Verifying
                        | Phase::Memory
                        | Phase::ReadyToCommit
                ))
        {
            baseline_authorized(c)?;
        }
        let allowed = match (&c.phase, &phase) {
            (Phase::Research, Phase::IntentReview) => c.research.is_some(),
            (Phase::IntentReview, Phase::Planning) => c.core_decision.as_ref().is_some_and(|d| {
                d.pass
                    && c.research
                        .as_ref()
                        .is_some_and(|r| r.digest == d.research_digest)
            }),
            // Writers start only after every frozen check ran in the baseline
            // and the acceptance infrastructure was classified ready.
            (Phase::Planning, Phase::Engineering) => {
                c.plan.is_some() && !c.steps.is_empty() && baseline_authorized(c).is_ok()
            }
            (Phase::Engineering, Phase::Assembling) => c
                .steps
                .values()
                .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Content))
                .all(|s| {
                    c.attempts
                        .values()
                        .any(|a| a.step_id == s.step_id && completed_attempt(c, a))
                }),
            (Phase::Assembling, Phase::Verifying) => assembly_evidence(c),
            (Phase::Verifying, Phase::Memory) => {
                !c.checks.is_empty()
                    && c.checks.keys().all(|x| successful_check(c, x))
                    && valid_review(c)
            }
            (Phase::Memory, Phase::ReadyToCommit) => {
                assembly_evidence(c)
                    && c.receipts
                        .get("memory")
                        .is_some_and(|r| artifact(r).is_ok())
                    && valid_review(c)
            }
            (Phase::ReadyToCommit, Phase::Released) => {
                ready_evidence(c)
                    && c.receipts
                        .get("release")
                        .is_some_and(|r| artifact(r).is_ok())
            }
            (_, Phase::Cancelled) => c.reservations.is_empty(),
            (a, b) if a == b => true,
            _ => false,
        };
        if !allowed {
            return Err(fail("phase transition missing evidence"));
        }
        c.status = match phase {
            Phase::IntentReview => RunStatus::NeedsIntentReview,
            Phase::ReadyToCommit => RunStatus::ReadyToCommit,
            Phase::Released => RunStatus::Released,
            Phase::Cancelled => RunStatus::Cancelled,
            _ => RunStatus::Running,
        };
        c.phase = phase;
        Ok(())
    })?
    .revision)
}
pub fn reconcile_run(
    store: &Path,
    id: &str,
    rev: u64,
    obs: &[RecoveryObservation],
) -> Res<ReconcileResult> {
    let m = mutate(store, id, rev, |c| {
        let mut actions = vec![];
        for pending in &mut c.pending_evidence {
            if !pending.projected {
                let bytes = bounded_artifact_bytes(store, &c.repo.canonical_root, &pending.report)?;
                let dest = run_dir(store, id)?
                    .join("evidence")
                    .join(format!("{}.json", pending.evidence_id));
                let projected = immutable(&dest, &bytes)?;
                if projected.digest != pending.report.digest {
                    return Err(fail("projection mismatch"));
                }
                pending.projected = true;
            }
        }
        let pending: Vec<String> = c
            .attempts
            .values()
            .filter(|a| a.state != AttemptState::Terminal)
            .map(|a| a.attempt_id.clone())
            .collect();
        for attempt_id in pending {
            let dir = run_dir(store, id)?.join("reports");
            let path = dir.join(format!("{attempt_id}.json"));
            let outcome_path = dir.join(format!("{attempt_id}.outcome.json"));
            if path.is_file() && outcome_path.is_file() {
                let bytes = std::fs::read(path).map_err(io)?;
                let outcome = serde_json::from_slice::<AttemptOutcome>(
                    &std::fs::read(outcome_path).map_err(io)?,
                );
                let writer = c
                    .attempts
                    .get(&attempt_id)
                    .and_then(|a| c.steps.get(&a.step_id))
                    .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler));
                if writer
                    && (outcome.as_ref().is_err()
                        || outcome.as_ref().is_ok_and(|o| {
                            validate_scope_audit(store, c, &attempt_id, o, &bytes).is_err()
                        }))
                {
                    continue;
                }
                let _: Value = serde_json::from_slice(&bytes).map_err(io)?;
                let outcome = outcome.map_err(io)?;
                finish_result(store, id, c, &attempt_id, outcome, &bytes)?;
            }
        }
        for a in c.attempts.values() {
            let o = obs.iter().find(|o| o.attempt_id == a.attempt_id);
            let writer = c
                .steps
                .get(&a.step_id)
                .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler));
            if writer
                && !matches!(a.state, AttemptState::Prepared | AttemptState::Terminal)
                && !o.is_some_and(|o| o.live_in_process)
            {
                actions.push(RecoveryAction::NeedsDecision {
                    attempt_id: a.attempt_id.clone(),
                    reason: "writer outcome lacks its original durable scope audit; retain its reservation and provide explicit stop and repair evidence".into(),
                });
                continue;
            }
            if let Some(r) = o.and_then(|x| x.terminal_report.as_ref()) {
                artifact(r)?;
                actions.push(RecoveryAction::ReplayResult {
                    attempt_id: a.attempt_id.clone(),
                });
                continue;
            }
            if a.state != AttemptState::Terminal
                && run_dir(store, id)?
                    .join("reports")
                    .join(format!("{}.json", a.attempt_id))
                    .is_file()
            {
                actions.push(RecoveryAction::ReplayResult {
                    attempt_id: a.attempt_id.clone(),
                });
                continue;
            }
            let action = match a.state {
                AttemptState::Prepared
                    if c.steps.get(&a.step_id).is_some_and(|s| {
                        matches!(s.purpose, Purpose::Engineer | Purpose::Assembler | Purpose::Content)
                    }) && baseline_authorized(c).is_err() => RecoveryAction::NeedsDecision {
                        attempt_id: a.attempt_id.clone(),
                        reason: "prepared work has no current complete baseline; preserve the evidence and start a new run".into(),
                    },
                AttemptState::Prepared => RecoveryAction::DispatchPrepared {
                    attempt_id: a.attempt_id.clone(),
                },
                AttemptState::Terminal
                    if c.pending_evidence
                        .iter()
                        .any(|p| p.attempt_id == a.attempt_id && !p.projected) =>
                {
                    RecoveryAction::ReplayResult {
                        attempt_id: a.attempt_id.clone(),
                    }
                }
                AttemptState::Terminal => continue,
                _ if o.is_some_and(|o| o.live_in_process) => RecoveryAction::JoinLocal {
                    attempt_id: a.attempt_id.clone(),
                },
                _ if a.thread_id.is_some() => RecoveryAction::ReadHistory {
                    attempt_id: a.attempt_id.clone(),
                },
                _ => RecoveryAction::NeedsDecision {
                    attempt_id: a.attempt_id.clone(),
                    reason: "execution outcome cannot be established".into(),
                },
            };
            actions.push(action);
        }
        Ok(actions)
    })?;
    Ok(ReconcileResult {
        revision: m.revision,
        actions: m.value,
    })
}
pub fn resolve_attempt(store: &Path, id: &str, rev: u64, decision: &RecoveryDecision) -> Res<u64> {
    match decision {
        RecoveryDecision::RetryPlanning { reason } => {
            nonempty(reason)?;
            Ok(mutate(store, id, rev, |c| {
                if c.phase != Phase::Planning || c.plan.is_some() || !c.checks.is_empty()
                    || !approved_research(c) || !c.reservations.is_empty()
                    || c.attempts.values().any(|a| a.state != AttemptState::Terminal
                        || c.steps.get(&a.step_id).is_none_or(|s|
                            !matches!(s.purpose, Purpose::Dispatcher | Purpose::Research | Purpose::Planner)
                            && !(s.purpose == Purpose::Verifier && s.step_id.starts_with("evaluation-prep-"))))
                {
                    return Err(fail("planning retry requires approved research, no frozen plan, no writes and all attempts terminal"));
                }
                let epoch = planning_epoch(c) + 1;
                if epoch > 2 {
                    return Err(fail("planning retry limit reached; retain evidence and start a new run"));
                }
                let attempts: Vec<_> = c.attempts.values().filter(|a|
                    c.steps[&a.step_id].purpose != Purpose::Research
                    && !a.step_id.starts_with("coord-research-")).map(|a| a.attempt_id.clone()).collect();
                if attempts.is_empty() {
                    return Err(fail("no planning attempt to retire"));
                }
                let bytes = serde_json::to_vec(&serde_json::json!({"epoch":epoch,"reason":reason,"attempt_ids":attempts})).map_err(io)?;
                let path = run_dir(store, id)?.join(format!("planning-retry-{epoch}.json"));
                base::atomic_write(&path, &bytes).map_err(io)?;
                let receipt = ArtifactRef {path, digest:digest(&bytes)};
                c.receipts.insert(format!("planning-retry-{epoch}"), receipt.clone());
                for attempt in attempts {
                    c.receipts.insert(format!("planning-retired-{attempt}"), receipt.clone());
                }
                c.status = RunStatus::Running;
                Ok(())
            })?.revision)
        }
        RecoveryDecision::AcceptTerminal {
            attempt_id,
            report,
            reason,
        } => {
            nonempty(reason)?;
            Ok(mutate(store, id, rev, |c| {
                let bytes = bounded_artifact_bytes(store, &c.repo.canonical_root, report)?;
                let value: Value = serde_json::from_slice(&bytes).map_err(io)?;
                let a = c
                    .attempts
                    .get(attempt_id)
                    .ok_or_else(|| fail("unknown attempt"))?;
                if a.state != AttemptState::OutcomeUnknown
                    || a.turn_id.as_deref().is_none_or(|s| s.trim().is_empty())
                    || value.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
                    || value.get("thread_id").and_then(Value::as_str) != a.thread_id.as_deref()
                    || value.get("generation").and_then(Value::as_u64) != a.generation
                    || value.get("turn_id").and_then(Value::as_str) != a.turn_id.as_deref()
                {
                    return Err(fail("terminal evidence identity mismatch"));
                }
                let outcome = match value.get("outcome").and_then(Value::as_str) {
                    Some("completed") => AttemptOutcome::Completed,
                    Some("failed") => AttemptOutcome::Failed,
                    Some("cancelled") => AttemptOutcome::Cancelled,
                    Some("blocked") => AttemptOutcome::Blocked,
                    _ => return Err(fail("terminal evidence missing outcome")),
                };
                finish_result(store, id, c, attempt_id, outcome, &bytes)?;
                Ok(())
            })?
            .revision)
        }
        RecoveryDecision::RetryAfterRepair {
            attempt_id,
            stopped_evidence,
            repair_evidence,
            reason,
        } => {
            nonempty(reason)?;
            if stopped_evidence == repair_evidence {
                return Err(fail("separate stop and repair evidence required"));
            }
            Ok(mutate(store, id, rev, |c| {
                let stop_bytes =
                    bounded_artifact_bytes(store, &c.repo.canonical_root, stopped_evidence)?;
                let repair_bytes =
                    bounded_artifact_bytes(store, &c.repo.canonical_root, repair_evidence)?;
                let is_writer = c
                    .attempts
                    .get(attempt_id)
                    .and_then(|a| c.steps.get(&a.step_id))
                    .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler));
                let a = c
                    .attempts
                    .get_mut(attempt_id)
                    .ok_or_else(|| fail("unknown attempt"))?;
                let terminal_writer = is_writer
                    && a.state == AttemptState::Terminal
                    && matches!(
                        a.outcome,
                        Some(AttemptOutcome::Failed | AttemptOutcome::Cancelled)
                    );
                if a.state != AttemptState::OutcomeUnknown && !terminal_writer {
                    return Err(fail(
                        "attempt is neither unknown nor a stopped failed writer",
                    ));
                }
                let stop: Value = serde_json::from_slice(&stop_bytes).map_err(io)?;
                if stop.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
                    || stop.get("terminal").and_then(Value::as_bool) != Some(true)
                    || stop.get("thread_id").and_then(Value::as_str) != a.thread_id.as_deref()
                    || stop.get("generation").and_then(Value::as_u64) != a.generation
                    || stop.get("turn_id").and_then(Value::as_str) != a.turn_id.as_deref()
                {
                    return Err(fail("stop evidence must identify a terminal attempt"));
                }
                let repair: Value = serde_json::from_slice(&repair_bytes).map_err(io)?;
                if repair.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
                    || repair.get("repaired").and_then(Value::as_bool) != Some(true)
                {
                    return Err(fail("repair evidence must identify repaired attempt"));
                }
                // Keep the exact verified bytes even if the supplied source
                // paths later change; never overwrite those source files.
                let evidence = run_dir(store, id)?.join("evidence");
                let stop = immutable(
                    &evidence.join(format!("stop-{attempt_id}.json")),
                    &stop_bytes,
                )?;
                let repair = immutable(
                    &evidence.join(format!("repair-{attempt_id}.json")),
                    &repair_bytes,
                )?;
                if !terminal_writer {
                    a.state = AttemptState::Terminal;
                    a.outcome = Some(AttemptOutcome::Cancelled);
                    a.raw_result = Some(stop.clone());
                }
                c.reservations.retain(|r| r.attempt_id != *attempt_id);
                c.receipts.insert(format!("stop-{attempt_id}"), stop);
                c.receipts.insert(format!("repair-{attempt_id}"), repair);
                c.status = RunStatus::Blocked;
                Ok(())
            })?
            .revision)
        }
    }
}
pub fn planning_epoch(c: &Checkpoint) -> usize {
    c.receipts
        .keys()
        .filter(|k| k.starts_with("planning-retry-"))
        .count()
}

fn prepared_without_dispatch(c: &Checkpoint, a: &Attempt) -> bool {
    a.state == AttemptState::Prepared
        && a.generation.is_none()
        && a.thread_id.is_none()
        && a.turn_id.is_none()
        && a.last_event_sequence == 0
        && a.raw_result.is_none()
        && a.outcome.is_none()
        && !c
            .receipts
            .contains_key(&format!("attempt-model-{}", a.attempt_id))
}

fn batch_retry_matches(
    store: &Path,
    c: &Checkpoint,
    retry: &PreparedBatchRetry,
    before_retry: bool,
) -> Res<()> {
    let a = c
        .attempts
        .get(&retry.coordinator.attempt_id)
        .ok_or_else(|| fail("retry coordinator missing"))?;
    if a != &retry.coordinator
        || a.state != AttemptState::Terminal
        || a.outcome != Some(AttemptOutcome::Failed)
        || c.steps
            .get(&a.step_id)
            .is_none_or(|s| s.purpose != Purpose::Dispatcher)
    {
        return Err(fail("retry is not a known failed read-only dispatcher"));
    }
    terminal_attempt_report(store, c, &a.attempt_id)?;
    let dispatched_by_this_attempt = c
        .receipts
        .iter()
        .filter(|(key, _)| key.starts_with("scope-dispatch-"))
        .any(|(_, reference)| {
            if check_evidence_store(c, reference).is_none() {
                return false;
            }
            let Ok(bytes) = bounded_artifact_bytes(store, &c.repo.canonical_root, reference) else {
                return false;
            };
            let Ok(dispatch) = serde_json::from_slice::<ScopeBatchDispatch>(&bytes) else {
                return false;
            };
            dispatch.run_id == c.run_id
                && dispatch.input_digest == a.input_digest
                && dispatch.batch == retry.batch
                && dispatch.caller.attempt_id == a.attempt_id
                && Some(dispatch.caller.generation) == a.generation
                && Some(dispatch.caller.thread_id.as_str()) == a.thread_id.as_deref()
                && Some(dispatch.caller.turn_id.as_str()) == a.turn_id.as_deref()
                && !dispatch.caller.turn_id.trim().is_empty()
                && !dispatch.caller.call_id.trim().is_empty()
        });
    if !dispatched_by_this_attempt {
        return Err(fail(
            "retry batch lacks its actual dispatcher callback binding",
        ));
    }
    let batch: ScopeBatch = serde_json::from_slice(&bounded_artifact_bytes(
        store,
        &c.repo.canonical_root,
        &retry.batch,
    )?)
    .map_err(io)?;
    let ids = batch.members.keys().collect::<Vec<_>>();
    let step_id = format!(
        "coord-batch-{}",
        &digest(&serde_json::to_vec(&ids).map_err(io)?)[..16]
    );
    if a.step_id != step_id
        || retry.members.len() != batch.members.len()
        || retry.members.is_empty()
        || !c
            .receipts
            .iter()
            .any(|(key, value)| key.starts_with("scope-batch-") && value == &retry.batch)
    {
        return Err(fail("retry batch does not belong to its coordinator"));
    }
    let mut prepared = false;
    for (step, attempt_id) in batch.members {
        let snapshot = retry
            .members
            .get(&step)
            .ok_or_else(|| fail("retry batch member missing"))?;
        let current = c
            .attempts
            .get(&attempt_id)
            .ok_or_else(|| fail("retry attempt missing"))?;
        if snapshot.attempt_id != attempt_id
            || snapshot.step_id != step
            || current.step_id != step
            || current.input_digest != snapshot.input_digest
        {
            return Err(fail("retry batch member identity changed"));
        }
        if snapshot.state == AttemptState::Prepared {
            if snapshot.generation.is_some()
                || snapshot.thread_id.is_some()
                || snapshot.turn_id.is_some()
                || snapshot.last_event_sequence != 0
                || snapshot.raw_result.is_some()
                || snapshot.outcome.is_some()
                || (before_retry && !prepared_without_dispatch(c, current))
            {
                return Err(fail("retry contains a dispatched or uncertain member"));
            }
            prepared = true;
        } else if snapshot != current || !completed_attempt(c, current) {
            return Err(fail("retry contains work without a verified completion"));
        }
    }
    if !prepared {
        return Err(fail("retry has no undispatched work"));
    }
    Ok(())
}

fn retired_prepared_batch(c: &Checkpoint, a: &Attempt) -> bool {
    let Some(reference) = c.receipts.get(&format!("retry-prepared-{}", a.attempt_id)) else {
        return false;
    };
    let Some(store) = check_evidence_store(c, reference) else {
        return false;
    };
    let Ok(bytes) = bounded_artifact_bytes(&store, &c.repo.canonical_root, reference) else {
        return false;
    };
    let Ok(retry) = serde_json::from_slice::<PreparedBatchRetry>(&bytes) else {
        return false;
    };
    retry.coordinator.attempt_id == a.attempt_id
        && batch_retry_matches(&store, c, &retry, false).is_ok()
}

/// An explicit Resume may retry untouched Prepared work. Retire only the known
/// failed read-only coordinator of that exact batch, never uncertain writers or
/// unrelated known failures. The original failure remains immutable evidence.
pub fn retry_prepared_batches(store: &Path, id: &str, rev: u64) -> Res<u64> {
    let c = load_run(store, id)?;
    if c.phase != Phase::Engineering {
        return Ok(rev);
    }
    let mut retries = Vec::new();
    for coordinator in c.attempts.values().filter(|a| {
        a.state == AttemptState::Terminal
            && a.outcome == Some(AttemptOutcome::Failed)
            && !c
                .receipts
                .contains_key(&format!("retry-prepared-{}", a.attempt_id))
    }) {
        for (_, batch) in c
            .receipts
            .iter()
            .filter(|(key, _)| key.starts_with("scope-batch-"))
        {
            let Ok(bytes) = bounded_artifact_bytes(store, &c.repo.canonical_root, batch) else {
                continue;
            };
            let Ok(members) = serde_json::from_slice::<ScopeBatch>(&bytes) else {
                continue;
            };
            let snapshots = members
                .members
                .iter()
                .filter_map(|(step, attempt)| {
                    c.attempts.get(attempt).cloned().map(|a| (step.clone(), a))
                })
                .collect();
            let retry = PreparedBatchRetry {
                coordinator: coordinator.clone(),
                batch: batch.clone(),
                members: snapshots,
            };
            if batch_retry_matches(store, &c, &retry, true).is_ok() {
                retries.push(retry);
                break;
            }
        }
    }
    if retries.is_empty() {
        return Ok(rev);
    }
    Ok(mutate(store, id, rev, |c| {
        for retry in retries {
            batch_retry_matches(store, c, &retry, true)?;
            let content = digest(&serde_json::to_vec(&retry).map_err(io)?);
            let reference = save_evidence(store, id, "retry-prepared", &content, &retry)?;
            c.receipts.insert(
                format!("retry-prepared-{}", retry.coordinator.attempt_id),
                reference,
            );
        }
        c.status = RunStatus::Running;
        Ok(())
    })?
    .revision)
}

pub fn failed_attempt_needs_repair(c: &Checkpoint, a: &Attempt) -> bool {
    (a.outcome == Some(AttemptOutcome::Failed)
        || (a.outcome == Some(AttemptOutcome::Cancelled)
            && c.steps
                .get(&a.step_id)
                .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))))
        && !c
            .receipts
            .contains_key(&format!("planning-retired-{}", a.attempt_id))
        && !c.receipts.contains_key(&format!("repair-{}", a.attempt_id))
        && !retired_prepared_batch(c, a)
}
pub fn record_receipt(
    store: &Path,
    id: &str,
    rev: u64,
    kind: &str,
    receipt: &ArtifactRef,
) -> Res<u64> {
    base::require_slug("receipt kind", kind)?;
    if kind.starts_with("check-")
        || kind.starts_with("baseline-")
        || kind == BASELINE_GATE
        || kind == DISPATCH_FAILURE
        || kind.starts_with("review-")
        || kind.starts_with("repair-")
        || kind.starts_with("stop-")
        || kind.starts_with("scope-")
        || kind.starts_with("rework-")
        || kind.starts_with("intent-rejection-")
        || kind.starts_with("rejected-research-")
        || kind.starts_with("event-")
        || kind.starts_with("attempt-model-")
        || kind.starts_with("cancel-prepared-")
        || kind.starts_with("retry-prepared-")
        || kind.starts_with("runner-")
        || kind.starts_with("planning-retry-")
        || kind.starts_with("planning-retired-")
    {
        return Err(fail("reserved receipt kind"));
    }
    artifact(receipt)?;
    Ok(mutate(store, id, rev, |c| {
        bounded_artifact(store, &c.repo.canonical_root, receipt)?;
        if let Some(old) = c.receipts.get(kind) {
            if old != receipt {
                return Err(fail("receipt conflict"));
            }
        } else {
            c.receipts.insert(kind.into(), receipt.clone());
        }
        if kind == "research" {
            if c.research.as_ref().is_some_and(|r| r != receipt) {
                return Err(fail("research conflict"));
            }
            c.research = Some(receipt.clone());
        }
        Ok(())
    })?
    .revision)
}

#[cfg(test)]
mod verified_artifact_tests {
    use super::*;
    use std::fs;

    #[cfg(any(unix, windows))]
    fn directory_link(link: &Path, target: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
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
    }

    fn reference(path: &Path, bytes: &[u8]) -> ArtifactRef {
        fs::write(path, bytes).unwrap();
        ArtifactRef {
            path: path.to_path_buf(),
            digest: digest(bytes),
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, Checkpoint) {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("store");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(&repo).unwrap();
        let intent = reference(&store.join("intent"), b"intent");
        let c = create_run(
            &store,
            &RunSeed {
                run_id: "run".into(),
                repo: RepoIdentity {
                    canonical_root: repo.canonicalize().unwrap(),
                    identity_digest: "repo".into(),
                },
                intent,
            },
        )
        .unwrap();
        (temp, store, c)
    }

    fn unknown_attempt() -> Attempt {
        Attempt {
            attempt_id: "attempt".into(),
            step_id: "step".into(),
            input_digest: "input".into(),
            state: AttemptState::OutcomeUnknown,
            generation: Some(1),
            thread_id: Some("thread".into()),
            turn_id: Some("turn".into()),
            raw_result: None,
            outcome: None,
            last_event_sequence: 2,
        }
    }

    fn fixture_model(store: &Path, c: &mut Checkpoint, attempt_id: &str, model: &str) {
        let a = &c.attempts[attempt_id];
        let event = AttemptEvent {
            event_id: format!("{attempt_id}-fixture-dispatch"),
            attempt_id: attempt_id.into(),
            sequence: 1,
            kind: AttemptEventKind::Dispatching {
                generation: a.generation.unwrap(),
                thread_id: a.thread_id.clone().unwrap(),
            },
        };
        let receipt = AttemptModelReceipt {
            run_id: c.run_id.clone(),
            step_id: a.step_id.clone(),
            attempt_id: attempt_id.into(),
            input_digest: a.input_digest.clone(),
            generation: a.generation.unwrap(),
            thread_id: a.thread_id.clone().unwrap(),
            dispatch_event_id: event.event_id.clone(),
            dispatch_sequence: 1,
            requested_model: model.into(),
            configured_model: model.into(),
            requested_effort: Some("high".into()),
        };
        let event_ref = save_evidence(store, &c.run_id, "event", &event.event_id, &event).unwrap();
        let model_ref =
            save_evidence(store, &c.run_id, "attempt-model", attempt_id, &receipt).unwrap();
        c.receipts
            .insert(format!("event-{}", event.event_id), event_ref);
        c.receipts
            .insert(format!("attempt-model-{attempt_id}"), model_ref);
    }

    fn mark_unknown(store: &Path, c: &mut Checkpoint) {
        c.status = RunStatus::OutcomeUnknown;
        c.attempts.insert("attempt".into(), unknown_attempt());
        c.reservations.push(ScopeReservation {
            attempt_id: "attempt".into(),
            paths: vec!["src".into()],
        });
        save(store, c).unwrap();
    }

    fn clear_hook() {
        AFTER_ARTIFACT_VERIFICATION.with(|hook| {
            hook.borrow_mut().take();
        });
    }

    #[test]
    fn every_terminal_entry_requires_bound_turn_model_and_report_identity() {
        for entry in ["commit", "accept", "reconcile"] {
            for case in [
                "valid",
                "no-turn",
                "missing-receipt",
                "wrong-model",
                "wrong-turn",
            ] {
                let (_temp, store, mut c) = setup();
                let mut a = unknown_attempt();
                if case == "no-turn" {
                    a.turn_id = None;
                }
                c.steps.insert(
                    "step".into(),
                    StepSpec {
                        run_id: "run".into(),
                        step_id: "step".into(),
                        purpose: Purpose::Content,
                        brief_ref: c.intent.clone(),
                        scope: vec![],
                        dependency_ids: vec![],
                        check_ids: vec![],
                    },
                );
                c.attempts.insert("attempt".into(), a.clone());
                c.status = RunStatus::OutcomeUnknown;
                if case != "missing-receipt" {
                    fixture_model(&store, &mut c, "attempt", "test");
                }
                save(&store, &c).unwrap();
                let report = serde_json::json!({"attempt_id":"attempt","generation":1,"thread_id":"thread",
                    "turn_id":if case=="wrong-turn" {Some("other-turn")} else {a.turn_id.as_deref()},
                    "model":if case=="wrong-model" {Some("other")} else {None},"model_source":"thread/read",
                    "status":"completed","outcome":"completed","structured_output":{"status":"completed","summary":"done","artifact":"content"}});
                let bytes = serde_json::to_vec(&report).unwrap();
                let result = match entry {
                    "commit" => commit_result(
                        &store,
                        "run",
                        c.revision,
                        "attempt",
                        AttemptOutcome::Completed,
                        &report,
                    )
                    .map(|_| ()),
                    "accept" => {
                        let raw = reference(&store.join("external-terminal.json"), &bytes);
                        resolve_attempt(
                            &store,
                            "run",
                            c.revision,
                            &RecoveryDecision::AcceptTerminal {
                                attempt_id: "attempt".into(),
                                report: raw,
                                reason: "reviewed actual terminal evidence".into(),
                            },
                        )
                        .map(|_| ())
                    }
                    _ => {
                        let reports = run_dir(&store, "run").unwrap().join("reports");
                        fs::create_dir_all(&reports).unwrap();
                        fs::write(reports.join("attempt.json"), &bytes).unwrap();
                        fs::write(reports.join("attempt.outcome.json"), b"\"completed\"").unwrap();
                        reconcile_run(&store, "run", c.revision, &[]).map(|_| ())
                    }
                };
                let saved = load_run(&store, "run").unwrap();
                if case == "valid" {
                    result.unwrap();
                    assert!(
                        completed_attempt(&saved, &saved.attempts["attempt"]),
                        "{entry}/{case}"
                    );
                    assert!(serde_json::from_slice::<Value>(
                        &artifact_bytes(saved.attempts["attempt"].raw_result.as_ref().unwrap())
                            .unwrap()
                    )
                    .unwrap()["model"]
                        .is_null());
                } else {
                    assert!(result.is_err(), "{entry}/{case}");
                    assert_eq!(saved.attempts["attempt"], a, "{entry}/{case}");
                    let mut forged = saved.clone();
                    let raw = reference(&store.join("forged-terminal.json"), &bytes);
                    let terminal = forged.attempts.get_mut("attempt").unwrap();
                    terminal.state = AttemptState::Terminal;
                    terminal.outcome = Some(AttemptOutcome::Completed);
                    terminal.raw_result = Some(raw);
                    assert!(!completed_attempt(&forged, &forged.attempts["attempt"]));
                }
            }
        }
    }

    #[test]
    fn cancelling_prepared_reservations_never_cancels_uncertain_dispatch() {
        for state in [
            AttemptState::Prepared,
            AttemptState::Dispatching,
            AttemptState::OutcomeUnknown,
        ] {
            let (_temp, store, mut c) = setup();
            let mut a = unknown_attempt();
            a.state = state.clone();
            if state == AttemptState::Prepared {
                a.generation = None;
                a.thread_id = None;
                a.turn_id = None;
                a.last_event_sequence = 0;
            }
            c.steps.insert(
                "step".into(),
                StepSpec {
                    run_id: "run".into(),
                    step_id: "step".into(),
                    purpose: Purpose::Engineer,
                    brief_ref: c.intent.clone(),
                    scope: vec!["src".into()],
                    dependency_ids: vec![],
                    check_ids: vec![],
                },
            );
            c.attempts.insert("attempt".into(), a);
            c.reservations.push(ScopeReservation {
                attempt_id: "attempt".into(),
                paths: vec!["src".into()],
            });
            save(&store, &c).unwrap();
            let result = cancel_run(
                &store,
                "run",
                c.revision,
                "cancel only work whose turn was never sent",
            );
            let saved = load_run(&store, "run").unwrap();
            if state == AttemptState::Prepared {
                result.unwrap();
                assert_eq!(saved.status, RunStatus::Cancelled);
                assert!(saved.reservations.is_empty());
                assert_eq!(
                    saved.attempts["attempt"].outcome,
                    Some(AttemptOutcome::Cancelled)
                );
                assert!(!completed_attempt(&saved, &saved.attempts["attempt"]));
                let reference = &saved.receipts["cancel-prepared-attempt"];
                assert!(record_receipt(
                    &store,
                    "run",
                    saved.revision,
                    "cancel-prepared-forged",
                    reference
                )
                .is_err());
            } else {
                assert_eq!(result.unwrap_err().kind, "outcome_unknown");
                assert_eq!(saved, c);
            }
        }
    }

    #[test]
    fn prepared_batch_resume_never_retires_uncertain_work_or_unrelated_failures() {
        for state in [
            AttemptState::Prepared,
            AttemptState::Dispatching,
            AttemptState::OutcomeUnknown,
        ] {
            for purpose in [Purpose::Dispatcher, Purpose::Research] {
                let (_temp, store, mut c) = setup();
                c.phase = Phase::Engineering;
                let batch_id = format!(
                    "coord-batch-{}",
                    &digest(&serde_json::to_vec(&vec!["unit"]).unwrap())[..16]
                );
                let mut coordinator = unknown_attempt();
                coordinator.step_id = batch_id.clone();
                coordinator.state = AttemptState::Terminal;
                coordinator.outcome = Some(AttemptOutcome::Failed);
                coordinator.raw_result = Some(reference(
                    &store.join("coordinator.json"),
                    br#"{"status":"failed","thread_id":"thread","turn_id":"turn","model":"test"}"#,
                ));
                c.steps.insert(
                    batch_id.clone(),
                    StepSpec {
                        run_id: "run".into(),
                        step_id: batch_id,
                        purpose: purpose.clone(),
                        brief_ref: c.intent.clone(),
                        scope: vec![],
                        dependency_ids: vec![],
                        check_ids: vec![],
                    },
                );
                c.attempts.insert("attempt".into(), coordinator);
                fixture_model(&store, &mut c, "attempt", "test");
                let mut child = unknown_attempt();
                child.attempt_id = "child".into();
                child.step_id = "unit".into();
                child.state = state.clone();
                if state == AttemptState::Prepared {
                    child.generation = None;
                    child.thread_id = None;
                    child.turn_id = None;
                    child.last_event_sequence = 0;
                }
                c.steps.insert(
                    "unit".into(),
                    StepSpec {
                        run_id: "run".into(),
                        step_id: "unit".into(),
                        purpose: Purpose::Engineer,
                        brief_ref: c.intent.clone(),
                        scope: vec!["src".into()],
                        dependency_ids: vec![],
                        check_ids: vec![],
                    },
                );
                c.attempts.insert("child".into(), child.clone());
                let batch = ScopeBatch {
                    members: BTreeMap::from([("unit".into(), "child".into())]),
                };
                let batch_ref =
                    save_evidence(&store, "run", "scope-batch", "fixture", &batch).unwrap();
                c.receipts
                    .insert("scope-batch-fixture".into(), batch_ref.clone());
                // This fixture represents a callback recorded while the
                // coordinator was Running, before the observed child state.
                let dispatch = ScopeBatchDispatch {
                    run_id: "run".into(),
                    input_digest: c.attempts["attempt"].input_digest.clone(),
                    caller: ScopeBatchCaller {
                        attempt_id: "attempt".into(),
                        generation: 1,
                        thread_id: "thread".into(),
                        turn_id: "turn".into(),
                        call_id: "batch-call".into(),
                    },
                    batch: batch_ref,
                };
                let dispatch_ref =
                    save_evidence(&store, "run", "scope-dispatch", "fixture", &dispatch).unwrap();
                c.receipts
                    .insert("scope-dispatch-fixture".into(), dispatch_ref);
                c.reservations.push(ScopeReservation {
                    attempt_id: "child".into(),
                    paths: vec!["src".into()],
                });
                save(&store, &c).unwrap();
                assert!(failed_attempt_needs_repair(&c, &c.attempts["attempt"]));
                retry_prepared_batches(&store, "run", c.revision).unwrap();
                let mut saved = load_run(&store, "run").unwrap();
                let eligible = state == AttemptState::Prepared && purpose == Purpose::Dispatcher;
                assert_eq!(
                    saved.receipts.contains_key("retry-prepared-attempt"),
                    eligible,
                    "{state:?}/{purpose:?}"
                );
                assert_eq!(
                    failed_attempt_needs_repair(&saved, &saved.attempts["attempt"]),
                    !eligible
                );
                assert_eq!(saved.attempts["child"], child);
                assert_eq!(saved.reservations, c.reservations);
                if eligible {
                    let reference = saved.receipts["retry-prepared-attempt"].clone();
                    assert!(record_receipt(
                        &store,
                        "run",
                        saved.revision,
                        "retry-prepared-forged",
                        &reference
                    )
                    .is_err());
                    fs::write(&reference.path, b"tampered retry").unwrap();
                    assert!(failed_attempt_needs_repair(
                        &saved,
                        &saved.attempts["attempt"]
                    ));
                    saved.receipts.remove("retry-prepared-attempt");
                    assert!(failed_attempt_needs_repair(
                        &saved,
                        &saved.attempts["attempt"]
                    ));
                }
            }
        }
    }

    #[test]
    fn prepared_retry_requires_the_failed_coordinators_own_batch_callback() {
        let (_temp, store, mut c) = setup();
        c.phase = Phase::Engineering;
        let step_id = format!(
            "coord-batch-{}",
            &digest(&serde_json::to_vec(&vec!["unit"]).unwrap())[..16]
        );
        let spec = StepSpec {
            run_id: "run".into(),
            step_id: step_id.clone(),
            purpose: Purpose::Dispatcher,
            brief_ref: c.intent.clone(),
            scope: vec![],
            dependency_ids: vec![],
            check_ids: vec![],
        };
        c.steps.insert(step_id.clone(), spec.clone());
        let mut a = unknown_attempt();
        a.step_id = step_id.clone();
        a.state = AttemptState::Running;
        c.attempts.insert(a.attempt_id.clone(), a);
        fixture_model(&store, &mut c, "attempt", "test");
        c.steps.insert(
            "unit".into(),
            StepSpec {
                run_id: "run".into(),
                step_id: "unit".into(),
                purpose: Purpose::Engineer,
                brief_ref: c.intent.clone(),
                scope: vec!["src".into()],
                dependency_ids: vec![],
                check_ids: vec![],
            },
        );
        let child = Attempt {
            attempt_id: "child".into(),
            step_id: "unit".into(),
            input_digest: "child-input".into(),
            state: AttemptState::Prepared,
            generation: None,
            thread_id: None,
            turn_id: None,
            raw_result: None,
            outcome: None,
            last_event_sequence: 0,
        };
        c.attempts.insert("child".into(), child.clone());
        c.reservations.push(ScopeReservation {
            attempt_id: "child".into(),
            paths: vec!["src".into()],
        });
        save(&store, &c).unwrap();
        let caller = ScopeBatchCaller {
            attempt_id: "attempt".into(),
            generation: 1,
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            call_id: "call-a".into(),
        };
        let members = [("unit".into(), "child".into())];
        for field in ["attempt", "generation", "thread", "turn", "call"] {
            let mut wrong = caller.clone();
            match field {
                "attempt" => wrong.attempt_id = "child".into(),
                "generation" => wrong.generation = 2,
                "thread" => wrong.thread_id = "wrong-thread".into(),
                "turn" => wrong.turn_id = "wrong-turn".into(),
                _ => wrong.call_id.clear(),
            }
            assert!(
                register_scope_batch(&store, "run", c.revision, &members, &wrong).is_err(),
                "{field}"
            );
            assert_eq!(load_run(&store, "run").unwrap(), c);
        }
        let registered =
            register_scope_batch(&store, "run", c.revision, &members, &caller).unwrap();
        let report = serde_json::json!({"status":"failed","thread_id":"thread","turn_id":"turn","model":"test"});
        commit_result(
            &store,
            "run",
            registered.revision,
            "attempt",
            AttemptOutcome::Failed,
            &report,
        )
        .unwrap();
        let saved_a = load_run(&store, "run").unwrap();
        let mut legacy = saved_a.clone();
        legacy
            .receipts
            .retain(|key, _| !key.starts_with("scope-dispatch-"));
        save(&store, &legacy).unwrap();
        retry_prepared_batches(&store, "run", legacy.revision).unwrap();
        let blocked_legacy = load_run(&store, "run").unwrap();
        assert!(!blocked_legacy
            .receipts
            .contains_key("retry-prepared-attempt"));
        assert!(failed_attempt_needs_repair(
            &blocked_legacy,
            &blocked_legacy.attempts["attempt"]
        ));
        assert!(
            serde_json::from_slice::<ScopeBatch>(&artifact_bytes(&registered.value).unwrap())
                .is_ok(),
            "legacy batches still serve scope audits"
        );
        save(&store, &saved_a).unwrap();
        retry_prepared_batches(&store, "run", saved_a.revision).unwrap();
        let retired_a = load_run(&store, "run").unwrap();
        assert!(retired_a.receipts.contains_key("retry-prepared-attempt"));
        assert!(!failed_attempt_needs_repair(
            &retired_a,
            &retired_a.attempts["attempt"]
        ));
        let b = claim_step(&store, retired_a.revision, &spec, "input").unwrap();
        let revision = record_attempt_dispatch(
            &store,
            "run",
            b.revision,
            &AttemptEvent {
                event_id: "b-dispatch".into(),
                attempt_id: b.attempt.attempt_id.clone(),
                sequence: 1,
                kind: AttemptEventKind::Dispatching {
                    generation: 2,
                    thread_id: "thread-b".into(),
                },
            },
            "test",
            "test",
            Some("high"),
        )
        .unwrap();
        let revision = record_attempt_event(
            &store,
            "run",
            revision,
            &AttemptEvent {
                event_id: "b-running".into(),
                attempt_id: b.attempt.attempt_id.clone(),
                sequence: 2,
                kind: AttemptEventKind::Running {
                    turn_id: "turn-b".into(),
                },
            },
        )
        .unwrap();
        // B has the same StepSpec but failed before any dispatch_batch callback.
        commit_result(&store,"run",revision,&b.attempt.attempt_id,AttemptOutcome::Failed,
            &serde_json::json!({"status":"failed","thread_id":"thread-b","turn_id":"turn-b","model":"test"})).unwrap();
        for _ in 0..3 {
            let before = load_run(&store, "run").unwrap();
            retry_prepared_batches(&store, "run", before.revision).unwrap();
            let after = load_run(&store, "run").unwrap();
            assert_eq!(after, before);
            assert!(!after
                .receipts
                .contains_key(&format!("retry-prepared-{}", b.attempt.attempt_id)));
            assert!(failed_attempt_needs_repair(
                &after,
                &after.attempts[&b.attempt.attempt_id]
            ));
            assert_eq!(after.attempts["child"], child);
            assert_eq!(
                after
                    .receipts
                    .keys()
                    .filter(|k| k.starts_with("scope-dispatch-"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn baseline_gate_rejects_forged_missing_and_contradictory_evidence() {
        let (_temp, store, mut c) = setup();
        let plan = reference(&store.join("plan"), b"plan");
        let research = reference(&store.join("research"), b"research");
        let brief = reference(&store.join("brief"), b"brief");
        c.plan = Some(plan.clone());
        c.research = Some(research.clone());
        c.core_decision = Some(CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "approved".into(),
        });
        c.phase = Phase::Planning;
        let mut check = FrozenCheck {
            check_id: "check".into(),
            program: "verify".into(),
            args: vec![],
            env: BTreeMap::new(),
            timeout_s: 10,
            snapshot: Snapshot {
                source_digest: format!("tree-v2:{}:{}", "a".repeat(40), "b".repeat(64)),
                plan_digest: plan.digest.clone(),
                checks_digest: String::new(),
            },
            spec_digest: String::new(),
        };
        check.spec_digest = check_spec_digest(&check).unwrap();
        check.snapshot.checks_digest =
            digest(&serde_json::to_vec(&vec![("check", &check.spec_digest)]).unwrap());
        c.checks.insert("check".into(), check.clone());
        c.steps.insert(
            "baseline".into(),
            StepSpec {
                run_id: "run".into(),
                step_id: "baseline".into(),
                purpose: Purpose::Verifier,
                brief_ref: brief,
                scope: vec![],
                dependency_ids: vec![],
                check_ids: vec!["check".into()],
            },
        );
        let result = CheckResult {
            check_id: "check".into(),
            snapshot: check.snapshot,
            spec_digest: check.spec_digest,
            exit_code: Some(1),
            policy_applied: true,
            stdout: reference(&store.join("baseline-stdout"), b""),
            stderr: reference(&store.join("baseline-stderr"), b"product absent"),
            executor: ExecutorIdentity {
                attempt_id: "baseline-attempt".into(),
                thread_id: "baseline-thread".into(),
                model: "test-model".into(),
                purpose: Purpose::Verifier,
                policy: ExecutionPolicy::ScratchCheck,
            },
        };
        let result_ref = save_evidence(&store, "run", "baseline", "fixture", &result).unwrap();
        c.receipts
            .insert("baseline-check".into(), result_ref.clone());
        let make_report = |ready: bool, observations: Value| {
            serde_json::json!({
                "status":"completed", "model":"test-model", "thread_id":"baseline-thread", "turn_id":"baseline-turn",
                "structured_output":{"infrastructure_ready":ready,"summary":"test", "observations":observations}
            })
        };
        let valid = make_report(
            true,
            serde_json::json!([{"check_id":"check","classification":"product_missing"}]),
        );
        let mut attempt = unknown_attempt();
        attempt.attempt_id = "baseline-attempt".into();
        attempt.step_id = "baseline".into();
        attempt.thread_id = Some("baseline-thread".into());
        attempt.turn_id = Some("baseline-turn".into());
        attempt.state = AttemptState::Terminal;
        attempt.outcome = Some(AttemptOutcome::Completed);
        let mut report_ref = reference(&store.join("report"), &serde_json::to_vec(&valid).unwrap());
        attempt.raw_result = Some(report_ref.clone());
        c.attempts.insert(attempt.attempt_id.clone(), attempt);
        fixture_model(&store, &mut c, "baseline-attempt", "test-model");
        save(&store, &c).unwrap();

        for (index, report) in [
            make_report(
                false,
                serde_json::json!([{"check_id":"check","classification":"product_missing"}]),
            ),
            make_report(
                true,
                serde_json::json!([{"check_id":"check","classification":"passed"}]),
            ),
            make_report(
                true,
                serde_json::json!([{"check_id":"check","classification":"infrastructure_failure"}]),
            ),
            make_report(true, serde_json::json!([])),
            make_report(
                true,
                serde_json::json!([
                {"check_id":"check","classification":"product_missing"},
                {"check_id":"check","classification":"product_missing"}]),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            report_ref = reference(
                &store.join(format!("bad-report-{index}")),
                &serde_json::to_vec(&report).unwrap(),
            );
            c.attempts.get_mut("baseline-attempt").unwrap().raw_result = Some(report_ref.clone());
            save(&store, &c).unwrap();
            assert!(record_baseline_gate(&store, "run", c.revision, &report_ref).is_err());
            assert!(!load_run(&store, "run")
                .unwrap()
                .receipts
                .contains_key(BASELINE_GATE));
        }
        report_ref = reference(
            &store.join("valid-report"),
            &serde_json::to_vec(&valid).unwrap(),
        );
        c.attempts.get_mut("baseline-attempt").unwrap().raw_result = Some(report_ref.clone());
        save(&store, &c).unwrap();
        let forged = reference(&store.join("forged"), &serde_json::to_vec(&valid).unwrap());
        assert!(record_baseline_gate(&store, "run", c.revision, &forged).is_err());
        fs::write(&result_ref.path, b"tampered").unwrap();
        assert!(record_baseline_gate(&store, "run", c.revision, &report_ref).is_err());
        fs::write(&result_ref.path, serde_json::to_vec(&result).unwrap()).unwrap();
        fs::remove_file(&result.stderr.path).unwrap();
        assert!(record_baseline_gate(&store, "run", c.revision, &report_ref).is_err());
        fs::write(&result.stderr.path, b"product absent").unwrap();
        assert!(record_baseline_gate(&store, "run", c.revision, &report_ref).is_ok());
    }

    #[test]
    fn public_classification_uses_the_verified_plan_bytes() {
        let (_temp, store, mut c) = setup();
        let original = br#"{"checks":[{"id":"public"}]}"#;
        let replaced = br#"{"checks":[{"id":"public"},{"id":"independent-private"}]}"#;
        let plan = reference(&store.join("plan.json"), original);
        c.plan = Some(plan.clone());
        let snapshot = Snapshot {
            source_digest: "source".into(),
            plan_digest: plan.digest.clone(),
            checks_digest: "checks".into(),
        };
        for id in ["public", "independent-private"] {
            c.checks.insert(
                id.into(),
                FrozenCheck {
                    check_id: id.into(),
                    program: "check".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                    timeout_s: 10,
                    snapshot: snapshot.clone(),
                    spec_digest: "spec".into(),
                },
            );
        }
        let prep_bytes = serde_json::to_vec(&serde_json::json!({"structured_output": {
            "status":"ready", "exposure":"private",
            "checks":[{"id":"independent-private"}]
        }}))
        .unwrap();
        let prep = reference(&store.join("prep.json"), &prep_bytes);
        c.receipts.insert("evaluation-0".into(), prep.clone());
        let mut attempt = unknown_attempt();
        attempt.step_id = "evaluation-prep-0".into();
        attempt.state = AttemptState::Terminal;
        attempt.outcome = Some(AttemptOutcome::Completed);
        attempt.raw_result = Some(prep);
        c.attempts.insert(attempt.attempt_id.clone(), attempt);

        let target = plan.path.canonicalize().unwrap();
        AFTER_ARTIFACT_VERIFICATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                if path == target {
                    fs::write(&target, replaced).unwrap();
                }
            }));
        });
        let public = public_check_ids(&store, &c).unwrap();
        clear_hook();
        assert_eq!(public, BTreeSet::from(["public".to_string()]));
        assert!(
            public_check_ids(&store, &c).is_err(),
            "later reads reject the replaced plan"
        );
    }

    #[test]
    fn terminal_recovery_consumes_only_the_verified_report() {
        let (temp, store, mut c) = setup();
        mark_unknown(&store, &mut c);
        c.steps.insert(
            "step".into(),
            StepSpec {
                run_id: "run".into(),
                step_id: "step".into(),
                purpose: Purpose::Content,
                brief_ref: c.intent.clone(),
                scope: vec![],
                dependency_ids: vec![],
                check_ids: vec![],
            },
        );
        fixture_model(&store, &mut c, "attempt", "test");
        save(&store, &c).unwrap();
        let original = br#"{"attempt_id":"attempt","thread_id":"thread","generation":1,"turn_id":"turn","model":"test","outcome":"failed"}"#;
        let replacement = br#"{"attempt_id":"attempt","thread_id":"thread","generation":1,"turn_id":"turn","model":"test","outcome":"completed"}"#;
        let report = reference(&store.join("candidate.json"), original);

        // A valid artifact outside the store/repository must still be rejected.
        let outside = reference(&temp.path().join("outside.json"), original);
        assert!(resolve_attempt(
            &store,
            "run",
            c.revision,
            &RecoveryDecision::AcceptTerminal {
                attempt_id: "attempt".into(),
                report: outside,
                reason: "reviewed".into(),
            }
        )
        .is_err());
        assert_eq!(
            load_run(&store, "run").unwrap().attempts["attempt"].state,
            AttemptState::OutcomeUnknown
        );

        let target = report.path.canonicalize().unwrap();
        AFTER_ARTIFACT_VERIFICATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                if path == target {
                    fs::write(&target, replacement).unwrap();
                }
            }));
        });
        resolve_attempt(
            &store,
            "run",
            c.revision,
            &RecoveryDecision::AcceptTerminal {
                attempt_id: "attempt".into(),
                report,
                reason: "reviewed".into(),
            },
        )
        .unwrap();
        clear_hook();
        let saved = load_run(&store, "run").unwrap();
        let attempt = &saved.attempts["attempt"];
        assert_eq!(attempt.outcome, Some(AttemptOutcome::Failed));
        assert_eq!(
            fs::read(&attempt.raw_result.as_ref().unwrap().path).unwrap(),
            original
        );
        assert!(saved.reservations.is_empty());
    }

    #[test]
    fn repair_recovery_preserves_the_verified_evidence_bytes() {
        let (_temp, store, mut c) = setup();
        mark_unknown(&store, &mut c);
        let stop_original = br#"{"attempt_id":"attempt","thread_id":"thread","generation":1,"turn_id":"turn","terminal":true}"#;
        let repair_original = br#"{"attempt_id":"attempt","repaired":true}"#;
        let stop = reference(&store.join("stop-input.json"), stop_original);
        let repair = reference(&store.join("repair-input.json"), repair_original);
        let stop_path = stop.path.canonicalize().unwrap();
        let repair_path = repair.path.canonicalize().unwrap();
        AFTER_ARTIFACT_VERIFICATION.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |path| {
                if path == stop_path || path == repair_path {
                    fs::write(path, b"replaced").unwrap();
                }
            }));
        });
        resolve_attempt(
            &store,
            "run",
            c.revision,
            &RecoveryDecision::RetryAfterRepair {
                attempt_id: "attempt".into(),
                stopped_evidence: stop.clone(),
                repair_evidence: repair.clone(),
                reason: "reviewed".into(),
            },
        )
        .unwrap();
        clear_hook();
        let saved = load_run(&store, "run").unwrap();
        let attempt = &saved.attempts["attempt"];
        assert_eq!(attempt.outcome, Some(AttemptOutcome::Cancelled));
        let saved_stop = attempt.raw_result.as_ref().unwrap();
        let saved_repair = &saved.receipts["repair-attempt"];
        assert_ne!(saved_stop.path, stop.path);
        assert_ne!(saved_repair.path, repair.path);
        assert_eq!(fs::read(&saved_stop.path).unwrap(), stop_original);
        assert_eq!(fs::read(&saved_repair.path).unwrap(), repair_original);
        assert_eq!(saved_stop.digest, stop.digest);
        assert_eq!(saved_repair.digest, repair.digest);
        assert!(saved.reservations.is_empty());
    }

    #[test]
    fn verified_outcome_rejects_modified_or_missing_check_output() {
        let (temp, store, mut c) = setup();
        let repo = c.repo.canonical_root.clone();
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join("source"), b"source").unwrap();
        let plan = reference(&store.join("plan"), b"plan");
        let research = reference(&store.join("research"), b"research");
        let brief = reference(&store.join("brief"), b"brief");
        c.plan = Some(plan.clone());
        c.research = Some(research.clone());
        c.core_decision = Some(CoreDecision {
            research_digest: research.digest,
            pass: true,
            reason: "approved".into(),
        });
        let mut check = FrozenCheck {
            check_id: "check".into(),
            program: "check".into(),
            args: vec![],
            env: BTreeMap::new(),
            timeout_s: 10,
            snapshot: Snapshot {
                source_digest: String::new(),
                plan_digest: String::new(),
                checks_digest: String::new(),
            },
            spec_digest: String::new(),
        };
        check.spec_digest = check_spec_digest(&check).unwrap();
        check.snapshot = frozen_snapshot(&repo, &plan, &[check.clone()]).unwrap();
        c.checks.insert("check".into(), check.clone());
        let step = |id: &str, purpose| StepSpec {
            run_id: "run".into(),
            step_id: id.into(),
            purpose,
            brief_ref: brief.clone(),
            scope: vec!["source".into()],
            dependency_ids: vec![],
            check_ids: vec!["check".into()],
        };
        c.steps.insert(
            "verification".into(),
            step("verification", Purpose::Verifier),
        );
        let mut verifier = unknown_attempt();
        verifier.step_id = "verification".into();
        verifier.thread_id = Some("verifier-thread".into());
        verifier.turn_id = Some("verifier-turn".into());
        verifier.state = AttemptState::Running;
        c.attempts.insert(verifier.attempt_id.clone(), verifier);
        let output_dir = store.join("output");
        fs::create_dir(&output_dir).unwrap();
        let stdout = reference(&output_dir.join("stdout"), b"expected output");
        let stderr = reference(&output_dir.join("stderr"), b"");
        let baseline_result = CheckResult {
            check_id: "check".into(),
            snapshot: check.snapshot.clone(),
            spec_digest: check.spec_digest.clone(),
            exit_code: Some(1),
            policy_applied: true,
            stdout: reference(&output_dir.join("baseline-stdout"), b""),
            stderr: reference(&output_dir.join("baseline-stderr"), b"not implemented"),
            executor: ExecutorIdentity {
                attempt_id: "baseline-attempt".into(),
                thread_id: "baseline-thread".into(),
                model: "test".into(),
                purpose: Purpose::Verifier,
                policy: ExecutionPolicy::ScratchCheck,
            },
        };
        let baseline_receipt =
            save_evidence(&store, "run", "baseline", "test", &baseline_result).unwrap();
        c.receipts.insert("baseline-check".into(), baseline_receipt);
        let report = serde_json::json!({
            "status":"completed", "model":"test", "thread_id":"baseline-thread", "turn_id":"baseline-turn",
            "structured_output":{"infrastructure_ready":true,"summary":"product missing",
                "observations":[{"check_id":"check","classification":"product_missing"}]}
        });
        let baseline_report = reference(
            &store.join("baseline-report"),
            &serde_json::to_vec(&report).unwrap(),
        );
        let mut baseline_attempt = unknown_attempt();
        baseline_attempt.attempt_id = "baseline-attempt".into();
        baseline_attempt.step_id = "baseline".into();
        baseline_attempt.thread_id = Some("baseline-thread".into());
        baseline_attempt.turn_id = Some("baseline-turn".into());
        baseline_attempt.state = AttemptState::Terminal;
        baseline_attempt.outcome = Some(AttemptOutcome::Completed);
        baseline_attempt.raw_result = Some(baseline_report.clone());
        c.steps
            .insert("baseline".into(), step("baseline", Purpose::Verifier));
        c.attempts
            .insert("baseline-attempt".into(), baseline_attempt);
        fixture_model(&store, &mut c, "baseline-attempt", "test");
        fixture_model(&store, &mut c, "attempt", "test");
        c.receipts.insert(BASELINE_GATE.into(), baseline_report);
        c.phase = Phase::Verifying;
        save(&store, &c).unwrap();
        let executor = ExecutorIdentity {
            attempt_id: "attempt".into(),
            thread_id: "verifier-thread".into(),
            model: "test".into(),
            purpose: Purpose::Verifier,
            policy: ExecutionPolicy::ScratchCheck,
        };
        let result = CheckResult {
            check_id: "check".into(),
            snapshot: check.snapshot.clone(),
            spec_digest: check.spec_digest.clone(),
            exit_code: Some(0),
            policy_applied: true,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            executor: executor.clone(),
        };
        let revision = record_check(&store, "run", c.revision, &result).unwrap();
        let terminal = commit_result(&store, "run", revision, "attempt", AttemptOutcome::Completed,
            &serde_json::json!({"status":"completed", "model":"test", "thread_id":"verifier-thread", "turn_id":"verifier-turn",
                "structured_output":{"pass":true,"summary":"checked","defect_owner":"none"}})).unwrap();
        let review = IndependentReview {
            review_id: "attempt".into(),
            snapshot: check.snapshot,
            executor: ExecutorIdentity {
                policy: ExecutionPolicy::ProductReadOnly,
                ..executor
            },
            check_ids: vec!["check".into()],
            pass: true,
            report: terminal.value,
        };
        record_independent_review(&store, "run", terminal.revision, &review).unwrap();
        let mut c = load_run(&store, "run").unwrap();
        c.steps
            .insert("assembly".into(), step("assembly", Purpose::Assembler));
        let mut assembly_attempt = unknown_attempt();
        assembly_attempt.attempt_id = "assembly-attempt".into();
        assembly_attempt.step_id = "assembly".into();
        assembly_attempt.thread_id = Some("assembly-thread".into());
        assembly_attempt.turn_id = Some("assembly-turn".into());
        assembly_attempt.state = AttemptState::Running;
        assembly_attempt.outcome = None;
        assembly_attempt.raw_result = None;
        c.attempts
            .insert(assembly_attempt.attempt_id.clone(), assembly_attempt);
        fixture_model(&store, &mut c, "assembly-attempt", "test");
        c.phase = Phase::Assembling;
        save(&store, &c).unwrap();
        let actual_contents = crate::orch::lifecycle::contents(&repo).unwrap();
        let (assembly, _, outside) = commit_scoped_result(&store, "run", c.revision, "assembly-attempt",
            AttemptOutcome::Completed, &serde_json::json!({"status":"completed","thread_id":"assembly-thread","turn_id":"assembly-turn","model":"test","file_changes":[]}),
            None, &output_dir, &actual_contents, &actual_contents).unwrap();
        assert!(!outside);
        record_receipt(
            &store,
            "run",
            assembly.revision,
            "assembly",
            &assembly.value,
        )
        .unwrap();
        let mut c = load_run(&store, "run").unwrap();
        c.phase = Phase::Verifying;
        assert!(verified_outcome(&c));
        let ready = |checkpoint: &Checkpoint| {
            let mut checkpoint = checkpoint.clone();
            checkpoint.phase = Phase::ReadyToCommit;
            checkpoint.receipts.insert("memory".into(), brief.clone());
            ready_evidence(&checkpoint)
        };
        assert!(ready(&c));

        // Both outcome gates rely on the review's complete check validation.
        // Hash-valid replacement receipts must not weaken that requirement.
        for field in ["exit", "policy", "spec", "source", "model", "thread"] {
            let mut changed = result.clone();
            match field {
                "exit" => changed.exit_code = Some(1),
                "policy" => changed.policy_applied = false,
                "spec" => changed.spec_digest = "changed".into(),
                "source" => changed.snapshot.source_digest = "changed".into(),
                "model" => changed.executor.model = "changed".into(),
                "thread" => changed.executor.thread_id = "changed".into(),
                _ => unreachable!(),
            }
            let receipt = save_evidence(&store, "run", "check", field, &changed).unwrap();
            let mut altered = c.clone();
            altered.receipts.insert("check-check".into(), receipt);
            assert!(!verified_outcome(&altered), "accepted changed {field}");
            assert!(!ready(&altered), "ready accepted changed {field}");
        }
        fs::write(repo.join("source"), b"changed").unwrap();
        assert!(!verified_outcome(&c));
        assert!(!ready(&c));
        fs::write(repo.join("source"), b"source").unwrap();
        for key in ["check-check", "review-attempt"] {
            let path = &c.receipts[key].path;
            let original = fs::read(path).unwrap();
            fs::write(path, b"tampered").unwrap();
            assert!(!verified_outcome(&c), "accepted tampered {key}");
            assert!(!ready(&c), "ready accepted tampered {key}");
            fs::write(path, original).unwrap();
        }
        assert!(verified_outcome(&c));
        assert!(ready(&c));

        // Even a hash-valid replacement raw report must agree with the typed
        // review decision and attribution. A real failed assembly review is
        // still valid evidence for rework; upstream is never assembly rework.
        for (declared, owner, recorded, valid) in [
            (true, "assembly", true, false),
            (false, "assembly", true, false),
            (false, "assembly", false, true),
            (false, "upstream", false, true),
        ] {
            let raw = serde_json::json!({"status":"completed", "model":"test", "thread_id":"verifier-thread", "turn_id":"verifier-turn",
                "structured_output":{"pass":declared,"summary":"checked","defect_owner":owner}});
            let raw = reference(
                &store.join(format!("variant-{declared}-{owner}-{recorded}")),
                &serde_json::to_vec(&raw).unwrap(),
            );
            let mut altered = c.clone();
            altered.attempts.get_mut("attempt").unwrap().raw_result = Some(raw.clone());
            let mut candidate = review.clone();
            candidate.report = raw;
            candidate.pass = recorded;
            assert_eq!(validate_review(&altered, &store, &candidate).is_ok(), valid);
        }
        let mut missing = c.clone();
        missing.receipts.remove("check-check");
        assert!(validate_review(&missing, &store, &review).is_err());
        let mut wrong_attempt = result.clone();
        wrong_attempt.executor.attempt_id = "baseline-attempt".into();
        let wrong_receipt =
            save_evidence(&store, "run", "check", "wrong-attempt", &wrong_attempt).unwrap();
        let mut wrong = c.clone();
        wrong.receipts.insert("check-check".into(), wrong_receipt);
        assert!(validate_review(&wrong, &store, &review).is_err());
        let mut stale = c.clone();
        stale
            .receipts
            .insert("rework-1".into(), c.receipts["assembly"].clone());
        assert!(
            verified_review(&stale).is_none(),
            "an old verification cannot authorize a new rework epoch"
        );
        let mut ambiguous = c.clone();
        let mut duplicate = ambiguous.attempts["attempt"].clone();
        duplicate.attempt_id = "duplicate".into();
        ambiguous
            .attempts
            .insert(duplicate.attempt_id.clone(), duplicate);
        assert!(
            verified_review(&ambiguous).is_none(),
            "ambiguous verifier attempts cannot authorize acceptance"
        );

        fs::write(&stdout.path, b"changed").unwrap();
        assert!(!ready(&c));
        assert!(
            !verified_outcome(&c),
            "modified stdout cannot support acceptance"
        );
        fs::write(&stdout.path, b"expected output").unwrap();
        assert!(verified_outcome(&c));
        fs::write(&stderr.path, b"changed").unwrap();
        assert!(!ready(&c));
        assert!(
            !verified_outcome(&c),
            "modified stderr cannot support acceptance"
        );
        fs::remove_file(&stderr.path).unwrap();
        assert!(!ready(&c));
        assert!(
            !verified_outcome(&c),
            "missing output cannot support acceptance"
        );
        fs::write(&stderr.path, b"").unwrap();
        assert!(verified_outcome(&c));

        #[cfg(any(unix, windows))]
        {
            // Equal bytes outside the two authorized roots must not become
            // readable evidence merely because their digests happen to match.
            let outside = temp.path().join("outside-output");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join("stdout"), b"expected output").unwrap();
            fs::write(outside.join("stderr"), b"").unwrap();
            let saved_output = store.join("output-saved");
            fs::rename(&output_dir, &saved_output).unwrap();
            directory_link(&output_dir, &outside);
            assert!(
                !verified_outcome(&c),
                "an output link cannot escape its roots"
            );
            #[cfg(unix)]
            fs::remove_file(&output_dir).unwrap();
            #[cfg(windows)]
            fs::remove_dir(&output_dir).unwrap();
            fs::rename(saved_output, &output_dir).unwrap();
            assert!(verified_outcome(&c));

            // Even an in-store junction to another evidence directory cannot
            // be used to infer an authority root for the check receipt.
            let evidence = store.join("runs/run/evidence");
            let saved_evidence = store.join("runs/run/evidence-saved");
            fs::rename(&evidence, &saved_evidence).unwrap();
            directory_link(&evidence, &saved_evidence);
            assert!(
                !verified_outcome(&c),
                "a linked evidence directory is not authoritative"
            );
        }
    }
}
