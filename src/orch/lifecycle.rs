pub mod scheduler;
use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// Test scratch space under the project root; the sandbox cannot write to the system temp directory.
pub const TEMP_DIR: &str = ".axiom-tmp/";
pub enum Begin {
    Run {
        brief: Brief,
        attempt: u32,
        attempt_id: String,
    },
    Reused,
}
#[derive(serde::Serialize, Debug, Clone)]
pub struct Owner {
    pub wish: String,
    pub module: String,
    pub unit: String,
    pub attempt: String,
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Finished {
    pub state: String,
    pub verify_exit: Option<i32>,
    pub verify_tail: String,
    pub verify_output: Option<PathBuf>,
    pub outside_scope: Vec<String>,
    pub ignored: Vec<(String, String)>,
    pub unattributed: Vec<String>,
    pub changed: Vec<String>,
    pub pack: Option<PathBuf>,
    pub pack_error: Option<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Checkpoint {
    unit: String,
    attempt: u32,
    attempt_id: String,
    base_tree: String,
    brief_sha256: String,
    routing: Option<crate::codex::routing::RoutingSnapshot>,
    stage: String,
    pid: u32,
    updated: String,
    #[serde(default)]
    worker_result: Option<Value>,
    #[serde(default)]
    finished: Option<Finished>,
    #[serde(default)]
    finished_tree: String,
    #[serde(default)]
    evidence_start: usize,
    #[serde(default)]
    red_tree: Option<String>,
}
pub(crate) fn checkpoint_worker(store: &Path, unit: &str, attempt: &str, report: Option<&Value>) -> Res<Option<Value>> {
    with_lock(store, false, || {
        let data = std::fs::read(checkpoint_path(store, unit)).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut cp: Checkpoint = serde_json::from_slice(&data).map_err(|e| Failure::new("io", e.to_string()))?;
        if cp.attempt_id != attempt { return Err(Failure::new("claim_mismatch", "checkpoint attempt mismatch")); }
        if let Some(report) = report {
            cp.worker_result = Some(report.clone());
            if cp.stage == "claimed" { cp.stage = "worker_done".into(); }
            cp.updated = base::now_iso();
            write_checkpoint(store, &cp)?;
        }
        Ok(cp.worker_result)
    })
}
// Stage transitions and their durable result share the claim's store lock.
pub(crate) fn checkpoint_stage(store: &Path, unit: &str, attempt: &str, stage: &str, repo: &Path) -> Res<()> {
    with_lock(store, false, || {
        let path = checkpoint_path(store, unit);
        let mut cp: Checkpoint = serde_json::from_slice(&std::fs::read(&path)
            .map_err(|e| Failure::new("io", e.to_string()))?)
            .map_err(|e| Failure::new("io", e.to_string()))?;
        if cp.attempt_id != attempt { return Err(Failure::new("claim_mismatch", "checkpoint attempt mismatch")); }
        let rank = |s: &str| match s { "reviewed" => 4, "selfchecked" => 3, "verified" => 2, "worker_done" => 1, _ => 0 };
        if rank(stage) > rank(&cp.stage) { cp.stage = stage.into(); }
        // A self-check may repair the tree and verify it again. Cache that result,
        // not the pre-repair tree, so a crash after self-check does not verify twice.
        if stage == "selfchecked" {
            let brief = unit_brief(store, unit)?;
            let tree = snapshot(repo, &brief.scope);
            if let Some(check) = evidence(store, unit)?.iter().rev().find(|r| r.kind == "verify" && r.commit == tree) {
                cp.finished_tree = tree;
                if let Some(finished) = &mut cp.finished { finished.verify_exit = Some(check.exit_code); }
            }
        }
        cp.updated = base::now_iso();
        write_checkpoint(store, &cp)
    })
}
fn checkpoint_path(store: &Path, unit: &str) -> PathBuf {
    store.join("checkpoints").join(format!("{unit}.json"))
}
fn write_checkpoint(store: &Path, cp: &Checkpoint) -> Res<()> {
    let path = checkpoint_path(store, &cp.unit);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| Failure::new("io", e.to_string()))?;
    let data = serde_json::to_vec(cp).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&path, &data).map_err(|e| Failure::new("io", e.to_string()))
}
fn completed_brief_matches(store: &Path, unit: &str, brief_path: &Path) -> Res<bool> {
    let saved: Option<Value> = std::fs::read(store.join("reports").join(format!("{unit}.json")))
        .ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    // Older reports have no digest; preserve their existing resume behavior.
    let Some(digest) = saved.as_ref().and_then(|v| v["brief_sha256"].as_str()) else {
        return Ok(true);
    };
    let bytes = std::fs::read(brief_path).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(crate::seams::sha256_hex(&bytes) == digest)
}
pub(crate) fn checkpoint_routing(store: &Path, unit: &str, snapshot: &crate::codex::routing::RoutingSnapshot) -> Res<()> {
    with_lock(store, false, || {
        let path = checkpoint_path(store, unit);
        let data = std::fs::read(path).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut cp: Checkpoint = serde_json::from_slice(&data).map_err(|e| Failure::new("io", e.to_string()))?;
        if cp.attempt_id != snapshot.attempt_id { return Err(Failure::new("claim_mismatch", "checkpoint attempt mismatch")); }
        cp.routing = Some(snapshot.clone());
        cp.updated = base::now_iso();
        write_checkpoint(store, &cp)
    })
}
struct Claim {
    dirty: BTreeSet<String>,
    attempt_id: String,
    base: String,
}
static RUNNING: OnceLock<Mutex<HashMap<String, Claim>>> = OnceLock::new();
fn key(store: &Path, unit: &str) -> String {
    format!("{}|{unit}", store.display())
}
fn claims() -> &'static Mutex<HashMap<String, Claim>> {
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}
fn claim_owner(store: &Path, c: &DiskClaim, units: &[Unit]) -> Owner {
    Owner {
        wish: if c.wish.is_empty() {
            wish_of_store(store)
        } else {
            c.wish.clone()
        },
        module: if c.module.is_empty() {
            units
                .iter()
                .find(|u| u.id == c.unit)
                .map(|u| u.track.clone())
                .unwrap_or_default()
        } else {
            c.module.clone()
        },
        unit: c.unit.clone(),
        attempt: c.attempt.clone(),
    }
}
fn owner_text(owner: &Owner) -> String {
    format!(
        "wish {}, module {}, unit {}, attempt {}",
        owner.wish, owner.module, owner.unit, owner.attempt
    )
}
pub fn unit_owner(store: &Path, unit: &str) -> Res<Option<Owner>> {
    with_lock(store, false, || {
        let units = read_units(store)?;
        if let Some(c) = read_claims(store)?.iter().find(|c| c.unit == unit) {
            return Ok(Some(claim_owner(store, c, &units)));
        }
        let Some(u) = units.iter().find(|u| u.id == unit) else {
            return Ok(None);
        };
        let rows = evidence(store, unit)?;
        if derive_state(&rows, needs_accept(store, unit)?) != "verified" {
            return Ok(None);
        }
        Ok(Some(Owner {
            wish: wish_of_store(store),
            module: u.track.clone(),
            unit: unit.into(),
            attempt: rows.iter().filter(|r| r.kind == "run").count().to_string(),
        }))
    })
}
fn dirty(repo: &Path) -> Res<BTreeSet<String>> {
    let out = base::proc::cmd("git")
        .current_dir(repo)
        .args([
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
            "--ignored=traditional",
        ])
        .output()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    if !out.status.success() {
        return Ok(BTreeSet::new());
    }
    let prefix = super::report::project_prefix(repo)?;
    let project: Vec<_> = prefix.split('/').filter(|p| !p.is_empty()).collect();
    let relative = |path: &[u8]| {
        let path = String::from_utf8_lossy(path).replace('\\', "/");
        let parts: Vec<_> = path.split('/').collect();
        let common = project
            .iter()
            .zip(&parts)
            .take_while(|(a, b)| a == b)
            .count();
        let mut result = vec![".."; project.len() - common];
        result.extend_from_slice(&parts[common..]);
        result.join("/")
    };
    let mut parts = out.stdout.split(|b| *b == 0);
    let mut result = BTreeSet::new();
    while let Some(p) = parts.next() {
        if p.len() < 4 {
            continue;
        }
        let status = &p[..2];
        result.insert(relative(&p[3..]));
        if status.contains(&b'R') || status.contains(&b'C') {
            if let Some(old) = parts.next() {
                result.insert(relative(old));
            }
        }
    }
    Ok(result)
}
fn known_track(store: &Path, u: &Unit) -> Res<bool> {
    if !store.join("modules.md").exists() {
        return Ok(true);
    }
    let text = std::fs::read_to_string(store.join("modules.md"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let map = parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))?;
    Ok(map.modules.iter().any(|m| m.id == u.track))
}
fn dependencies(store: &Path, u: &Unit, units: &[Unit]) -> Res<Vec<String>> {
    if !store.join("modules.md").exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(store.join("modules.md"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let map = parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))?;
    let deps = map
        .modules
        .iter()
        .find(|m| m.id == u.track)
        .map(|m| m.depends.as_slice())
        .unwrap_or(&[]);
    let mut missing = Vec::new();
    for dep in deps {
        let registered: Vec<_> = units.iter().filter(|x| &x.track == dep).collect();
        if registered.is_empty() {
            missing.push(format!("{dep} (no units registered)"));
        }
        let mut seen = BTreeSet::new();
        for other in registered {
            let Some(standing) = standing_unit(units, other) else {
                missing.push(format!(
                    "{} ({}; replacement chain is broken)",
                    other.id, other.state
                ));
                continue;
            };
            if !seen.insert(&standing.id) {
                continue;
            }
            let rows = evidence(store, &standing.id)?;
            let needs = needs_accept(store, &standing.id)?;
            let state = derive_state(&rows, needs);
            if state != "verified" {
                missing.push(format!(
                    "{} ({state}; next {})",
                    standing.id,
                    next_step(&rows, &state, needs)
                ));
            }
        }
    }
    Ok(missing)
}
fn next_step(rows: &[Evidence], state: &str, needs_accept: bool) -> String {
    match state {
        "built" => {
            let verify = rows.iter().rposition(|r| r.kind == "verify");
            if needs_accept
                && verify.is_some_and(|v| !rows[v + 1..].iter().any(|r| r.kind == "accept"))
            {
                "accept".into()
            } else {
                "review".into()
            }
        }
        "accept-failed" | "review-failed" => "fix".into(),
        "failed" if rows.iter().filter(|r| r.kind == "run").count() >= 2 => "fix".into(),
        _ => "run".into(),
    }
}
pub fn begin_unit(
    store: &Path,
    unit: &str,
    role: &str,
    repo: &Path,
    astra_required: bool,
) -> Res<Begin> {
    Ok(begin_inner(store, unit, role, repo, astra_required, false, &mut None)?.unwrap_or(Begin::Reused))
}

/// Performs the normal begin gates atomically, returning None for a resumable build.
pub fn begin_unit_cycle(
    store: &Path,
    unit: &str,
    role: &str,
    repo: &Path,
    astra_required: bool,
) -> Res<Option<Begin>> {
    begin_inner(store, unit, role, repo, astra_required, true, &mut None)
}

pub(crate) fn begin_dispatch(
    store: &Path,
    unit: &str,
    role: &str,
    repo: &Path,
    astra_required: bool,
    cycle: bool,
) -> (Res<Option<Begin>>, Option<Owner>) {
    let mut owner = None;
    let result = begin_inner(store, unit, role, repo, astra_required, cycle, &mut owner);
    (result, owner)
}

fn begin_inner(
    store: &Path,
    unit: &str,
    role: &str,
    repo: &Path,
    astra_required: bool,
    _cycle: bool,
    refused_owner: &mut Option<Owner>,
) -> Res<Option<Begin>> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        let units = read_units(store)?;
        let u = units
            .iter()
            .find(|u| u.id == unit)
            .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
        u.refuse_superseded()?;
        let rows = evidence(store, unit)?;
        let mut disk = read_claims(store)?;
        if let Some(index) = disk.iter().position(|c| c.unit == unit) {
            let c = &disk[index];
            let held = active.get(&key(store, unit)).is_some_and(|a| a.attempt_id == c.attempt);
            let owner = claim_owner(store, c, &units);
            let text = owner_text(&owner);
            *refused_owner = Some(owner);
            if held || c.pid.parse().ok().is_some_and(base::proc::pid_alive) {
                return Err(Failure::new("already_running", format!("unit {unit} is already running: {text}")));
            }
            let cp: Checkpoint = std::fs::read(checkpoint_path(store, unit)).ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
                .filter(|cp: &Checkpoint| cp.unit == unit && cp.attempt_id == c.attempt && matches!(cp.stage.as_str(), "claimed" | "worker_done" | "verified" | "selfchecked" | "reviewed"))
                .ok_or_else(|| Failure::new("outcome_unknown", format!("claim outcome unknown: {text}")))?;
            let bytes = std::fs::read(&u.brief).map_err(|e| Failure::new("io", e.to_string()))?;
            if crate::seams::sha256_hex(&bytes) != cp.brief_sha256 {
                return Err(Failure::new("brief_changed", "brief changed since claim"));
            }
            let brief = unit_brief(store, unit)?;
            let before = dirty(repo)?;
            disk[index].pid = std::process::id().to_string();
            let mut cp = cp;
            cp.pid = std::process::id();
            cp.updated = base::now_iso();
            write_checkpoint(store, &cp)?;
            save_claims(store, &disk)?;
            active.insert(key(store, unit), Claim { dirty: before, attempt_id: cp.attempt_id.clone(), base: cp.base_tree.clone() });
            return Ok(Some(Begin::Run { brief, attempt: cp.attempt, attempt_id: cp.attempt_id }));
        }
        let brief = unit_brief(store, unit)?;
        if let Some(c) = disk.iter().find(|c| overlaps(&brief.scope, &c.scope)) {
            return Err(Failure::new(
                "busy",
                format!(
                    "scope busy: {unit}; {}",
                    owner_text(&claim_owner(store, c, &units))
                ),
            ));
        }
        if role != "sol" && !known_track(store, u)? {
            return Err(Failure::new(
                "unknown_track",
                format!(
                    "unit {unit} track {} is not a module in modules.md",
                    u.track
                ),
            ));
        }
        let missing = dependencies(store, u, &units)?;
        if !missing.is_empty() {
            let ready = ready_units(store)?;
            return Err(Failure::new(
                "not_ready",
                format!(
                    "{unit} waits on {}; {}",
                    missing.join(", "),
                    if ready.is_empty() {
                        "none ready".to_string()
                    } else {
                        format!("ready now: {}", ready.join(", "))
                    }
                ),
            ));
        }
        if derive_state(&rows, needs_accept(store, unit)?) == "verified"
            && rows
                .iter()
                .rev()
                .find(|r| r.kind == "verify")
                .is_some_and(|r| !r.commit.is_empty() && snapshot(repo, &brief.scope) == r.commit)
        {
            let owner = Owner {
                wish: wish_of_store(store),
                module: u.track.clone(),
                unit: unit.into(),
                attempt: rows.iter().filter(|r| r.kind == "run").count().to_string(),
            };
            let text = owner_text(&owner);
            *refused_owner = Some(owner);
            return Err(Failure::new(
                "already_verified",
                format!("unit {unit} is already verified: {text}; nothing to run"),
            ));
        }
        // A matching scope tree alone cannot justify reusing a changed brief.
        if derive_state(&rows, needs_accept(store, unit)?) == "built"
            && completed_brief_matches(store, unit, Path::new(&u.brief))?
            && rows
                .iter()
                .rev()
                .find(|r| r.kind == "verify")
                .is_some_and(|r| {
                    r.exit_code == 0
                        && !r.commit.is_empty()
                        && snapshot(repo, &brief.scope) == r.commit
                })
        {
            return Ok(None);
        }
        let count = rows.iter().filter(|r| r.kind == "run").count();
        if role == "sol" {
            let full = super::evidence::full_evidence(store)?;
            if !full
                .iter()
                .rev()
                .find(|r| r.kind == "full")
                .is_some_and(|r| r.exit_code != 0)
            {
                return Err(Failure::new(
                    "sol_not_allowed",
                    "sol requires a failing full test",
                ));
            }
        }
        if astra_required {
            astra_gate(store)?;
        }
        if count >= 2 {
            return Err(Failure::new(
                "fix_limit",
                format!("unit {unit} has reached its fix limit"),
            ));
        }
        let state = derive_state(&rows, needs_accept(store, unit)?);
        if state != "built"
            && state != "verified"
            && brief.test_matrix.is_empty()
            && crate::codex::routing::routed_store(store)?
        {
            return Err(Failure::new(
                "contract_required",
                "routed unit requires a TEST_MATRIX",
            ));
        }
        let attempt = count as u32 + 1;
        let attempt_id = format!("{}-{}", attempt, base::unique());
        let before = dirty(repo)?;
        let base = base_snapshot(repo, &brief.scope);
        let peers = disk.iter().map(|c| c.attempt.clone()).collect();
        for c in &mut disk {
            c.peers.push(attempt_id.clone());
        }
        disk.push(DiskClaim {
            unit: unit.into(),
            attempt: attempt_id.clone(),
            scope: brief.scope.clone(),
            started: base::now_iso(),
            module: u.track.clone(),
            wish: wish_of_store(store),
            pid: std::process::id().to_string(),
            peers,
        });
        let bytes = std::fs::read(&u.brief).map_err(|e| Failure::new("io", e.to_string()))?;
        let cp = Checkpoint { unit: unit.into(), attempt, attempt_id: attempt_id.clone(), base_tree: base.clone(), brief_sha256: crate::seams::sha256_hex(&bytes), routing: None, stage: "claimed".into(), pid: std::process::id(), updated: base::now_iso(), worker_result: None, finished: None, finished_tree: String::new(), evidence_start: rows.len(), red_tree: None };
        write_checkpoint(store, &cp)?;
        save_claims(store, &disk)?;
        active.insert(
            key(store, unit),
            Claim {
                dirty: before,
                attempt_id: attempt_id.clone(),
                base,
            },
        );
        Ok(Some(Begin::Run {
            brief,
            attempt,
            attempt_id,
        }))
    })
}
/// Resume only a built unit whose verified scope still matches and is unclaimed.
pub fn resume_built(store: &Path, unit: &str, repo: &Path) -> Res<bool> {
    let active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        let units = read_units(store)?;
        let u = units
            .iter()
            .find(|u| u.id == unit)
            .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
        u.refuse_superseded()?;
        let brief = unit_brief(store, unit)?;
        if let Some(c) = read_claims(store)?
            .iter()
            .find(|c| c.unit == unit || overlaps(&brief.scope, &c.scope))
        {
            return Err(
                if active
                    .get(&key(store, &c.unit))
                    .is_some_and(|a| a.attempt_id == c.attempt)
                {
                    Failure::new("busy", format!("scope busy: {unit}"))
                } else {
                    Failure::new(
                        "outcome_unknown",
                        format!("claim outcome unknown: {}", c.attempt),
                    )
                },
            );
        }
        let rows = evidence(store, unit)?;
        Ok(derive_state(&rows, needs_accept(store, unit)?) == "built"
            && completed_brief_matches(store, unit, Path::new(&u.brief))?
            && rows
                .iter()
                .rev()
                .find(|r| r.kind == "verify")
                .is_some_and(|r| {
                    r.exit_code == 0
                        && !r.commit.is_empty()
                        && snapshot(repo, &brief.scope) == r.commit
                }))
    })
}
/// Relinquish ownership without finishing (or discarding) the attempt.
pub(crate) fn suspend_claim(store: &Path, unit: &str, attempt_id: &str) -> Res<()> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        let mut disk = read_claims(store)?;
        let claim = disk.iter_mut().find(|c| c.unit == unit && c.attempt == attempt_id)
            .ok_or_else(|| Failure::new("claim_mismatch", "claim does not match"))?;
        let mut cp: Checkpoint = serde_json::from_slice(&std::fs::read(checkpoint_path(store, unit))
            .map_err(|e| Failure::new("io", e.to_string()))?)
            .map_err(|e| Failure::new("io", e.to_string()))?;
        if cp.attempt_id != attempt_id { return Err(Failure::new("claim_mismatch", "checkpoint does not match")); }
        cp.pid = 0;
        cp.updated = base::now_iso();
        write_checkpoint(store, &cp)?;
        // Empty is deliberately not a live PID (including on Unix, where PID 0 is a group).
        claim.pid.clear();
        save_claims(store, &disk)?;
        active.remove(&key(store, unit));
        Ok(())
    })
}
pub fn release_claim(store: &Path, unit: &str, attempt_id: &str) -> Res<()> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        let mut disk = read_claims(store)?;
        let index = disk
            .iter()
            .position(|c| c.unit == unit && c.attempt == attempt_id)
            .ok_or_else(|| {
                Failure::new(
                    "claim_mismatch",
                    "claim not found or attempt does not match",
                )
            })?;
        let removed = disk.remove(index);
        save_claims(store, &disk)?;
        match std::fs::remove_file(checkpoint_path(store, unit)) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(Failure::new("io", e.to_string())),
        }

        if active
            .get(&key(store, unit))
            .is_some_and(|c| c.attempt_id == attempt_id)
        {
            active.remove(&key(store, unit));
        }
        if store.join("modules.md").is_file()
            && !removed.module.is_empty()
            && scheduler::module_verified(store, &removed.module)?
        {
            super::events::append_event(
                store,
                super::events::MODULE_VERIFIED,
                Some(&removed.module),
                None,
                serde_json::json!({}),
            )?;
        }
        Ok(())
    })
}
pub fn abort_unit(store: &Path, unit: &str, attempt_id: &str) -> bool {
    release_claim(store, unit, attempt_id).is_ok()
}
pub fn running() -> Vec<String> {
    let mut v: Vec<_> = claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .map(|k| k.rsplit('|').next().unwrap_or(k).to_owned())
        .collect();
    v.sort();
    v
}
pub fn ready_units(store: &Path) -> Res<Vec<String>> {
    let units = read_units(store)?;
    let disk = read_claims(store)?;
    let mut selected: Vec<Vec<String>> = disk.iter().map(|c| c.scope.clone()).collect();
    let mut result = vec![];
    for u in units.iter().filter(|u| u.superseded_by().is_none()) {
        let rows = evidence(store, &u.id)?;
        let state = derive_state(&rows, needs_accept(store, &u.id)?);
        if !known_track(store, u)?
            || !["pending", "failed"].contains(&state.as_str())
            || rows.iter().filter(|r| r.kind == "run").count() >= 2
            || !dependencies(store, u, &units)?.is_empty()
        {
            continue;
        }
        let brief = match unit_brief(store, &u.id) {
            Ok(b) => b,
            Err(e) if e.kind == "no_brief" => continue,
            Err(e) => return Err(e),
        };
        if disk.iter().any(|c| c.unit == u.id) || selected.iter().any(|s| overlaps(s, &brief.scope))
        {
            continue;
        }
        selected.push(brief.scope);
        result.push(u.id.clone());
    }
    Ok(result)
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct Waiting {
    pub unit: String,
    pub state: String,
    pub next: String,
}

pub fn waiting_units(store: &Path) -> Res<Vec<Waiting>> {
    let ready = ready_units(store)?;
    let active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let disk = read_claims(store)?;
    let mut waiting = Vec::new();
    for u in units.iter().filter(|u| u.superseded_by().is_none()) {
        let rows = evidence(store, &u.id)?;
        let needs = needs_accept(store, &u.id)?;
        let state = derive_state(&rows, needs);
        let known = known_track(store, u)?;
        if (known && state == "verified" && !disk.iter().any(|c| c.unit == u.id))
            || ready.contains(&u.id)
        {
            continue;
        }
        let next = if !known {
            "unknown_track".into()
        } else if let Some(c) = disk.iter().find(|c| c.unit == u.id) {
            if active
                .get(&key(store, &u.id))
                .is_some_and(|a| a.attempt_id == c.attempt)
            {
                "running".to_string()
            } else {
                format!("unknown {}", c.attempt)
            }
        } else if ["built", "accept-failed", "review-failed"].contains(&state.as_str()) {
            next_step(&rows, &state, needs)
        } else {
            let missing = dependencies(store, u, &units)?;
            if !missing.is_empty() {
                format!("wait {}", missing.join(", "))
            } else if rows.iter().filter(|r| r.kind == "run").count() >= 2
                || unit_brief(store, &u.id).is_err()
            {
                "blocked".into()
            } else {
                "busy".into()
            }
        };
        waiting.push(Waiting {
            unit: u.id.clone(),
            state,
            next,
        });
    }
    Ok(waiting)
}
pub fn next_step_for(store: &Path, unit: &str) -> Res<String> {
    if ready_units(store)?.iter().any(|id| id == unit) {
        return Ok("ready".into());
    }
    Ok(waiting_units(store)?
        .into_iter()
        .find(|w| w.unit == unit)
        .map(|w| w.next)
        .unwrap_or_else(|| "verified".into()))
}
#[derive(Debug)]
pub enum AutoAccept {
    Ran {
        module: String,
        exit_code: i32,
        output: String,
        output_file: PathBuf,
        states: Vec<(String, String)>,
    },
    Skipped {
        reason: String,
        detail: String,
    },
}

pub fn accept_if_ready(store: &Path, unit: &str, repo: &Path) -> Res<AutoAccept> {
    let skipped = |reason: &str, detail: String| AutoAccept::Skipped {
        reason: reason.into(),
        detail,
    };
    if !store.join("modules.md").exists() {
        return Ok(skipped("not_module", String::new()));
    }
    let units = read_units(store)?;
    let Some(trigger) = units.iter().find(|u| u.id == unit) else {
        return Err(Failure::new("not_found", format!("unit {unit} not found")));
    };
    if !known_track(store, trigger)? {
        return Ok(skipped("not_module", String::new()));
    }
    static LOCKS: OnceLock<Mutex<HashMap<(PathBuf, String), Arc<Mutex<()>>>>> = OnceLock::new();
    let store_key = std::fs::canonicalize(store).unwrap_or_else(|_| store.to_path_buf());
    let lock = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry((store_key, trigger.track.clone()))
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let module_units: Vec<_> = units
        .iter()
        .filter(|u| u.track == trigger.track && u.superseded_by().is_none())
        .collect();
    let claimed = with_lock(store, false, || {
        let claims = read_claims(store)?;
        let mut claimed = Vec::new();
        for other in &module_units {
            let Some(claim) = claims.iter().find(|c| c.unit == other.id) else { continue; };
            if claim.unit == unit && claim.pid == std::process::id().to_string() { continue; }
            // Resume keeps ownership through ACCEPT/review. A peer that has finished
            // self-check can no longer mutate its build and must not block ACCEPT.
            let checkpoint = match std::fs::read(checkpoint_path(store, &other.id)) {
                Ok(bytes) => Some(serde_json::from_slice::<Checkpoint>(&bytes)
                    .map_err(|e| Failure::new("io", e.to_string()))?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(Failure::new("io", e.to_string())),
            };
            if !checkpoint.is_some_and(|cp| cp.attempt_id == claim.attempt
                && matches!(cp.stage.as_str(), "selfchecked" | "reviewed")
                && cp.finished.is_some_and(|f| f.verify_exit == Some(0)))
            {
                claimed.push(other.id.clone());
            }
        }
        Ok(claimed)
    })?;
    if !claimed.is_empty() {
        return Ok(skipped("claimed", claimed.join(", ")));
    }
    let rows = evidence(store, unit)?;
    let built_verify = |rows: &[Evidence]| -> Option<usize> {
        let run = rows.iter().rposition(|r| r.kind == "run")?;
        if rows[run].exit_code != 0 {
            return None;
        }
        let verify = rows.iter().rposition(|r| r.kind == "verify")?;
        if verify <= run
            || rows[verify].exit_code != 0
            || rows[run + 1..]
                .iter()
                .any(|r| r.kind == "scope" && r.exit_code != 0)
        {
            return None;
        }
        Some(verify)
    };
    let Some(verify) = built_verify(&rows) else {
        return Ok(skipped("not_built", String::new()));
    };
    if rows[verify + 1..].iter().any(|r| r.kind == "accept") {
        return Ok(skipped("already_accepted", String::new()));
    }
    let mut pending = Vec::new();
    for other in module_units.iter().filter(|u| u.id != unit) {
        let rows = evidence(store, &other.id)?;
        if built_verify(&rows).is_none() {
            pending.push(format!("{}={}", other.id, derive_state(&rows, true)));
        }
    }
    if !pending.is_empty() {
        return Ok(skipped("units_pending", pending.join(", ")));
    }
    let result = record_accept(store, &trigger.track, repo)?;
    let states = module_units
        .iter()
        .map(|u| Ok((u.id.clone(), derive_state(&evidence(store, &u.id)?, true))))
        .collect::<Res<Vec<_>>>()?;
    let output: String = result
        .output
        .chars()
        .rev()
        .take(2000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Ok(AutoAccept::Ran {
        module: trigger.track.clone(),
        exit_code: result.exit_code,
        output,
        output_file: super::evidence::accept_output_path(store, &trigger.track),
        states,
    })
}
/// Project-relative paths of a worker report's `file_changes`.
pub(crate) fn report_paths(report: &Value, repo: &Path) -> Vec<String> {
    report
        .get("file_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("path").and_then(Value::as_str))
        .map(|p| {
            let p = Path::new(p);
            let p = if p.is_absolute() {
                p.strip_prefix(repo).unwrap_or(p)
            } else {
                p
            };
            p.to_string_lossy().replace('\\', "/")
        })
        .collect()
}

fn ignored_reason(repo: &Path, path: &str) -> Option<&'static str> {
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or("");
    let second = parts.next().unwrap_or("");
    let scratch = path.starts_with(TEMP_DIR)
        || (first
            .strip_prefix("pytest-of-")
            .is_some_and(|name| !name.is_empty())
            && second.strip_prefix("pytest-").is_some_and(|n| {
                n == "current" || (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            && parts.next().is_some());
    if scratch {
        return Some("test-scratch");
    }
    base::proc::cmd("git")
        .current_dir(repo)
        .args(["check-ignore", "-q", "--", path])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|_| "git-ignored")
}
pub fn finish_unit(
    store: &Path,
    unit: &str,
    attempt_id: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
) -> Res<Finished> {
    finish_inner(store, unit, attempt_id, repo, role, model, report, false)
}
pub(crate) fn finish_checkpointed(store: &Path, unit: &str, attempt_id: &str, repo: &Path, role: &str, model: &str, report: &Value) -> Res<Finished> {
    finish_inner(store, unit, attempt_id, repo, role, model, report, true)
}
fn finish_inner(store: &Path, unit: &str, attempt_id: &str, repo: &Path, role: &str, model: &str, report: &Value, keep_claim: bool) -> Res<Finished> {
    struct Release<'a>(&'a Path, &'a str, &'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            abort_unit(self.0, self.1, self.2);
        }
    }

    let brief = unit_brief(store, unit)?;
    let (before, had_peers, base_tree) = {
        let active = claims().lock().unwrap_or_else(|e| e.into_inner());
        let c = active
            .get(&key(store, unit))
            .filter(|c| c.attempt_id == attempt_id)
            .ok_or_else(|| Failure::new("claim_mismatch", "unit attempt is not running"))?;
        let had_peers = with_lock(store, false, || {
            let disk = read_claims(store)?;
            let own = disk
                .iter()
                .find(|d| d.unit == unit && d.attempt == attempt_id)
                .ok_or_else(|| Failure::new("claim_mismatch", "disk claim does not match"))?;
            Ok(!own.peers.is_empty() || disk.iter().any(|d| d.attempt != attempt_id))
        })?;
        (c.dirty.clone(), had_peers, c.base.clone())
    };
    let _release = (!keep_claim).then(|| Release(store, unit, attempt_id));
    let mut cp: Checkpoint = with_lock(store, false, || {
        serde_json::from_slice(&std::fs::read(checkpoint_path(store, unit))
            .map_err(|e| Failure::new("io", e.to_string()))?)
            .map_err(|e| Failure::new("io", e.to_string()))
    })?;
    if cp.finished_tree == snapshot(repo, &brief.scope) {
        if let Some(finished) = &cp.finished { return Ok(finished.clone()); }
    }
    let append_once = |row: Evidence| -> Res<()> {
        with_lock(store, false, || {
            if evidence(store, unit)?.iter().skip(cp.evidence_start).any(|r| r.kind == row.kind) { return Ok(()); }
            super::evidence::append_evidence_locked(store, unit, &row)
        })
    };
    struct ScratchCleanup(PathBuf, PathBuf, crate::storage::Owner);
    impl Drop for ScratchCleanup {
        fn drop(&mut self) {
            let result = crate::storage::retention::remove_scratch(
                &self.1,
                &self.0,
                &self.2,
                "unit_ended",
                false,
            );
            if result.outcome != "done" {
                eprintln!("scratch cleanup: {}", result.outcome);
            }
        }
    }
    let (home, owner) = super::store::storage_context(store);
    let attempt_scratch = scratch_dir(store, &[unit, attempt_id]);
    crate::storage::ensure_dir(
        &home,
        &attempt_scratch,
        crate::storage::Class::Scratch,
        &owner,
    )?;
    let _scratch = ScratchCleanup(attempt_scratch, home.clone(), owner.clone());
    let status = report
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let mk = |kind: &str, command: &str, code: i32, commit: String| Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code: code,
        commit,
        ts: base::now_iso(),
    };
    append_once(mk(
            "run",
            model,
            if status == "completed" { 0 } else { 1 },
            base_tree,
        ))?;
    let dir = store.join("reports");
    crate::storage::ensure_dir(&home, &dir, crate::storage::Class::Evidence, &owner)?;
    let md = dir.join(format!("{unit}.md"));
    base::atomic_write(
        &md,
        report
            .get("final_message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes(),
    )
    .map_err(|e| Failure::new("io", e.to_string()))?;
    let mut saved_report = report.clone();
    saved_report["brief_sha256"] = serde_json::json!(cp.brief_sha256);
    let json = serde_json::to_vec_pretty(&saved_report)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    base::atomic_write(&dir.join(format!("{unit}.json")), &json)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    append_inbox(store, role, unit, status, Some(&md.to_string_lossy()))?;
    let tree: BTreeSet<String> = dirty(repo)?.difference(&before).cloned().collect();
    let mut paths: BTreeSet<String> = report_paths(report, repo).into_iter().collect();
    if !had_peers {
        paths.extend(tree.iter().cloned());
    }
    let changed: Vec<_> = paths.into_iter().collect();
    if let Ok(bytes) = serde_json::to_vec(&changed) {
        let _ = base::atomic_write(&dir.join(format!("{unit}.changed.json")), &bytes);
    }
    let ignored: Vec<_> = changed
        .iter()
        .filter_map(|p| ignored_reason(repo, p).map(|reason| (p.clone(), reason.to_owned())))
        .collect();
    let outside_scope: Vec<_> = changed
        .iter()
        .filter(|p| !ignored.iter().any(|(path, _)| path == *p) && !covers(&brief.scope, p))
        .cloned()
        .collect();
    let unattributed: Vec<_> = if had_peers {
        tree.into_iter()
            .filter(|p| {
                !changed.contains(p)
                    && !covers(&brief.scope, p)
                    && ignored_reason(repo, p).is_none()
            })
            .collect()
    } else {
        vec![]
    };
    append_once(mk(
            "scope",
            &outside_scope.join(" "),
            if outside_scope.is_empty() { 0 } else { 1 },
            String::new(),
        ))?;
    if !ignored.is_empty() {
        let command = ignored
            .iter()
            .map(|(path, reason)| format!("{path} ({reason})"))
            .collect::<Vec<_>>()
            .join("; ");
        append_once(mk("scope-ignored", &command, 0, String::new()))?;
    }
    if !unattributed.is_empty() {
        append_once(mk(
                "scope-unattributed",
                &unattributed.join(" "),
                0,
                String::new(),
            ))?;
    }
    let scope_json = serde_json::json!({"outside": outside_scope, "ignored": ignored.iter().map(|(path, reason)| serde_json::json!({"path": path, "reason": reason})).collect::<Vec<_>>(), "unattributed": unattributed});
    let bytes = serde_json::to_vec_pretty(&scope_json)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    base::atomic_write(&dir.join(format!("{unit}.scope.json")), &bytes)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    let mut verify_output = None;
    let mut verify_exit = None;
    let mut verify_tail = String::new();
    let mut verify_error = None;
    let mut pack = None;
    let mut pack_error = None;
    if status == "completed" && outside_scope.is_empty() {
        let verified_tree = snapshot(repo, &brief.scope);
        let existing = evidence(store, unit)?.into_iter().skip(cp.evidence_start)
            .rev().find(|r| r.kind == "verify" && r.command == brief.verify && r.commit == verified_tree);
        let output_path = dir.join(format!("{unit}.verify.txt"));
        if let Some(check) = existing.filter(|_| output_path.is_file()) {
            verify_exit = Some(check.exit_code);
            verify_tail = std::fs::read_to_string(&output_path).unwrap_or_default();
            verify_output = Some(output_path);
            if check.exit_code == 0 {
                match super::report::review_pack(store, unit, repo) {
                    Ok(info) => pack = Some(info.path),
                    Err(e) => pack_error = Some(e.message),
                }
            }
        } else {
        let verify_scratch = scratch_dir(store, &[unit, attempt_id, "verify"]);
        crate::storage::ensure_dir(
            &home,
            &verify_scratch,
            crate::storage::Class::Scratch,
            &owner,
        )?;
        let verify_cleanup = ScratchCleanup(verify_scratch.clone(), home.clone(), owner.clone());
        let lease = crate::storage::Ledger::open(&home)?.lease(&verify_scratch)?;
        let mut verify_env = base::proc::scratch_env(&verify_scratch)
            .map_err(|e| Failure::new("io", e.to_string()))?;
        verify_env.extend(crate::storage::worktree::worker_env(&home, repo, &owner)?);
        let verify_result =
            base::proc::run_shell_env(&brief.verify, repo, Duration::from_secs(1800), &verify_env);
        drop(lease);
        drop(verify_cleanup);
        match verify_result {
            Ok(result) => {
                verify_exit = Some(result.exit_code);
                let path = dir.join(format!("{unit}.verify.txt"));
                base::atomic_write(&path, result.output.as_bytes())
                    .map_err(|e| Failure::new("io", e.to_string()))?;
                verify_output = Some(path);
                verify_tail = result
                    .output
                    .chars()
                    .rev()
                    .take(2000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                super::evidence::append_check(
                    store,
                    unit,
                    repo,
                    &mk(
                        "verify",
                        &brief.verify,
                        result.exit_code,
                        snapshot(repo, &brief.scope),
                    ),
                )?;
                if result.exit_code == 0 {
                    match super::report::review_pack(store, unit, repo) {
                        Ok(info) => pack = Some(info.path),
                        Err(e) => pack_error = Some(e.message),
                    }
                }
            }
            Err(e) => verify_error = Some(e.message),
        }
        }
    }
    let temp = repo.join(TEMP_DIR);
    let _ = std::fs::remove_dir_all(temp.join(unit));
    let _ = std::fs::remove_dir(&temp);
    let line = match (&verify_output, verify_exit) {
        (Some(path), Some(code)) => format!("BRIDGE VERIFY exit={code} output={}", path.display()),
        _ if verify_error.is_some() => format!("BRIDGE VERIFY not run: {}", verify_error.unwrap()),
        _ if status != "completed" => "BRIDGE VERIFY not run: worker did not complete".to_string(),
        _ => "BRIDGE VERIFY not run: changes outside scope".to_string(),
    };
    let message = report
        .get("final_message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .replace("\r\n", "\n");
    base::atomic_write(
        &md,
        format!(
            "{}{}{}\n",
            message,
            if message.is_empty() || message.ends_with('\n') {
                ""
            } else {
                "\n"
            },
            line
        )
        .as_bytes(),
    )
    .map_err(|e| Failure::new("io", e.to_string()))?;
    let state = update_unit_state(store, unit)?;
    let finished = Finished {
        state,
        verify_exit,
        verify_tail,
        verify_output,
        outside_scope,
        ignored,
        unattributed,
        changed,
        pack,
        pack_error,
    };
    cp.finished = Some(finished.clone());
    cp.finished_tree = snapshot(repo, &brief.scope);
    cp.stage = if finished.verify_exit.is_some() { "verified" } else { "worker_done" }.into();
    cp.updated = base::now_iso();
    with_lock(store, false, || write_checkpoint(store, &cp))?;
    Ok(finished)
}


fn matrix_tree(repo: &Path, brief: &Brief) -> String {
    snapshot(repo, &brief.test_matrix.iter().map(|r| r.file.clone()).collect::<Vec<_>>())
}
fn matrix_cp(store: &Path, unit: &str, attempt: &str) -> Res<Checkpoint> {
    let path = checkpoint_path(store, unit);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::read(store.join("reports").join(format!("{unit}.{attempt}.red.json")))
            .map_err(|e| Failure::new("red_invalid", format!("matrix row has no engine RED receipt: {e}")))?,
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let cp: Checkpoint = serde_json::from_slice(&bytes)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    if cp.attempt_id != attempt { return Err(Failure::new("red_invalid", "RED attempt mismatch")); }
    Ok(cp)
}
fn matrix_command(store: &Path, unit: &str, attempt: &str, row: &super::parse::MatrixRow, repo: &Path, kind: &str) -> Res<base::proc::ShellRun> {
    let (home, owner) = super::store::storage_context(store);
    let scratch = scratch_dir(store, &[unit, attempt, kind, &row.id]);
    crate::storage::ensure_dir(&home, &scratch, crate::storage::Class::Scratch, &owner)?;
    let mut env = base::proc::scratch_env(&scratch).map_err(|e| Failure::new("io", e.to_string()))?;
    env.extend(crate::storage::worktree::worker_env(&home, repo, &owner)?);
    let lease = crate::storage::Ledger::open(&home)?.lease(&scratch)?;
    let result = base::proc::run_shell_env(&row.command, repo, Duration::from_secs(1800), &env);
    drop(lease);
    let _ = crate::storage::retention::remove_scratch(&home, &scratch, &owner, "unit_ended", false);
    let result = result?;
    let dir = store.join("reports");
    crate::storage::ensure_dir(&home, &dir, crate::storage::Class::Evidence, &owner)?;
    base::atomic_write(&dir.join(format!("{unit}.{attempt}.{kind}.{}.txt", row.id)), result.output.as_bytes())
        .map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(result)
}
fn compile_or_missing(output: &str, code: i32, case: &str) -> bool {
    let o = output.to_lowercase();
    [127,9009].contains(&code) || ["could not compile", "error[e", "syntaxerror", "compilation failed", "not recognized"].iter().any(|s|o.contains(s)) ||
        (!failing_case(output,case) && (o.contains("not found") || o.lines().any(|l|l.trim_start().starts_with("error:"))))
}
fn failing_case(output: &str, case: &str) -> bool {
    output.lines().any(|line| {
        let l=line.trim(); let lower=l.to_lowercase();
        l.contains(case) && (lower.starts_with("fail:") || lower.starts_with("failed ") || lower.split_whitespace().any(|s|s=="failed") || lower.ends_with("... fail") || lower.ends_with("... failed") || (lower.starts_with("test ") && lower.ends_with(" failed")))
    })
}
fn passing_tests(output: &str) -> bool {
    output.lines().any(|line| {
        let l=line.trim().to_lowercase();
        (l.starts_with("test ") && l.ends_with(" ... ok")) || l.ends_with(" ... ok") ||
        l.split_whitespace().collect::<Vec<_>>().windows(2).any(|w| w[0].parse::<usize>().is_ok_and(|n|n>0) && matches!(w[1], "passed" | "passed;"))
    }) || (output.lines().any(|l|l.trim()=="OK") && output.lines().any(|l| {
        let words:Vec<_>=l.split_whitespace().collect(); words.len()>=3 && words[0]=="Ran" && words[1].parse::<usize>().is_ok_and(|n|n>0) && words[2].starts_with("test")
    }))
}
/// Only engine-observed RED rows belonging to this checkpoint and test-file tree count.
pub fn validate_red(store: &Path, unit: &str, attempt: &str, repo: &Path) -> Res<bool> {
    let brief=unit_brief(store,unit)?;
    if brief.test_matrix.is_empty() { return Ok(true); }
    let cp=matrix_cp(store,unit,attempt)?;
    let Some(tree)=cp.red_tree else { return Ok(false); };
    if tree.is_empty() || tree!=matrix_tree(repo,&brief) {
        return Err(Failure::new("red_tampered", format!("matrix row {} test files changed after RED", brief.test_matrix[0].id)));
    }
    let rows=evidence(store,unit)?;
    for entry in &brief.test_matrix {
        let command=format!("{attempt}|{}|{}",entry.id,entry.command);
        let output=std::fs::read_to_string(store.join("reports").join(format!("{unit}.{attempt}.red.{}.txt",entry.id))).unwrap_or_default();
        if !rows.iter().skip(cp.evidence_start).any(|r|r.kind=="red" && r.command==command && r.commit==tree && r.exit_code!=0 && !compile_or_missing(&output,r.exit_code,&entry.case) && failing_case(&output,&entry.case)) {
            return Err(Failure::new("red_invalid",format!("matrix row {} has no valid RED for this attempt and tree",entry.id)));
        }
    }
    Ok(true)
}
pub fn record_red(store: &Path, unit: &str, attempt: &str, repo: &Path) -> Res<()> {
    let brief=unit_brief(store,unit)?;
    let tree=matrix_tree(repo,&brief);
    let mut invalid = None;
    for entry in &brief.test_matrix {
        if !repo.join(&entry.file).is_file() {
            return Err(Failure::new("red_invalid",format!("matrix row {} test file is missing",entry.id)));
        }
        let result=matrix_command(store,unit,attempt,entry,repo,"red")?;
        super::evidence::append_evidence(store,unit,&Evidence{kind:"red".into(),command:format!("{attempt}|{}|{}",entry.id,entry.command),exit_code:result.exit_code,commit:tree.clone(),ts:base::now_iso()})?;
        if result.exit_code==0 || compile_or_missing(&result.output,result.exit_code,&entry.case) || !failing_case(&result.output,&entry.case) {
            if invalid.is_none() { invalid = Some(Failure::new("red_invalid",format!("matrix row {} did not fail a named assertion test",entry.id))); }
        }
    }
    if let Some(error) = invalid { return Err(error); }
    if tree.is_empty() || matrix_tree(repo,&brief)!=tree { return Err(Failure::new("red_invalid","matrix row files changed during RED")); }
    with_lock(store,false,|| {
        let mut cp=matrix_cp(store,unit,attempt)?;
        cp.red_tree=Some(tree);
        let bytes=serde_json::to_vec(&cp).map_err(|e|Failure::new("io",e.to_string()))?;
        base::atomic_write(&store.join("reports").join(format!("{unit}.{attempt}.red.json")),&bytes)
            .map_err(|e|Failure::new("io",e.to_string()))?;
        write_checkpoint(store,&cp)
    })
}
pub fn final_matrix(store: &Path, unit: &str, repo: &Path, attempt: &str) -> Res<()> {
    let brief=unit_brief(store,unit)?;
    if brief.test_matrix.is_empty() { return Ok(()); }
    if !validate_red(store,unit,attempt,repo)? { return Err(Failure::new("red_invalid","matrix row has no engine RED")); }
    let mut failed = None;
    for entry in &brief.test_matrix {
        let result=matrix_command(store,unit,attempt,entry,repo,"matrix")?;
        super::evidence::append_evidence(store,unit,&Evidence{kind:"matrix".into(),command:format!("{attempt}|{}|{}",entry.id,entry.command),exit_code:result.exit_code,commit:snapshot(repo,&brief.scope),ts:base::now_iso()})?;
        let kind=if result.exit_code!=0 { Some("matrix_failed") } else if !passing_tests(&result.output) { Some("matrix_not_run") } else { None };
        if failed.is_none() { if let Some(kind)=kind { failed=Some(Failure::new(kind,format!("matrix row {} did not pass tests",entry.id))); } }
    }
    validate_red(store,unit,attempt,repo)?;
    super::report::review_pack(store,unit,repo)?;
    if let Some(error) = failed { return Err(error); }
    Ok(())
}
