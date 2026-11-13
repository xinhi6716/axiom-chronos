use crate::base::{self, Failure, Res};
use crate::orch::{parse_brief, Brief};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

const HEADER: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief";
const COMMANDS: &str = "init, map, status, unit add|set|get|list|counts, ready, verify <unit> --repo <dir>, accept <module> --repo <dir>, review <unit> --repo <dir> --reviewer chronos:reviewer --verdict-file <path>, fulltest --repo <dir> [--command <cmd>], evidence <unit>, handoff [--module <m>] --next <text> [--in-flight|--stuck|--decisions <text>] [--repo <dir>], resume [--module <m>] [--repo <dir>], release <unit> --attempt <id>, unit supersede <unit> --by <unit>, unit add <id> --track <t> [--brief <path>] [--fixes <old>] [--routing <tier>/<effort> --reason <text>], waive <unit> <n> --reason <text>, inbox push|count|drain, ledger record|check|summary, gate park|resolve|list, standing show|add, frontier show, map [--repo <dir>], module-context <module> --repo <dir>, schedule next [--cursor N] [--timeout S] | claim <module> | release <module> --reason <text>, activity start|finish --module <m> --actor core|dispatcher --phase brief|review|decide [--id <id>], setup start --repo <dir> | status | keep <id>... | cancel [<id>...]";
const SUPERSEDED: &str = "superseded";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unit {
    pub id: String,
    pub track: String,
    pub state: String,
    pub branch: String,
    pub pr: String,
    pub sha: String,
    pub brief: String,
}
impl Unit {
    pub fn superseded_by(&self) -> Option<&str> {
        self.state.strip_prefix(SUPERSEDED)?.strip_prefix(':')
    }
    fn state_name(&self) -> &str {
        if self.superseded_by().is_some() {
            SUPERSEDED
        } else {
            &self.state
        }
    }
    pub(crate) fn refuse_superseded(&self) -> Res<()> {
        match self.superseded_by() {
            Some(by) => Err(Failure::new(
                "superseded",
                format!(
                    "unit {} was superseded by {by}; work on {by} instead",
                    self.id
                ),
            )),
            None => Ok(()),
        }
    }
}
/// The unit whose result counts for `unit`: itself, or the end of its replacement chain. `None` when that chain is broken.
pub fn standing_unit<'a>(units: &'a [Unit], unit: &'a Unit) -> Option<&'a Unit> {
    let mut current = unit;
    for _ in 0..=units.len() {
        let Some(by) = current.superseded_by() else {
            return Some(current);
        };
        current = units.iter().find(|u| u.id == by)?;
    }
    None
}
#[derive(Debug, Clone)]
pub struct Output {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub fn store_for(home: &Path, project: &str) -> Res<PathBuf> {
    base::require_slug("project", project)?;
    wish_store(home, project, current_wish(home, project)?.as_deref())
}
pub fn is_wish_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 22
        && &bytes[..2] == b"w-"
        && bytes[2..15].iter().all(u8::is_ascii_digit)
        && bytes[15] == b'-'
        && bytes[16..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}
pub fn wish_store(home: &Path, project: &str, wish_id: Option<&str>) -> Res<PathBuf> {
    base::require_slug("project", project)?;
    let store = home.join("orchestrate").join(project);
    match wish_id {
        Some(id) if is_wish_id(id) => Ok(store.join(id)),
        Some(id) => Err(fail(format!("invalid wish id: {id:?}"))),
        None => Ok(store),
    }
}
pub fn wish_stores(project_store: &Path) -> Vec<PathBuf> {
    let mut stores: Vec<_> = std::fs::read_dir(project_store)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry.file_name().to_str().is_some_and(is_wish_id)
        })
        .map(|entry| entry.path())
        .collect();
    stores.sort();
    stores
}
pub fn current_wish(home: &Path, project: &str) -> Res<Option<String>> {
    base::require_slug("project", project)?;
    let entries = match std::fs::read_dir(home.join("goals")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    let mut active: Option<String> = None;
    let mut any: Option<String> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(goal) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<crate::goal::Goal>(&b).ok())
        else {
            continue;
        };
        if goal.project.as_deref() != Some(project) {
            continue;
        }
        if let Some(id) = goal.wish_id {
            if goal.status == "active" && active.as_ref().map_or(true, |old| id > *old) {
                active = Some(id.clone());
            }
            if any.as_ref().map_or(true, |old| id > *old) {
                any = Some(id);
            }
        }
    }
    Ok(active.or(any))
}
pub fn store_for_goal(home: &Path, goal: &crate::goal::Goal) -> Res<PathBuf> {
    let project = goal
        .project
        .as_deref()
        .ok_or_else(|| fail("goal has no project"))?;
    wish_store(home, project, goal.wish_id.as_deref())
}
pub(crate) fn storage_context(store: &Path) -> (std::path::PathBuf, crate::storage::Owner) {
    let name = store
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let wish = is_wish_id(&name).then_some(name);
    let project_dir = if wish.is_some() {
        store.parent().unwrap_or(store)
    } else {
        store
    };
    let project = project_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let home = project_dir
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "orchestrate"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::base::axiom_home);
    (home, crate::storage::Owner { project, wish })
}

pub fn wish_of_store(store: &Path) -> String {
    store
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn fail(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn clean(s: &str) -> String {
    let s = s.replace(['\t', '\r', '\n'], " ");
    if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s
    }
}
fn required(s: &str, name: &str) -> Res<String> {
    let s = clean(s);
    if s.trim().is_empty() {
        Err(fail(format!("{name} must not be empty")))
    } else {
        Ok(s)
    }
}
fn read_file(store: &Path, name: &str) -> Res<String> {
    std::fs::read_to_string(store.join(name)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            fail(format!(
                "store is not initialized at {}; run orch init",
                store.display()
            ))
        } else {
            io(e)
        }
    })
}
fn write(store: &Path, name: &str, text: &str) -> Res<()> {
    base::atomic_write(&store.join(name), text.as_bytes()).map_err(io)
}

fn adjudication_entries(store: &Path) -> Res<Vec<String>> {
    match std::fs::read_to_string(store.join("adjudication.pending")) {
        Ok(text) => Ok(text.lines().filter(|s| !s.trim().is_empty()).map(str::to_owned).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(io(e)),
    }
}

// Caller holds the store lock, including the read/modify/write cycle.
pub(crate) fn add_adjudication_locked(store: &Path, reason: &str, target: &str) -> Res<()> {
    let mut entries = adjudication_entries(store)?;
    let entry = format!("{reason} {target}");
    if !entries.contains(&entry) {
        entries.push(entry);
        write(store, "adjudication.pending", &(entries.join("\n") + "\n"))?;
    }
    Ok(())
}

pub(crate) fn clear_adjudication(store: &Path, target: &str, module: bool) -> Res<()> {
    with_lock(store, false, || {
        let mut entries = adjudication_entries(store)?;
        if entries.is_empty() { return Ok(()); }
        let units = read_units(store)?;
        let track = units.iter().find(|u| u.id == target).map(|u| u.track.as_str());
        let mut module_resolved = !module && track.is_some();
        if module_resolved {
            for unit in units.iter().filter(|u| Some(u.track.as_str()) == track && u.id != target && !u.state.starts_with(SUPERSEDED)) {
                if entries.contains(&format!("fix_rounds {}", unit.id)) {
                    module_resolved = false;
                    break;
                }
                if let Some(verdict) = super::read_verdict(store, &unit.id)? {
                    let waived = super::waivers(&super::evidence(store, &unit.id)?)
                        .into_iter().map(|w| w.number).collect();
                    if !super::passes(&verdict, &waived) {
                        module_resolved = false;
                        break;
                    }
                }
            }
        }
        let before = entries.len();
        entries.retain(|entry| match entry.split_once(' ') {
            Some(("fix_rounds", unit)) => {
                if module { !units.iter().any(|u| u.id == unit && u.track == target) }
                else { unit != target }
            }
            Some(("stuck", id)) => if module { id != target } else { !(module_resolved && Some(id) == track) },
            _ => true,
        });
        if entries.len() != before {
            let text = if entries.is_empty() { String::new() } else { entries.join("\n") + "\n" };
            write(store, "adjudication.pending", &text)?;
        }
        Ok(())
    })
}

pub fn read_units(store: &Path) -> Res<Vec<Unit>> {
    let text = read_file(store, "units.tsv")?.replace('\r', "");
    let mut lines = text.split('\n');
    if lines.next() != Some(HEADER) {
        return Err(fail("units.tsv has an invalid header"));
    }
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let c: Vec<_> = l.split('\t').collect();
            if c.len() != 7 {
                return Err(fail("units.tsv has a malformed row"));
            }
            Ok(Unit {
                id: c[0].into(),
                track: c[1].into(),
                state: c[2].into(),
                branch: c[3].into(),
                pr: c[4].into(),
                sha: c[5].into(),
                brief: c[6].into(),
            })
        })
        .collect()
}
fn unit_line(u: &Unit) -> String {
    [
        &u.id, &u.track, &u.state, &u.branch, &u.pr, &u.sha, &u.brief,
    ]
    .iter()
    .map(|s| clean(s))
    .collect::<Vec<_>>()
    .join("\t")
}
fn save_units(store: &Path, rows: &[Unit]) -> Res<()> {
    let mut s = format!("{HEADER}\n");
    for row in rows {
        s.push_str(&unit_line(row));
        s.push('\n');
    }
    write(store, "units.tsv", &s)
}
pub fn unit_brief(store: &Path, unit: &str) -> Res<Brief> {
    let row = read_units(store)?
        .into_iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    brief_for_row(&row)
}
fn brief_for_row(row: &Unit) -> Res<Brief> {
    if row.brief.is_empty() {
        return Err(Failure::new(
            "no_brief",
            format!("unit {} has no brief", row.id),
        ));
    }
    let text = std::fs::read_to_string(&row.brief).map_err(io)?;
    parse_brief(&text).map_err(|why| fail(format!("brief {} {why}", row.brief)))
}
fn validate_add_brief(store: &Path, row: &Unit) -> Res<()> {
    if !row.brief.is_empty() {
        let text = std::fs::read_to_string(&row.brief).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                fail(format!(
                    "brief {} does not exist; write the brief file, then add the unit",
                    row.brief
                ))
            } else {
                io(e)
            }
        })?;
        let b = parse_brief(&text).map_err(|why| fail(format!("brief {} {why}", row.brief)))?;
        let path = store.join("modules.md");
        if path.exists() {
            let map = crate::orch::parse_map(&std::fs::read_to_string(path).map_err(io)?)
                .map_err(fail)?;
            if let Some(module) = map.modules.iter().find(|m| m.id == row.track) {
                for path in &b.scope {
                    if !crate::orch::covers(&module.paths, path) {
                        return Err(fail(format!(
                            "{path} is outside module {}; its PATHS are {}",
                            module.id,
                            module.paths.join(", ")
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn fixed_unit(store: &Path, unit: &str) -> Res<Option<String>> {
    Ok(crate::orch::evidence(store, unit)?
        .into_iter()
        .find(|row| row.kind == crate::orch::FIXES_KIND)
        .map(|row| row.command))
}
static LOCKS: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
const LOCK_WAIT: Duration = Duration::from_secs(30);
struct LockFile {
    path: PathBuf,
    pid: String,
}
impl Drop for LockFile {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).is_ok_and(|s| s.trim() == self.pid) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
/// Holds the lock of a store until dropped; the file goes before the next thread may enter.
pub struct StoreLock {
    _file: LockFile,
    _threads: MutexGuard<'static, ()>,
}
/// True when `holder` is a pid and no process runs under it.
fn dead(holder: &str) -> bool {
    holder
        .parse::<u32>()
        .ok()
        .filter(|p| *p > 0 && p.to_string() == holder)
        .is_some_and(|p| p > i32::MAX as u32 || !base::proc::pid_alive(p))
}
/// A claimant writes its pid right after creating its marker; an empty marker older than this was abandoned.
const EMPTY_MARKER_GRACE: Duration = Duration::from_secs(5);
/// Dead reclaimers in a row after which a waiter gives up rather than claim yet another level.
const MAX_RECLAIM_LEVELS: usize = 8;
/// The claimant of a reclaim marker that will never finish: it died, left the marker empty, or is
/// this process, whose threads take the store's lock one at a time.
fn abandoned(marker: &Path) -> Option<String> {
    let claimant = std::fs::read_to_string(marker).ok()?;
    let claimant = claimant.trim();
    if claimant.is_empty() {
        let age = std::fs::metadata(marker)
            .ok()?
            .modified()
            .ok()?
            .elapsed()
            .ok()?;
        return (age > EMPTY_MARKER_GRACE).then(|| "unknown".to_string());
    }
    (dead(claimant) || claimant == std::process::id().to_string()).then(|| claimant.to_string())
}
/// Only the process that creates the marker may delete a dead holder's lock, so two that
/// saw the same dead holder cannot delete the fresh lock one of them has just taken. A marker
/// whose claimant died is passed over by claiming one named after that claimant, which is just
/// as exclusive, so a crash mid-reclaim never keeps the lock from being reclaimed.
fn clear_dead_lock(path: &Path, holder: &str, force: bool) -> bool {
    let mut name = format!("lock.{holder}.stale");
    let mut markers = Vec::new();
    loop {
        let marker = path.with_extension(&name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
        {
            Ok(mut file) => {
                let _ = writeln!(file, "{}", std::process::id());
                markers.push(marker);
                break;
            }
            Err(_) if force => {
                markers.push(marker);
                break;
            }
            Err(_) => {
                let Some(claimant) =
                    abandoned(&marker).filter(|_| markers.len() < MAX_RECLAIM_LEVELS)
                else {
                    return false;
                };
                markers.push(marker);
                name = format!("{name}.{claimant}.stale");
            }
        }
    }
    let cleared = std::fs::read_to_string(path).is_ok_and(|s| s.trim() == holder)
        && std::fs::remove_file(path).is_ok();
    for marker in &markers {
        let _ = std::fs::remove_file(marker);
    }
    cleared
}
fn acquire(store: &Path, force: bool, note: &mut String, wait: Duration) -> Res<StoreLock> {
    let key = std::fs::canonicalize(store).unwrap_or_else(|_| store.to_path_buf());
    let threads: &'static Mutex<()> = *LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))));
    let threads = threads.lock().unwrap_or_else(|e| e.into_inner());
    let path = store.join(".orch.lock");
    let pid = std::process::id().to_string();
    let create = || -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        writeln!(file, "{pid}")
    };
    let deadline = Instant::now() + wait;
    let mut pause = 1;
    loop {
        match create() {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            // Windows answers access denied while the previous lock file is still being deleted.
            Err(e)
                if cfg!(windows)
                    && e.kind() == std::io::ErrorKind::PermissionDenied
                    && Instant::now() < deadline => {}
            Err(e) => return Err(io(e)),
        }
        let holder = std::fs::read_to_string(&path).unwrap_or_default();
        let holder = if holder.trim().is_empty() {
            "unknown"
        } else {
            holder.trim()
        };
        let dead = dead(holder);
        if dead && clear_dead_lock(&path, holder, force) {
            note.push_str(&format!(
                "replacing stale store lock (pid {holder} is dead)
"
            ));
            continue;
        }
        if !dead && force && std::fs::remove_file(&path).is_ok() {
            note.push_str(&format!(
                "stealing store lock held by pid {holder}
"
            ));
            continue;
        }
        if holder == pid || Instant::now() >= deadline {
            return Err(fail(format!("store lock held by pid {holder}")));
        }
        std::thread::sleep(Duration::from_millis(pause));
        pause = (pause * 2).min(50);
    }
    Ok(StoreLock {
        _file: LockFile { path, pid },
        _threads: threads,
    })
}
fn locked<T>(store: &Path, force: bool, note: &mut String, f: impl FnOnce() -> Res<T>) -> Res<T> {
    let _lock = acquire(store, force, note, LOCK_WAIT)?;
    f()
}
pub fn with_lock<T>(store: &Path, force: bool, f: impl FnOnce() -> Res<T>) -> Res<T> {
    locked(store, force, &mut String::new(), f)
}
pub fn lock_store(store: &Path) -> Res<StoreLock> {
    acquire(store, false, &mut String::new(), LOCK_WAIT)
}
#[derive(Clone, Debug, PartialEq)]
pub struct DiskClaim {
    pub unit: String,
    pub attempt: String,
    pub scope: Vec<String>,
    pub started: String,
    pub module: String,
    pub wish: String,
    pub pid: String,
    pub peers: Vec<String>,
}
pub fn claims_columns(header: &str) -> Option<usize> {
    match header.strip_suffix('\r').unwrap_or(header) {
        "unit\tattempt\tscope\tstarted" => Some(4),
        "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers" => Some(8),
        _ => None,
    }
}
pub fn parse_claim_row(line: &str, columns: usize) -> Option<DiskClaim> {
    let c: Vec<_> = line
        .strip_suffix('\r')
        .unwrap_or(line)
        .split('\t')
        .collect();
    if !matches!(columns, 4 | 8) || c.len() != columns || c[0].is_empty() || c[1].is_empty() {
        return None;
    }
    Some(DiskClaim {
        unit: c[0].into(),
        attempt: c[1].into(),
        scope: c[2].split_whitespace().map(str::to_owned).collect(),
        started: c[3].into(),
        module: c.get(4).unwrap_or(&"").to_string(),
        wish: c.get(5).unwrap_or(&"").to_string(),
        pid: c.get(6).unwrap_or(&"").to_string(),
        peers: c
            .get(7)
            .unwrap_or(&"")
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    })
}
pub(crate) fn read_claims(store: &Path) -> Res<Vec<DiskClaim>> {
    let text = match std::fs::read_to_string(store.join("claims.tsv")) {
        Ok(s) => s.replace('\r', ""),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(io(e)),
    };
    let mut lines = text.lines();
    let columns = lines
        .next()
        .and_then(claims_columns)
        .ok_or_else(|| fail("claims.tsv has an invalid header"))?;
    lines
        .filter(|l| !l.is_empty())
        .map(|line| {
            parse_claim_row(line, columns).ok_or_else(|| fail("claims.tsv has a malformed row"))
        })
        .collect()
}
pub(crate) fn save_claims(store: &Path, rows: &[DiskClaim]) -> Res<()> {
    let mut text = String::from("unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\n");
    for c in rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.unit.replace(['\t', '\r', '\n'], " "),
            c.attempt.replace(['\t', '\r', '\n'], " "),
            c.scope.join(" ").replace(['\t', '\r', '\n'], " "),
            c.started.replace(['\t', '\r', '\n'], " "),
            c.module.replace(['\t', '\r', '\n'], " "),
            c.wish.replace(['\t', '\r', '\n'], " "),
            c.pid.replace(['\t', '\r', '\n'], " "),
            c.peers.join(" ").replace(['\t', '\r', '\n'], " ")
        ));
    }
    write(store, "claims.tsv", &text)
}
pub fn set_unit_state(store: &Path, unit: &str, state: &str) -> Res<()> {
    with_lock(store, false, || save_unit_state(store, unit, state))
}
/// The caller holds the store lock.
pub(crate) fn save_unit_state(store: &Path, unit: &str, state: &str) -> Res<()> {
    let mut rows = read_units(store)?;
    let row = rows
        .iter_mut()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    if row.superseded_by().is_some() {
        return Ok(());
    }
    row.state = required(state, "state")?;
    save_units(store, &rows)
}
fn supersede(store: &Path, id: &str, by: &str) -> Res<Unit> {
    let mut rows = read_units(store)?;
    let (out, cross_track) = supersede_rows(store, &mut rows, id, by)?;
    save_units(store, &rows)?;
    if let Some((old_track, new_track, basis)) = cross_track {
        crate::orch::events::append_event(
            store,
            "unit_superseded_cross_track",
            None,
            None,
            serde_json::json!({"old": id, "new": by, "old_track": old_track, "new_track": new_track, "basis": basis}),
        )?;
    }
    Ok(out)
}
fn supersede_rows(
    store: &Path,
    rows: &mut [Unit],
    id: &str,
    by: &str,
) -> Res<(Unit, Option<(String, String, &'static str)>)> {
    let find = |name: &str| {
        rows.iter()
            .find(|u| u.id == name)
            .ok_or_else(|| Failure::new("not_found", format!("unit {name} not found")))
    };
    let (old, new) = (find(id)?, find(by)?);
    if id == by {
        return Err(fail(format!("unit {id} cannot supersede itself")));
    }
    if let Some(current) = old.superseded_by() {
        return Err(fail(format!(
            "unit {id} is already superseded by {current}"
        )));
    }
    if new.superseded_by().is_some() {
        return Err(fail(match standing_unit(rows, new) {
            Some(end) if end.id == id => format!("superseding {id} by {by} would form a cycle"),
            Some(end) => format!("unit {by} is already superseded; name {} instead", end.id),
            None => format!("unit {by} is already superseded and its replacement chain is broken"),
        }));
    }
    if read_claims(store)?.iter().any(|c| c.unit == id) {
        return Err(Failure::new(
            "busy",
            format!("unit {id} has a running claim; wait for its result or release it, then supersede it"),
        ));
    }
    let derived = crate::orch::derive_state(
        &crate::orch::evidence(store, id)?,
        crate::orch::needs_accept(store, id)?,
    );
    if old.state == "verified" || derived == "verified" {
        return Err(fail(format!(
            "unit {id} is verified and cannot be superseded; if its files changed, run orch accept <module> --repo <dir> first"
        )));
    }
    let cross_track = if old.track != new.track {
        let basis = if new.state == "verified" {
            Some("state")
        } else if read_units(store)?.iter().any(|u| u.id == by)
            && crate::orch::derive_state(
                &crate::orch::evidence(store, by)?,
                crate::orch::needs_accept(store, by)?,
            ) == "verified"
        {
            Some("evidence")
        } else {
            None
        };
        let Some(basis) = basis else {
            return Err(fail(format!(
                "unit {by} belongs to {}, not {}; a replacement stays in its unit's track",
                new.track, old.track
            )));
        };
        Some((old.track.clone(), new.track.clone(), basis))
    } else {
        None
    };
    // The replacement inherits the completion of `id` and of every unit that already stands on it.
    let mut scope: Vec<String> = vec![];
    for u in rows
        .iter()
        .filter(|u| standing_unit(rows, u).is_some_and(|end| end.id == id))
    {
        match brief_for_row(u) {
            Ok(brief) => scope.extend(brief.scope),
            Err(e) if e.kind == "no_brief" => {}
            Err(e) => return Err(e),
        }
    }
    let replacement = brief_for_row(new)?.scope;
    let mut missing: Vec<&str> = scope
        .iter()
        .filter(|path| !crate::orch::covers(&replacement, path))
        .map(String::as_str)
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if !missing.is_empty() {
        return Err(fail(format!(
            "unit {by} does not cover {} of the SCOPE it would take over from {id}; a replacement's SCOPE must cover every file of the units it replaces",
            missing.join(", ")
        )));
    }
    let row = rows.iter_mut().find(|u| u.id == id).unwrap();
    row.state = format!("{SUPERSEDED}:{by}");
    Ok((row.clone(), cross_track))
}
fn push(
    store: &Path,
    agent: &str,
    unit: &str,
    status: &str,
    report: Option<&str>,
) -> Res<(serde_json::Value, String)> {
    let ts = base::now_iso();
    let a = required(agent, "agent")?;
    let u = required(unit, "unit")?;
    let s = required(status, "status")?;
    let r = report
        .map(|x| required(x, "report"))
        .transpose()?
        .unwrap_or_default();
    let filename = format!(
        "{}-{}-{}.tsv",
        ts.replace([':', '.'], "-"),
        std::process::id(),
        base::unique()
    );
    write(
        store,
        &format!("inbox/{filename}"),
        &format!(
            "{}\t{}\t{}\t{}\t{}\n",
            clean(&ts),
            clean(&a),
            clean(&u),
            clean(&s),
            clean(&r)
        ),
    )?;
    Ok((
        serde_json::json!({"ts":ts,"agent":a,"unit":u,"status":s,"report":r}),
        filename,
    ))
}
pub fn append_inbox(
    store: &Path,
    agent: &str,
    unit: &str,
    status: &str,
    report: Option<&str>,
) -> Res<()> {
    with_lock(store, false, || {
        push(store, agent, unit, status, report)?;
        Ok(())
    })
}
fn option<'a>(args: &'a [String], name: &str) -> Res<Option<&'a str>> {
    match args.iter().position(|x| x == name) {
        Some(i) => Ok(Some(
            args.get(i + 1)
                .ok_or_else(|| fail(format!("{name} requires a value")))?,
        )),
        None => Ok(None),
    }
}
fn arg<'a>(args: &'a [String], i: usize) -> Res<&'a str> {
    args.get(i)
        .map(String::as_str)
        .ok_or_else(|| fail("a valid command is required"))
}
fn emit(value: serde_json::Value, compact: String, json: bool) -> String {
    let mut s = if json {
        serde_json::to_string_pretty(&value).unwrap_or_default()
    } else {
        compact
    };
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

const LEDGER_HEADER: &str = "pr\tsha\tverdict\tevidence\tverifier\tts";
const VERDICTS: [&str; 5] = [
    "live-ui-verified",
    "unit-test-verified",
    "type-check-only",
    "verifier-blocked",
    "verifier-failed",
];
fn line(s: &str, label: &str) -> Res<String> {
    let v = s.replace(['\r', '\n'], " ").trim().to_string();
    if v.is_empty() {
        Err(fail(format!("{label} must not be empty")))
    } else {
        Ok(v)
    }
}
fn required_option(args: &[String], name: &str) -> Res<String> {
    Ok(option(args, name)?
        .ok_or_else(|| fail(format!("{name} is required")))?
        .to_string())
}
fn pr(s: &str) -> Res<String> {
    if s.starts_with('0')
        || !s.bytes().all(|c| c.is_ascii_digit())
        || s.parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= 9007199254740991)
            .is_none()
    {
        return Err(fail("PR must be a positive integer"));
    }
    Ok(s.to_string())
}
fn ledger(store: &Path) -> Res<Vec<serde_json::Value>> {
    let text = read_file(store, "ledger.tsv")?.replace('\r', "");
    let mut lines = text.split('\n');
    if lines.next() != Some(LEDGER_HEADER) {
        return Err(fail("ledger.tsv has an invalid header"));
    }
    lines.filter(|x| !x.is_empty()).map(|x| {
        let c: Vec<_> = x.split('\t').collect();
        if c.len()!=6 { return Err(fail("ledger.tsv has a malformed row")); }
        if !VERDICTS.contains(&c[2]) { return Err(fail(format!("ledger.tsv has invalid verdict {}",c[2]))); }
        Ok(serde_json::json!({"pr":c[0],"sha":c[1],"verdict":c[2],"evidence":c[3],"verifier":c[4],"ts":c[5]}))
    }).collect()
}
fn cell(v: &serde_json::Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}
fn save_ledger(store: &Path, rows: &[serde_json::Value]) -> Res<()> {
    let mut out = format!("{LEDGER_HEADER}\n");
    for r in rows {
        out.push_str(
            &["pr", "sha", "verdict", "evidence", "verifier", "ts"]
                .iter()
                .map(|k| clean(&cell(r, k)))
                .collect::<Vec<_>>()
                .join("\t"),
        );
        out.push('\n');
    }
    write(store, "ledger.tsv", &out)
}
fn counts(vals: impl Iterator<Item = String>) -> serde_json::Value {
    let mut m = std::collections::BTreeMap::<String, usize>::new();
    for x in vals {
        *m.entry(x).or_default() += 1;
    }
    serde_json::json!(m)
}
fn count_line(v: &serde_json::Value) -> String {
    let xs: Vec<_> = v
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    if xs.is_empty() {
        "none".into()
    } else {
        xs.join(", ")
    }
}
/// `None` when the pointer has gone. On Windows a file being deleted refuses readers for a moment.
fn read_pointer(path: &Path) -> Res<Option<String>> {
    for _ in 0..40 {
        match std::fs::read_to_string(path) {
            Ok(text) => return Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(e) => return Err(io(e)),
        }
    }
    std::fs::read_to_string(path).map(Some).map_err(io)
}
/// Another process may hold the pointer open for a moment, so a failed delete is retried.
fn remove_pointer(path: &Path) -> Res<()> {
    for _ in 0..250 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    std::fs::remove_file(path).map_err(io)
}
fn pointers(store: &Path) -> Res<Vec<serde_json::Value>> {
    Ok(pointer_files(store)?
        .into_iter()
        .map(|(_, row)| row)
        .collect())
}
fn pointer_files(store: &Path) -> Res<Vec<(PathBuf, serde_json::Value)>> {
    let dir = store.join("inbox");
    let mut files = std::fs::read_dir(&dir)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                fail(format!(
                    "store is not initialized at {}; run orch init",
                    store.display()
                ))
            } else {
                io(e)
            }
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io)?;
    files.sort_by_key(|e| e.file_name());
    let mut rows = Vec::new();
    for f in files {
        if !f.file_type().map_err(io)?.is_file()
            || !f.file_name().to_string_lossy().ends_with(".tsv")
        {
            continue;
        }
        let Some(mut raw) = read_pointer(&f.path())? else {
            continue;
        };
        if raw.ends_with('\n') {
            raw.pop();
            if raw.ends_with('\r') {
                raw.pop();
            }
        }
        let c: Vec<_> = raw.split('\t').collect();
        if c.len() != 5 || raw.contains(['\r', '\n']) {
            return Err(fail(format!(
                "inbox pointer {} is malformed",
                f.file_name().to_string_lossy()
            )));
        }
        rows.push((
            f.path(),
            serde_json::json!({"ts":c[0],"agent":c[1],"unit":c[2],"status":c[3],"report":c[4]}),
        ));
    }
    Ok(rows)
}
fn gates(store: &Path) -> Res<Vec<serde_json::Value>> {
    let raw = read_file(store, "gates.md")?.replace('\r', "");
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(vec![]);
    }
    let rest = raw
        .strip_prefix("# Gates\n\n## ")
        .ok_or_else(|| fail("gates.md has an invalid heading"))?;
    let mut rows = Vec::new();
    for block in rest.split("\n\n## ") {
        let mut lines = block.lines().filter(|x| !x.is_empty());
        let id = lines.next().unwrap_or("");
        let mut f = std::collections::HashMap::new();
        for l in lines {
            let (k, v) = l
                .strip_prefix("- ")
                .and_then(|x| x.split_once(": "))
                .ok_or_else(|| fail(format!("gates.md has a malformed gate {id}")))?;
            f.insert(k, v);
        }
        let (Some(q), Some(o), Some(d)) = (f.get("Question"), f.get("Options"), f.get("Default"))
        else {
            return Err(fail(format!("gates.md has a malformed gate {id}")));
        };
        if id.is_empty() {
            return Err(fail(format!("gates.md has a malformed gate {id}")));
        }
        let status = *f.get("Status").unwrap_or(&"");
        let mut row =
            serde_json::json!({"kind":status,"id":id,"question":q,"options":o,"defaultAnswer":d});
        if status == "resolved" && f.contains_key("Answer") {
            row["answer"] = serde_json::json!(f["Answer"])
        } else if status != "open" {
            return Err(fail(format!("gates.md has invalid status {status}")));
        }
        if rows.iter().any(|v| cell(v, "id") == id) {
            return Err(fail("gates.md has duplicate gate ids"));
        }
        rows.push(row);
    }
    Ok(rows)
}
fn save_gates(store: &Path, rows: &[serde_json::Value]) -> Res<()> {
    if rows.is_empty() {
        return write(store, "gates.md", "");
    }
    let blocks: Vec<_> = rows
        .iter()
        .map(|g| {
            let mut x = format!(
                "## {}\n\n- Status: {}\n- Question: {}\n- Options: {}\n- Default: {}",
                cell(g, "id"),
                cell(g, "kind"),
                cell(g, "question"),
                cell(g, "options"),
                cell(g, "defaultAnswer")
            );
            if cell(g, "kind") == "resolved" {
                x.push_str(&format!("\n- Answer: {}", cell(g, "answer")))
            }
            x
        })
        .collect();
    write(
        store,
        "gates.md",
        &format!("# Gates\n\n{}\n", blocks.join("\n\n")),
    )
}
fn standing(store: &Path) -> Res<Vec<serde_json::Value>> {
    let raw = read_file(store, "preferences.md")?.replace('\r', "");
    let mut rows = Vec::new();
    for l in raw.lines().filter(|x| !x.is_empty()) {
        let (n, v) = l
            .split_once(". ")
            .ok_or_else(|| fail("preferences.md has malformed numbering"))?;
        if n != format!("{}", rows.len() + 1) || v.is_empty() {
            return Err(fail("preferences.md has malformed numbering"));
        }
        rows.push(serde_json::json!({"number":rows.len()+1,"line":v}));
    }
    Ok(rows)
}
fn frontier(store: &Path) -> Res<serde_json::Value> {
    let raw = read_file(store, "frontier.json")?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| fail("frontier.json is not valid JSON"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| fail("frontier.json must contain an object"))?;
    if obj.is_empty() {
        return Ok(serde_json::json!({"generation":0,"prs":[],"lowestUnmerged":null}));
    }
    if v["generation"]
        .as_u64()
        .filter(|x| *x <= 9007199254740991)
        .is_none()
        || !v["prs"].is_array()
        || !(v["lowestUnmerged"].is_null() || v["lowestUnmerged"].as_i64().is_some())
    {
        return Err(fail("frontier.json has an invalid shape"));
    }
    for r in v["prs"].as_array().unwrap() {
        if r["pr"]
            .as_u64()
            .filter(|x| *x > 0 && *x <= 9007199254740991)
            .is_none()
            || r["branches"].as_str().is_none_or(|s| s.is_empty())
            || !r["sha"].is_string()
            || !["OPEN", "MERGED", "CLOSED"].contains(&r["state"].as_str().unwrap_or(""))
        {
            return Err(fail("frontier.json has an invalid PR row"));
        }
    }
    Ok(
        serde_json::json!({"generation":v["generation"],"prs":v["prs"].as_array().unwrap().iter().map(|r|serde_json::json!({"pr":r["pr"],"branches":r["branches"],"sha":r["sha"],"state":r["state"]})).collect::<Vec<_>>(),"lowestUnmerged":v["lowestUnmerged"]}),
    )
}
fn rows_display(
    rows: &[serde_json::Value],
    f: impl Fn(&serde_json::Value) -> String,
    empty: &str,
    limit: Option<usize>,
) -> String {
    if rows.is_empty() {
        return empty.into();
    }
    let n = limit.unwrap_or(rows.len());
    let mut out: Vec<_> = rows.iter().take(n).map(f).collect();
    if rows.len() > n {
        out.push(format!("... {} more; use --json", rows.len() - n))
    }
    out.join("\n")
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    if rows.is_empty() {
        return "(none)".into();
    }
    let mut lines = vec![
        format!("| {} |", headers.join(" | ")),
        format!(
            "| {} |",
            headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ];
    for r in rows {
        lines.push(format!(
            "| {} |",
            r.iter()
                .map(|s| s.replace('\\', "\\\\").replace('|', "\\|"))
                .collect::<Vec<_>>()
                .join(" | ")
        ))
    }
    lines.join("\n")
}
fn status(store: &Path) -> Res<(serde_json::Value, String)> {
    let units = read_units(store)?;
    let ledger = ledger(store)?;
    let frontier = frontier(store)?;
    let gates = gates(store)?;
    let states = counts(units.iter().map(|r| r.state_name().to_string()));
    let verdicts = counts(ledger.iter().map(|r| cell(r, "verdict")));
    let mut ids: Vec<_> = gates
        .iter()
        .filter(|r| cell(r, "kind") == "open")
        .map(|r| cell(r, "id"))
        .collect();
    ids.sort();
    let summary = serde_json::json!({"unitStates":states,"ledgerVerdicts":verdicts,"frontierGeneration":frontier["generation"],"openGateIds":ids});
    let previous = std::fs::read_to_string(store.join("status.md"))
        .ok()
        .and_then(|x| {
            x.split("<!-- orch-summary ")
                .nth(1)
                .and_then(|x| x.split(" -->").next())
                .and_then(|x| serde_json::from_str::<serde_json::Value>(x).ok())
        })
        .filter(|v| {
            v["unitStates"]
                .as_object()
                .is_some_and(|m| m.values().all(|v| v.as_u64().is_some()))
                && v["ledgerVerdicts"]
                    .as_object()
                    .is_some_and(|m| m.values().all(|v| v.as_u64().is_some()))
                && v["frontierGeneration"].is_number()
                && v["openGateIds"]
                    .as_array()
                    .is_some_and(|a| a.iter().all(|v| v.is_string()))
        });
    let changed = if let Some(old) = previous {
        let mut changes = Vec::new();
        for (key, label) in [("unitStates", "units"), ("ledgerVerdicts", "ledger")] {
            let a = old[key].as_object().unwrap();
            let b = summary[key].as_object().unwrap();
            let names: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for n in names {
                let x = a.get(n).and_then(|v| v.as_u64()).unwrap_or(0);
                let y = b.get(n).and_then(|v| v.as_u64()).unwrap_or(0);
                if x != y {
                    changes.push(format!("{label} {n} {x}->{y}"))
                }
            }
        }
        if old["frontierGeneration"] != summary["frontierGeneration"] {
            changes.push(format!(
                "frontier generation {}->{}",
                old["frontierGeneration"], summary["frontierGeneration"]
            ))
        }
        if old["openGateIds"] != summary["openGateIds"] {
            changes.push(format!(
                "open gates {}->{}",
                old["openGateIds"].as_array().unwrap().len(),
                ids.len()
            ))
        }
        if changes.is_empty() {
            "no derived changes".into()
        } else {
            changes.join("; ")
        }
    } else {
        "first render".into()
    };
    let unit_table = table(
        &["ID", "Track", "State", "Branch", "PR", "SHA", "Brief"],
        units
            .iter()
            .map(|u| {
                vec![
                    u.id.clone(),
                    u.track.clone(),
                    u.state.clone(),
                    u.branch.clone(),
                    u.pr.clone(),
                    u.sha.clone(),
                    u.brief.clone(),
                ]
            })
            .collect(),
    );
    let ledger_table = table(
        &["PR", "SHA", "Verdict", "Evidence", "Verifier", "Timestamp"],
        ledger
            .iter()
            .map(|r| {
                ["pr", "sha", "verdict", "evidence", "verifier", "ts"]
                    .iter()
                    .map(|k| cell(r, k))
                    .collect()
            })
            .collect(),
    );
    let frontier_table = table(
        &["Branch", "PR", "SHA", "State"],
        frontier["prs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                vec![
                    cell(r, "branches"),
                    r["pr"].to_string(),
                    cell(r, "sha"),
                    cell(r, "state"),
                ]
            })
            .collect(),
    );
    let gates_table = table(
        &["ID", "Status", "Question", "Options", "Default", "Answer"],
        gates
            .iter()
            .map(|r| {
                [
                    "id",
                    "kind",
                    "question",
                    "options",
                    "defaultAnswer",
                    "answer",
                ]
                .iter()
                .map(|k| cell(r, k))
                .collect()
            })
            .collect(),
    );
    let lowest = if frontier["lowestUnmerged"].is_null() {
        "none".into()
    } else {
        frontier["lowestUnmerged"].to_string()
    };
    let markdown=format!("# Orchestrate status\n\nGenerated: {}\n\n## Units\n\nStates: {}\n\n{}\n\n## Verification ledger\n\nVerdicts: {}\n\n{}\n\n## Frontier\n\nGeneration: {}\nLowest unmerged: {}\n\n{}\n\n## Gates\n\n{}\n\n<!-- orch-summary {} -->\n",base::now_iso(),count_line(&states),unit_table,count_line(&verdicts),ledger_table,frontier["generation"],lowest,frontier_table,gates_table,summary);
    write(store, "status.md", &markdown)?;
    let suffix = if ids.is_empty() {
        String::new()
    } else {
        format!(
            "; ids={}{}",
            ids.iter().take(4).cloned().collect::<Vec<_>>().join(","),
            if ids.len() > 4 {
                format!(",+{} more", ids.len() - 4)
            } else {
                String::new()
            }
        )
    };
    let compact = format!(
        "counts: units={}; states={}; ledger={}\nchanged: {}\ngates open: {}{}",
        units.len(),
        count_line(&states),
        count_line(&verdicts),
        changed,
        ids.len(),
        suffix
    );
    Ok((
        serde_json::json!({"units":units,"ledger":ledger,"frontier":frontier,"gates":gates,"summary":summary,"changed":changed}),
        compact,
    ))
}
pub fn run(store: &Path, args: &[String], json: bool, force: bool) -> Output {
    let mut note = String::new();
    let mut ledger_missing = false;
    let mut extra_output = None;
    let result: Res<String> = (|| {
        let cmd = arg(args, 0)?;
        let sub = args.get(1).map(String::as_str).unwrap_or("");
        match (cmd, sub) {
            ("init", _) => {
                let (home, owner) = storage_context(store);
                crate::storage::ensure_dir(&home, store, crate::storage::Class::UnitStore, &owner)?;
                for name in ["reports", "receipts"] {
                    crate::storage::ensure_dir(
                        &home,
                        &store.join(name),
                        crate::storage::Class::Evidence,
                        &owner,
                    )?;
                }
                locked(store, force, &mut note, || {
                    std::fs::create_dir_all(store.join("inbox")).map_err(io)?;
                    for (name, content) in [
                        ("units.tsv", format!("{HEADER}\n")),
                        (
                            "ledger.tsv",
                            "pr\tsha\tverdict\tevidence\tverifier\tts\n".into(),
                        ),
                        ("gates.md", String::new()),
                        ("preferences.md", String::new()),
                        ("frontier.json", "{}\n".into()),
                    ] {
                        if !store.join(name).exists() {
                            write(store, name, &content)?;
                        }
                    }
                    Ok(())
                })?;
                let path = store.to_string_lossy().to_string();
                Ok(emit(
                    serde_json::json!({"store":path}),
                    format!("initialized {path}"),
                    json,
                ))
            }
            ("unit", "add") => {
                let id = required(arg(args, 2)?, "unit id")?;
                base::require_slug("unit id", &id)?;
                let track = required(
                    option(args, "--track")?.ok_or_else(|| fail("--track is required"))?,
                    "track",
                )?;
                let brief = option(args, "--brief")?
                    .map(|s| required(s, "brief"))
                    .transpose()?
                    .unwrap_or_default();
                let fixes = option(args, "--fixes")?
                    .map(|s| required(s, "fixes"))
                    .transpose()?;
                let routing_arg = option(args, "--routing").map_err(|e| {
                    Failure::new(
                        "invalid_argument",
                        format!("invalid_argument: {}", e.message),
                    )
                })?;
                let reason_arg = option(args, "--reason").map_err(|e| {
                    Failure::new(
                        "invalid_argument",
                        format!("invalid_argument: {}", e.message),
                    )
                })?;
                if routing_arg.is_some() != reason_arg.is_some()
                    || (routing_arg.is_some() && fixes.is_none())
                    || reason_arg.is_some_and(|s| s.trim().is_empty())
                {
                    return Err(Failure::new("invalid_argument", "invalid_argument: --routing and non-empty --reason require each other and --fixes"));
                }
                let explicit = routing_arg
                    .map(|s| -> Res<_> {
                        let (tier, effort) = s.split_once('/').ok_or_else(|| {
                            Failure::new(
                                "invalid_argument",
                                "invalid_argument: routing must be tier/effort",
                            )
                        })?;
                        let tier = crate::codex::routing::parse_tier(tier).map_err(|e| {
                            Failure::new("invalid_argument", format!("invalid_argument: {e}"))
                        })?;
                        if tier == crate::codex::routing::Tier::Luna {
                            return Err(Failure::new(
                                "invalid_argument",
                                "invalid_argument: luna is not valid for an exception",
                            ));
                        }
                        let effort = crate::codex::routing::parse_effort(effort).map_err(|e| {
                            Failure::new("invalid_argument", format!("invalid_argument: {e}"))
                        })?;
                        Ok((tier, effort))
                    })
                    .transpose()?;
                if fixes.is_some() && brief.is_empty() {
                    return Err(fail("--fixes requires --brief"));
                }
                let row = Unit {
                    id,
                    track,
                    state: "pending".into(),
                    branch: String::new(),
                    pr: String::new(),
                    sha: String::new(),
                    brief,
                };
                if fixes.is_none() {
                    validate_add_brief(store, &row)?;
                }
                let fix_routing = locked(store, force, &mut note, || {
                    if fixes.is_some() {
                        validate_add_brief(store, &row)?;
                    }
                    let mut rows = read_units(store)?;
                    if let Some(old) = &fixes {
                        if !rows.iter().any(|u| u.id == *old) {
                            return Err(Failure::new("not_found", format!("unit {old} not found")));
                        }
                    }
                    if rows.iter().any(|u| u.id == row.id) {
                        return Err(fail(format!("unit {} already exists", row.id)));
                    }
                    let chain = if let Some(old) = &fixes {
                        rows.iter()
                            .filter(|u| standing_unit(&rows, u).is_some_and(|end| end.id == *old))
                            .map(|u| u.id.clone())
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    rows.push(row.clone());
                    if let Some(old) = &fixes {
                        supersede_rows(store, &mut rows, old, &row.id)?;
                        let mut fix_units = Vec::new();
                        for unit in &chain {
                            if fixed_unit(store, unit)?.is_some() {
                                fix_units.push(unit.as_str());
                            }
                        }
                        if fix_units.len() >= 2 {
                            add_adjudication_locked(store, "fix_rounds", old)?;
                            return Err(Failure::new("fix_rounds", format!(
                                "fix_rounds: fix units {} already belong to this chain; report to the core, which can waive the findings or close the wish as blocked",
                                fix_units.join(", ")
                            )));
                        }
                        use crate::codex::routing::{
                            self, Escalation, FixRecord, Risk, Routing, Rule,
                        };
                        let parent = rows.iter().find(|u| u.id == *old).unwrap();
                        let snapshot = routing::read_snapshot(store, old)?;
                        let map_text = match std::fs::read_to_string(store.join("modules.md")) {
                            Ok(s) => s,
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                            Err(e) => return Err(io(e)),
                        };
                        let line = crate::orch::parse::parse_map(&map_text)
                            .ok()
                            .and_then(|m| m.modules.into_iter().find(|m| m.id == parent.track))
                            .and_then(|m| {
                                m.routing
                                    .iter()
                                    .find(|r| r.key == *old)
                                    .or_else(|| m.routing.iter().find(|r| r.key == "default"))
                                    .cloned()
                            });
                        let invalid = |e| Failure::new("invalid_argument", e);
                        let missing = || Failure::new(
                            "routing_missing",
                            format!("routing_missing: parent unit {old} needs a routing decision and risk (ROUTING or RISK)"),
                        );
                        let (parent_tier, parent_effort) =
                            if let Some(s) = snapshot.as_ref().filter(|s| s.tier.is_some()) {
                                (s.tier, s.effort)
                            } else if snapshot.as_ref().is_some_and(|s| {
                                s.rule == Rule::ExplicitModel && s.tier.is_none()
                            }) {
                                if explicit.is_none() {
                                    return Err(Failure::new("routing_missing", format!("routing_missing: parent unit {old} has an explicit model without a tier; supply --routing <tier>/<effort> --reason")));
                                }
                                (None, snapshot.as_ref().unwrap().effort)
                            } else if let Some(l) = &line {
                                (
                                    Some(routing::parse_tier(&l.tier).map_err(invalid)?),
                                    Some(routing::parse_effort(&l.effort).map_err(invalid)?),
                                )
                            } else {
                                return Err(missing());
                            };
                        let risk = if let Some(r) = snapshot.as_ref().and_then(|s| s.risk) {
                            r
                        } else if let Some(l) = &line {
                            serde_json::from_value(serde_json::Value::String(l.risk.clone()))
                                .map_err(|e| invalid(format!("{e}")))?
                        } else {
                            let text = std::fs::read_to_string(&parent.brief).map_err(|e| {
                                if e.kind() == std::io::ErrorKind::NotFound || parent.brief.is_empty() {
                                    missing()
                                } else {
                                    io(e)
                                }
                            })?;
                            let parent_brief = parse_brief(&text).map_err(invalid)?;
                            let value = parent_brief.risk.ok_or_else(missing)?;
                            serde_json::from_value::<Risk>(serde_json::Value::String(value))
                                .map_err(|e| invalid(format!("{e}")))?
                        };
                        let (route, rule) = if let Some((tier, effort)) = explicit {
                            let r = Routing {
                                tier,
                                effort,
                                risk: Some(risk),
                                reason: reason_arg.unwrap().trim().into(),
                            };
                            routing::check_rubric(&r).map_err(|e| {
                                Failure::new("routing_rubric", format!("routing_rubric: {e}"))
                            })?;
                            (r, Rule::PlanException)
                        } else {
                            match routing::escalate(parent_tier.ok_or_else(missing)?, parent_effort) {
                                Escalation::Next(r) => (r, Rule::FixEscalation),
                                Escalation::Saturated => {
                                    return Err(Failure::new(
                                        "escalation_saturated",
                                        "escalation_saturated: parent routing is saturated",
                                    ))
                                }
                            }
                        };
                        let brief_text = std::fs::read_to_string(&row.brief).map_err(io)?;
                        let parsed = parse_brief(&brief_text).map_err(invalid)?;
                        if parsed
                            .tier
                            .as_deref()
                            .is_some_and(|s| routing::parse_tier(s).ok() != Some(route.tier))
                            || parsed.effort.as_deref().is_some_and(|s| {
                                routing::parse_effort(s).ok() != Some(route.effort)
                            })
                        {
                            return Err(Failure::new(
                                "routing_mismatch",
                                "routing_mismatch: brief conflicts with routing",
                            ));
                        }
                        let tier = serde_json::to_value(route.tier)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned();
                        let effort = serde_json::to_value(route.effort)
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned();
                        let mut additions = String::new();
                        if parsed.tier.is_none() {
                            additions.push_str(&format!("TIER {tier}\n"));
                        }
                        if parsed.effort.is_none() {
                            additions.push_str(&format!("EFFORT {effort}\n"));
                        }
                        let mut updated = String::new();
                        for part in brief_text.split_inclusive('\n') {
                            updated.push_str(part);
                            if part
                                .strip_prefix("GOAL")
                                .is_some_and(|s| s.is_empty() || s.starts_with(char::is_whitespace))
                                && !additions.is_empty()
                            {
                                if !part.ends_with('\n') {
                                    updated.push('\n');
                                }
                                updated.push_str(&additions);
                                additions.clear();
                            }
                        }
                        let record = FixRecord {
                            unit: row.id.clone(),
                            parent: old.clone(),
                            parent_tier,
                            parent_effort,
                            tier: route.tier,
                            effort: route.effort,
                            rule,
                            reason: route.reason,
                        };
                        // Validate existing evidence before either write.
                        crate::orch::evidence(store, &row.id)?;
                        // The add response computes readiness; reject existing invalid data
                        // before changing either the units or evidence files.
                        crate::orch::ready_units(store)?;
                        crate::orch::waiting_units(store)?;
                        let original_units = std::fs::read(store.join("units.tsv")).map_err(io)?;
                        let evidence_path = store
                            .join("reports")
                            .join(format!("{}.evidence.tsv", row.id));
                        let original_evidence = match std::fs::read(&evidence_path) {
                            Ok(v) => Some(v),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                            Err(e) => return Err(io(e)),
                        };
                        let fix_path = routing::fix_record_path(store, &row.id);
                        let original_fix = match std::fs::read(&fix_path) {
                            Ok(v) => Some(v),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                            Err(e) => return Err(io(e)),
                        };
                        save_units(store, &rows)?;
                        let appended = crate::orch::append_evidence_locked(
                            store,
                            &row.id,
                            &crate::orch::Evidence {
                                kind: crate::orch::FIXES_KIND.into(),
                                command: old.clone(),
                                exit_code: 0,
                                commit: String::new(),
                                ts: base::now_iso(),
                            },
                        );
                        let result = appended
                            .and_then(|_| routing::write_fix(store, &record))
                            .and_then(|_| {
                                if updated != brief_text {
                                    // Test-only final-write failure, gated like crash_point.
                                    if cfg!(debug_assertions)
                                        && std::env::var("AXIOM_TEST_FAIL_FIX_BRIEF_WRITE")
                                            .ok().as_deref() == Some("1")
                                    {
                                        return Err(io(std::io::Error::new(
                                            std::io::ErrorKind::PermissionDenied,
                                            "injected fix brief write failure",
                                        )));
                                    }
                                    base::atomic_write(Path::new(&row.brief), updated.as_bytes())
                                        .map_err(io)?;
                                }
                                Ok(())
                            });
                        if let Err(error) = result {
                            base::atomic_write(&store.join("units.tsv"), &original_units)
                                .map_err(io)?;
                            for (path, original) in [
                                (&evidence_path, original_evidence),
                                (&fix_path, original_fix),
                            ] {
                                if let Some(bytes) = original {
                                    base::atomic_write(path, &bytes).map_err(io)?;
                                } else if let Err(e) = std::fs::remove_file(path) {
                                    if e.kind() != std::io::ErrorKind::NotFound {
                                        return Err(io(e));
                                    }
                                }
                            }
                            return Err(error);
                        }
                        Ok(Some(record))
                    } else {
                        save_units(store, &rows)?;
                        Ok(None)
                    }
                })?;
                let next = crate::orch::next_step_for(store, &row.id)?;
                let mut value = serde_json::to_value(&row).unwrap();
                value["next"] = serde_json::json!(next);
                if let Some(old) = fixes {
                    value["fixes"] = serde_json::json!(old);
                }
                let mut text = format!("{}\n{next}", unit_line(&row));
                if let Some(r) = fix_routing {
                    let rule = serde_json::to_value(r.rule).unwrap();
                    let tier = serde_json::to_value(r.tier).unwrap();
                    let effort = serde_json::to_value(r.effort).unwrap();
                    text.push_str(&format!(
                        "\nrouting {}/{} {}",
                        tier.as_str().unwrap(),
                        effort.as_str().unwrap(),
                        rule.as_str().unwrap()
                    ));
                    value["routing"] = serde_json::json!({"tier": tier, "effort": effort, "rule": rule, "reason": r.reason, "parent": {"tier": r.parent_tier, "effort": r.parent_effort}});
                }
                Ok(emit(value, text, json))
            }
            ("unit", "set") => {
                let id = required(arg(args, 2)?, "unit id")?;
                let state = required(
                    option(args, "--state")?.ok_or_else(|| fail("--state is required"))?,
                    "state",
                )?;
                if state == "built" || state == "verified" {
                    return Err(fail(format!(
                        "state {state} is derived from evidence; it cannot be set"
                    )));
                }
                if state.starts_with(SUPERSEDED) {
                    return Err(fail(
                        "state superseded is recorded by orch unit supersede <unit> --by <unit>",
                    ));
                }
                let row = locked(store, force, &mut note, || {
                    let mut rows = read_units(store)?;
                    let u = rows
                        .iter_mut()
                        .find(|u| u.id == id)
                        .ok_or_else(|| Failure::new("not_found", format!("unit {id} not found")))?;
                    u.refuse_superseded()?;
                    if u.state == "verified" {
                        return Err(fail(format!(
                            "unit {id} is verified; its state follows its evidence and cannot be set"
                        )));
                    }
                    u.state = state;
                    if let Some(v) = option(args, "--branch")? {
                        u.branch = required(v, "branch")?;
                    }
                    if let Some(v) = option(args, "--sha")? {
                        u.sha = required(v, "SHA")?;
                    }
                    if let Some(v) = option(args, "--pr")? {
                        if v.parse::<u64>().ok().filter(|n| *n > 0).is_none() {
                            return Err(fail("PR must be a positive integer"));
                        }
                        u.pr = v.into();
                    }
                    let out = u.clone();
                    save_units(store, &rows)?;
                    Ok(out)
                })?;
                Ok(emit(
                    serde_json::to_value(&row).unwrap(),
                    unit_line(&row),
                    json,
                ))
            }
            ("unit", "supersede") => {
                let id = required(arg(args, 2)?, "unit id")?;
                let by = required(&required_option(args, "--by")?, "--by")?;
                let row = locked(store, force, &mut note, || supersede(store, &id, &by))?;
                let next = crate::orch::next_step_for(store, &by)?;
                let mut value = serde_json::to_value(&row).unwrap();
                value["next"] = serde_json::json!(next);
                Ok(emit(value, format!("{}\n{next}", unit_line(&row)), json))
            }
            ("unit", "get") => {
                let id = required(arg(args, 2)?, "unit id")?;
                let row = read_units(store)?
                    .into_iter()
                    .find(|u| u.id == id)
                    .ok_or_else(|| Failure::new("not_found", format!("unit {id} not found")))?;
                Ok(emit(
                    serde_json::to_value(&row).unwrap(),
                    unit_line(&row),
                    json,
                ))
            }
            ("unit", "list") => {
                let state = option(args, "--state")?;
                let track = option(args, "--track")?;
                let rows: Vec<_> = read_units(store)?
                    .into_iter()
                    .filter(|u| {
                        state.is_none_or(|s| u.state == s || u.state_name() == s)
                            && track.is_none_or(|s| u.track == s)
                    })
                    .collect();
                let mut lines: Vec<_> = rows.iter().take(4).map(unit_line).collect();
                if rows.len() > 4 {
                    lines.push(format!("... {} more; use --json", rows.len() - 4));
                }
                let compact = if lines.is_empty() {
                    "(no units)".into()
                } else {
                    lines.join("\n")
                };
                Ok(emit(serde_json::to_value(rows).unwrap(), compact, json))
            }
            ("unit", "counts") => {
                let mut counts = std::collections::BTreeMap::<String, usize>::new();
                for u in read_units(store)? {
                    *counts.entry(u.state_name().to_string()).or_default() += 1;
                }
                let compact = if counts.is_empty() {
                    "none".into()
                } else {
                    counts
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                Ok(emit(serde_json::to_value(counts).unwrap(), compact, json))
            }
            ("inbox", "push") => {
                let (v, name) = locked(store, force, &mut note, || {
                    push(
                        store,
                        arg(args, 2)?,
                        arg(args, 3)?,
                        arg(args, 4)?,
                        option(args, "--report")?,
                    )
                })?;
                Ok(emit(
                    v.clone(),
                    format!(
                        "{}\t{}\t{name}",
                        v["unit"].as_str().unwrap_or(""),
                        v["status"].as_str().unwrap_or("")
                    ),
                    json,
                ))
            }

            ("ledger", "record") => {
                let pr = pr(arg(args, 2)?)?;
                let sha = required(arg(args, 3)?, "SHA")?;
                let verdict = arg(args, 4)?;
                if !VERDICTS.contains(&verdict) {
                    return Err(fail("verdict must be live-ui-verified, unit-test-verified, type-check-only, verifier-blocked, or verifier-failed"));
                }
                let evidence = required(&required_option(args, "--evidence")?, "evidence")?;
                let verifier = option(args, "--verifier")?
                    .map(|s| required(s, "verifier"))
                    .transpose()?
                    .unwrap_or_default();
                let row = serde_json::json!({"pr":pr,"sha":sha,"verdict":verdict,"evidence":evidence,"verifier":verifier,"ts":base::now_iso()});
                locked(store, force, &mut note, || {
                    let mut rows = ledger(store)?;
                    if let Some(i) = rows
                        .iter()
                        .position(|r| r["pr"] == row["pr"] && r["sha"] == row["sha"])
                    {
                        rows[i] = row.clone()
                    } else {
                        rows.push(row.clone())
                    }
                    save_ledger(store, &rows)
                })?;
                Ok(emit(
                    row.clone(),
                    format!(
                        "{}\t{}\t{}",
                        cell(&row, "pr"),
                        cell(&row, "sha"),
                        cell(&row, "verdict")
                    ),
                    json,
                ))
            }
            ("ledger", "check") => {
                let pr = pr(arg(args, 2)?)?;
                let sha = required(arg(args, 3)?, "SHA")?;
                let row = ledger(store)?
                    .into_iter()
                    .find(|r| cell(r, "pr") == pr && cell(r, "sha") == sha);
                match row {
                    Some(r) => Ok(emit(r.clone(), cell(&r, "verdict"), json)),
                    None => {
                        ledger_missing = true;
                        return Ok(emit(
                            serde_json::json!({"pr":pr,"sha":sha,"verdict":"NOT-VERIFIED"}),
                            "NOT-VERIFIED".into(),
                            json,
                        ));
                    }
                }
            }
            ("ledger", "summary") => {
                let v = counts(ledger(store)?.iter().map(|r| cell(r, "verdict")));
                Ok(emit(v.clone(), count_line(&v), json))
            }
            ("inbox", "count") => {
                let n = pointers(store)?.len();
                Ok(emit(serde_json::json!({"count":n}), n.to_string(), json))
            }
            ("inbox", "drain") => {
                let peek = args.iter().any(|a| a == "--peek");
                let read = || -> Res<Vec<serde_json::Value>> {
                    let mut rows = Vec::new();
                    for (path, row) in pointer_files(store)? {
                        if !peek {
                            remove_pointer(&path)?;
                        }
                        rows.push(row);
                    }
                    Ok(rows)
                };
                let rows = if peek {
                    read()?
                } else {
                    locked(store, force, &mut note, read)?
                };
                let compact = rows_display(
                    &rows,
                    |r| {
                        ["ts", "agent", "unit", "status", "report"]
                            .iter()
                            .map(|k| cell(r, k))
                            .collect::<Vec<_>>()
                            .join("\t")
                    },
                    "(empty)",
                    None,
                );
                Ok(emit(serde_json::json!(rows), compact, json))
            }
            ("gate", "park") => {
                let id = line(arg(args, 2)?, "gate id")?;
                let q = line(&required_option(args, "--question")?, "question")?;
                let o = line(&required_option(args, "--options")?, "options")?;
                let d = line(&required_option(args, "--default")?, "default")?;
                let row = serde_json::json!({"kind":"open","id":id,"question":q,"options":o,"defaultAnswer":d});
                locked(store, force, &mut note, || {
                    let mut rows = gates(store)?;
                    if let Some(i) = rows.iter().position(|r| r["id"] == row["id"]) {
                        rows[i] = row.clone()
                    } else {
                        rows.push(row.clone())
                    }
                    save_gates(store, &rows)
                })?;
                Ok(emit(
                    row.clone(),
                    format!("{}\topen", cell(&row, "id")),
                    json,
                ))
            }
            ("gate", "resolve") => {
                let id = line(arg(args, 2)?, "gate id")?;
                let answer = line(&required_option(args, "--answer")?, "answer")?;
                let row = locked(store, force, &mut note, || {
                    let mut rows = gates(store)?;
                    let r = rows
                        .iter_mut()
                        .find(|r| cell(r, "id") == id)
                        .ok_or_else(|| Failure::new("not_found", format!("gate {id} not found")))?;
                    r["kind"] = serde_json::json!("resolved");
                    r["answer"] = serde_json::json!(answer);
                    let out = r.clone();
                    save_gates(store, &rows)?;
                    Ok(out)
                })?;
                Ok(emit(
                    row.clone(),
                    format!("{}\tresolved\t{}", id, cell(&row, "answer")),
                    json,
                ))
            }
            ("gate", "list") => {
                let rows: Vec<_> = gates(store)?
                    .into_iter()
                    .filter(|r| cell(r, "kind") == "open")
                    .collect();
                let compact = rows_display(
                    &rows,
                    |r| {
                        ["id", "question", "options", "defaultAnswer"]
                            .iter()
                            .map(|k| cell(r, k))
                            .collect::<Vec<_>>()
                            .join("\t")
                    },
                    "(no open gates)",
                    Some(4),
                );
                Ok(emit(serde_json::json!(rows), compact, json))
            }
            ("standing", "show") => {
                let rows = standing(store)?;
                let compact = rows_display(
                    &rows,
                    |r| format!("{}. {}", r["number"], cell(r, "line")),
                    "(no standing orders)",
                    Some(4),
                );
                Ok(emit(serde_json::json!(rows), compact, json))
            }
            ("standing", "add") => {
                let value = line(arg(args, 2)?, "standing order")?;
                let row = locked(store, force, &mut note, || {
                    let mut rows = standing(store)?;
                    let row = serde_json::json!({"number":rows.len()+1,"line":value});
                    rows.push(row.clone());
                    let text = rows
                        .iter()
                        .map(|r| format!("{}. {}\n", r["number"], cell(r, "line")))
                        .collect::<String>();
                    write(store, "preferences.md", &text)?;
                    Ok(row)
                })?;
                Ok(emit(
                    row.clone(),
                    format!("{}. {}", row["number"], cell(&row, "line")),
                    json,
                ))
            }
            ("frontier", "show") => {
                let v = frontier(store)?;
                let prs = v["prs"].as_array().unwrap();
                let list = if prs.is_empty() {
                    "none".into()
                } else {
                    prs.iter()
                        .map(|r| {
                            format!(
                                "{}#{}@{}:{}",
                                cell(r, "branches"),
                                r["pr"],
                                cell(r, "sha"),
                                cell(r, "state")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let lowest = if v["lowestUnmerged"].is_null() {
                    "none".into()
                } else {
                    v["lowestUnmerged"].to_string()
                };
                Ok(emit(
                    v.clone(),
                    format!(
                        "generation={} prs={} lowest-unmerged={}",
                        v["generation"], list, lowest
                    ),
                    json,
                ))
            }
            ("status", _) => {
                let (v, compact) = locked(store, force, &mut note, || status(store))?;
                Ok(emit(v, compact, json))
            }
            ("frontier", "set") => Err(fail("frontier set is not supported")),
            ("map", _) => {
                let map_path = store.join("modules.md");
                let text = std::fs::read_to_string(&map_path).map_err(|e| {
                    Failure::new("io", format!("cannot read {}: {e}", map_path.display()))
                })?;
                let plan_path = store.join("astra").join("plan-final.md");
                let plan = match std::fs::read_to_string(&plan_path) {
                    Ok(text) => Some(text),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        return Err(Failure::new(
                            "io",
                            format!("cannot read {}: {e}", plan_path.display()),
                        ))
                    }
                };
                let repo = option(args, "--repo")?.map(Path::new);
                if repo.is_some_and(|p| !p.is_dir()) {
                    return Err(fail("--repo must be an existing directory"));
                }
                let problems = crate::orch::lint_map(&text, plan.as_deref(), repo);
                if !problems.is_empty() {
                    let mut message = format!(
                        "modules.md has {} problems; fix them all, then run orch map again:",
                        problems.len()
                    );
                    for p in problems {
                        message.push_str(&format!(
                            "\n- {}{}: {}",
                            p.module.map(|m| format!("{m}: ")).unwrap_or_default(),
                            p.kind,
                            p.message
                        ));
                    }
                    return Err(fail(message));
                }
                if repo.is_none() {
                    note.push_str("paths not checked: pass --repo <dir>\n");
                }
                let map = crate::orch::parse_map(&text).map_err(fail)?;
                let mut map_json = serde_json::to_value(&map).unwrap();
                for module in map_json["modules"].as_array_mut().unwrap() {
                    let module = module.as_object_mut().unwrap();
                    module.remove("new_paths");
                    module.remove("shared_owner");
                }
                Ok(emit(
                    map_json,
                    map.modules
                        .iter()
                        .map(|m| {
                            format!("{}\t{}\t{}", m.id, m.depends.join(","), m.paths.join(","))
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    json,
                ))
            }
            ("module-context", _) => {
                let module = arg(args, 1)?;
                let repo = option(args, "--repo")?.ok_or_else(|| fail("--repo is required"))?;
                let context = crate::orch::module_context(store, module, Path::new(repo))?;
                Ok(emit(
                    serde_json::to_value(&context).unwrap(),
                    crate::orch::render_module_context(&context),
                    json || args.iter().any(|arg| arg == "--json"),
                ))
            }
            _ => {
                if let Some(out) = crate::orch::commands::run_extra(store, args, json) {
                    extra_output = Some(out);
                    return Ok(String::new());
                }
                Err(fail(format!(
                    "unknown command {}; commands: {COMMANDS}",
                    args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
                )))
            }
        }
    })();
    if let Some(out) = extra_output {
        return out;
    }
    match result {
        Ok(stdout) => Output {
            exit_code: if ledger_missing { 2 } else { 0 },
            stdout,
            stderr: note,
        },
        Err(e) => Output {
            exit_code: if e.kind == "not_found" { 2 } else { 1 },
            stdout: String::new(),
            stderr: format!("{note}error: {}\n", e.message),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn a_holder_that_never_lets_go_ends_the_wait_with_an_error() {
        let t = tempfile::tempdir().unwrap();
        let mut command = crate::base::proc::cmd(if cfg!(windows) { "cmd" } else { "sh" });
        command.args(if cfg!(windows) {
            ["/C", "more"]
        } else {
            ["-c", "cat"]
        });
        let mut holder = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let lock = t.path().join(".orch.lock");
        std::fs::write(
            &lock,
            format!(
                "{}
",
                holder.id()
            ),
        )
        .unwrap();
        let started = Instant::now();
        let refused = acquire(
            t.path(),
            false,
            &mut String::new(),
            Duration::from_millis(150),
        )
        .err()
        .expect("a live holder keeps the lock");
        assert!(refused.message.contains("store lock held"));
        assert!(started.elapsed() >= Duration::from_millis(150));
        assert_eq!(
            std::fs::read_to_string(&lock).unwrap().trim(),
            holder.id().to_string()
        );
        drop(holder.stdin.take());
        holder.wait().unwrap();
    }

    fn lock_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".orch.lock"))
            .collect()
    }

    #[test]
    fn a_reclaimer_that_died_mid_reclaim_does_not_keep_a_dead_holders_lock() {
        let short = Duration::from_millis(300);
        let t = tempfile::tempdir().unwrap();
        let lock = t.path().join(".orch.lock");
        // Pid 4294967295 held the lock and died; pid 4294967294 claimed the reclaim and died too.
        std::fs::write(&lock, "4294967295\n").unwrap();
        let marker = t.path().join(".orch.lock.4294967295.stale");
        std::fs::write(&marker, "4294967294\n").unwrap();
        let mut note = String::new();
        let held = acquire(t.path(), false, &mut note, short)
            .expect("an abandoned reclaim marker is itself reclaimed");
        assert!(note.contains("replacing stale store lock"), "{note}");
        drop(held);
        assert!(
            lock_files(t.path()).is_empty(),
            "{:?}",
            lock_files(t.path())
        );

        // A reclaimer that died before writing its pid leaves an empty marker.
        std::fs::write(&lock, "4294967295\n").unwrap();
        let empty = std::fs::File::create(&marker).unwrap();
        empty
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        drop(empty);
        drop(acquire(t.path(), false, &mut String::new(), short).expect("an old empty marker"));
        assert!(
            lock_files(t.path()).is_empty(),
            "{:?}",
            lock_files(t.path())
        );

        // A live reclaimer keeps its claim, and every level of a dead chain is cleared once it ends.
        let mut command = crate::base::proc::cmd(if cfg!(windows) { "cmd" } else { "sh" });
        command.args(if cfg!(windows) {
            ["/C", "more"]
        } else {
            ["-c", "cat"]
        });
        let mut claimant = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        std::fs::write(&lock, "4294967295\n").unwrap();
        std::fs::write(&marker, "4294967294\n").unwrap();
        let second = t
            .path()
            .join(".orch.lock.4294967295.stale.4294967294.stale");
        std::fs::write(&second, format!("{}\n", claimant.id())).unwrap();
        assert!(acquire(t.path(), false, &mut String::new(), short).is_err());
        assert_eq!(std::fs::read_to_string(&lock).unwrap().trim(), "4294967295");
        drop(claimant.stdin.take());
        claimant.wait().unwrap();
        drop(acquire(t.path(), false, &mut String::new(), short).expect("the claimant died"));
        assert!(
            lock_files(t.path()).is_empty(),
            "{:?}",
            lock_files(t.path())
        );
    }
}
