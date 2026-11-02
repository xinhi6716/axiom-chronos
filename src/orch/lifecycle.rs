use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

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
}
struct Claim {
    // Units of different projects never block or excuse each other.
    store: PathBuf,
    scope: Vec<String>,
    dirty: BTreeSet<String>,
    attempt_id: String,
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
        return Ok(BTreeSet::new());
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
    for other in units.iter().filter(|x| deps.contains(&x.track)) {
        if derive_state(
            &evidence(store, &other.id)?,
            needs_accept(store, &other.id)?,
        ) != "verified"
        {
            missing.push(other.id.clone());
        }
    }
    Ok(missing)
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
        let missing = dependencies(store, u, &units)?;
        if !missing.is_empty() {
            return Err(Failure::new("not_ready", missing.join(", ")));
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
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let state = derive_state(&rows, needs_accept(store, &u.id)?);
        if !["pending", "failed"].contains(&state.as_str())
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
    for u in &units {
        let rows = evidence(store, &u.id)?;
        let needs = needs_accept(store, &u.id)?;
        let state = derive_state(&rows, needs);
        if (state == "verified" && !disk.iter().any(|c| c.unit == u.id)) || ready.contains(&u.id) {
            continue;
        }
        let next = if let Some(c) = disk.iter().find(|c| c.unit == u.id) {
            if active
                .get(&key(store, &u.id))
                .is_some_and(|a| a.attempt_id == c.attempt)
            {
                "running".to_string()
            } else {
                format!("unknown {}", c.attempt)
            }
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
            let missing = dependencies(store, u, &units)?;
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
    let (before, others) = {
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
            String::new(),
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
    if let Some(changes) = report.get("file_changes").and_then(Value::as_array) {
        for c in changes {
            if let Some(p) = c.get("path").and_then(Value::as_str) {
                let p = Path::new(p);
                let p = if p.is_absolute() {
                    p.strip_prefix(repo).unwrap_or(p)
                } else {
                    p
                };
                paths.insert(p.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let outside_scope: Vec<_> = paths
        .into_iter()
        .filter(|p| !covers(&brief.scope, p) && !covers(&others, p))
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
            Err(e) => verify_error = Some(e.message),
        }
    }
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
    let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
    set_unit_state(store, unit, &state)?;
    Ok(Finished {
        state,
        verify_exit,
        verify_tail,
        verify_output,
        outside_scope,
    })
}
