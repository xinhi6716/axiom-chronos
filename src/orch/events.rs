use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
    time::{Duration, Instant},
};

pub const MODULE_CLAIMED: &str = "module_claimed";
pub const MODULE_RELEASED: &str = "module_released";
pub const MODULE_VERIFIED: &str = "module_verified";
pub const UNIT_STATE: &str = "unit_state";
pub const ACTIVITY_START: &str = "activity_start";
pub const ACTIVITY_FINISH: &str = "activity_finish";
pub const HEARTBEAT: &str = "heartbeat";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub ts: String,
    pub ts_ms: i64,
    pub kind: String,
    pub wish: String,
    pub module: Option<String>,
    pub unit: Option<String>,
    #[serde(default)]
    pub data: Value,
}

fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn lock(store: &Path) -> Res<File> {
    fs::create_dir_all(store).map_err(io)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(store.join("events.lock"))
        .map_err(io)?;
    f.lock().map_err(io)?;
    Ok(f)
}

// Return the length of the valid prefix too, so an interrupted append can be repaired.
fn parse(bytes: &[u8]) -> Res<(Vec<Event>, usize)> {
    let mut events = Vec::new();
    let mut offset = 0;
    for part in bytes.split_inclusive(|b| *b == b'\n') {
        let end = offset + part.len();
        if !part.ends_with(b"\n") {
            break;
        }
        match serde_json::from_slice::<Event>(&part[..part.len() - 1]) {
            Ok(event) => events.push(event),
            Err(_) if end == bytes.len() => break,
            Err(e) => return Err(invalid(format!("invalid event: {e}"))),
        }
        offset = end;
    }
    Ok((events, offset))
}
fn load(store: &Path) -> Res<(Vec<Event>, usize)> {
    match fs::read(store.join("events.jsonl")) {
        Ok(bytes) => parse(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((vec![], 0)),
        Err(e) => Err(io(e)),
    }
}
fn append_locked(
    store: &Path,
    events: &[Event],
    valid_len: usize,
    kind: &str,
    module: Option<&str>,
    unit: Option<&str>,
    data: Value,
) -> Res<Event> {
    let seq = events.last().map_or(Ok(1), |e| {
        e.seq
            .checked_add(1)
            .ok_or_else(|| invalid("event sequence overflow"))
    })?;
    let ts = base::now_iso();
    let ts_ms = super::receipts::parse_timestamp(&ts)
        .ok_or_else(|| invalid("invalid current timestamp"))?;
    let event = Event {
        seq,
        ts,
        ts_ms,
        kind: kind.into(),
        wish: super::wish_of_store(store),
        module: module.map(str::to_owned),
        unit: unit.map(str::to_owned),
        data,
    };
    let mut line = serde_json::to_vec(&event).map_err(|e| invalid(e.to_string()))?;
    line.push(b'\n');
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(store.join("events.jsonl"))
        .map_err(io)?;
    if f.metadata().map_err(io)?.len() != valid_len as u64 {
        f.set_len(valid_len as u64).map_err(io)?;
    }
    f.seek(SeekFrom::End(0)).map_err(io)?;
    if f.write(&line).map_err(io)? != line.len() {
        return Err(io(std::io::Error::new(
            std::io::ErrorKind::WriteZero,
            "incomplete event write",
        )));
    }
    Ok(event)
}
pub fn append_event(
    store: &Path,
    kind: &str,
    module: Option<&str>,
    unit: Option<&str>,
    data: Value,
) -> Res<Event> {
    let _lock = lock(store)?;
    let (events, len) = load(store)?;
    append_locked(store, &events, len, kind, module, unit, data)
}
pub fn append_if(
    store: &Path,
    kind: &str,
    module: Option<&str>,
    unit: Option<&str>,
    data: Value,
    pred: impl FnOnce(&[Event]) -> Res<()>,
) -> Res<Event> {
    let _lock = lock(store)?;
    let (events, len) = load(store)?;
    pred(&events)?;
    append_locked(store, &events, len, kind, module, unit, data)
}

#[cfg(test)]
mod compare_append_tests {
    use super::*;
    #[test]
    fn predicate_checks_history_before_append() {
        let dir = tempfile::tempdir().unwrap();
        append_event(dir.path(), "first", None, None, json!({})).unwrap();
        let result = append_if(dir.path(), "second", None, None, json!({}), |events| {
            assert_eq!(events.len(), 1);
            Err(Failure::new("conflict", "already present"))
        });
        assert_eq!(result.unwrap_err().kind, "conflict");
        assert_eq!(read_events(dir.path(), 0).unwrap().len(), 1);
        let event = append_if(dir.path(), "second", None, None, json!({}), |events| {
            assert_eq!(events[0].kind, "first");
            Ok(())
        })
        .unwrap();
        assert_eq!(event.seq, 2);
    }
    #[test]
    fn concurrent_predicates_allow_only_one_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    append_if(&path, "once", None, None, json!({}), |events| {
                        if events.iter().any(|event| event.kind == "once") {
                            Err(Failure::new("conflict", "already appended"))
                        } else {
                            Ok(())
                        }
                    })
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .filter(|r| r.is_ok())
                .count(),
            1
        );
        assert_eq!(read_events(dir.path(), 0).unwrap().len(), 1);
    }
}
pub fn read_events(store: &Path, after: u64) -> Res<Vec<Event>> {
    Ok(load(store)?
        .0
        .into_iter()
        .filter(|e| e.seq > after)
        .collect())
}
pub fn last_seq(store: &Path) -> Res<u64> {
    Ok(load(store)?.0.last().map_or(0, |e| e.seq))
}
pub fn wait_events(store: &Path, after: u64, timeout: Duration) -> Res<Vec<Event>> {
    let start = Instant::now();
    loop {
        let rows = read_events(store, after)?;
        if !rows.is_empty() || start.elapsed() >= timeout {
            return Ok(rows);
        }
        std::thread::sleep(
            timeout
                .saturating_sub(start.elapsed())
                .min(Duration::from_millis(100)),
        );
    }
}
#[derive(Serialize, Clone, Debug)]
pub struct Activity {
    pub id: String,
    pub module: String,
    pub actor: String,
    pub phase: String,
    pub started_at: String,
}
fn activities(events: &[Event]) -> Vec<Activity> {
    let mut open: Vec<Activity> = Vec::new();
    for e in events {
        let Some(id) = e.data.get("activity").and_then(Value::as_str) else {
            continue;
        };
        if e.kind == ACTIVITY_START {
            if let (Some(module), Some(actor), Some(phase)) = (
                e.module.as_ref(),
                e.data.get("actor").and_then(Value::as_str),
                e.data.get("phase").and_then(Value::as_str),
            ) {
                open.push(Activity {
                    id: id.into(),
                    module: module.clone(),
                    actor: actor.into(),
                    phase: phase.into(),
                    started_at: e.ts.clone(),
                });
            }
        } else if e.kind == ACTIVITY_FINISH {
            open.retain(|a| a.id != id);
        }
    }
    open
}
fn validate(actor: &str, phase: &str) -> Res<()> {
    if !["core", "dispatcher"].contains(&actor) || !["brief", "review", "decide"].contains(&phase) {
        return Err(invalid("invalid actor or phase"));
    }
    Ok(())
}
fn validate_outcome(outcome: &str) -> Res<()> {
    if !["finished", "cancelled", "failed", "exit"].contains(&outcome) {
        return Err(invalid("invalid outcome"));
    }
    Ok(())
}
pub fn activity_start(store: &Path, module: &str, actor: &str, phase: &str) -> Res<Event> {
    validate(actor, phase)?;
    let _lock = lock(store)?;
    let (events, len) = load(store)?;
    let seq = events.last().map_or(Ok(1), |e| {
        e.seq
            .checked_add(1)
            .ok_or_else(|| invalid("event sequence overflow"))
    })?;
    append_locked(
        store,
        &events,
        len,
        ACTIVITY_START,
        Some(module),
        None,
        json!({"activity": format!("a{seq}"), "actor": actor, "phase": phase}),
    )
}
pub fn activity_finish(
    store: &Path,
    module: &str,
    actor: &str,
    phase: &str,
    id: Option<&str>,
    outcome: &str,
) -> Res<Option<Event>> {
    validate(actor, phase)?;
    validate_outcome(outcome)?;
    let _lock = lock(store)?;
    let (events, len) = load(store)?;
    let open = activities(&events);
    let selected = open.iter().rev().find(|a| {
        a.module == module && a.actor == actor && a.phase == phase && id.is_none_or(|id| a.id == id)
    });
    selected
        .map(|a| {
            append_locked(
                store,
                &events,
                len,
                ACTIVITY_FINISH,
                Some(module),
                None,
                json!({"activity": a.id, "actor": actor, "phase": phase, "outcome": outcome}),
            )
        })
        .transpose()
}
pub fn open_activities(store: &Path) -> Res<Vec<Activity>> {
    Ok(activities(&load(store)?.0))
}
pub fn close_open_activities(
    store: &Path,
    module: Option<&str>,
    outcome: &str,
) -> Res<Vec<String>> {
    validate_outcome(outcome)?;
    let _lock = lock(store)?;
    let (mut events, mut len) = load(store)?;
    let open = activities(&events);
    let mut ids = Vec::new();
    for a in open
        .into_iter()
        .filter(|a| module.is_none_or(|m| a.module == m))
    {
        let event = append_locked(
            store,
            &events,
            len,
            ACTIVITY_FINISH,
            Some(&a.module),
            None,
            json!({"activity": a.id, "actor": a.actor, "phase": a.phase, "outcome": outcome}),
        )?;
        len = fs::metadata(store.join("events.jsonl")).map_err(io)?.len() as usize;
        ids.push(a.id);
        events.push(event);
    }
    Ok(ids)
}
fn latest(events: &[Event]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|e| e.kind != HEARTBEAT)
        .map(|e| e.ts.clone())
}
pub fn latest_activity_at(store: &Path) -> Res<Option<String>> {
    Ok(latest(&load(store)?.0))
}
pub fn heartbeat(store: &Path) -> Res<Event> {
    let _lock = lock(store)?;
    let (events, len) = load(store)?;
    append_locked(
        store,
        &events,
        len,
        HEARTBEAT,
        None,
        None,
        json!({"emitted_at": base::now_iso(), "latest_activity_at": latest(&events)}),
    )
}
