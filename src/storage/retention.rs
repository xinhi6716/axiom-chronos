//! Conservative retention planning, execution and auditing.
use super::{archive, ledger, Class, Entry, EntryState, Ledger, Owner, Policy};
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const SAFE_CLASSES: [Class; 3] = [Class::Scratch, Class::BuildCache, Class::EngineCopy];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Delete,
    Compress,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Routine,
    Pressure,
}
#[derive(Debug, Clone, Serialize)]
pub struct Planned {
    pub path: PathBuf,
    pub class: Class,
    pub owner: Owner,
    pub action: Action,
    pub reason: String,
    pub size_bytes: Option<u64>,
    pub last_used_ms: i64,
}
#[derive(Debug, Default, Serialize)]
pub struct Plan {
    pub items: Vec<Planned>,
    pub kept: Vec<(PathBuf, String)>,
    pub reclaimable_bytes: u64,
    pub unmeasured: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub ts: String,
    pub path: String,
    pub class: Class,
    pub size_bytes: Option<u64>,
    pub action: Action,
    pub reason: String,
    pub outcome: String,
    pub project: String,
    pub wish: Option<String>,
}
#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub freed_bytes: u64,
    pub records: Vec<AuditRecord>,
}
fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("storage", e.to_string())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn shared_busy(e: &Entry) -> bool {
    e.meta["shared"].as_bool() == Some(true)
        && !e.meta["users"].as_array().is_some_and(|u| u.is_empty())
}
fn cache_reason(e: &Entry, policy: &Policy, mode: Mode, now_ms: i64) -> Option<&'static str> {
    if shared_busy(e) {
        return None;
    }
    let reason = match e.meta["worktree_state"].as_str()? {
        "merged" => "merged_worktree_cache",
        "abandoned" => "abandoned_worktree_cache",
        "gone" => "gone_worktree_cache",
        _ => return None,
    };
    if mode == Mode::Routine {
        let since = e.meta["state_since_ms"].as_i64()?;
        let days = policy
            .build_cache_days
            .saturating_mul(86_400_000)
            .min(i64::MAX as u64) as i64;
        if now_ms.saturating_sub(since) < days {
            return None;
        }
    }
    Some(reason)
}
pub fn is_reclaimable(e: &Entry, policy: &Policy, now_ms: i64) -> bool {
    if e.state != EntryState::Present {
        return false;
    }
    match e.class {
        Class::Scratch => now_ms.saturating_sub(e.last_used_ms) > 600_000,
        Class::BuildCache => cache_reason(e, policy, Mode::Routine, now_ms).is_some(),
        _ => false,
    }
}
fn same(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn within(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        PathBuf::from(a.to_string_lossy().to_lowercase())
            .starts_with(PathBuf::from(b.to_string_lossy().to_lowercase()))
    }
    #[cfg(not(windows))]
    {
        a.starts_with(b)
    }
}
fn link(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}
// Reject links anywhere in the path, including junctions in its parents.
fn no_links(path: &Path) -> bool {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if link(&m) => return false,
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return false,
        }
    }
    true
}
fn planned(e: &Entry, path: PathBuf, action: Action, reason: &str, size: Option<u64>) -> Planned {
    Planned {
        path,
        class: e.class,
        owner: e.owner.clone(),
        action,
        reason: reason.into(),
        size_bytes: size,
        last_used_ms: e.last_used_ms,
    }
}
fn add(plan: &mut Plan, item: Planned) {
    if item.action == Action::Delete {
        if let Some(n) = item.size_bytes {
            plan.reclaimable_bytes = plan.reclaimable_bytes.saturating_add(n);
        } else {
            plan.unmeasured += 1;
        }
    }
    plan.items.push(item);
}
fn collect_files(
    e: &Entry,
    path: &Path,
    entries: &[Entry],
    now: SystemTime,
    days: u64,
    plan: &mut Plan,
) -> Res<()> {
    if !no_links(path) {
        return Ok(());
    }
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(x) if x.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(x) => return Err(error(x)),
    };
    if !same(path, &e.path) && entries.iter().any(|other| same(&other.path, path)) {
        return Ok(());
    }
    if m.is_dir() {
        for child in fs::read_dir(path).map_err(error)? {
            collect_files(e, &child.map_err(error)?.path(), entries, now, days, plan)?;
        }
    } else if m.is_file() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name != "events.jsonl"
            && !name.ends_with(".lock")
            && archive::eligible(path, now, days)
            && !archive::archived_path(path).exists()
        {
            add(
                plan,
                planned(
                    e,
                    path.to_path_buf(),
                    Action::Compress,
                    "aged_30_days",
                    Some(m.len()),
                ),
            );
        }
    }
    Ok(())
}
pub fn plan(home: &Path, policy: &Policy, mode: Mode, now_ms: i64) -> Res<Plan> {
    let ledger = Ledger::open(home)?;
    let entries = ledger.entries()?;
    let mut result = Plan::default();
    let current = ledger::normalize(&home.join("engine").join(format!(
        "{}-{}",
        env!("CARGO_PKG_VERSION"),
        crate::engine::build_id()
    )));
    let previous = entries
        .iter()
        .filter(|e| e.class == Class::EngineCopy && !same(&e.path, &current))
        .max_by_key(|e| {
            if e.last_used_ms > 0 {
                e.last_used_ms
            } else {
                fs::metadata(&e.path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
                    .unwrap_or(0)
            }
        })
        .map(|e| e.id);
    let system_time = UNIX_EPOCH
        .checked_add(Duration::from_millis(now_ms.max(0) as u64))
        .unwrap_or(UNIX_EPOCH);
    for e in &entries {
        let used = ledger.in_use(&e.path)?;
        let reason = match e.class {
            Class::Scratch if !used && now_ms.saturating_sub(e.last_used_ms) > 600_000 => {
                Some("orphan_scratch")
            }
            Class::BuildCache if !used => cache_reason(e, policy, mode, now_ms),
            Class::EngineCopy if !used && !same(&e.path, &current) && Some(e.id) != previous => {
                Some("engine_retention")
            }
            _ => None,
        };
        if let Some(reason) = reason {
            add(
                &mut result,
                planned(e, e.path.clone(), Action::Delete, reason, e.size_bytes),
            );
        } else {
            result.kept.push((
                e.path.clone(),
                if used { "in_use" } else { "retained" }.into(),
            ));
            if mode == Mode::Routine && !used && matches!(e.class, Class::Logs | Class::Evidence) {
                collect_files(
                    e,
                    &e.path,
                    &entries,
                    system_time,
                    policy.compress_after_days,
                    &mut result,
                )?;
            }
        }
    }
    Ok(result)
}
pub fn audit_log(home: &Path) -> PathBuf {
    home.join("storage").join("deletions.jsonl")
}
fn audit(home: &Path, record: &AuditRecord) -> Res<()> {
    fs::create_dir_all(home.join("storage")).map_err(error)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(audit_log(home))
        .map_err(error)?;
    file.lock().map_err(error)?;
    let mut line = serde_json::to_vec(record).map_err(error)?;
    line.push(b'\n');
    file.write_all(&line).map_err(error)?;
    file.flush().map_err(error)?;
    Ok(())
}
fn record(item: &Planned, outcome: String) -> AuditRecord {
    AuditRecord {
        ts: base::now_iso(),
        path: item.path.to_string_lossy().into_owned(),
        class: item.class,
        size_bytes: item.size_bytes,
        action: item.action,
        reason: item.reason.clone(),
        outcome,
        project: item.owner.project.clone(),
        wish: item.owner.wish.clone(),
    }
}
fn check(home: &Path, ledger: &Ledger, item: &Planned) -> Res<Option<String>> {
    if !no_links(&item.path) || !same(&ledger::normalize(&item.path), &item.path) {
        return Ok(Some("path changed or contains a link".into()));
    }
    let entries = ledger.entries()?;
    let entry = if item.action == Action::Delete {
        entries.iter().find(|e| same(&e.path, &item.path))
    } else {
        entries
            .iter()
            .filter(|e| within(&item.path, &e.path))
            .max_by_key(|e| e.path.components().count())
    };
    let Some(e) = entry else {
        return Ok(Some("entry is not present".into()));
    };
    if e.class != item.class || e.owner != item.owner {
        return Ok(Some("class or owner changed".into()));
    }
    if e.last_used_ms > item.last_used_ms {
        return Ok(Some("entry used since planning".into()));
    }
    if ledger.in_use(&e.path)? {
        return Ok(Some("in use".into()));
    }
    if item.action == Action::Delete {
        if !SAFE_CLASSES.contains(&item.class) {
            return Ok(Some("class is protected".into()));
        }
        if item.class == Class::BuildCache && shared_busy(e) {
            return Ok(Some("shared cache has users".into()));
        }
        if entries
            .iter()
            .any(|other| other.id != e.id && within(&other.path, &item.path))
        {
            return Ok(Some("registered descendant".into()));
        }
        let root = ledger::normalize(home);
        let allowed = !within(&root, &item.path)
            && (within(&item.path, &root)
                || (e.class == Class::Scratch
                    && e.path
                        .ancestors()
                        .skip(1)
                        .any(|p| p.file_name().is_some_and(|name| name == "scratch")))
                || entries.iter().any(|r| {
                    matches!(r.class, Class::Worktree | Class::BuildCache)
                        && within(&item.path, &r.path)
                }));
        if !allowed {
            return Ok(Some("outside permitted roots".into()));
        }
    } else {
        if !matches!(item.class, Class::Logs | Class::Evidence) {
            return Ok(Some("compression outside permitted roots".into()));
        }
        let name = item.path.file_name().unwrap_or_default().to_string_lossy();
        if name == "events.jsonl"
            || name.ends_with(".lock")
            || archive::archived_path(&item.path).exists()
        {
            return Ok(Some("protected file or archive exists".into()));
        }
    }
    Ok(None)
}
fn perform(
    home: &Path,
    ledger: &Ledger,
    item: &Planned,
    now_ms: i64,
    cancelled: bool,
) -> (AuditRecord, u64) {
    let mut freed = 0;
    let outcome = if cancelled {
        "cancelled".into()
    } else {
        match check(home, ledger, item) {
            Err(e) => format!("failed: {e}"),
            Ok(Some(why)) => format!("skipped: {why}"),
            Ok(None) => match item.action {
                Action::Delete => {
                    let result = fs::symlink_metadata(&item.path).and_then(|m| {
                        if m.is_dir() {
                            fs::remove_dir_all(&item.path)
                        } else {
                            fs::remove_file(&item.path)
                        }
                    });
                    match result {
                        Err(e) => format!("failed: {e}"),
                        Ok(()) => match ledger.mark_deleted(&item.path, now_ms) {
                            Ok(()) => {
                                freed = item.size_bytes.unwrap_or(0);
                                "done".into()
                            }
                            Err(e) => format!("failed: {e}"),
                        },
                    }
                }
                Action::Compress => match archive::compress(&item.path) {
                    Ok(c) => {
                        freed = c.original_bytes.saturating_sub(c.archived_bytes);
                        "done".into()
                    }
                    Err(e) => format!("failed: {e}"),
                },
            },
        }
    };
    (record(item, outcome), freed)
}
pub fn execute(home: &Path, plan: &Plan, now_ms: i64, cancel: &AtomicBool) -> Res<Outcome> {
    let ledger = Ledger::open(home)?;
    let mut out = Outcome::default();
    for item in &plan.items {
        let (rec, bytes) = perform(home, &ledger, item, now_ms, cancel.load(Ordering::SeqCst));
        audit(home, &rec)?;
        out.freed_bytes = out.freed_bytes.saturating_add(bytes);
        out.records.push(rec);
    }
    Ok(out)
}
pub fn remove_entry(home: &Path, path: &Path, reason: &str, now_ms: i64) -> AuditRecord {
    remove_one(home, path, None, reason, now_ms, false)
}
pub fn remove_scratch(
    home: &Path,
    path: &Path,
    owner: &Owner,
    reason: &str,
    cancelled: bool,
) -> AuditRecord {
    remove_one(home, path, Some(owner), reason, now(), cancelled)
}
fn remove_one(
    home: &Path,
    path: &Path,
    owner: Option<&Owner>,
    reason: &str,
    now_ms: i64,
    cancelled: bool,
) -> AuditRecord {
    let result = (|| -> Res<AuditRecord> {
        let ledger = Ledger::open(home)?;
        let entry = ledger.entries()?.into_iter().find(|e| {
            same(&e.path, path) || (no_links(path) && same(&e.path, &ledger::normalize(path)))
        });
        let Some(e) = entry else {
            return Ok(AuditRecord {
                ts: base::now_iso(),
                path: path.to_string_lossy().into_owned(),
                class: Class::Scratch,
                size_bytes: None,
                action: Action::Delete,
                reason: reason.into(),
                outcome: if cancelled {
                    "cancelled".into()
                } else {
                    "skipped: entry is not present".into()
                },
                project: owner.map(|o| o.project.clone()).unwrap_or_default(),
                wish: owner.and_then(|o| o.wish.clone()),
            });
        };
        let mut item = planned(&e, e.path.clone(), Action::Delete, reason, e.size_bytes);
        if let Some(o) = owner {
            item.owner = o.clone();
            item.class = Class::Scratch;
        }
        Ok(perform(home, &ledger, &item, now_ms, cancelled).0)
    })();
    let mut rec = result.unwrap_or_else(|e| AuditRecord {
        ts: base::now_iso(),
        path: path.to_string_lossy().into_owned(),
        class: Class::Scratch,
        size_bytes: None,
        action: Action::Delete,
        reason: reason.into(),
        outcome: format!("failed: {e}"),
        project: owner.map(|o| o.project.clone()).unwrap_or_default(),
        wish: owner.and_then(|o| o.wish.clone()),
    });
    if let Err(e) = audit(home, &rec) {
        rec.outcome = format!("failed: audit: {e}");
    }
    rec
}
pub fn pressure_cleanup(home: &Path, policy: &Policy, now_ms: i64) -> Res<Outcome> {
    let mut plan = plan(home, policy, Mode::Pressure, now_ms)?;
    plan.items.retain(|i| i.action == Action::Delete);
    execute(home, &plan, now_ms, &AtomicBool::new(false))
}
