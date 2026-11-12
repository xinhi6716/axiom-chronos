use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

mod batch;
pub(crate) mod progress;
pub use batch::{sleep_unless_stopped, wait_for_batch, Batch, BatchEnd, Waited, BATCH_POLL};

fn is_false(v: &bool) -> bool {
    !v
}
fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub wish: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wish_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_limit: Option<u32>,
    #[serde(default)]
    pub cwd: Option<String>,
    pub status: String,
    #[serde(default)]
    pub amendments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<Batch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waited: Option<Waited>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub mode_locked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub asked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dispatched: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interjection: Option<Interjection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queued: Vec<Interjection>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub interjection_counter: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<Value>,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interjection {
    #[serde(default)]
    pub id: Option<String>,
    pub prompt: String,
    pub prior_status: String,
    pub prior_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amendment_index: Option<usize>,
}

fn path(home: &Path, session: &str) -> Res<PathBuf> {
    if session.is_empty()
        || session.len() > 64
        || !session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(Failure::new(
            "invalid_argument",
            format!("not a session id: {session:?}"),
        ));
    }
    Ok(home.join("goals").join(format!("{session}.json")))
}
fn io_error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
pub fn load(home: &Path, session: &str) -> Res<Option<Goal>> {
    let file = path(home, session)?;
    match std::fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Failure::new("invalid_json", e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(e)),
    }
}
fn save(home: &Path, session: &str, goal: &Goal) -> Res<()> {
    let file = path(home, session)?;
    std::fs::create_dir_all(file.parent().unwrap()).map_err(io_error)?;
    let mut bytes = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    goal.serialize(&mut serde_json::Serializer::with_formatter(
        &mut bytes, formatter,
    ))
    .map_err(io_error)?;
    base::atomic_write(&file, &bytes).map_err(io_error)
}
/// Every read-modify-write of a goal runs under this: hooks are separate processes.
fn lock(home: &Path) -> Res<crate::orch::StoreLock> {
    let dir = home.join("goals");
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    crate::orch::lock_store(&dir)
}
fn required(home: &Path, session: &str) -> Res<Goal> {
    load(home, session)?
        .ok_or_else(|| Failure::new("not_found", format!("no goal for session {session}")))
}
pub fn open(home: &Path, session: &str, wish: &str, cwd: Option<&str>) -> Res<Goal> {
    let _lock = lock(home)?;
    let goal = Goal {
        wish: wish.into(),
        wish_id: Some(mint_wish_id(home)),
        turn_limit: None,
        cwd: cwd.map(str::to_owned),
        status: "active".into(),
        amendments: vec![],
        note: None,
        batch: None,
        waited: None,
        project: cwd.map(|c| base::slug_for_dir(Path::new(c))),
        mode: None,
        mode_locked: false,
        asked: false,
        dispatched: false,
        reason: None,
        interjection: None,
        queued: vec![],
        interjection_counter: 0,
        provenance: vec![],
        other: Map::new(),
    };
    save(home, session, &goal)?;
    progress::clear(home, session)?;
    Ok(goal)
}
fn mint_wish_id(home: &Path) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let prefix = format!("w-{:013}-", now.as_millis());
        let mut suffix = ((now.as_nanos() as u64
            ^ u64::from(std::process::id())
            ^ COUNTER.fetch_add(1, Ordering::Relaxed))
            & 0xffffff) as u32;
        if let Ok(entries) = std::fs::read_dir(home.join("goals")) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let Some(goal) = std::fs::read(entry.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Goal>(&b).ok())
                else {
                    continue;
                };
                if let Some(id) = goal
                    .wish_id
                    .as_deref()
                    .and_then(|id| id.strip_prefix(&prefix))
                {
                    if id.len() == 6 {
                        if let Ok(previous) = u32::from_str_radix(id, 16) {
                            suffix = suffix.max(previous + 1);
                        }
                    }
                }
            }
        }
        if suffix <= 0xffffff {
            return format!("{prefix}{suffix:06x}");
        }
        std::thread::yield_now();
    }
}
fn leave_waiting(goal: &mut Goal, end: Option<BatchEnd>) {
    if goal.status == "waiting" {
        goal.waited = Some(Waited {
            note: goal.note.clone().unwrap_or_default(),
            batch: goal.batch.clone(),
            end,
        });
    }
}
pub fn interrupt(home: &Path, session: &str, cause: &str) -> Res<Option<Goal>> {
    let _lock = lock(home)?;
    let Some(mut goal) = load(home, session)? else {
        return Ok(None);
    };
    if goal.status != "active" {
        return Ok(None);
    }
    goal.status = "blocked".into();
    goal.reason = Some("interrupted".into());
    goal.note = Some(format!(
        "{cause}. Resume with: axiom wish --resume {session}"
    ));
    goal.interjection = None;
    save(home, session, &goal)?;
    Ok(Some(goal))
}
pub fn cut_off(home: &Path, session: &str, turn_limit: u32) -> Res<Option<Goal>> {
    let _lock = lock(home)?;
    let Some(mut goal) = load(home, session)? else {
        return Ok(None);
    };
    if goal.status != "active" {
        return Ok(None);
    }
    goal.status = "blocked".into();
    goal.reason = Some("max_turns".into());
    goal.turn_limit = Some(turn_limit);
    goal.note = Some(format!(
        "cut off at {turn_limit} turns. Resume with: axiom wish --resume {session}"
    ));
    goal.interjection = None;
    save(home, session, &goal)?;
    Ok(Some(goal))
}
pub fn reopen(home: &Path, session: &str) -> Res<Goal> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.status == "done" {
        return Err(Failure::new(
            "invalid_argument",
            format!("goal {session} is already done"),
        ));
    }
    leave_waiting(&mut goal, None);
    goal.status = "active".into();
    goal.note = None;
    goal.batch = None;
    goal.reason = None;
    goal.turn_limit = None;
    goal.interjection = None;
    save(home, session, &goal)?;
    progress::clear(home, session)?;
    Ok(goal)
}
pub fn interject(home: &Path, session: &str, prompt: &str) -> Res<Option<Goal>> {
    if load(home, session)?.is_none() || prompt.trim_start().starts_with('<') {
        return Ok(None);
    }
    let _lock = lock(home)?;
    let Some(mut goal) = load(home, session)? else {
        return Ok(None);
    };
    let (prior_status, prior_note) = match &goal.interjection {
        Some(pending) => {
            let oldest = goal.queued.first().unwrap_or(pending);
            (oldest.prior_status.clone(), oldest.prior_note.clone())
        }
        None => (goal.status.clone(), goal.note.clone()),
    };
    leave_waiting(&mut goal, None);
    goal.interjection_counter = goal
        .interjection_counter
        .checked_add(1)
        .ok_or_else(|| Failure::new("invalid_argument", "interjection counter overflow"))?;
    let pending = Interjection {
        id: Some(format!("ij-{}", goal.interjection_counter)),
        prompt: prompt.into(),
        prior_status,
        prior_note,
        amendment_index: Some(goal.amendments.len()),
    };
    if let Some(previous) = goal.interjection.replace(pending) {
        goal.queued.push(previous);
    }
    goal.amendments.push(prompt.into());
    goal.status = "active".into();
    goal.mode_locked = false;
    goal.reason = None;
    save(home, session, &goal)?;
    Ok(Some(goal))
}
pub fn classify(home: &Path, session: &str, kind: &str) -> Res<Goal> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    let pending = goal
        .interjection
        .take()
        .ok_or_else(|| Failure::new("invalid_argument", "no pending interjection"))?;
    match kind {
        "amend" | "replan" | "pause_batch" => (),
        "answer" => {
            if pending.prior_status == "waiting" {
                goal.waited = None;
            }
            goal.status = pending.prior_status;
            goal.note = pending.prior_note;
            let index = pending
                .amendment_index
                .filter(|&i| goal.amendments.get(i) == Some(&pending.prompt))
                .or_else(|| goal.amendments.iter().rposition(|a| a == &pending.prompt));
            if let Some(i) = index {
                goal.amendments.remove(i);
                for queued in &mut goal.queued {
                    if let Some(index) = &mut queued.amendment_index {
                        if *index > i {
                            *index -= 1;
                        }
                    }
                }
            }
        }
        "stop" => {
            goal.status = "blocked".into();
            goal.note = Some(pending.prompt);
            goal.reason = Some("user_stop".into());
        }
        _ => {
            return Err(Failure::new(
                "invalid_argument",
                format!("invalid classification: {kind}"),
            ))
        }
    }
    if goal.status != "waiting" {
        goal.batch = None;
    }
    if let Some(mut next) = goal.queued.pop() {
        next.prior_status = goal.status.clone();
        next.prior_note = goal.note.clone();
        goal.interjection = Some(next);
    }
    save(home, session, &goal)?;
    Ok(goal)
}
pub fn set_mode(home: &Path, session: &str, mode: &str) -> Res<Goal> {
    if !["review", "answer", "change"].contains(&mode) {
        return Err(Failure::new(
            "invalid_argument",
            format!("invalid mode: {mode}"),
        ));
    }
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.mode_locked {
        return Err(Failure::new("mode_locked", "goal mode is locked"));
    }
    goal.mode = Some(mode.into());
    goal.mode_locked = true;
    save(home, session, &goal)?;
    Ok(goal)
}
/// Spends the goal's one round of questions; false when it is gone or work was dispatched.
pub fn mark_asked(home: &Path, session: &str) -> Res<bool> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.asked || goal.dispatched {
        return Ok(false);
    }
    goal.asked = true;
    save(home, session, &goal)?;
    Ok(true)
}
pub fn mark_dispatched(home: &Path, session: &str) -> Res<()> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    goal.dispatched = true;
    save(home, session, &goal)
}
/// A resumed run that matches the last record adds nothing; one that differs is a new entry.
pub fn add_provenance(home: &Path, session: &str, block: Value) -> Res<()> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.provenance.last() == Some(&block) {
        return Ok(());
    }
    goal.provenance.push(block);
    save(home, session, &goal)
}
pub fn set_status(home: &Path, session: &str, status: &str, note: Option<&str>) -> Res<Value> {
    set_status_with_batch(home, session, status, note, None)
}
pub fn set_status_with_batch(
    home: &Path,
    session: &str,
    status: &str,
    note: Option<&str>,
    batch: Option<Batch>,
) -> Res<Value> {
    if !["done", "blocked", "waiting"].contains(&status) {
        return Err(Failure::new(
            "invalid_argument",
            "status must be done, blocked or waiting",
        ));
    }
    if status == "waiting" && note.unwrap_or("").trim().is_empty() {
        return Err(Failure::new(
            "invalid_argument",
            "a waiting goal needs a note naming the batch, its progress file and its completion marker",
        ));
    }
    if status == "blocked" && note.unwrap_or("").trim().is_empty() {
        return Err(Failure::new(
            "invalid_argument",
            "a blocked goal needs a note saying what only the user can provide",
        ));
    }
    if let Some(batch) = &batch {
        if status != "waiting" {
            return Err(Failure::new(
                "invalid_argument",
                "progress_file, marker_file and pid belong to status waiting",
            ));
        }
        batch.validate()?;
    }
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.status == "done" && status != "done" {
        return Err(Failure::new(
            "invalid_argument",
            format!("goal {session} is already done"),
        ));
    }
    if status == "done" {
        if goal.project.is_some() {
            crate::orch::done_gate(
                &crate::orch::store_for_goal(home, &goal)?,
                goal.mode.as_deref() == Some("change"),
            )?;
        }
    }
    let mut note = note.unwrap_or("").to_string();
    if status == "blocked" {
        if goal.project.is_some() {
            let store = crate::orch::store_for_goal(home, &goal)?;
            let pending = crate::orch::pending_handoffs(&store)?;
            if let Some(h) = pending
                .iter()
                .find(|h| !h.offered && crate::orch::stalled_out(&store, h).is_none())
            {
                return Err(Failure::new(
                    "handoff_pending",
                    crate::orch::handoff_blocker(&store, h),
                ));
            }
            for h in pending {
                note.push_str(&format!(
                    "\nPending handoff: {} (next: {})",
                    crate::orch::handoff_path(&store, h.module.as_deref()).display(),
                    h.next
                ));
            }
        }
        leave_waiting(&mut goal, None);
    } else {
        goal.waited = None;
    }
    goal.status = status.into();
    goal.note = Some(note);
    goal.batch = batch;
    goal.interjection = None;
    save(home, session, &goal)?;
    Ok(serde_json::json!({"status": "completed", "goal": goal}))
}
/// Records how the batch this goal waits on ended and makes the goal active for the resumed session.
/// False when the goal no longer waits on that batch, so the caller must not resume it.
pub fn batch_ended(home: &Path, session: &str, batch: &Batch, end: BatchEnd) -> Res<bool> {
    let _lock = lock(home)?;
    let mut goal = required(home, session)?;
    if goal.status != "waiting" || goal.batch.as_ref() != Some(batch) {
        return Ok(false);
    }
    leave_waiting(&mut goal, Some(end));
    goal.status = "active".into();
    goal.note = None;
    goal.batch = None;
    save(home, session, &goal)?;
    Ok(true)
}
pub fn find_active(home: &Path, project: &str) -> Res<Option<(String, Goal)>> {
    let entries = match std::fs::read_dir(home.join("goals")) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    };
    let mut newest = None;
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if self::path(home, session).is_err() {
            continue;
        }
        let Some(goal) = load(home, session)? else {
            continue;
        };
        if goal.status != "active" || goal.project.as_deref() != Some(project) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .map_err(io_error)?;
        if newest
            .as_ref()
            .map_or(true, |(time, _, _)| modified > *time)
        {
            newest = Some((modified, session.to_owned(), goal));
        }
    }
    Ok(newest.map(|(_, session, goal)| (session, goal)))
}
pub fn accept_hold(home: &Path, project: &str) -> Res<Option<String>> {
    let entries = match std::fs::read_dir(home.join("goals")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    };
    let mut newest = None;
    for entry in entries {
        let entry = entry.map_err(io_error)?;
        let file = entry.path();
        if file.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = file.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path(home, session).is_err() {
            continue;
        }
        let Some(goal) = load(home, session)? else {
            continue;
        };
        if goal.project.as_deref() != Some(project) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .map_err(io_error)?;
        if newest.as_ref().map_or(true, |(time, _)| modified > *time) {
            newest = Some((modified, goal));
        }
    }
    Ok(newest.and_then(|(_, goal)| {
        if goal.interjection.is_some() {
            Some("amend_pending".into())
        } else if goal.status == "blocked" && goal.reason.as_deref() == Some("user_stop") {
            Some("stopped".into())
        } else if goal.status != "active" {
            Some(format!("goal_{}", goal.status))
        } else {
            None
        }
    }))
}
fn marker(goal: &Goal, wanted: &str) -> bool {
    let mut rest = goal.wish.trim_start();
    while rest.starts_with('[') {
        let Some(end) = rest.find(']') else { break };
        if &rest[..=end] == wanted {
            return true;
        }
        rest = rest[end + 1..].trim_start();
    }
    false
}
pub fn creative_requested(goal: &Goal) -> bool {
    marker(goal, "[creative]")
}
