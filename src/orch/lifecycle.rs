use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

pub enum Begin {
    Run { brief: Brief, attempt: u32 },
    Reused,
}
pub struct Finished {
    pub state: String,
    pub verify_exit: Option<i32>,
    pub verify_tail: String,
    pub outside_scope: Vec<String>,
    pub verify_receipt: Option<Value>,
}
struct Claim {
    _lease: Arc<WorkerLease>,
    // Units of different projects never block or excuse each other.
    store: PathBuf,
    scope: Vec<String>,
    // Scopes whose claims overlapped this run, including peers already released.
    peers: Vec<String>,
    dirty: HashMap<String, String>,
    contract: String,
    dispatch_id: Option<String>,
    quarantined: bool,
}
// A store has one process owner for the entire model + verification lifetime.
// In-process disjoint scopes still run concurrently and retain peer attribution.
// Never steal a crashed owner's lease: its remote writer may still be running.
struct WorkerLease {
    path: PathBuf,
    token: String,
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|s| s == self.token) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn worker_lease(store: &Path, active: &HashMap<String, Claim>) -> Res<Arc<WorkerLease>> {
    let path = store.join(".worker.claim");
    if let Some(claim) = active.values().find(|c| c.store == store) {
        if !std::fs::read_to_string(&path).is_ok_and(|s| s == claim._lease.token) {
            return Err(Failure::new(
                "busy",
                "worker lease changed; explicit recovery required",
            ));
        }
        return Ok(claim._lease.clone());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            Failure::new(
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    "busy"
                } else {
                    "io"
                },
                format!("worker store claim: {e}; abandoned claims require explicit recovery"),
            )
        })?;
    let lease = Arc::new(WorkerLease {
        path,
        token: format!("{}:{}", std::process::id(), base::unique()),
    });
    use std::io::Write;
    file.write_all(lease.token.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(lease)
}
static RUNNING: OnceLock<Mutex<HashMap<String, Claim>>> = OnceLock::new();
fn key(store: &Path, unit: &str) -> String {
    format!("{}|{unit}", store.display())
}
fn claims() -> &'static Mutex<HashMap<String, Claim>> {
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}
fn dirty(repo: &Path) -> Res<BTreeSet<String>> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(["status", "--porcelain", "-z", "--untracked-files=all"])
        .output()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    if !out.status.success() {
        return Err(Failure::new(
            "process",
            format!(
                "git status failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    let mut parts = out.stdout.split(|b| *b == 0);
    let mut result = BTreeSet::new();
    while let Some(p) = parts.next() {
        if p.len() < 4 {
            continue;
        }
        let status = &p[..2];
        result.insert(String::from_utf8_lossy(&p[3..]).replace('\\', "/"));
        if status.contains(&b'R') || status.contains(&b'C') {
            if let Some(old) = parts.next() {
                result.insert(String::from_utf8_lossy(old).replace('\\', "/"));
            }
        }
    }
    Ok(result)
}
fn dirty_identities(repo: &Path) -> Res<HashMap<String, String>> {
    dirty(repo)?
        .into_iter()
        .map(|path| {
            let identity = match std::fs::read(repo.join(&path)) {
                Ok(bytes) => format!("sha256:{:x}", Sha256::digest(bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing".into(),
                Err(e) => return Err(Failure::new("io", format!("{path}: {e}"))),
            };
            Ok((path, identity))
        })
        .collect()
}
fn dependencies(store: &Path, u: &Unit, units: &[Unit], repo: Option<&Path>) -> Res<Vec<String>> {
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
        if !units.iter().any(|u| &u.track == dep) {
            missing.push(dep.clone());
        }
    }
    for other in units.iter().filter(|x| deps.contains(&x.track)) {
        if !task_released(&wait_state(store)?, &other.id)
            || match repo {
                Some(repo) => !current_verified_unit(store, &other.id, repo)?,
                None => {
                    derive_state(
                        &evidence(store, &other.id)?,
                        needs_accept(store, &other.id)?,
                    ) != "verified"
                }
            }
        {
            missing.push(other.id.clone());
        }
    }
    Ok(missing)
}
pub fn begin_unit(
    store: &Path,
    unit: &str,
    _role: &str,
    repo: &Path,
    astra_required: bool,
) -> Res<Begin> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    if store.join("quarantine").exists()
        || active.values().any(|claim| claim.store == store && claim.quarantined) {
        return Err(Failure::new(
            "quarantined",
            "unconfirmed writer; explicit recovery required",
        ));
    }
    let lease = with_lock(store, false, || worker_lease(store, &active))?;
    let units = read_units(store)?;
    let u = units
        .iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let brief = unit_brief(store, unit)?;
    let rows = evidence(store, unit)?;
    let missing = dependencies(store, u, &units, Some(repo))?;
    if !missing.is_empty() {
        return Err(Failure::new("not_ready", missing.join(", ")));
    }
    if active.contains_key(&key(store, unit))
        || active
            .values()
            .any(|c| c.store == store && overlaps(&brief.scope, &c.scope))
    {
        return Err(Failure::new("busy", format!("scope busy: {unit}")));
    }
    let wait = wait_state(store)?;
    let enrolled = wait.checkpoints.iter().any(|c| c.task == unit);
    if store.join("wait-required").exists() && !enrolled {
        return Err(Failure::new(
            "not_ready",
            "opted-in workflow requires a task checkpoint",
        ));
    }
    if let Some(c) = wait
        .checkpoints
        .iter()
        .filter(|c| c.task == unit)
        .max_by_key(|c| c.round)
    {
        if c.contract != super::contract_identity(store, unit)? || c.module != u.track {
            return Err(Failure::new(
                "not_ready",
                "checkpoint contract/module is stale",
            ));
        }
        if std::fs::canonicalize(&c.repo).ok() != std::fs::canonicalize(repo).ok() {
            return Err(Failure::new("not_ready", "checkpoint workspace mismatch"));
        }
    }
    if current_verified_unit(store, unit, repo)? && task_released(&wait, unit) {
        return Ok(Begin::Reused);
    }
    if !pre_dispatch_review_ready(&wait, unit) {
        return Err(Failure::new(
            "not_ready",
            "round six and later require a verified pre-dispatch MAX review",
        ));
    }
    let count = rows.iter().filter(|r| r.kind == "run").count();
    if count >= 2 && !enrolled {
        return Err(Failure::new(
            "fix_limit",
            format!("unit {unit} has reached its fix limit"),
        ));
    }
    if astra_required {
        astra_gate(store)?;
    }
    let mut claim = Claim {
        _lease: lease,
        store: store.to_path_buf(),
        scope: brief.scope.clone(),
        peers: Vec::new(),
        dirty: dirty_identities(repo)?,
        contract: super::evidence::contract_identity(store, unit)?,
        dispatch_id: None,
        quarantined: false,
    };
    // Persist the dispatch claim before the caller can start a worker.
    super::wait::commit_dispatch_bound(store, unit, Some(&claim.contract))?;
    claim.dispatch_id = wait_state(store)?
        .events
        .iter()
        .rev()
        .find(|e| e.task == unit && e.kind == "dispatch")
        .map(|e| e.id.clone());
    for peer in active.values_mut().filter(|c| c.store == store) {
        claim.peers.extend(peer.scope.clone());
        peer.peers.extend(brief.scope.clone());
    }
    active.insert(key(store, unit), claim);
    Ok(Begin::Run {
        brief,
        attempt: count as u32 + 1,
    })
}
pub fn abort_unit(store: &Path, unit: &str) {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    if !active.get(&key(store, unit)).is_some_and(|c| c.quarantined) {
        active.remove(&key(store, unit));
    }
}
/// Prevent the in-process guard from releasing a claim when no safe journal
/// commit is available. This must not itself depend on filesystem I/O.
pub(crate) fn retain_unit(store: &Path, unit: &str) -> Res<()> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let claim = active.get_mut(&key(store, unit))
        .ok_or_else(|| Failure::new("not_found", "unit is not running"))?;
    claim.quarantined = true;
    Ok(())
}
/// Quarantine the whole store after an unconfirmed writer. Never expires automatically.
/// Call before releasing the claim; recovery requires external confirmation all writers stopped.
pub fn quarantine_unit(store: &Path, unit: &str) -> Res<()> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let claim = active
        .get_mut(&key(store, unit))
        .ok_or_else(|| Failure::new("not_found", "unit is not running"))?;
    claim.quarantined = true;
    let path = store.join("quarantine");
    std::fs::create_dir_all(&path).map_err(|e| Failure::new("io", e.to_string()))?;
    let name = format!("{:x}", Sha256::digest(unit.as_bytes()));
    std::fs::write(path.join(name), unit).map_err(|e| Failure::new("io", e.to_string()))
}
/// Clear quarantine only after independently confirming every uncertain writer has stopped.
pub fn recover_quarantine(store: &Path) -> Res<()> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        recover_quarantine_inner(store, &mut active)
    })
}
fn recover_quarantine_inner(store: &Path, active: &mut HashMap<String, Claim>) -> Res<()> {
    if active.values().any(|c| c.store == store && !c.quarantined) {
        return Err(Failure::new(
            "busy",
            "cannot recover a confirmed active worker",
        ));
    }
    let lease = store.join(".worker.claim");
    if let Ok(token) = std::fs::read_to_string(&lease) {
        let owner = token.split(':').next().and_then(|s| s.parse::<u32>().ok());
        if owner.is_some_and(|pid| pid != std::process::id() && base::proc::pid_alive(pid)) {
            return Err(Failure::new(
                "busy",
                "worker owner is still alive; close it before recovery",
            ));
        }
    }
    let path = store.join("quarantine");
    let mut entries = Vec::new();
    let files = match std::fs::read_dir(&path) {
        Ok(files) => Some(files),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    for entry in files.into_iter().flatten() {
        let entry = entry.map_err(|e| Failure::new("io", e.to_string()))?;
        let unit =
            std::fs::read_to_string(entry.path()).map_err(|e| Failure::new("io", e.to_string()))?;
        entries.push((entry.path(), unit));
    }
    for (file, _) in &entries {
        std::fs::remove_file(file).map_err(|e| Failure::new("io", e.to_string()))?;
    }
    match std::fs::remove_file(lease) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    }
    if path.exists() {
        std::fs::remove_dir(path).map_err(|e| Failure::new("io", e.to_string()))?;
    }
    for (_, unit) in entries {
        active.remove(&key(store, &unit));
    }
    Ok(())
}
// Serialize explicit recovery against both local begin and cross-process lease
// acquisition. Persist recovery first; a cleanup failure remains fail-closed.
pub(crate) fn with_dispatch_recovery<T>(store: &Path, f: impl FnOnce() -> Res<T>) -> Res<T> {
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        if active.values().any(|c| c.store == store && !c.quarantined) {
            return Err(Failure::new(
                "busy",
                "release/reap local workers before dispatch recovery",
            ));
        }
        if let Ok(token) = std::fs::read_to_string(store.join(".worker.claim")) {
            let owner = token
                .split(':')
                .next()
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|p| *p > 0)
                .ok_or_else(|| {
                    Failure::new("busy", "invalid worker owner; manual inspection required")
                })?;
            if base::proc::pid_alive(owner)
                && !(owner == std::process::id()
                    && active.values().any(|c| c.store == store && c.quarantined))
            {
                return Err(Failure::new("busy", "worker owner still alive"));
            }
        }
        let result = f()?;
        recover_quarantine_inner(store, &mut active)?;
        Ok(result)
    })
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
    let active = claims().lock().unwrap_or_else(|e| e.into_inner());
    let units = read_units(store)?;
    let wait = wait_state(store)?;
    let mut selected: Vec<Vec<String>> = active
        .values()
        .filter(|c| c.store == store)
        .map(|c| c.scope.clone())
        .collect();
    let mut result = vec![];
    if store.join("quarantine").exists()
        || (store.join(".worker.claim").exists() && !active.values().any(|c| c.store == store))
    {
        return Ok(result);
    }
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let state = derive_state(&rows, needs_accept(store, &u.id)?);
        let enrolled = wait.checkpoints.iter().any(|c| c.task == u.id);
        if store.join("wait-required").exists() && !enrolled {
            continue;
        }
        if let Some(c) = wait
            .checkpoints
            .iter()
            .filter(|c| c.task == u.id)
            .max_by_key(|c| c.round)
        {
            if !pre_dispatch_review_ready(&wait, &u.id)
                || c.module != u.track
                || c.contract != super::contract_identity(store, &u.id)?
            {
                continue;
            }
        }
        if (!enrolled
            && (!["pending", "failed"].contains(&state.as_str())
                || rows.iter().filter(|r| r.kind == "run").count() >= 2))
            || (enrolled && !dispatch_ready(&wait, &u.id))
            || !dependencies(store, u, &units, None)?.is_empty()
        {
            continue;
        }
        let brief = match unit_brief(store, &u.id) {
            Ok(b) => b,
            Err(e) if e.kind == "no_brief" => continue,
            Err(e) => return Err(e),
        };
        if selected.iter().any(|s| overlaps(s, &brief.scope)) {
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
    let mut waiting = Vec::new();
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let needs = needs_accept(store, &u.id)?;
        let state = derive_state(&rows, needs);
        let wait = wait_state(store)?;
        if (state == "verified" && task_released(&wait, &u.id)) || ready.contains(&u.id) {
            continue;
        }
        let next = if active.contains_key(&key(store, &u.id)) {
            "running".to_string()
        } else if !dispatch_ready(&wait, &u.id) && !task_released(&wait, &u.id) {
            "checkpoint: review/release or explicit dispatch recovery".into()
        } else if state == "built" {
            let verify = rows.iter().rposition(|r| r.kind == "verify");
            if needs && verify.is_some_and(|v| !rows[v + 1..].iter().any(|r| r.kind == "accept")) {
                "accept".into()
            } else {
                "review".into()
            }
        } else if state == "accept-failed" || state == "review-failed" {
            "fix".into()
        } else {
            let missing = dependencies(store, u, &units, None)?;
            if !missing.is_empty() {
                format!("wait {}", missing.join(","))
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
pub fn finish_unit(
    store: &Path,
    unit: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
) -> Res<Finished> {
    finish_unit_inner(store, unit, repo, role, model, report, true, None)
}
pub(crate) fn finish_unit_held(
    store: &Path,
    unit: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
    before_verify: &mut dyn FnMut() -> Res<()>,
) -> Res<Finished> {
    finish_unit_inner(store, unit, repo, role, model, report, false, Some(before_verify))
}
/// Run the contracted check once and persist the complete Host-owned receipt.
pub(crate) fn host_verification_check(
    store: &Path, unit: &str, repo: &Path, verify: &str, binding: Option<&Value>,
    before_run: Option<&mut dyn FnMut() -> Res<()>>,
) -> Res<(base::proc::ShellRun, Value, String)> {
    let dir = store.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    let before = snapshot(repo, &[".".into()]);
    if before.is_empty() { return Err(Failure::new("verify_failed", "source identity unavailable")); }
    let id = base::unique();
    let out = dir.join(format!("verify-{unit}-{id}.stdout"));
    let err = dir.join(format!("verify-{unit}-{id}.stderr"));
    let target = base::cargo_target(repo, &format!("verify:{unit}:{id}"))?;
    // The redirection captures both streams without the process runner's bounded tail.
    #[cfg(windows)]
    let command = format!("set \"CARGO_TARGET_DIR={}\" && ({}) 1>\"{}\" 2>\"{}\"", target.display(), verify, out.display(), err.display());
    #[cfg(not(windows))]
    let command = {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        format!("export CARGO_TARGET_DIR={}; ( {} ) >{} 2>{}", quote(&target.to_string_lossy()), verify, quote(&out.to_string_lossy()), quote(&err.to_string_lossy()))
    };
    if let Some(before_run) = before_run { before_run()?; }
    let run = base::proc::run_shell(&command, repo, Duration::from_secs(1800));
    let after = snapshot(repo, &[".".into()]);
    let stdout = std::fs::read(&out).ok();
    let stderr = std::fs::read(&err).ok();
    let receipt = serde_json::json!({"binding":binding,"command":verify,"target_dir":target,"source_before":before,"source_after":after,"exit_code":run.as_ref().ok().map(|r| r.exit_code),"timed_out":run.as_ref().ok().map(|r| r.timed_out),"stdout":stdout.as_ref().map(|v| String::from_utf8_lossy(v).into_owned()),"stderr":stderr.as_ref().map(|v| String::from_utf8_lossy(v).into_owned()),"error":run.as_ref().err().map(|e| e.to_json())});
    let path = dir.join(format!("verify-{unit}-{id}.json"));
    let receipt_bytes = serde_json::to_vec(&receipt).map_err(|e| Failure::new("io", e.to_string()))?;
    base::atomic_write(&path, &receipt_bytes).map_err(|e| Failure::new("io", e.to_string()))?;
    let mut result = run?;
    if before != after || after.is_empty() || stdout.is_none() || stderr.is_none() || result.timed_out {
        result.exit_code = -1;
    }
    result.output = format!("{}{}", stdout.as_ref().map(|v| String::from_utf8_lossy(v)).unwrap_or_default(), stderr.as_ref().map(|v| String::from_utf8_lossy(v)).unwrap_or_default());
    Ok((result, serde_json::json!({"path":path,"receipt":receipt}), format!("sha256:{:x}", Sha256::digest(&receipt_bytes))))
}

fn finish_unit_inner(
    store: &Path,
    unit: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
    release: bool,
    before_verify: Option<&mut dyn FnMut() -> Res<()>>,
) -> Res<Finished> {
    struct Release<'a>(&'a Path, &'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            abort_unit(self.0, self.1);
        }
    }
    let _release = release.then(|| Release(store, unit));
    let brief = unit_brief(store, unit)?;
    let (before, contract, dispatch_id) = {
        let active = claims().lock().unwrap_or_else(|e| e.into_inner());
        let c = active
            .get(&key(store, unit))
            .ok_or_else(|| Failure::new("not_found", "unit is not running"))?;
        (c.dirty.clone(), c.contract.clone(), c.dispatch_id.clone())
    };
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
    append_evidence(
        store,
        unit,
        &mk(
            "run",
            model,
            if status == "completed" { 0 } else { 1 },
            String::new(),
        ),
    )?;
    append_evidence(store, unit, &mk("contract", &contract, 0, String::new()))?;
    if let Some(dispatch_id) = dispatch_id {
        append_evidence(store, unit, &mk("dispatch", &dispatch_id, 0, String::new()))?;
    }
    let dir = store.join("reports");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
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
    let json = serde_json::to_vec_pretty(report)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    base::atomic_write(&dir.join(format!("{unit}.json")), &json)
        .map_err(|e| Failure::new("io", e.to_string()))?;
    append_inbox(store, role, unit, status, Some(&md.to_string_lossy()))?;
    let after = dirty_identities(repo)?;
    // Include peers that started while completion evidence/dirty contents were collected.
    let others = claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key(store, unit))
        .ok_or_else(|| Failure::new("not_found", "unit is not running"))?
        .peers
        .clone();
    let mut outside_scope: BTreeSet<String> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .filter(|p| !covers(&brief.scope, p) && !covers(&others, p))
        .cloned()
        .collect();
    if let Some(changes) = report.get("file_changes").and_then(Value::as_array) {
        for c in changes {
            if let Some(p) = c.get("path").and_then(Value::as_str) {
                let p = Path::new(p);
                let p = if p.is_absolute() {
                    p.strip_prefix(repo).unwrap_or(p)
                } else {
                    p
                };
                let p = p.to_string_lossy().replace('\\', "/");
                if !covers(&brief.scope, &p) {
                    outside_scope.insert(p);
                }
            }
        }
    }
    let outside_scope: Vec<_> = outside_scope.into_iter().collect();
    append_evidence(
        store,
        unit,
        &mk(
            "scope",
            &outside_scope.join(" "),
            if outside_scope.is_empty() { 0 } else { 1 },
            String::new(),
        ),
    )?;
    let mut verify_exit = None;
    let mut verify_receipt = None;
    let mut verify_tail = String::new();
    if status == "completed" && outside_scope.is_empty() {
        let result = if !release {
            let (result, receipt, hash) = host_verification_check(store, unit, repo, &brief.verify, None, before_verify)?;
            append_evidence(store, unit, &mk(
                "verification_receipt",
                receipt["path"].as_str().unwrap_or(""),
                result.exit_code,
                hash,
            ))?;
            verify_receipt = Some(receipt);
            result
        } else {
            base::proc::run_shell(&brief.verify, repo, Duration::from_secs(1800))?
        };
        verify_exit = Some(result.exit_code);
        verify_tail = result
            .output
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        append_evidence(
            store,
            unit,
            &mk(
                "verify",
                &brief.verify,
                result.exit_code,
                snapshot(repo, &brief.scope),
            ),
        )?;
    }
    let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
    set_unit_state(store, unit, &state)?;
    Ok(Finished {
        state,
        verify_exit,
        verify_tail,
        outside_scope,
        verify_receipt,
    })
}
