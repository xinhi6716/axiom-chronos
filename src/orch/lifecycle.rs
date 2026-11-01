use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
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
}
struct Claim {
    // Units of different projects never block or excuse each other.
    store: PathBuf,
    scope: Vec<String>,
    files: BTreeMap<String, String>,
    peers: Vec<Vec<String>>,
}
static RUNNING: OnceLock<Mutex<HashMap<String, Claim>>> = OnceLock::new();
fn key(store: &Path, unit: &str) -> String {
    format!("{}|{unit}", store.display())
}
fn claims() -> &'static Mutex<HashMap<String, Claim>> {
    RUNNING.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(crate) fn contents(repo: &Path) -> Res<BTreeMap<String, String>> {
    use sha2::{Digest, Sha256};
    let out = Command::new("git")
        .current_dir(repo)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    if !out.status.success() {
        return Err(Failure::new("process", "cannot enumerate worktree"));
    }
    let mut files = BTreeMap::new();
    for p in out.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let name = String::from_utf8(p.to_vec())
            .map_err(|e| Failure::new("invalid_argument", e.to_string()))?
            .replace('\\', "/");
        let path = repo.join(&name);
        let value = match std::fs::read(&path) {
            Ok(bytes) => format!("{:x}", Sha256::digest(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => "deleted".into(),
            Err(e) => return Err(Failure::new("io", e.to_string())),
        };
        files.insert(name, value);
    }
    Ok(files)
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
        let rows = evidence(store, &other.id)?;
        let valid = rows.iter().rposition(|r| r.kind == "run").is_some_and(|i| {
            let run = &rows[i];
            let scope = rows[i + 1..].iter().find(|r| r.kind == "scope");
            let verify = rows[i + 1..].iter().find(|r| r.kind == "verify");
            !run.commit.is_empty()
                && run.exit_code == 0
                && scope.is_some_and(|r| r.exit_code == 0 && r.commit == run.commit)
                && verify.is_some_and(|r| r.exit_code == 0 && r.commit == run.commit)
        });
        if !valid {
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
    let units = read_units(store)?;
    let u = units
        .iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let brief = unit_brief(store, unit)?;
    let rows = evidence(store, unit)?;
    let missing = dependencies(store, u, &units)?;
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
    if astra_required {
        astra_gate(store)?;
    }
    let files = contents(repo)?;
    let peers: Vec<_> = active
        .values()
        .filter(|c| c.store == store)
        .map(|c| c.scope.clone())
        .collect();
    for peer in active.values_mut().filter(|c| c.store == store) {
        peer.peers.push(brief.scope.clone());
    }
    active.insert(
        key(store, unit),
        Claim {
            store: store.to_path_buf(),
            scope: brief.scope.clone(),
            files,
            peers,
        },
    );
    Ok(Begin::Run {
        brief,
        attempt: count as u32 + 1,
    })
}
pub fn abort_unit(store: &Path, unit: &str) {
    claims()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&key(store, unit));
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
    let mut selected: Vec<Vec<String>> = active
        .values()
        .filter(|c| c.store == store)
        .map(|c| c.scope.clone())
        .collect();
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
        if state == "verified" || ready.contains(&u.id) {
            continue;
        }
        let next = if active.contains_key(&key(store, &u.id)) {
            "running".to_string()
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
    repo: &Path,
    role: &str,
    model: &str,
    report: &Value,
) -> Res<Finished> {
    struct Release<'a>(&'a Path, &'a str);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            abort_unit(self.0, self.1);
        }
    }
    let _release = Release(store, unit);
    let brief = unit_brief(store, unit)?;
    let (before, others) = {
        let active = claims().lock().unwrap_or_else(|e| e.into_inner());
        let c = active
            .get(&key(store, unit))
            .ok_or_else(|| Failure::new("not_found", "unit is not running"))?;
        (
            c.files.clone(),
            c.peers.iter().flatten().cloned().collect::<Vec<_>>(),
        )
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
    let result_snapshot = snapshot(repo, &brief.scope);
    append_evidence(
        store,
        unit,
        &mk(
            "run",
            model,
            if status == "completed" { 0 } else { 1 },
            result_snapshot.clone(),
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
    let after = contents(repo)?;
    let mut paths: BTreeSet<String> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p) && !covers(&others, p))
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
                paths.insert(p.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let outside_scope: Vec<_> = paths
        .into_iter()
        .filter(|p| !covers(&brief.scope, p))
        .collect();
    append_evidence(
        store,
        unit,
        &mk(
            "scope",
            &outside_scope.join(" "),
            if outside_scope.is_empty() { 0 } else { 1 },
            result_snapshot.clone(),
        ),
    )?;
    let mut verify_exit = None;
    let mut verify_tail = String::new();
    if status == "completed" && outside_scope.is_empty() {
        let result = base::proc::run_shell(&brief.verify, repo, Duration::from_secs(1800))?;
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
        let current = snapshot(repo, &brief.scope);
        if result.exit_code == 0 && !current.is_empty() && current == result_snapshot {
            let root =
                std::fs::canonicalize(repo).map_err(|e| Failure::new("io", e.to_string()))?;
            let provenance = serde_json::json!({"repo":root, "unit":unit, "snapshot":current});
            base::atomic_write(
                &dir.join(format!("{unit}.repo.json")),
                &serde_json::to_vec(&provenance).map_err(|e| Failure::new("io", e.to_string()))?,
            )
            .map_err(|e| Failure::new("io", e.to_string()))?;
        }
    }
    let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
    set_unit_state(store, unit, &state)?;
    Ok(Finished {
        state,
        verify_exit,
        verify_tail,
        outside_scope,
    })
}
