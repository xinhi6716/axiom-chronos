//! Versioned, replay-validated checkpoints. Local store/CLI access is trusted;
//! actor labels are provenance, not cryptographic authentication. Host enforces
//! core authority, and final release additionally requires actual unit evidence.
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub task: String,
    pub module: String,
    pub round: u32,
    /// Legacy task metadata retained for evidence compatibility, not product policy.
    #[serde(default = "default_min_rounds")]
    pub min_rounds: u32,
    pub owner: String,
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub dependency_versions: BTreeMap<String, u64>,
    pub outputs: Vec<String>,
    pub evidence: Vec<String>,
    pub next_action: String,
    pub issue: String,
    pub proposal: String,
    pub evidence_version: u64,
    pub contract: String,
    pub repo: String,
    pub severity: String,
    pub findings: Vec<String>,
}
fn default_min_rounds() -> u32 {
    5
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub task: String,
    pub round: u32,
    pub issue: String,
    pub proposal: String,
    pub evidence_version: u64,
    pub kind: String,
    pub evidence: String,
    pub actor: String,
    pub dispatch_id: String,
    pub contract: String,
    pub artifact: String,
}
/// Policy journal only. A finished run still needs the separate product barrier;
/// neither a unit release nor this journal is whole-product release authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductRun {
    pub id: String,
    pub rounds: Vec<ProductRound>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductRound {
    pub round: u32,
    /// Only execution evidence appended after this explicit checkpoint counts.
    pub event_start: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_start: Option<Vec<Checkpoint>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engineering_start: Option<String>,
    pub transition: Option<ProductTransition>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductTransition {
    pub verification_ids: Vec<String>,
    pub assessment: String,
    pub action: String,
    pub event_end: usize,
    #[serde(default)]
    pub checkpoint_end: Option<Vec<Checkpoint>>,
    #[serde(default)]
    pub engineering_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitState {
    pub schema_version: u32,
    /// Sticky: removing units.tsv must never switch a registered journal to legacy.
    #[serde(default)]
    pub engineering_enrolled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_archive_hash: Option<String>,
    #[serde(default)]
    pub product_runs: Vec<ProductRun>,
    pub checkpoints: Vec<Checkpoint>,
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<EngineeringOperation>,
}
/// Execution metadata belongs to the existing checkpoint journal, never a second
/// scheduler. A saved boundary has no writer; an abandoned executing phase is
/// uncertain even if its process died. Only explicit dispatch recovery clears it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringOperation {
    pub id: String,
    pub parent: String,
    pub task: String,
    pub round: u32,
    pub owner: u32,
    pub owner_generation: u64,
    pub owner_token: String,
    pub worker_lease: String,
    pub deadline_ms: u64,
    pub phase: String,
    pub executing: bool,
    pub outcome: String,
    pub source: String,
    pub initial_source: String,
    pub receipts: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_failure: Option<serde_json::Value>,
    pub outbox: Vec<serde_json::Value>,
}
impl Default for WaitState {
    fn default() -> Self {
        Self {
            schema_version: 2,
            engineering_enrolled: false,
            legacy_archive_hash: None,
            product_runs: vec![],
            checkpoints: vec![],
            events: vec![],
            operations: vec![],
        }
    }
}
fn invalid(s: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", s)
}
fn latest<'a>(state: &'a WaitState, task: &str) -> Option<&'a Checkpoint> {
    state
        .checkpoints
        .iter()
        .filter(|c| c.task == task)
        .max_by_key(|c| c.round)
}
fn bound(e: &Event, c: &Checkpoint) -> bool {
    e.task == c.task
        && e.round == c.round
        && e.issue == c.issue
        && e.proposal == c.proposal
        && e.evidence_version == c.evidence_version
        && e.contract == c.contract
}
fn dispatch<'a>(state: &'a WaitState, c: &Checkpoint) -> Option<&'a Event> {
    state
        .events
        .iter()
        .rev()
        .find(|e| bound(e, c) && e.kind == "dispatch")
}
fn approved(state: &WaitState, c: &Checkpoint) -> bool {
    has(state,c,"core_approve") || has(state,c,"test_approve")
}
pub fn current_checkpoint(state: &WaitState, task: &str) -> Option<Checkpoint> {
    latest(state, task).cloned()
}
fn pending_review<'a>(state: &'a WaitState, c: &Checkpoint) -> Option<&'a Event> {
    state.events.iter().rev().find(|e| {
        bound(e, c)
            && e.kind == "start_review"
            && !state.events.iter().any(|d| {
                (d.kind == "dispatch" && d.id == e.dispatch_id)
                    || (d.kind == "recovery" && d.dispatch_id == e.dispatch_id)
            })
    })
}
fn has(state: &WaitState, c: &Checkpoint, kind: &str) -> bool {
    let Some(d) = dispatch(state, c) else {
        return false;
    };
    state
        .events
        .iter()
        .any(|e| bound(e, c) && e.kind == kind && e.dispatch_id == d.id)
}
pub(crate) fn read(store: &Path) -> Res<WaitState> {
    require_registry(store)?;
    let state: WaitState = match std::fs::read(store.join("core-wait.json")) {
        Ok(data) => serde_json::from_slice(&data)
            .map_err(|e| invalid(format!("invalid core-wait state: {e}")))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if store.join("wait-required").exists() {
                return Err(invalid("enrolled core-wait journal is missing"));
            }
            return Ok(WaitState::default());
        }
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    require_state_registry(store, &state)?;
    if state.schema_version == 1 {
        return Err(invalid("legacy core-wait schema requires explicit wait migrate; unit releases are not product rounds"));
    }
    if state.schema_version != 2 {
        return Err(invalid("unsupported core-wait schema"));
    }
    replay_stored(store, &state)?;
    let mut ids = BTreeSet::new();
    for op in &state.operations {
        if !ids.insert(&op.id) || op.parent.is_empty() || op.owner_token.is_empty()
            || op.owner_generation == 0 || op.deadline_ms == 0
            || !state.events.iter().any(|e| e.id == op.id && e.kind == "dispatch"
                && e.task == op.task && e.round == op.round)
            || !["build", "assembly", "host_test", "package", "verification", "acceptance", "done"].contains(&op.phase.as_str())
            || !["running", "blocked", "awaiting_core", "failed", "timeout", "cancelled", "unconfirmed", "protocol_incomplete"].contains(&op.outcome.as_str())
        { return Err(invalid("invalid durable operation binding")); }
        let phases = ["build", "assembly", "host_test", "package", "verification"];
        let count = phases.iter().take_while(|p| op.receipts.contains_key(**p)).count();
        if op.receipts.len() != count
            || (op.phase != if count < phases.len() { phases[count] } else { "acceptance" }
                && !(op.phase == "done" && count == phases.len() && op.outcome == "awaiting_core")) {
            return Err(invalid("operation skipped a required phase"));
        }
        if let Some(report) = &op.terminal_failure {
            if !["build", "assembly"].contains(&op.phase.as_str())
                || report["binding"]["attempt"] != op.id
                || report["task"] != op.task || report["round"] != op.round
                || !report["turn_id"].as_str().is_some_and(|s| !s.is_empty())
                || report["error"]["kind"] == "turn_unconfirmed"
                || !["completed", "failed", "interrupted", "cancelled", "timeout"].contains(&report["status"].as_str().unwrap_or("")) {
                return Err(invalid("invalid terminal failure receipt"));
            }
        }
        for phase in phases.iter().filter(|p| **p != "host_test") {
            if let Some(receipt) = op.receipts.get(*phase) {
                let event = state.events.iter().find(|e| e.dispatch_id == op.id && e.kind == *phase)
                    .ok_or_else(|| invalid("operation receipt lacks authoritative checkpoint event"))?;
                let mut saved: serde_json::Value = serde_json::from_str(event.evidence.trim_start_matches("host:"))
                    .map_err(|_| invalid("invalid operation event receipt"))?;
                let mut receipt = receipt.clone();
                saved.as_object_mut().ok_or_else(|| invalid("invalid operation report"))?.remove("binding");
                receipt.as_object_mut().ok_or_else(|| invalid("invalid phase receipt"))?.remove("binding");
                if saved != receipt { return Err(invalid("operation receipt differs from checkpoint authority")); }
            }
        }
        let mut deliveries = BTreeSet::new();
        for delivery in &op.outbox {
            if delivery["operation_id"] != op.id || delivery["parent_id"] != op.parent
                || !delivery["delivery_id"].as_str().is_some_and(|id| !id.is_empty() && deliveries.insert(id)) {
                return Err(invalid("invalid operation outbox identity"));
            }
        }
    }
    Ok(state)
}
// The exemption belongs only to byte-identified schema-1 history, never to
// later events or product transitions. Missing/changed archives fail closed.
fn replay_stored(store: &Path, state: &WaitState) -> Res<()> {
    use sha2::Digest;
    let legacy = if let Some(hash) = &state.legacy_archive_hash {
        let bytes = std::fs::read(store.join("core-wait.v1.json"))
            .map_err(|_| invalid("missing legacy archive"))?;
        if *hash != format!("sha256:{:x}", sha2::Sha256::digest(&bytes)) {
            return Err(invalid("legacy archive identity mismatch"));
        }
        let old: WaitState =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid legacy archive"))?;
        if old.schema_version != 1
            || old.legacy_archive_hash.is_some()
            || !old.product_runs.is_empty()
            || !state.checkpoints.starts_with(&old.checkpoints)
            || !state.events.starts_with(&old.events)
        {
            return Err(invalid("legacy archive history mismatch"));
        }
        Some(old.events.len())
    } else {
        None
    };
    replay_state(state, legacy.unwrap_or(0))
}
fn replay_state(state: &WaitState, legacy_events: usize) -> Res<()> {
    // Replay checkpoints and events in round order. Old formats fail closed;
    // unknown/corrupted transitions must never accidentally enable dispatch.
    let mut replay = WaitState::default();
    for c in &state.checkpoints {
        validate_checkpoint(&replay, c)?;
        replay.checkpoints.push(c.clone());
        for (index, e) in state
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.task == c.task && e.round == c.round)
        {
            validate_event_policy(&replay, e, true, index < legacy_events)?;
            replay.events.push(e.clone());
        }
    }
    if replay.events.len() != state.events.len() {
        return Err(invalid("orphan event"));
    }
    validate_products(state)?;
    Ok(())
}
pub fn wait_state(store: &Path) -> Res<WaitState> {
    super::with_lock(store, false, || read(store))
}

#[cfg(test)]
mod outbox_race_tests {
    use super::*;
    #[test]
    fn concurrent_notification_creation_is_atomic_idempotent_and_bound() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let c: Checkpoint = serde_json::from_value(serde_json::json!({
            "task":"t","module":"m","round":1,"min_rounds":5,"owner":"terra",
            "dependencies":[],"outputs":["out"],"evidence":[],"next_action":"verdict",
            "issue":"i","proposal":"p","evidence_version":1,"contract":"contract",
            "repo":"","severity":"normal","findings":[]
        })).unwrap();
        checkpoint(store, c.clone()).unwrap();
        commit_dispatch(store, "t").unwrap();
        let attempt = wait_state(store).unwrap().events[0].id.clone();
        for (kind, actor) in [("build", "worker"), ("assembly", "sol"), ("verification", "astra-verify")] {
            let mut e = new_event(&c, kind, actor, "synthetic legacy fixture, not live execution");
            e.id = format!("fixture-{kind}");
            e.dispatch_id = attempt.clone();
            e.artifact = "fixture-artifact".into();
            wait_event(store, e).unwrap();
        }
        let barrier = std::sync::Barrier::new(16);
        let replies = std::thread::scope(|scope| {
            let jobs: Vec<_> = (0..16).map(|_| scope.spawn(|| {
                barrier.wait();
                core_notices(store).unwrap()
            })).collect();
            jobs.into_iter().map(|j| j.join().unwrap()).collect::<Vec<_>>()
        });
        assert_eq!(replies[0].len(), 1);
        for reply in &replies { assert_eq!(reply, &replies[0]); }
        let state = wait_state(store).unwrap();
        assert_eq!(state.events.iter().filter(|e| e.kind == "notification").count(), 1);
        let e = &replies[0][0];
        assert_eq!(e.dispatch_id, attempt);
        let body: serde_json::Value = serde_json::from_str(e.evidence.strip_prefix("host:").unwrap()).unwrap();
        assert_eq!(body["verification_event"], "fixture-verification");
        assert_eq!(body["binding"]["attempt"], attempt);
        // Delivery never implies a verdict; only the correct bound core can ack it.
        assert!(core_verdict(store, "t", "stale-verification", true, "fixture").is_err());
        assert_eq!(core_notices(store).unwrap(), replies[0]);
    }
}
/// Commit the journal before the enrollment marker, under the same store lock.
pub(crate) fn enroll(store: &Path) -> Res<WaitState> {
    super::with_lock(store, false, || {
        super::read_units(store)?;
        let mut state = read(store)?;
        state.engineering_enrolled = true;
        save(store, &mut state)?;
        base::atomic_write(&store.join("wait-required"), b"1\n")
            .map_err(|e| Failure::new("io", e.to_string()))?;
        Ok(state)
    })
}
fn save(store: &Path, state: &mut WaitState) -> Res<()> {
    require_state_registry(store, state)?;
    // In particular, refreshing a prepared checkpoint must not silently rewrite
    // the archived prefix and leave a journal that fails its next read.
    if state.legacy_archive_hash.is_some() {
        replay_stored(store, state)?;
    }
    state.engineering_enrolled |=
        store.join("units.tsv").is_file() || store.join("wait-required").exists();
    let bytes = serde_json::to_vec(state).map_err(|e| invalid(e.to_string()))?;
    base::atomic_write(&store.join("core-wait.json"), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))
}
fn nonempty(s: &str) -> bool {
    !s.trim().is_empty()
}
impl EngineeringOperation {
    pub fn reply(&self) -> serde_json::Value {
        self.outbox.last().cloned().unwrap_or_else(|| serde_json::json!({
            "status":"unconfirmed", "transport_status":"unconfirmed", "outcome":"unconfirmed",
            "operation_id":self.id,"parent_id":self.parent,"unit":self.task,"round":self.round,
            "owner_generation":self.owner_generation,"stage":self.phase,
            "error":{"kind":"unconfirmed","message":"operation has no committed delivery"}
        }))
    }
}
fn operation_delivery(op: &mut EngineeringOperation, error: Option<&Failure>) {
    let outcome = op.outcome.as_str();
    let status = match outcome {
        "awaiting_core" => "completed", "blocked" => "blocked",
        "timeout" => "timeout", "cancelled" => "interrupted", "unconfirmed" => "unconfirmed",
        _ => "failed",
    };
    let typed: base::DomainOutcome = serde_json::from_value(serde_json::json!(outcome))
        .unwrap_or(base::DomainOutcome::ProtocolIncomplete);
    let release = op.receipts.get("package").map(|p| serde_json::json!({
        "source_identity":p["source_identity"],"package_hash":p["package_hash"],
        "package_path":p["package_path"],"verification_receipt":p["verification_receipt"]["path"]
    }));
    let delivery = serde_json::json!({"status":status,"transport_status":if outcome == "unconfirmed" { "unconfirmed" } else { "completed" },
        "outcome":typed,"state":typed,"operation_id":op.id,"attempt":op.id,
        "parent_id":op.parent,"unit":op.task,"round":op.round,
        "owner_generation":op.owner_generation,"stage":op.phase,"deadline_ms":op.deadline_ms,
        "source_identity":op.source,"delivery_id":format!("{}:{}:{}",op.id,op.outbox.len(),op.phase),
        "release_identity":release,
        "blocking_cause":error.map(Failure::to_json).unwrap_or_else(|| serde_json::json!(if outcome == "blocked" { "saved stage boundary" } else { "" })),
        "next":if outcome == "blocked" && !op.executing { "resume the same operation at its saved stage boundary" } else if outcome == "awaiting_core" { "bound core verdict required; not a release" } else if op.executing { "writer termination or phase completion must be independently established; do not redispatch" } else { "terminal failure recorded; explicit same-operation continuation is not available; do not redispatch" },
        "error":error.map(Failure::to_json)});
    op.outbox.push(delivery);
}
/// Returns None only for a new attempt; otherwise claims only a saved
/// boundary. Terminal deliveries replay verbatim, without launching any model.
pub(crate) fn resume_operation(store: &Path, task: &str, source: &str) -> Res<Option<EngineeringOperation>> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        let c = latest(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
        let Some(d) = dispatch(&state, c) else { return Ok(None) };
        if has(&state, c, "recovery") {
            return Err(invalid("recovered attempt cannot be implicitly redispatched; explicit new-operation identity is required"));
        }
        let id = d.id.clone();
        if !dependencies_current(&state, c) { return Err(invalid("operation dependencies changed")); }
        let Some(op) = state.operations.iter_mut().find(|op| op.id == id) else { return Ok(None) };
        if op.outcome == "awaiting_core" && op.source != source {
            return Err(invalid("completed operation source changed; prior evidence is not current acceptance"));
        }
        if op.outcome == "running" {
            if base::proc::pid_alive(op.owner) {
                return Err(Failure::new("busy", "operation already has an owner; no second writer"));
            }
            op.outcome = "unconfirmed".into();
            operation_delivery(op, Some(&Failure::new("turn_unconfirmed", "abandoned phase; independently confirm every writer stopped before recovery")));
        } else if op.outcome == "blocked" && !op.executing {
            if base::proc::unix_ms() >= op.deadline_ms {
                op.outcome = "timeout".into();
                operation_delivery(op, Some(&Failure::new("timeout", "persisted operation budget exhausted")));
            } else if op.source != source {
                return Err(invalid("saved operation source changed; explicit recovery required"));
            } else {
                // A crash can leave the local lease after the terminal stage
                // receipt was saved. Reap ONLY that exact dead owner's lease;
                // never another task's or an unknown writer's claim.
                match std::fs::read_to_string(store.join(".worker.claim")) {
                    Ok(token) => {
                        if token != op.worker_lease || base::proc::pid_alive(op.owner) {
                            return Err(Failure::new("busy", "saved boundary still has a live or foreign lease"));
                        }
                        std::fs::remove_file(store.join(".worker.claim")).map_err(|e| Failure::new("io", e.to_string()))?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(Failure::new("io", e.to_string())),
                }
                op.owner = std::process::id();
                op.owner_generation += 1;
                op.owner_token = base::unique();
                op.outcome = "running".into();
            }
        }
        let result = op.clone();
        save(store, &mut state)?;
        Ok(Some(result))
    })
}
pub(crate) fn open_operation(store: &Path, task: &str, parent: &str, budget_s: u64, source: &str) -> Res<EngineeringOperation> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        let c = latest(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
        let d = dispatch(&state, c).ok_or_else(|| invalid("missing dispatch"))?;
        if state.operations.iter().any(|op| op.id == d.id) { return Err(invalid("operation already exists")); }
        let op = EngineeringOperation { id:d.id.clone(),parent:parent.into(),task:task.into(),round:c.round,
            owner:std::process::id(),owner_generation:1,owner_token:base::unique(),
            worker_lease:std::fs::read_to_string(store.join(".worker.claim")).map_err(|e| Failure::new("io", e.to_string()))?,
            deadline_ms:base::proc::unix_ms().saturating_add(budget_s.saturating_mul(1000)),
            phase:"build".into(),executing:false,outcome:"running".into(),source:source.into(),initial_source:source.into(),receipts:BTreeMap::new(),terminal_failure:None,outbox:vec![] };
        state.operations.push(op.clone());
        save(store, &mut state)?;
        Ok(op)
    })
}
pub(crate) fn save_operation(store: &Path, op: &mut EngineeringOperation, deliver: bool, error: Option<&Failure>) -> Res<()> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        let old = state.operations.iter_mut().find(|o| o.id == op.id).ok_or_else(|| invalid("missing operation"))?;
        if old.owner_token != op.owner_token || old.owner_generation != op.owner_generation {
            return Err(invalid("stale operation owner generation"));
        }
        if old.deadline_ms != op.deadline_ms || old.initial_source != op.initial_source
            || old.parent != op.parent || old.task != op.task || old.round != op.round {
            return Err(invalid("immutable operation identity or budget changed"));
        }
        if deliver { operation_delivery(op, error); }
        *old = op.clone();
        save(store, &mut state)
    })
}
pub fn dispatch_ready(state: &WaitState, task: &str) -> bool {
    match latest(state, task) {
        None => true,
        Some(c) => {
            dependencies_current(state, c)
                && (dispatch(state, c).is_none() || has(state, c, "recovery"))
        }
    }
}
fn new_event(c: &Checkpoint, kind: &str, actor: &str, evidence: &str) -> Event {
    Event {
        id: format!("{kind}-{}", base::unique()),
        task: c.task.clone(),
        round: c.round,
        issue: c.issue.clone(),
        proposal: c.proposal.clone(),
        evidence_version: c.evidence_version,
        kind: kind.into(),
        evidence: evidence.into(),
        actor: actor.into(),
        dispatch_id: String::new(),
        contract: c.contract.clone(),
        artifact: String::new(),
    }
}
pub fn commit_dispatch(store: &Path, task: &str) -> Res<()> {
    commit_dispatch_bound(store, task, None)
}
pub(crate) fn commit_dispatch_bound(store: &Path, task: &str, contract: Option<&str>) -> Res<()> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        let Some(c) = latest(&state, task) else {
            if store.join("wait-required").exists() {
                return Err(invalid("opted-in workflow requires checkpoint enrollment"));
            }
            return Ok(());
        };
        if contract.is_some_and(|identity| identity != c.contract)
            || (store.join("units.tsv").exists()
                && super::contract_identity(store, task)? != c.contract)
        {
            return Err(invalid("dispatch contract changed"));
        }
        // Reacquire the local scope lease at a saved stage boundary. This is
        // NOT a new dispatch and cannot make an unknown writer runnable.
        if dispatch(&state, c).is_some_and(|d| state.operations.iter().any(|op|
            op.id == d.id && op.owner == std::process::id() && !op.executing
                && op.outcome == "running" && dependencies_current(&state, c)
                && !has(&state, c, "recovery") && !has(&state, c, "release"))) {
            return Ok(());
        }
        let mut e = new_event(
            c,
            "dispatch",
            "lifecycle",
            "durable dispatch claim; launch may be uncertain",
        );
        if store.join("units.tsv").exists() && c.round >= 6 {
            let review = pending_review(&state, c)
                .ok_or_else(|| invalid("missing actual pre-dispatch Astra MAX review"))?;
            let report: serde_json::Value =
                serde_json::from_str(review.evidence.trim_start_matches("host:"))
                    .map_err(|_| invalid("invalid pre-review report"))?;
            if report["source_identity"] != super::snapshot(Path::new(&c.repo), &[".".into()])
                || review.artifact
                    != super::snapshot(Path::new(&c.repo), &super::unit_brief(store, task)?.scope)
            {
                return Err(invalid(
                    "pre-dispatch MAX review source changed; explicit recovery required",
                ));
            }
            e.id = review.dispatch_id.clone();
        }
        e.dispatch_id = e.id.clone();
        validate_event(&state, &e, true)?;
        state.events.push(e);
        save(store, &mut state)
    })
}
pub fn task_released(state: &WaitState, task: &str) -> bool {
    latest(state, task).is_none() || may_finish(state, task)
}
fn validate_checkpoint(state: &WaitState, c: &Checkpoint) -> Res<()> {
    if [
        &c.task,
        &c.module,
        &c.owner,
        &c.next_action,
        &c.issue,
        &c.proposal,
        &c.contract,
    ]
    .iter()
    .any(|s| !nonempty(s))
        || c.round == 0
        || c.min_rounds == 0
        || c.evidence_version == 0
        || !["normal", "severe", "excellent"].contains(&c.severity.as_str())
        || c.dependencies
            .iter()
            .chain(c.outputs.iter())
            .chain(c.evidence.iter())
            .chain(c.findings.iter())
            .any(|s| !nonempty(s))
    {
        return Err(invalid("invalid checkpoint fields"));
    }
    if c.dependency_versions.len() != c.dependencies.len()
        || c.dependencies.iter().any(|dep| {
            dep == &c.task
                || latest(state, dep).is_none()
                || !may_finish(state, dep)
                || latest(state, dep)
                    .is_some_and(|d| c.dependency_versions.get(dep) != Some(&d.evidence_version))
        })
    {
        return Err(invalid("dependency not finally released"));
    }
    let mut pending = c.dependencies.clone();
    let mut visited = BTreeSet::new();
    while let Some(dep) = pending.pop() {
        if dep == c.task {
            return Err(invalid("checkpoint dependency cycle"));
        }
        if visited.insert(dep.clone()) {
            if let Some(d) = latest(state, &dep) {
                pending.extend(d.dependencies.clone());
            }
        }
    }
    match latest(state, &c.task) {
        None if c.round != 1 => return Err(invalid("first round must be 1")),
        Some(old)
            if c.round != old.round + 1
                || !(has(state, old, "release") || has(state, old, "test_reject"))
                || c.evidence_version <= old.evidence_version
                || c.module != old.module =>
        {
            return Err(invalid(
                "previous round not released or stale version/module",
            ))
        }
        Some(old) if c.min_rounds != old.min_rounds => {
            return Err(invalid("minimum rounds cannot change for a task"));
        }
        _ => (),
    }
    Ok(())
}
pub fn checkpoint(store: &Path, c: Checkpoint) -> Res<WaitState> {
    super::with_lock(store, false, || checkpoint_locked(store, read(store)?, c))
}

fn checkpoint_locked(store: &Path, mut state: WaitState, c: Checkpoint) -> Res<WaitState> {
    if store.join("wait-required").exists() && !store.join("units.tsv").exists() {
        return Err(invalid(
            "opted-in workflow requires a registered unit store",
        ));
    }
    validate_checkpoint(&state, &c)?;
    if store.join("units.tsv").exists() {
        base::require_directory(&c.repo)?;
        let units = super::read_units(store)?;
        let u = units
            .iter()
            .find(|u| u.id == c.task)
            .ok_or_else(|| invalid("unknown unit"))?;
        if u.track != c.module || super::contract_identity(store, &c.task)? != c.contract {
            return Err(invalid("checkpoint unit/contract mismatch"));
        }
        let brief = super::unit_brief(store, &c.task)?;
        if brief.delivery.is_none() || brief.outputs.as_ref() != Some(&c.outputs) {
            return Err(invalid("checkpoint DELIVERY/OUTPUTS mismatch"));
        }
    }
    state.checkpoints.push(c);
    save(store, &mut state)?;
    Ok(state)
}

// Caller holds the store lock from reading the registered brief through commit.
pub(crate) fn prepare_locked(
    store: &Path,
    mut state: WaitState,
    mut c: Checkpoint,
) -> Res<WaitState> {
    let brief = super::unit_brief(store, &c.task)?;
    if brief.delivery.is_none() || brief.outputs.as_ref() != Some(&c.outputs) {
        return Err(invalid("new engineering contract requires matching DELIVERY/OUTPUTS"));
    }
    if store.join(".worker.claim").exists() || store.join("quarantine").exists() {
        return Err(invalid("execution claim or quarantine exists"));
    }
    if let Some(old) = latest(&state, &c.task).cloned() {
        if c.min_rounds != old.min_rounds {
            return Err(invalid("minimum rounds cannot change for a task"));
        }
        let released = has(&state, &old, "release") || has(&state,&old,"test_reject");
        if released {
            c.round = old
                .round
                .checked_add(1)
                .ok_or_else(|| invalid("round overflow"))?;
        } else {
            if state
                .events
                .iter()
                .any(|e| e.task == old.task && e.round == old.round)
            {
                return Err(invalid("prepared round has execution or claim evidence"));
            }
            if c.module != old.module {
                return Err(invalid("prepared round module cannot change"));
            }
            c.round = old.round;
            c.evidence_version = old.evidence_version;
            if c == old {
                return Ok(state);
            }
            let index = state
                .checkpoints
                .iter()
                .position(|x| x.task == old.task && x.round == old.round)
                .unwrap();
            state.checkpoints.remove(index);
        }
        c.evidence_version = old
            .evidence_version
            .checked_add(1)
            .ok_or_else(|| invalid("evidence version overflow"))?;
    }
    checkpoint_locked(store, state, c)
}
fn validate_event(state: &WaitState, e: &Event, internal: bool) -> Res<()> {
    validate_event_policy(state, e, internal, false)
}
fn validate_event_policy(state: &WaitState, e: &Event, internal: bool, legacy: bool) -> Res<()> {
    if !nonempty(&e.id) || !nonempty(&e.evidence) || state.events.iter().any(|x| x.id == e.id) {
        return Err(invalid("event id/evidence missing or duplicate"));
    }
    let c = latest(state, &e.task)
        .filter(|c| c.round == e.round)
        .ok_or_else(|| invalid("not current checkpoint"))?;
    if !bound(e, c) {
        return Err(invalid("event binding mismatch"));
    }
    if e.kind == "review_intent" {
        if !internal
            || c.round < 6
            || !dispatch_ready(state, &c.task)
            || state.events.iter().any(|x| {
                bound(x, c)
                    && x.kind == "review_intent"
                    && !state
                        .events
                        .iter()
                        .any(|r| r.kind == "recovery" && r.dispatch_id == x.dispatch_id)
            })
            || e.dispatch_id.is_empty()
        {
            return Err(invalid(
                "pre-review intent already claimed; manual recovery required",
            ));
        }
        return Ok(());
    }
    if e.kind == "recovery" && dispatch(state, c).is_none() {
        if !internal
            || e.actor != "core"
            || !state
                .events
                .iter()
                .any(|i| bound(i, c) && i.kind == "review_intent" && i.dispatch_id == e.dispatch_id)
            || state
                .events
                .iter()
                .any(|i| i.kind == "recovery" && i.dispatch_id == e.dispatch_id)
        {
            return Err(invalid("invalid pre-review recovery"));
        }
        return Ok(());
    }
    if e.kind != "recovery" && !dependencies_current(state, c) {
        return Err(invalid("checkpoint dependency version/release changed"));
    }
    // A real MAX review is persisted BEFORE the writing dispatch claim. Its
    // proposed attempt id is consumed by exactly one dispatch, including replay.
    if e.kind == "start_review" && e.evidence.starts_with("host:") {
        if !internal
            || c.round < 6
            || e.actor != "reviewer"
            || !dispatch_ready(state, &c.task)
            || pending_review(state, c).is_some()
            || e.dispatch_id.is_empty()
            || state.events.iter().any(|d| d.id == e.dispatch_id)
        {
            return Err(invalid("invalid or late pre-dispatch review"));
        }
        validate_host_report(e, "astra", "max")?;
        if !state
            .events
            .iter()
            .any(|i| bound(i, c) && i.kind == "review_intent" && i.dispatch_id == e.dispatch_id)
        {
            return Err(invalid("missing durable pre-review intent"));
        }
        return Ok(());
    }
    if e.kind == "dispatch" {
        if !internal
            || e.actor != "lifecycle"
            || e.dispatch_id != e.id
            || !dispatch_ready(state, &c.task)
        {
            return Err(Failure::new(
                "not_ready",
                "round already dispatched or invalid dispatch",
            ));
        }
        return Ok(());
    }
    let d = dispatch(state, c).ok_or_else(|| invalid("missing dispatch"))?;
    if e.dispatch_id != d.id
        || has(state, c, "recovery")
        || has(state, c, "release")
        || has(state, c, &e.kind)
    {
        return Err(invalid("stale, duplicate or late event"));
    }
    let (actor, valid) = match e.kind.as_str() {
        "recovery" => ("core", internal && !has(state, c, "completion")),
        "start_review" => ("reviewer", c.round >= 6),
        "build" => ("worker", c.round < 6 || has(state, c, "start_review")),
        "assembly" => ("sol", has(state, c, "build")),
        "assembly_intent" => ("lifecycle", internal && has(state, c, "build")),
        "verification_intent" => ("lifecycle", internal && has(state, c, "package")),
        "package_intent" => ("lifecycle", internal && has(state, c, "assembly")),
        "verification" => (
            "astra-verify",
            has(state, c, "assembly") && nonempty(&e.artifact),
        ),
        "notification" => ("lifecycle", internal && has(state, c, "verification")),
        "package" => ("lifecycle", internal && has(state, c, "assembly")),
        "suspended" => ("lifecycle", internal),
        "core_approve" | "core_reject" => (
            "core",
            has(state, c, "verification")
                && !approved(state, c)
                && !has(state, c, "core_reject") && !has(state,c,"test_reject"),
        ),
        "test_approve" | "test_reject" => ("lifecycle", internal && has(state,c,"verification") && !approved(state,c) && !has(state,c,"test_reject") && !has(state,c,"core_reject")),
        "completion" => (if has(state,c,"test_approve") { "lifecycle" } else { "core" }, approved(state, c)),
        "release" => (if has(state,c,"test_approve") { "lifecycle" } else { "core" }, has(state, c, "completion")),
        _ => ("", false),
    };
    if !valid || e.actor != actor {
        return Err(invalid("invalid authority or early transition"));
    }
    if matches!(e.kind.as_str(),"test_approve" | "test_reject") {
        let v = state.events.iter().find(|v| bound(v,c) && v.dispatch_id == d.id && v.kind == "verification")
            .ok_or_else(|| invalid("missing test evidence"))?;
        let report: serde_json::Value = serde_json::from_str(v.evidence.strip_prefix("host:").ok_or_else(|| invalid("test approval requires Host evidence"))?)
            .map_err(|_| invalid("invalid test evidence"))?;
        validate_host_report(v,"astra","high")?;
        let issues=report["structured_output"]["issues"].as_array().ok_or_else(||invalid("missing test findings"))?;
        if (e.kind=="test_approve") != issues.is_empty() || report["structured_output"]["severity"] != "normal" {
            return Err(invalid("test findings must return to dispatcher, never automatic acceptance"));
        }
    }
    if e.evidence.starts_with("host:") {
        match e.kind.as_str() {
            "build" => validate_host_report(e, "terra", "high")?,
            "assembly" => {
                if !has(state, c, "assembly_intent") {
                    return Err(invalid("missing assembly intent"));
                }
                validate_host_report(e, "sol", "high")?;
            }
            "verification" => {
                if !has(state, c, "verification_intent") {
                    return Err(invalid("missing verification intent"));
                }
                validate_host_report(e, "astra", "high")?;
                let package = state
                    .events
                    .iter()
                    .find(|p| bound(p, c) && p.dispatch_id == e.dispatch_id && p.kind == "package")
                    .ok_or_else(|| invalid("missing package"))?;
                let p: serde_json::Value =
                    serde_json::from_str(package.evidence.trim_start_matches("host:"))
                        .map_err(|_| invalid("invalid package"))?;
                let v: serde_json::Value =
                    serde_json::from_str(e.evidence.trim_start_matches("host:"))
                        .map_err(|_| invalid("invalid verification"))?;
                if p["package_hash"].as_str().is_none_or(|s| !nonempty(s))
                    || v["package_hash"] != p["package_hash"]
                    || v["source_identity"] != p["source_identity"]
                    || package.artifact != e.artifact
                {
                    return Err(invalid("package/source verification mismatch"));
                }
            }
            _ => (),
        }
    }
    if ["core_approve", "test_approve", "test_reject", "core_reject", "completion", "release"].contains(&e.kind.as_str()) {
        let v = state
            .events
            .iter()
            .find(|v| bound(v, c) && v.dispatch_id == d.id && v.kind == "verification")
            .unwrap();
        if e.artifact != v.artifact {
            return Err(invalid("verdict artifact mismatch"));
        }
        if ["core_approve", "core_reject"].contains(&e.kind.as_str())
            && v.evidence.starts_with("host:")
        {
            let report: serde_json::Value = serde_json::from_str(&v.evidence[5..])
                .map_err(|_| invalid("invalid actual verification report"))?;
            assess_core_verdict_policy(&c.severity, &e.evidence, &report, legacy)?;
        }
    }
    Ok(())
}
pub fn wait_event(store: &Path, event: Event) -> Res<WaitState> {
    event_impl(store, event, false)
}
fn event_impl(store: &Path, event: Event, internal: bool) -> Res<WaitState> {
    let state = super::with_lock(store, false, || {
        let mut state = read(store)?;
        validate_event(&state, &event, internal)?;
        if store.join("units.tsv").exists() {
            if [
                "review_intent",
                "assembly_intent",
                "package_intent",
                "verification_intent",
                "start_review",
                "build",
                "assembly",
                "package",
                "verification",
                "notification",
                "suspended",
            ]
            .contains(&event.kind.as_str())
                && !internal
            {
                return Err(invalid(
                    "registered progress requires actual host dispatch results",
                ));
            }
            match event.kind.as_str() {
                "build" => validate_host_report(&event, "terra", "high")?,
                "assembly" => validate_host_report(&event, "sol", "high")?,
                "verification" => {
                    validate_host_report(&event, "astra", "high")?;
                    let c = latest(&state, &event.task).unwrap();
                    if !has(&state, c, "package") {
                        return Err(invalid("missing Rust package"));
                    }
                }
                "core_approve" | "core_reject" => {
                    let c = latest(&state, &event.task).unwrap();
                    if !has(&state, c, "notification") {
                        return Err(invalid("core has not been notified"));
                    }
                    let verification = state
                        .events
                        .iter()
                        .find(|v| {
                            bound(v, c)
                                && v.dispatch_id == event.dispatch_id
                                && v.kind == "verification"
                        })
                        .unwrap();
                    let report: serde_json::Value = serde_json::from_str(
                        verification
                            .evidence
                            .strip_prefix("host:")
                            .ok_or_else(|| invalid("missing actual verification report"))?,
                    )
                    .map_err(|_| invalid("invalid actual verification report"))?;
                    assess_core_verdict(&c.severity, &event.evidence, &report)?;
                }
                _ => (),
            }
            if super::contract_identity(store, &event.task)? != event.contract {
                return Err(invalid("stale contract"));
            }
            if ["verification", "core_approve", "test_approve", "completion", "release"]
                .contains(&event.kind.as_str())
            {
                let rows = super::evidence(store, &event.task)?;
                let c = latest(&state, &event.task).unwrap();
                let run = rows
                    .iter()
                    .rposition(|r| r.kind == "run")
                    .ok_or_else(|| invalid("missing actual run"))?;
                if !rows[run + 1..]
                    .iter()
                    .any(|r| r.kind == "dispatch" && r.command == event.dispatch_id)
                    || super::snapshot(
                        Path::new(&c.repo),
                        &super::unit_brief(store, &event.task)?.scope,
                    ) != event.artifact
                {
                    return Err(invalid(
                        "stale artifact or actual run belongs to another dispatch",
                    ));
                }
                if !rows
                    .iter()
                    .rev()
                    .find(|r| r.kind == "verify")
                    .is_some_and(|r| r.exit_code == 0 && r.commit == event.artifact)
                    || !super::evidence::current_contract(store, &event.task, &rows)?
                {
                    return Err(invalid("missing matching actual verification"));
                }
                let package = state
                    .events
                    .iter()
                    .find(|p| {
                        bound(p, c) && p.dispatch_id == event.dispatch_id && p.kind == "package"
                    })
                    .ok_or_else(|| invalid("missing bound package"))?;
                let p: serde_json::Value =
                    serde_json::from_str(package.evidence.trim_start_matches("host:"))
                        .map_err(|_| invalid("invalid package evidence"))?;
                if p["source_identity"] != super::snapshot(Path::new(&c.repo), &[".".into()]) {
                    return Err(invalid("package source changed"));
                }
                let bytes = std::fs::read(
                    p["package_path"]
                        .as_str()
                        .ok_or_else(|| invalid("missing package path"))?,
                )
                .map_err(|_| invalid("package unavailable"))?;
                use sha2::Digest;
                if p["package_hash"] != format!("sha256:{:x}", sha2::Sha256::digest(bytes)) {
                    return Err(invalid("package bytes changed"));
                }
                if event.kind == "release"
                    && super::derive_state(&rows, super::needs_accept(store, &event.task)?)
                        != "verified"
                {
                    return Err(invalid(
                        "release requires independent review and acceptance",
                    ));
                }
            }
        }
        state.events.push(event.clone());
        save(store, &mut state)?;
        Ok(state)
    })?;
    // A crash between checkpoint and cached TSV update is safe: gates use the
    // checkpoint as authority and a repeated reconciliation restores the cache.
    reconcile_release(store, &state)?;
    Ok(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreVerdict {
    valid_issues: Vec<String>,
    severity: String,
    #[serde(default)]
    decision: Option<String>,
}
// Core's valid_issues are canonical unresolved root-cause IDs, not raw finding
// counts. Only IDs present in the bound independent report count toward R1-02.
fn assess_core_verdict(severity: &str, evidence: &str, report: &serde_json::Value) -> Res<bool> {
    assess_core_verdict_policy(severity, evidence, report, false)
}
fn assess_core_verdict_policy(
    severity: &str,
    evidence: &str,
    report: &serde_json::Value,
    legacy: bool,
) -> Res<bool> {
    let verdict: CoreVerdict = serde_json::from_str(evidence)
        .map_err(|_| invalid("core verdict requires valid_issues and severity"))?;
    if !["normal", "severe", "excellent"].contains(&verdict.severity.as_str())
        || !matches!(
            verdict.decision.as_deref(),
            None | Some("continue" | "finish")
        )
        || (verdict.severity != "normal" && verdict.decision.is_none())
        || verdict.valid_issues.iter().any(|s| !nonempty(s))
        || verdict.valid_issues.iter().collect::<BTreeSet<_>>().len() != verdict.valid_issues.len()
    {
        return Err(invalid("invalid core verdict assessment"));
    }
    let findings = report["structured_output"]["issues"]
        .as_array()
        .ok_or_else(|| invalid("missing verification issues"))?;
    if (severity != "normal" || report["structured_output"]["severity"] != "normal")
        && verdict.decision.is_none()
    {
        return Err(invalid(
            "severe/excellent evidence requires an explicit core decision",
        ));
    }
    if verdict.valid_issues.iter().any(|id| {
        !findings
            .iter()
            .any(|finding| finding.as_str() == Some(id.as_str()))
    }) {
        return Err(invalid("core issue is not in actual verification evidence"));
    }
    let supported = verdict.valid_issues.len() >= 3
        || (matches!(verdict.severity.as_str(), "severe" | "excellent")
            && report["structured_output"]["severity"] == verdict.severity
            && report["structured_output"]["evidence"]
                .as_str()
                .is_some_and(nonempty));
    // Schema 1 accepted normal unit continuation with fewer supported issues;
    // it still required valid issue IDs in the independent verification report.
    if !legacy && verdict.decision.as_deref() == Some("continue") && !supported {
        return Err(invalid(
            "continuation lacks independent supporting evidence",
        ));
    }
    Ok(supported)
}

fn validate_host_report(e: &Event, role: &str, effort: &str) -> Res<()> {
    let report: serde_json::Value = serde_json::from_str(
        e.evidence
            .strip_prefix("host:")
            .ok_or_else(|| invalid("missing host report"))?,
    )
    .map_err(|_| invalid("invalid host report"))?;
    if report["status"] != "completed"
        || report["actual_effort"] != effort
        || !report["model"]
            .as_str()
            .is_some_and(|m| crate::codex::role_matches(role, m))
        || !["thread_id", "turn_id"]
            .iter()
            .all(|k| report[*k].as_str().is_some_and(nonempty))
        || report["actual_sandbox"]
            != if role == "astra" {
                "readOnly"
            } else {
                "workspaceWrite"
            }
        || report["binding"]["task"] != e.task
        || report["binding"]["round"] != e.round
        || report["binding"]["attempt"] != e.dispatch_id
        || report["binding"]["artifact"] != e.artifact
        || report["binding"]["issue"] != e.issue
        || report["binding"]["evidence_version"] != e.evidence_version
        || report["binding"]["contract"] != e.contract
        || report["source_identity"]
            .as_str()
            .is_none_or(|s| !nonempty(s))
    {
        return Err(invalid(
            "missing, failed, or mismatched actual model/effort/attempt/sandbox evidence",
        ));
    }
    Ok(())
}
/// Only trusted bridge code may turn actual execution results into progress.
pub(crate) fn host_progress(
    store: &Path,
    task: &str,
    kind: &str,
    report: &serde_json::Value,
    artifact: &str,
) -> Res<Event> {
    let state = wait_state(store)?;
    let c = latest(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
    let actor = match kind {
        "start_review" => "reviewer",
        "build" => "worker",
        "assembly" => "sol",
        "verification" => "astra-verify",
        _ => "lifecycle",
    };
    let mut e = new_event(c, kind, actor, "");
    e.dispatch_id = if kind == "review_intent" {
        format!("dispatch-{}", base::unique())
    } else if kind == "start_review" {
        state
            .events
            .iter()
            .rev()
            .find(|i| bound(i, c) && i.kind == "review_intent")
            .ok_or_else(|| invalid("missing review intent"))?
            .dispatch_id
            .clone()
    } else {
        dispatch(&state, c)
            .ok_or_else(|| invalid("missing dispatch"))?
            .id
            .clone()
    };
    e.artifact = artifact.into();
    let mut report = report.clone();
    report["binding"] = serde_json::json!({"task":task,"round":c.round,"attempt":e.dispatch_id,"artifact":artifact,"issue":c.issue,"evidence_version":c.evidence_version,"contract":c.contract});
    e.evidence = format!("host:{report}");
    event_impl(store, e.clone(), true)?;
    Ok(e)
}
pub fn pre_dispatch_review_ready(state: &WaitState, task: &str) -> bool {
    latest(state, task).is_none_or(|c| c.round < 6 || pending_review(state, c).is_some()
        || dispatch(state, c).is_some_and(|d| state.operations.iter().any(|op|
            op.id == d.id && op.outcome == "running" && !op.executing
                && op.owner == std::process::id() && has(state, c, "start_review")
                && !has(state, c, "recovery"))))
}
pub fn reconcile_release(store: &Path, state: &WaitState) -> Res<()> {
    require_state_registry(store, state)?;
    if store.join("units.tsv").exists() {
        for u in super::read_units(store)? {
            if latest(state, &u.id).is_some() && task_released(state, &u.id) {
                let s = super::derive_state(
                    &super::evidence(store, &u.id)?,
                    super::needs_accept(store, &u.id)?,
                );
                super::set_unit_state(store, &u.id, &s)?;
            }
        }
    }
    Ok(())
}
/// Explicit, audited recovery only. The operator must independently confirm
/// that no previous writer can still run. Never infer non-launch from PID death.
pub fn recover_dispatch(store: &Path, event: Event, outcome: &str) -> Res<WaitState> {
    if !["not_started", "stopped"].contains(&outcome) || event.kind != "recovery" {
        return Err(invalid(
            "recovery requires not_started or stopped confirmation",
        ));
    }
    super::lifecycle::with_dispatch_recovery(store, || {
        let mut state = read(store)?;
        let mut event = event;
        event.evidence = format!("{outcome}: {}", event.evidence);
        // Resume cleanup if the process died after the recovery commit but
        // before removing the abandoned worker lease. Never add a second event
        // or acknowledge a recovery for an attempt that has since changed.
        if state.events.iter().any(|saved| saved == &event)
            && latest(&state, &event.task).is_some_and(|c| {
                dispatch(&state, c)
                    .map(|d| d.id == event.dispatch_id)
                    .unwrap_or_else(|| {
                        state
                            .events
                            .iter()
                            .rev()
                            .find(|i| bound(i, c) && i.kind == "review_intent")
                            .is_some_and(|i| i.dispatch_id == event.dispatch_id)
                    })
            })
        {
            return Ok(state);
        }
        validate_event(&state, &event, true)?;
        state.events.push(event);
        save(store, &mut state)?;
        Ok(state)
    })
}
pub fn may_finish(state: &WaitState, task: &str) -> bool {
    let Some(c) = latest(state, task) else {
        return false;
    };
    has(state, c, "release") && approved(state, c) && dependencies_current(state, c)
}
fn dependencies_current(state: &WaitState, c: &Checkpoint) -> bool {
    c.dependencies.iter().all(|dep| {
        latest(state, dep).is_some_and(|d| {
            c.dependency_versions.get(dep) == Some(&d.evidence_version) && may_finish(state, dep)
        })
    })
}
pub fn release_gate(store: &Path) -> Res<()> {
    if !store.exists() {
        return Ok(());
    }
    let state = wait_state(store)?;
    if store.join("wait-required").exists() && store.join("units.tsv").exists() {
        for unit in super::read_units(store)? {
            if latest(&state, &unit.id).is_none() {
                return Err(Failure::new(
                    "not_done",
                    format!("{} missing required checkpoint", unit.id),
                ));
            }
        }
    }
    for c in state
        .checkpoints
        .iter()
        .filter(|c| latest(&state, &c.task).is_some_and(|last| last.round == c.round))
    {
        if !may_finish(&state, &c.task) {
            return Err(Failure::new(
                "not_done",
                format!("{} awaits core release", c.task),
            ));
        }
        if store.join("units.tsv").exists()
            && !super::current_verified_unit(store, &c.task, Path::new(&c.repo))?
        {
            return Err(Failure::new(
                "not_done",
                format!("{} has stale release evidence", c.task),
            ));
        }
    }
    reconcile_release(store, &state)
}

pub(crate) fn package_current(state: &WaitState, task: &str) -> bool {
    let Some(c) = latest(state, task) else {
        return false;
    };
    let Some(d) = dispatch(state, c) else {
        return false;
    };
    let Some(package) = state
        .events
        .iter()
        .find(|e| bound(e, c) && e.dispatch_id == d.id && e.kind == "package")
    else {
        return false;
    };
    let Ok(report) =
        serde_json::from_str::<serde_json::Value>(package.evidence.trim_start_matches("host:"))
    else {
        return false;
    };
    if report["source_identity"] != super::snapshot(Path::new(&c.repo), &[".".into()]) {
        return false;
    }
    let Some(path) = report["package_path"].as_str() else {
        return false;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    use sha2::Digest;
    report["package_hash"] == format!("sha256:{:x}", sha2::Sha256::digest(bytes))
}

/// Persistent outbox. Repeated delivery carries the same event id; a receiver
/// must deduplicate it. There is no transactional remote-delivery API.
pub(crate) fn core_notices(store: &Path) -> Res<Vec<Event>> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        let mut added = false;
        // Check, bind and insert against one locked snapshot. A competing caller
        // sees the saved event, never a second independently generated event ID.
        for c in state.checkpoints.clone() {
            if latest(&state, &c.task).is_some_and(|last| last.round == c.round)
                && has(&state, &c, "verification")
                && !has(&state, &c, "notification")
                && !has(&state, &c, "recovery")
                && !has(&state, &c, "release")
            {
                let verification = state.events.iter().find(|e| {
                    bound(e, &c) && e.kind == "verification"
                        && dispatch(&state, &c).is_some_and(|d| d.id == e.dispatch_id)
                }).unwrap();
                let mut e = new_event(&c, "notification", "lifecycle", "");
                e.dispatch_id = verification.dispatch_id.clone();
                e.artifact = verification.artifact.clone();
                let source = serde_json::from_str::<serde_json::Value>(
                    verification.evidence.trim_start_matches("host:")
                ).ok().map(|v| v["source_identity"].clone());
                e.evidence = format!("host:{}", serde_json::json!({
                    "recipient":"core", "verification_event":verification.id,
                    "source_identity":source,
                    "delivery":"durable core-response outbox; remote acknowledgement unknown",
                    "binding":{"task":c.task,"round":c.round,"attempt":e.dispatch_id,
                        "artifact":e.artifact,"issue":c.issue,
                        "evidence_version":c.evidence_version,"contract":c.contract}
                }));
                validate_event(&state, &e, true)?;
                state.events.push(e);
                added = true;
            }
        }
        if added { save(store, &mut state)?; }
        Ok(state.events.iter().filter(|e| {
            e.kind == "notification" && latest(&state, &e.task).is_some_and(|c| {
                bound(e, c) && dispatch(&state, c).is_some_and(|d| d.id == e.dispatch_id)
                    && !approved(&state, c)
                    && !has(&state, c, "core_reject") && !has(&state,c,"test_reject")
                    && !has(&state, c, "recovery")
            })
        }).cloned().collect())
    })
}

/// Rust-only liveness diagnosis. Durable claims are uncertainty, not proof of
/// a live producer. The lifecycle registry supplies actual local ownership.
pub(crate) fn idle_blockers(store: &Path) -> Res<Vec<String>> {
    let state = wait_state(store)?;
    let units = super::read_units(store)?;
    if units.is_empty() && state.checkpoints.is_empty() { return Ok(vec![]); }
    let ready = if units.is_empty() { vec![] } else { super::ready_units(store)? };
    let waiting = if units.is_empty() { vec![] } else { super::waiting_units(store)? };
    let mut blockers = vec![];
    for unit in &units {
        let c = latest(&state, &unit.id);
        // Bound verification is handled exclusively by the independent core
        // outbox, including after synchronous dispatcher delivery.
        if c.is_some_and(|c| has(&state, c, "verification")
            && !approved(&state, c) && !has(&state, c, "core_reject") && !has(&state,c,"test_reject")
            && !has(&state, c, "recovery")) { continue; }
        if waiting.iter().any(|w| w.unit == unit.id && w.next == "running")
            && !store.join("quarantine").exists() { continue; }
        if ready.contains(&unit.id) {
            blockers.push(format!("{}: runnable TODO or recovered checkpoint has no producer", unit.id));
        } else if let Some(w) = waiting.iter().find(|w| w.unit == unit.id) {
            blockers.push(format!("{}: no confirmed producer; {} ({})", unit.id, w.next, w.state));
        }
    }
    for c in &state.checkpoints {
        if latest(&state, &c.task).is_some_and(|last| last.round == c.round)
            && !units.iter().any(|u| u.id == c.task)
            && !has(&state, c, "release")
            && !(has(&state, c, "verification") && !has(&state, c, "recovery")
                && !approved(&state, c) && !has(&state, c, "core_reject") && !has(&state,c,"test_reject"))
        {
            blockers.push(format!("{}: unfinished checkpoint has no registered producer", c.task));
        }
    }
    blockers.sort();
    Ok(blockers)
}

pub fn core_verdict(
    store: &Path,
    task: &str,
    verification_id: &str,
    approve: bool,
    assessment: &str,
) -> Res<WaitState> {
    let state = wait_state(store)?;
    let c = latest(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
    let d = dispatch(&state, c).ok_or_else(|| invalid("missing attempt"))?;
    let v = state
        .events
        .iter()
        .find(|e| bound(e, c) && e.dispatch_id == d.id && e.kind == "verification")
        .ok_or_else(|| invalid("missing verification"))?;
    if v.id != verification_id {
        return Err(invalid("stale core verification binding"));
    }
    let kind = if approve {
        "core_approve"
    } else {
        "core_reject"
    };
    let mut e = new_event(c, kind, "core", assessment);
    e.id = format!("{}-core-verdict", v.id);
    e.dispatch_id = d.id.clone();
    e.artifact = v.artifact.clone();
    if let Some(saved) = state.events.iter().find(|x| x.id == e.id) {
        return if saved == &e {
            Ok(state)
        } else {
            Err(invalid("conflicting core verdict replay"))
        };
    }
    wait_event(store, e)
}
/// Host-only unit release after actual independent test evidence. Core owns
/// the final round submission, not technical approval of individual modules.
pub(crate) fn accept_tested_unit(store: &Path, task: &str) -> Res<bool> {
    let state=wait_state(store)?;
    let c=latest(&state,task).ok_or_else(||invalid("missing checkpoint"))?;
    if has(&state,c,"test_reject") { return Ok(false); }
    let d=dispatch(&state,c).ok_or_else(||invalid("missing dispatch"))?;
    let v=state.events.iter().find(|e|bound(e,c)&&e.dispatch_id==d.id&&e.kind=="verification").ok_or_else(||invalid("missing verifier result"))?;
    let report:serde_json::Value=serde_json::from_str(v.evidence.strip_prefix("host:").ok_or_else(||invalid("missing Host test evidence"))?).map_err(|_|invalid("invalid test evidence"))?;
    if report["structured_output"]["issues"].as_array().is_some_and(|issues|!issues.is_empty()) {
        let mut e=new_event(c,"test_reject","lifecycle","Actual independent test findings require dispatcher repair, not core technical judgment");
        e.id=format!("{}-test_reject",v.id);e.dispatch_id=d.id.clone();e.artifact=v.artifact.clone();
        event_impl(store,e,true)?;
        return Ok(false);
    }
    for kind in ["test_approve", "completion", "release"] {
        let state=wait_state(store)?;
        let c=latest(&state,task).ok_or_else(|| invalid("missing checkpoint"))?;
        if has(&state,c,kind) { continue; }
        let d=dispatch(&state,c).ok_or_else(|| invalid("missing dispatch"))?;
        let v=state.events.iter().find(|e| bound(e,c) && e.dispatch_id==d.id && e.kind=="verification")
            .ok_or_else(|| invalid("missing verifier result"))?;
        let mut e=new_event(c,kind,"lifecycle","Host accepted independent Astra HIGH tests; no core technical verdict");
        e.id=format!("{}-{kind}",v.id);e.dispatch_id=d.id.clone();e.artifact=v.artifact.clone();
        event_impl(store,e,true)?;
    }
    Ok(true)
}

pub fn complete_round(store: &Path, task: &str, verification_id: &str) -> Res<WaitState> {
    for kind in ["completion", "release"] {
        let state = wait_state(store)?;
        let c = latest(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
        let d = dispatch(&state, c).ok_or_else(|| invalid("missing attempt"))?;
        if !state.events.iter().any(|e| {
            bound(e, c)
                && e.kind == "verification"
                && e.dispatch_id == d.id
                && e.id == verification_id
        }) {
            return Err(invalid("stale core completion binding"));
        }
        if has(&state, c, kind) {
            continue;
        }
        let d = dispatch(&state, c).ok_or_else(|| invalid("missing attempt"))?;
        let v = state
            .events
            .iter()
            .find(|e| bound(e, c) && e.dispatch_id == d.id && e.kind == "core_approve")
            .ok_or_else(|| invalid("missing core approval"))?;
        let mut e = new_event(c, kind, "core", "explicit approved round completion");
        e.id = format!("{}-{kind}", v.id);
        e.dispatch_id = d.id.clone();
        e.artifact = v.artifact.clone();
        wait_event(store, e)?;
    }
    wait_state(store)
}

fn require_state_registry(store: &Path, state: &WaitState) -> Res<()> {
    require_registry(store)?;
    if state.engineering_enrolled && !store.join("units.tsv").is_file() {
        return Err(invalid("registered engineering journal requires units.tsv"));
    }
    if state.engineering_enrolled || store.join("wait-required").exists() {
        super::read_units(store)?;
    }
    Ok(())
}

fn require_registry(store: &Path) -> Res<()> {
    // Enrollment is durable even when the registry has disappeared.
    if store.join("wait-required").exists() && !store.join("units.tsv").is_file() {
        return Err(invalid(
            "opted-in workflow requires a registered unit store",
        ));
    }
    Ok(())
}

/// Explicit v1 -> v2 migration. Preserve exact historical bytes and all task
/// evidence; create NO product run or completed product rounds from that history.
pub fn migrate_wait(store: &Path) -> Res<WaitState> {
    super::with_lock(store, false, || {
        require_registry(store)?;
        let bytes = std::fs::read(store.join("core-wait.json"))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        let mut state: WaitState = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("invalid legacy core-wait state: {e}")))?;
        require_state_registry(store, &state)?;
        if state.schema_version == 1
            && !store.join("units.tsv").is_file()
            && state.events.iter().any(|e| e.evidence.starts_with("host:"))
        {
            return Err(invalid(
                "legacy host evidence requires registered-store recovery before migration",
            ));
        }
        if state.schema_version == 2 {
            replay_stored(store, &state)?;
            return Ok(state);
        }
        if state.schema_version != 1
            || !state.product_runs.is_empty()
            || state.legacy_archive_hash.is_some()
        {
            return Err(invalid("unsupported or ambiguous legacy schema"));
        }
        replay_state(&state, state.events.len())?;
        let archive = store.join("core-wait.v1.json");
        match std::fs::read(&archive) {
            Ok(old) if old != bytes => return Err(invalid("legacy archive conflict")),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                base::atomic_write(&archive, &bytes)
                    .map_err(|e| Failure::new("io", e.to_string()))?;
            }
            Err(e) => return Err(Failure::new("io", e.to_string())),
        }
        use sha2::Digest;
        state.legacy_archive_hash = Some(format!("sha256:{:x}", sha2::Sha256::digest(&bytes)));
        state.schema_version = 2;
        save(store, &mut state)?;
        Ok(state)
    })
}

/// Begin an explicitly identified run, or checkpoint the next round following
/// a persisted continuation. Task attempt counters are never consulted.
pub fn product_checkpoint(store: &Path, run_id: &str) -> Res<WaitState> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        if !nonempty(run_id) {
            return Err(invalid("product run requires an explicit identity"));
        }
        let event_start = state.events.len();
        let checkpoint_start = state.checkpoints.clone();
        let engineering_start = engineering_identity(store)?;
        let finished = product_policy_finished_in_store(store, &state, run_id)?;
        let run = match state.product_runs.iter().position(|r| r.id == run_id) {
            Some(i) => &mut state.product_runs[i],
            None => {
                state.product_runs.push(ProductRun {
                    id: run_id.into(),
                    rounds: vec![],
                });
                state.product_runs.last_mut().unwrap()
            }
        };
        let follows_finish = run
            .rounds
            .last()
            .and_then(|r| r.transition.as_ref())
            .is_some_and(|t| t.action == "finish");
        let round =
            match run.rounds.last() {
                None => 1,
                Some(last) => {
                    if last.transition.as_ref().is_none_or(|t| {
                        t.action != "continue" && (t.action != "finish" || finished)
                    }) {
                        return Err(invalid("product round has no continuation transition"));
                    }
                    last.round
                        .checked_add(1)
                        .ok_or_else(|| invalid("product round overflow"))?
                }
            };
        run.rounds.push(ProductRound {
            round,
            event_start,
            checkpoint_start: follows_finish.then_some(checkpoint_start),
            engineering_start: follows_finish.then_some(engineering_start),
            transition: None,
        });
        validate_products(&state)?;
        save(store, &mut state)?;
        Ok(state)
    })
}

// Bound, replay-validated independent task verification is the input to product
// policy. A later barrier may aggregate more tasks, but cannot invent rounds.
fn product_support(state: &WaitState, round: &ProductRound, t: &ProductTransition) -> Res<bool> {
    if t.verification_ids.is_empty()
        || t.verification_ids.iter().collect::<BTreeSet<_>>().len() != t.verification_ids.len()
        || t.event_end < round.event_start
        || t.event_end > state.events.len()
    {
        return Err(invalid("missing or invalid product evidence range"));
    }
    let verdict: CoreVerdict = serde_json::from_str(&t.assessment)
        .map_err(|_| invalid("invalid product core assessment"))?;
    let mut findings = BTreeSet::new();
    let mut severity_supported = false;
    let mut observed_severity = false;
    for id in &t.verification_ids {
        let (index, v) = state
            .events
            .iter()
            .enumerate()
            .find(|(_, e)| e.id == *id && e.kind == "verification")
            .ok_or_else(|| invalid("unknown product verification"))?;
        if index < round.event_start || index >= t.event_end {
            return Err(invalid(
                "product evidence predates checkpoint or follows transition",
            ));
        }
        let c = state
            .checkpoints
            .iter()
            .find(|c| bound(v, c))
            .ok_or_else(|| invalid("unbound product verification"))?;
        validate_host_report(v, "astra", "high")?;
        if !state.events[..t.event_end]
            .iter()
            .any(|e| bound(e, c) && e.dispatch_id == v.dispatch_id && e.kind == "release")
        {
            return Err(invalid(
                "product evidence requires engineering-released attempt",
            ));
        }
        let report: serde_json::Value =
            serde_json::from_str(v.evidence.trim_start_matches("host:"))
                .map_err(|_| invalid("invalid product verification report"))?;
        let issues = report["structured_output"]["issues"]
            .as_array()
            .ok_or_else(|| invalid("missing product verification issues"))?;
        for issue in issues {
            let id = issue
                .as_str()
                .filter(|s| nonempty(s))
                .ok_or_else(|| invalid("invalid root cause identity"))?;
            findings.insert(id.to_owned());
        }
        let observed = &report["structured_output"]["severity"];
        if !matches!(observed.as_str(), Some("normal" | "severe" | "excellent")) {
            return Err(invalid("invalid independent severity"));
        }
        observed_severity |= observed != "normal";
        severity_supported |= observed == &verdict.severity
            && report["structured_output"]["evidence"]
                .as_str()
                .is_some_and(nonempty);
    }
    if observed_severity && verdict.decision.is_none() {
        return Err(invalid(
            "exceptional evidence requires explicit core decision",
        ));
    }
    let report = serde_json::json!({"structured_output": {
        "issues":findings, "severity": if severity_supported { verdict.severity.as_str() } else { "normal" },
        "evidence": if severity_supported { "bound independent evidence" } else { "" }
    }});
    assess_core_verdict("normal", &t.assessment, &report)
}
fn validate_products(state: &WaitState) -> Res<()> {
    let mut ids = BTreeSet::new();
    for run in &state.product_runs {
        if !nonempty(&run.id) || !ids.insert(&run.id) || run.rounds.is_empty() {
            return Err(invalid("invalid or duplicate product run identity"));
        }
        let mut previous_end = 0;
        for (i, round) in run.rounds.iter().enumerate() {
            if round.round as usize != i + 1
                || round.event_start < previous_end
                || round.event_start > state.events.len()
            {
                return Err(invalid("invalid product checkpoint sequence"));
            }
            match &round.transition {
                Some(t) => {
                    let supported = product_support(state, round, t)?;
                    let required = round.round < 5 || supported;
                    if t.action != if required { "continue" } else { "finish" } {
                        return Err(invalid(
                            "product transition violates five-round/continuation policy",
                        ));
                    }
                    if t.action == "finish" && i + 1 != run.rounds.len() {
                        let next = &run.rounds[i + 1];
                        if next.event_start == t.event_end
                            && t.checkpoint_end.is_some()
                            && next.checkpoint_start == t.checkpoint_end
                            && t.engineering_end.is_some()
                            && next.engineering_start == t.engineering_end
                        {
                            return Err(invalid("current finished product run cannot advance"));
                        }
                        if next.checkpoint_start.is_none() || next.engineering_start.is_none() {
                            return Err(invalid("missing product checkpoint snapshot"));
                        }
                    }
                    previous_end = t.event_end;
                }
                None if i + 1 == run.rounds.len() => (),
                None => return Err(invalid("product checkpoint lacks preceding transition")),
            }
        }
    }
    Ok(())
}

/// Commit a policy transition, not product release. All existing registered
/// engineering gates apply even when policy requires continuation.
pub fn product_transition(
    store: &Path,
    run_id: &str,
    round: u32,
    verification_ids: Vec<String>,
    assessment: &str,
    action: &str,
) -> Res<WaitState> {
    super::with_lock(store, false, || {
        let mut state = read(store)?;
        if !store.join("units.tsv").is_file() {
            return Err(invalid(
                "product transition requires registered engineering evidence",
            ));
        }
        // All enrolled units must remain released/current, not only the selected evidence.
        for unit in super::read_units(store)? {
            let c = latest(&state, &unit.id)
                .ok_or_else(|| invalid("missing engineering checkpoint"))?;
            if !super::current_verified_unit_in_state(store, &state, &unit.id, Path::new(&c.repo))?
            {
                return Err(invalid(
                    "product transition requires current engineering acceptance",
                ));
            }
        }
        let units = super::read_units(store)?;
        for id in &verification_ids {
            let v = state
                .events
                .iter()
                .find(|e| e.id == *id)
                .ok_or_else(|| invalid("unknown product verification"))?;
            let c = latest(&state, &v.task).ok_or_else(|| invalid("missing task checkpoint"))?;
            if !units.iter().any(|u| u.id == v.task) || !bound(v, c) || !has(&state, c, "release") {
                return Err(invalid("stale product verification"));
            }
            let rows = super::evidence(store, &v.task)?;
            let run = rows
                .iter()
                .rposition(|r| r.kind == "run")
                .ok_or_else(|| invalid("missing actual product evidence run"))?;
            if !rows[run + 1..]
                .iter()
                .any(|r| r.kind == "dispatch" && r.command == v.dispatch_id)
            {
                return Err(invalid("product evidence belongs to another dispatch"));
            }
        }
        let event_end = state.events.len();
        let checkpoint_end = Some(state.checkpoints.clone());
        let engineering_end = Some(engineering_identity(store)?);
        let current = state
            .product_runs
            .iter_mut()
            .find(|r| r.id == run_id)
            .and_then(|r| r.rounds.last_mut())
            .filter(|r| r.round == round)
            .ok_or_else(|| invalid("not current product checkpoint"))?;
        let transition = ProductTransition {
            verification_ids,
            assessment: assessment.into(),
            action: action.into(),
            event_end,
            checkpoint_end,
            engineering_end,
        };
        if let Some(saved) = &current.transition {
            return if saved == &transition {
                Ok(state)
            } else {
                Err(invalid("conflicting product transition"))
            };
        }
        current.transition = Some(transition);
        validate_products(&state)?;
        save(store, &mut state)?;
        Ok(state)
    })
}

/// Policy status only; intentionally not called a product release gate.
pub fn product_policy_finished(state: &WaitState, run_id: &str) -> bool {
    validate_products(state).is_ok()
        && state
            .product_runs
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| r.rounds.last())
            .and_then(|r| r.transition.as_ref())
            .is_some_and(|t| {
                t.action == "finish"
                    && t.event_end == state.events.len()
                    && t.checkpoint_end.as_ref() == Some(&state.checkpoints)
            })
}

fn engineering_identity(store: &Path) -> Res<String> {
    use sha2::{Digest, Sha256};
    let mut units = if store.join("units.tsv").is_file() {
        super::read_units(store)?
    } else {
        vec![]
    };
    units.sort_by(|a, b| a.id.cmp(&b.id));
    let mut records = Vec::new();
    for unit in units {
        records.push((unit.id.clone(), super::evidence(store, &unit.id)?));
    }
    let bytes = serde_json::to_vec(&records).map_err(|e| invalid(e.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

pub(crate) fn product_policy_finished_in_store(
    store: &Path,
    state: &WaitState,
    run_id: &str,
) -> Res<bool> {
    if !product_policy_finished(state, run_id) {
        return Ok(false);
    }
    let saved = state
        .product_runs
        .iter()
        .find(|r| r.id == run_id)
        .and_then(|r| r.rounds.last())
        .and_then(|r| r.transition.as_ref())
        .and_then(|t| t.engineering_end.as_ref());
    Ok(saved == Some(&engineering_identity(store)?))
}
