use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
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
wire_enum!(AttemptOutcome { Completed, Failed, Cancelled });
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
fn artifact(a: &ArtifactRef) -> Res<()> {
    nonempty(&a.digest)?;
    let bytes = std::fs::read(&a.path).map_err(io)?;
    if digest(&bytes) != a.digest {
        return Err(fail("artifact digest mismatch"));
    }
    Ok(())
}
fn bounded_artifact(store: &Path, repo: &Path, value: &ArtifactRef) -> Res<()> {
    artifact(value)?;
    let path = std::fs::canonicalize(&value.path).map_err(io)?;
    let root = std::fs::canonicalize(store).map_err(io)?;
    if !path.starts_with(root) && !path.starts_with(repo) {
        return Err(fail("artifact outside repository and store"));
    }
    Ok(())
}
fn snapshot_ok(s: &Snapshot) -> Res<()> {
    nonempty(&s.source_digest)?;
    nonempty(&s.plan_digest)?;
    nonempty(&s.checks_digest)
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
    if source.is_empty() {
        return Err(fail("cannot snapshot repository"));
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
        if matches!(spec.purpose, Purpose::Engineer | Purpose::Assembler)
            && (!approved_research(c) || c.plan.is_none())
        {
            return Err(fail(
                "engineering requires approved research and an installed plan",
            ));
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
            return Ok((
                if a.state == AttemptState::Terminal {
                    ClaimDisposition::ExistingTerminal
                } else {
                    ClaimDisposition::ExistingActive
                },
                a.clone(),
            ));
        }
        if c.attempts
            .values()
            .any(|a| a.step_id == spec.step_id && a.state != AttemptState::Terminal)
        {
            return Err(fail("active attempt has different input"));
        }
        for dep in &spec.dependency_ids {
            if !c.attempts.values().any(|a| {
                &a.step_id == dep
                    && a.state == AttemptState::Terminal
                    && a.outcome == Some(AttemptOutcome::Completed)
            }) {
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
    Ok(mutate(store, id, rev, |c| {
        let key = format!("event-{}", event.event_id);
        if let Some(old) = c.receipts.get(&key) {
            artifact(old)?;
            if std::fs::read(&old.path).map_err(io)? == serde_json::to_vec(event).map_err(io)? {
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
        let reference = save_evidence(store, id, "event", &event.event_id, event)?;
        c.receipts.insert(key, reference);
        Ok(())
    })?
    .revision)
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
pub(crate) fn successful_check(c: &Checkpoint, id: &str) -> bool {
    let Some(reference) = c.receipts.get(&format!("check-{id}")) else {
        return false;
    };
    if artifact(reference).is_err() {
        return false;
    }
    let Ok(bytes) = std::fs::read(&reference.path) else {
        return false;
    };
    let Ok(result) = serde_json::from_slice::<CheckResult>(&bytes) else {
        return false;
    };
    c.checks.get(id).is_some_and(|f| {
        result.check_id == id
            && result.snapshot == f.snapshot
            && result.spec_digest == f.spec_digest
            && check_spec_digest(f).ok().as_deref() == Some(f.spec_digest.as_str())
            && c.plan
                .as_ref()
                .is_some_and(|p| p.digest == f.snapshot.plan_digest && artifact(p).is_ok())
            && super::snapshot(&c.repo.canonical_root, &[".".into()]) == f.snapshot.source_digest
            && result.policy_applied
            && result.exit_code == Some(0)
    })
}
fn valid_review(c: &Checkpoint) -> bool {
    c.receipts
        .iter()
        .filter(|(k, _)| k.starts_with("review-"))
        .any(|(_, r)| {
            if artifact(r).is_err() {
                return false;
            }
            let Ok(bytes) = std::fs::read(&r.path) else {
                return false;
            };
            let Ok(review) = serde_json::from_slice::<IndependentReview>(&bytes) else {
                return false;
            };
            review.pass
                && review.executor.purpose == Purpose::Verifier
                && review.executor.policy == ExecutionPolicy::ProductReadOnly
                && !review.check_ids.is_empty()
                && review.check_ids.iter().all(|id| {
                    successful_check(c, id)
                        && c.checks
                            .get(id)
                            .is_some_and(|f| f.snapshot == review.snapshot)
                })
                && artifact(&review.report).is_ok()
        })
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
        && !c.checks.is_empty()
        && c.checks.keys().all(|id| successful_check(c, id))
        && valid_review(c)
        && assembly_evidence(c)
        && c.receipts
            .get("memory")
            .is_some_and(|r| artifact(r).is_ok())
}
pub fn verified_outcome(c: &Checkpoint) -> bool {
    approved_research(c)
        && assembly_evidence(c)
        && !c.checks.is_empty()
        && c.checks.keys().all(|id| successful_check(c, id))
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
                a.state == AttemptState::Terminal
                    && a.outcome == Some(AttemptOutcome::Completed)
                    && a.raw_result.as_ref() == Some(r)
                    && c.steps
                        .get(&a.step_id)
                        .is_some_and(|s| s.purpose == Purpose::Assembler)
            })
    })
}
pub fn record_check(store: &Path, id: &str, rev: u64, result: &CheckResult) -> Res<u64> {
    artifact(&result.stdout)?;
    artifact(&result.stderr)?;
    snapshot_ok(&result.snapshot)?;
    Ok(mutate(store, id, rev, |c| {
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
        let a = c
            .attempts
            .get(&result.executor.attempt_id)
            .ok_or_else(|| fail("unknown verifier"))?;
        if a.thread_id.as_deref() != Some(&result.executor.thread_id)
            || c.steps.get(&a.step_id).map(|s| &s.purpose) != Some(&Purpose::Verifier)
            || result.executor.model.trim().is_empty()
        {
            return Err(fail("verifier identity mismatch"));
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
    artifact(&review.report)?;
    snapshot_ok(&review.snapshot)?;
    Ok(mutate(store, id, rev, |c| {
        bounded_artifact(store, &c.repo.canonical_root, &review.report)?;
        if review.executor.purpose != Purpose::Verifier
            || review.executor.policy != ExecutionPolicy::ProductReadOnly
            || review.executor.thread_id.is_empty()
            || review.executor.model.is_empty()
            || review.check_ids.is_empty()
        {
            return Err(fail("invalid review isolation or result"));
        }
        let a = c
            .attempts
            .get(&review.executor.attempt_id)
            .ok_or_else(|| fail("unknown reviewer"))?;
        if a.thread_id.as_deref() != Some(&review.executor.thread_id)
            || c.steps.get(&a.step_id).map(|s| &s.purpose) != Some(&Purpose::Verifier)
        {
            return Err(fail("review identity mismatch"));
        }
        if c.attempts.values().any(|x| {
            x.attempt_id != a.attempt_id
                && x.thread_id.as_ref() == a.thread_id.as_ref()
                && c.steps
                    .get(&x.step_id)
                    .is_some_and(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))
        }) {
            return Err(fail("review thread is not independent"));
        }
        for check in &review.check_ids {
            let f = c
                .checks
                .get(check)
                .ok_or_else(|| fail("unknown review check"))?;
            if f.snapshot != review.snapshot || (review.pass && !successful_check(c, check)) {
                return Err(fail("review snapshot/check mismatch"));
            }
        }
        if super::snapshot(&c.repo.canonical_root, &[".".into()]) != review.snapshot.source_digest {
            return Err(fail("review source snapshot changed"));
        }
        let r = save_evidence(store, id, "review", &review.review_id, review)?;
        c.receipts.insert(format!("review-{}", review.review_id), r);
        Ok(())
    })?
    .revision)
}
/// Return a known, independently rejected assembly to SOL without changing
/// scope or frozen commands. Prior failures remain immutable evidence.
pub fn prepare_rework(store: &Path, id: &str, rev: u64) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        let count = c.receipts.keys().filter(|k| k.starts_with("rework-")).count();
        if count >= 2 || c.phase != Phase::Verifying || !c.reservations.is_empty()
            || c.attempts.values().any(|a| a.state != AttemptState::Terminal)
            || !c.receipts.iter().filter(|(k, _)| k.starts_with("review-")).any(|(_, r)| {
                artifact(r).is_ok() && std::fs::read(&r.path).ok()
                    .and_then(|b| serde_json::from_slice::<IndependentReview>(&b).ok())
                    .is_some_and(|r| !r.pass)
            })
        {
            return Err(fail("rework requires a known rejected review; at most two repairs"));
        }
        let evidence: BTreeMap<_, _> = c.receipts.iter()
            .filter(|(k, _)| (k.starts_with("check-") && !k.starts_with("check-independent-")) || k.starts_with("review-"))
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

pub fn advance_phase(store: &Path, id: &str, rev: u64, phase: Phase) -> Res<u64> {
    Ok(mutate(store, id, rev, |c| {
        let allowed = match (&c.phase, &phase) {
            (Phase::Research, Phase::IntentReview) => c.research.is_some(),
            (Phase::IntentReview, Phase::Planning) => c.core_decision.as_ref().is_some_and(|d| {
                d.pass
                    && c.research
                        .as_ref()
                        .is_some_and(|r| r.digest == d.research_digest)
            }),
            (Phase::Planning, Phase::Engineering) => c.plan.is_some() && !c.steps.is_empty(),
            (Phase::Engineering, Phase::Assembling) => c
                .steps
                .values()
                .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Content))
                .all(|s| {
                    c.attempts.values().any(|a| {
                        a.step_id == s.step_id && a.outcome == Some(AttemptOutcome::Completed)
                    })
                }),
            (Phase::Assembling, Phase::Verifying) => c.steps.values().any(|s| {
                s.purpose == Purpose::Assembler
                    && c.attempts.values().any(|a| {
                        a.step_id == s.step_id && a.outcome == Some(AttemptOutcome::Completed)
                    })
            }),
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
            (Phase::ReadyToCommit, Phase::Released) => c
                .receipts
                .get("release")
                .is_some_and(|r| artifact(r).is_ok()),
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
                bounded_artifact(store, &c.repo.canonical_root, &pending.report)?;
                let bytes = std::fs::read(&pending.report.path).map_err(io)?;
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
                let _: Value = serde_json::from_slice(&bytes).map_err(io)?;
                let outcome: AttemptOutcome =
                    serde_json::from_slice(&std::fs::read(outcome_path).map_err(io)?)
                        .map_err(io)?;
                finish_result(store, id, c, &attempt_id, outcome, &bytes)?;
            }
        }
        for a in c.attempts.values() {
            let o = obs.iter().find(|o| o.attempt_id == a.attempt_id);
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
            artifact(report)?;
            let bytes = std::fs::read(&report.path).map_err(io)?;
            let value: Value = serde_json::from_slice(&bytes).map_err(io)?;
            Ok(mutate(store, id, rev, |c| {
                bounded_artifact(store, &c.repo.canonical_root, report)?;
                let a = c
                    .attempts
                    .get(attempt_id)
                    .ok_or_else(|| fail("unknown attempt"))?;
                if a.state != AttemptState::OutcomeUnknown
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
            artifact(stopped_evidence)?;
            artifact(repair_evidence)?;
            if stopped_evidence == repair_evidence {
                return Err(fail("separate stop and repair evidence required"));
            }
            Ok(mutate(store, id, rev, |c| {
                bounded_artifact(store, &c.repo.canonical_root, stopped_evidence)?;
                bounded_artifact(store, &c.repo.canonical_root, repair_evidence)?;
                let a = c
                    .attempts
                    .get_mut(attempt_id)
                    .ok_or_else(|| fail("unknown attempt"))?;
                if a.state != AttemptState::OutcomeUnknown {
                    return Err(fail("attempt is not unknown"));
                }
                let stop: Value =
                    serde_json::from_slice(&std::fs::read(&stopped_evidence.path).map_err(io)?)
                        .map_err(io)?;
                if stop.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
                    || stop.get("terminal").and_then(Value::as_bool) != Some(true)
                    || stop.get("thread_id").and_then(Value::as_str) != a.thread_id.as_deref()
                    || stop.get("generation").and_then(Value::as_u64) != a.generation
                    || stop.get("turn_id").and_then(Value::as_str) != a.turn_id.as_deref()
                {
                    return Err(fail("stop evidence must identify a terminal attempt"));
                }
                let repair: Value =
                    serde_json::from_slice(&std::fs::read(&repair_evidence.path).map_err(io)?)
                        .map_err(io)?;
                if repair.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
                    || repair.get("repaired").and_then(Value::as_bool) != Some(true)
                {
                    return Err(fail("repair evidence must identify repaired attempt"));
                }
                a.state = AttemptState::Terminal;
                a.outcome = Some(AttemptOutcome::Cancelled);
                a.raw_result = Some(stopped_evidence.clone());
                c.reservations.retain(|r| r.attempt_id != *attempt_id);
                c.receipts
                    .insert(format!("repair-{attempt_id}"), repair_evidence.clone());
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

pub fn failed_attempt_needs_repair(c: &Checkpoint, a: &Attempt) -> bool {
    a.outcome == Some(AttemptOutcome::Failed)
        && !c
            .receipts
            .contains_key(&format!("planning-retired-{}", a.attempt_id))
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
        || kind.starts_with("review-")
        || kind.starts_with("repair-")
        || kind.starts_with("rework-")
        || kind.starts_with("intent-rejection-")
        || kind.starts_with("rejected-research-")
        || kind.starts_with("event-")
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
