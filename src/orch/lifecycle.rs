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
pub struct Finished {
    pub state: String,
    pub verify_exit: Option<i32>,
    pub verify_tail: String,
    pub verify_output: Option<PathBuf>,
    pub outside_scope: Vec<String>,
    pub pack: Option<PathBuf>,
    pub pack_error: Option<String>,
}
struct Claim {
    // Units of different projects never block or excuse each other.
    store: PathBuf,
    scope: Vec<String>,
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
fn dirty(repo: &Path) -> Res<BTreeSet<String>> {
    let out = base::proc::cmd("git")
        .current_dir(repo)
        .args(["status", "--porcelain", "-z", "--untracked-files=all"])
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
    let mut active = claims().lock().unwrap_or_else(|e| e.into_inner());
    with_lock(store, false, || {
        let units = read_units(store)?;
        let u = units
            .iter()
            .find(|u| u.id == unit)
            .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
        u.refuse_superseded()?;
        let brief = unit_brief(store, unit)?;
        let rows = evidence(store, unit)?;
        let mut disk = read_claims(store)?;
        if let Some(c) = disk
            .iter()
            .find(|c| c.unit == unit || overlaps(&brief.scope, &c.scope))
        {
            let held = active
                .get(&key(store, &c.unit))
                .is_some_and(|a| a.attempt_id == c.attempt);
            return Err(if held {
                Failure::new("busy", format!("scope busy: {unit}"))
            } else {
                Failure::new(
                    "outcome_unknown",
                    format!("claim outcome unknown: {}", c.attempt),
                )
            });
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
            return Ok(Begin::Reused);
        }
        let count = rows.iter().filter(|r| r.kind == "run").count();
        if count >= 2 {
            return Err(Failure::new(
                "fix_limit",
                format!("unit {unit} has reached its fix limit"),
            ));
        }
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
        let attempt = count as u32 + 1;
        let attempt_id = format!("{}-{}", attempt, base::unique());
        let before = dirty(repo)?;
        let base = base_snapshot(repo, &brief.scope);
        disk.push(DiskClaim {
            unit: unit.into(),
            attempt: attempt_id.clone(),
            scope: brief.scope.clone(),
            started: base::now_iso(),
        });
        save_claims(store, &disk)?;
        active.insert(
            key(store, unit),
            Claim {
                store: store.to_path_buf(),
                scope: brief.scope.clone(),
                dirty: before,
                attempt_id: attempt_id.clone(),
                base,
            },
        );
        Ok(Begin::Run {
            brief,
            attempt,
            attempt_id,
        })
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
        disk.remove(index);
        save_claims(store, &disk)?;
        if active
            .get(&key(store, unit))
            .is_some_and(|c| c.attempt_id == attempt_id)
        {
            active.remove(&key(store, unit));
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
    let claims = read_claims(store)?;
    let claimed: Vec<_> = module_units
        .iter()
        .filter(|u| claims.iter().any(|c| c.unit == u.id))
        .map(|u| u.id.clone())
        .collect();
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
pub fn finish_unit(
    store: &Path,
    unit: &str,
    attempt_id: &str,
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
) -> Res<Finished> {
    struct Release<'a>(&'a Path, &'a str, &'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            abort_unit(self.0, self.1, self.2);
        }
    }

    let brief = unit_brief(store, unit)?;
    let (before, others, base_tree) = {
        let active = claims().lock().unwrap_or_else(|e| e.into_inner());
        let c = active
            .get(&key(store, unit))
            .filter(|c| c.attempt_id == attempt_id)
            .ok_or_else(|| Failure::new("claim_mismatch", "unit attempt is not running"))?;
        if !read_claims(store)?
            .iter()
            .any(|d| d.unit == unit && d.attempt == attempt_id)
        {
            return Err(Failure::new("claim_mismatch", "disk claim does not match"));
        }
        (
            c.dirty.clone(),
            active
                .iter()
                .filter(|(id, c)| c.store == store && **id != key(store, unit))
                .flat_map(|(_, c)| c.scope.clone())
                .collect::<Vec<_>>(),
            c.base.clone(),
        )
    };
    let _release = Release(store, unit, attempt_id);
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
            base_tree,
        ),
    )?;
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
    let mut paths: BTreeSet<String> = dirty(repo)?.difference(&before).cloned().collect();
    paths.extend(report_paths(report, repo));
    let outside_scope: Vec<_> = paths
        .into_iter()
        .filter(|p| !p.starts_with(TEMP_DIR) && !covers(&brief.scope, p) && !covers(&others, p))
        .collect();
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
    let mut verify_output = None;
    let mut verify_exit = None;
    let mut verify_tail = String::new();
    let mut verify_error = None;
    let mut pack = None;
    let mut pack_error = None;
    if status == "completed" && outside_scope.is_empty() {
        match base::proc::run_shell(&brief.verify, repo, Duration::from_secs(1800)) {
            Ok(result) => {
                verify_exit = Some(result.exit_code);
                let path = dir.join(format!("{unit}.verify.txt"));
                let output: String = result
                    .output
                    .chars()
                    .rev()
                    .take(20000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                base::atomic_write(&path, output.as_bytes())
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
    Ok(Finished {
        state,
        verify_exit,
        verify_tail,
        verify_output,
        outside_scope,
        pack,
        pack_error,
    })
}
