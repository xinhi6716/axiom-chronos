use crate::{
    base::{Failure, Res},
    goal,
    graph::conflicts,
    orch::{self, events},
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Serialize, Debug, Clone)]
pub struct ModuleWait {
    pub module: String,
    pub on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
}
#[derive(Serialize, Debug, Clone)]
pub struct Stuck {
    pub module: String,
    pub reason: String,
}
#[derive(Serialize, Debug, Clone)]
pub struct ScheduleReply {
    pub cursor: u64,
    pub start: Vec<String>,
    pub running: Vec<String>,
    pub verified: Vec<String>,
    pub waiting: Vec<ModuleWait>,
    pub stuck: Vec<Stuck>,
    pub held: Option<String>,
    pub done: bool,
    pub timed_out: bool,
    pub events: Vec<events::Event>,
    pub events_truncated: bool,
}
fn map(store: &Path) -> Res<orch::Map> {
    let text = std::fs::read_to_string(store.join("modules.md")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Failure::new("no_map", "modules.md not found")
        } else {
            Failure::new("io", e.to_string())
        }
    })?;
    orch::parse_map(&text).map_err(|e| Failure::new("invalid_argument", e))
}
fn hold(store: &Path) -> Res<Option<String>> {
    if let Some(id) = crate::engine::interjections::paused(store)? {
        return Ok(Some(format!("interjection {id} paused")));
    }
    let project_store = if store
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(orch::is_wish_id)
    {
        store.parent()
    } else {
        Some(store)
    }
    .ok_or_else(|| Failure::new("invalid_argument", "invalid store path"))?;
    let project = project_store
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Failure::new("invalid_argument", "invalid project"))?;
    let home = project_store
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Failure::new("invalid_argument", "invalid store path"))?;
    goal::accept_hold(home, project)
}
/// Each module's latest claim, release or verified event. A release made while the module still
/// waited on an unverified dependency is dropped, so the module is unclaimed again and is
/// auto-claimed once its DEPENDS are verified. A release made while it was ready (its dispatcher
/// stopped without `module_done`) stays `module_released` until the core claims it, so a failing
/// dispatcher is not restarted in a loop.
fn states(store: &Path, map: &orch::Map) -> Res<HashMap<String, String>> {
    let all = events::read_events(store, 0)?;
    let mut result = HashMap::new();
    for (i, e) in all.iter().enumerate() {
        let Some(m) = &e.module else {
            continue;
        };
        if e.kind == events::MODULE_RELEASED && !release_holds(map, &all[..i], m, &e.data) {
            result.remove(m);
        } else if [
            events::MODULE_CLAIMED,
            events::MODULE_RELEASED,
            events::MODULE_VERIFIED,
        ]
        .contains(&e.kind.as_str())
        {
            result.insert(m.clone(), e.kind.clone());
        }
    }
    Ok(result)
}
fn release_holds(map: &orch::Map, before: &[events::Event], module: &str, data: &Value) -> bool {
    if let Some(ready) = data.get("ready").and_then(Value::as_bool) {
        return ready;
    }
    // A release written before `ready` was recorded: a dependency that was verified then had
    // reported module_verified.
    map.modules.iter().find(|m| m.id == module).map_or(true, |m| {
        m.depends.iter().all(|d| {
            before
                .iter()
                .any(|e| e.kind == events::MODULE_VERIFIED && e.module.as_deref() == Some(d))
        })
    })
}
pub fn module_verified(store: &Path, module: &str) -> Res<bool> {
    let units = orch::read_units(store)?;
    let relevant: Vec<_> = units
        .iter()
        .filter(|u| u.track == module && u.superseded_by().is_none())
        .collect();
    if relevant.is_empty() {
        return Ok(false);
    }
    let claims = orch::read_claims(store)?;
    if claims
        .iter()
        .any(|c| c.module == module || units.iter().any(|u| u.track == module && u.id == c.unit))
    {
        return Ok(false);
    }
    for u in relevant {
        if u.brief.is_empty() || orch::unit_brief(store, &u.id).is_err() {
            return Ok(false);
        }
        let rows = orch::evidence(store, &u.id)?;
        if orch::derive_state(&rows, orch::needs_accept(store, &u.id)?) != "verified" {
            return Ok(false);
        }
        // After the last run, a passing review and a passing accept must both have seen the
        // unit's latest snapshot, in either order: re-running ACCEPT on reviewed, unchanged code
        // keeps the module verified, and a review of older files does not count.
        let rest = &rows[rows.iter().rposition(|r| r.kind == "run").map_or(0, |i| i + 1)..];
        let Some(latest) = rest
            .iter()
            .rev()
            .find(|r| r.kind == "verify" || r.kind == "accept")
            .map(|r| r.commit.as_str())
        else {
            return Ok(false);
        };
        let accepted = rest
            .iter()
            .any(|r| r.kind == "accept" && r.exit_code == 0 && r.commit == latest);
        let reviewed = rest.iter().enumerate().any(|(n, r)| {
            r.kind == "review" && r.commit == latest && orch::evidence::review_passes(rest, n)
        });
        if !(accepted && reviewed) {
            return Ok(false);
        }
    }
    Ok(true)
}
pub fn claimed_modules(store: &Path) -> Res<Vec<String>> {
    let map = map(store)?;
    let state = states(store, &map)?;
    let mut out = Vec::new();
    for m in map.modules {
        if state
            .get(&m.id)
            .is_some_and(|s| s == events::MODULE_CLAIMED)
            && !module_verified(store, &m.id)?
        {
            out.push(m.id);
        }
    }
    Ok(out)
}
pub fn claim_ready(store: &Path) -> Res<Vec<String>> {
    orch::with_lock(store, false, || {
        let map = map(store)?;
        if hold(store)?.is_some() {
            return Ok(vec![]);
        }
        let state = states(store, &map)?;
        let mut active = Vec::new();
        for m in &map.modules {
            if state
                .get(&m.id)
                .is_some_and(|s| s == events::MODULE_CLAIMED)
                && !module_verified(store, &m.id)?
            {
                active.push(m.clone());
            }
        }
        let mut out = vec![];
        for m in map.modules {
            if state
                .get(&m.id)
                .is_some_and(|s| s == events::MODULE_CLAIMED || s == events::MODULE_RELEASED)
                || module_verified(store, &m.id)?
            {
                continue;
            }
            let mut ready = true;
            for d in &m.depends {
                if !module_verified(store, d)? {
                    ready = false;
                    break;
                }
            }
            if ready && conflicts::blocking(store, &m, &active)?.is_none() {
                events::append_event(
                    store,
                    events::MODULE_CLAIMED,
                    Some(&m.id),
                    None,
                    json!({"depends":m.depends}),
                )?;
                out.push(m.id.clone());
                active.push(m);
            }
        }
        Ok(out)
    })
}
pub fn claim(store: &Path, module: &str) -> Res<()> {
    orch::with_lock(store, false, || {
        let map = map(store)?;
        let m = map
            .modules
            .iter()
            .find(|m| m.id == module)
            .ok_or_else(|| Failure::new("not_found", format!("module {module} not found")))?;
        if let Some(reason) = hold(store)? {
            return Err(Failure::new("held", reason));
        }
        if module_verified(store, module)? {
            return Err(Failure::new(
                "already_verified",
                format!("{module} is verified"),
            ));
        }
        if states(store, &map)?
            .get(module)
            .is_some_and(|s| s == events::MODULE_CLAIMED)
        {
            return Err(Failure::new(
                "already_claimed",
                format!("{module} is claimed"),
            ));
        }
        let mut missing = Vec::new();
        for d in &m.depends {
            if !module_verified(store, d)? {
                missing.push(d.clone());
            }
        }
        if !missing.is_empty() {
            return Err(Failure::new(
                "not_ready",
                format!("unverified dependencies: {}", missing.join(", ")),
            ));
        }
        let claimed = claimed_modules(store)?;
        let active: Vec<_> = map
            .modules
            .iter()
            .filter(|m| claimed.contains(&m.id))
            .cloned()
            .collect();
        if let Some(conflict) = conflicts::blocking(store, m, &active)? {
            return Err(Failure::new(
                "conflict",
                format!("{}: {}", conflict.b, conflict.reason),
            ));
        }
        events::append_event(
            store,
            events::MODULE_CLAIMED,
            Some(module),
            None,
            json!({"depends":m.depends}),
        )?;
        Ok(())
    })
}
pub fn release(store: &Path, module: &str, reason: &str) -> Res<()> {
    orch::with_lock(store, false, || {
        if !claimed_modules(store)?.iter().any(|m| m == module) {
            return Err(Failure::new(
                "not_claimed",
                format!("{module} is not claimed"),
            ));
        }
        let mut ready = true;
        for m in map(store)?.modules.iter().filter(|m| m.id == module) {
            for d in &m.depends {
                ready &= module_verified(store, d)?;
            }
        }
        events::append_event(
            store,
            events::MODULE_RELEASED,
            Some(module),
            None,
            json!({"reason":reason,"ready":ready}),
        )?;
        events::close_open_activities(store, Some(module), "cancelled")?;
        Ok(())
    })
}
pub fn next(store: &Path, cursor: Option<u64>, timeout: Duration) -> Res<ScheduleReply> {
    let after = cursor.unwrap_or(0);
    let start_time = Instant::now();
    let mut observed = after;
    loop {
        claim_ready(store)?;
        let all = events::read_events(store, after)?;
        observed = observed.max(all.last().map_or(after, |e| e.seq));
        let m = map(store)?;
        let state = states(store, &m)?;
        let held = hold(store)?;
        let mut running = vec![];
        let mut verified = vec![];
        let mut waiting = vec![];
        let mut stuck = vec![];
        let mut active = Vec::new();
        for module in &m.modules {
            if state
                .get(&module.id)
                .is_some_and(|s| s == events::MODULE_CLAIMED)
                && !module_verified(store, &module.id)?
            {
                active.push(module.clone());
            }
        }
        for module in &m.modules {
            if module_verified(store, &module.id)? {
                verified.push(module.id.clone());
                continue;
            }
            if state
                .get(&module.id)
                .is_some_and(|s| s == events::MODULE_CLAIMED)
            {
                running.push(module.id.clone());
                continue;
            }
            let mut on = vec![];
            for d in &module.depends {
                if !module_verified(store, d)? {
                    on.push(d.clone());
                }
            }
            if !on.is_empty() {
                waiting.push(ModuleWait {
                    module: module.id.clone(),
                    on,
                    conflict: None,
                });
            } else if state
                .get(&module.id)
                .is_some_and(|s| s == events::MODULE_RELEASED)
            {
                orch::with_lock(store, false, || {
                    // A concurrent core claim must not be overwritten by this snapshot.
                    if states(store, &m)?.get(&module.id).is_some_and(|s| s == events::MODULE_RELEASED) {
                        orch::store::add_adjudication_locked(store, "stuck", &module.id)?;
                    }
                    Ok(())
                })?;
                stuck.push(Stuck {
                    module: module.id.clone(),
                    reason: "released".into(),
                });
            } else if let Some(conflict) = conflicts::blocking(store, module, &active)? {
                waiting.push(ModuleWait {
                    module: module.id.clone(),
                    on: vec![conflict.b],
                    conflict: Some(conflict.reason),
                });
            }
        }
        let done = verified.len() == m.modules.len();
        let signal = all.iter().any(|e| {
            [
                events::MODULE_CLAIMED,
                events::MODULE_RELEASED,
                events::MODULE_VERIFIED,
            ]
            .contains(&e.kind.as_str())
        });
        let timed_out = !signal && held.is_none() && !done && start_time.elapsed() >= timeout;
        if signal || held.is_some() || done || timed_out {
            let start = running
                .iter()
                .filter(|m| {
                    held.is_none()
                        && all.iter().any(|e| {
                            e.kind == events::MODULE_CLAIMED && e.module.as_ref() == Some(*m)
                        })
                })
                .cloned()
                .collect();
            let mut visible: Vec<_> = all
                .into_iter()
                .filter(|e| {
                    ![
                        events::HEARTBEAT,
                        events::ACTIVITY_START,
                        events::ACTIVITY_FINISH,
                    ]
                    .contains(&e.kind.as_str())
                })
                .collect();
            let events_truncated = visible.len() > 20;
            visible.truncate(20);
            return Ok(ScheduleReply {
                cursor: if events_truncated {
                    visible.last().unwrap().seq
                } else {
                    observed
                },
                start,
                running,
                verified,
                waiting,
                stuck,
                held,
                done,
                timed_out,
                events: visible,
                events_truncated,
            });
        }
        let _ = events::wait_events(
            store,
            observed,
            timeout
                .saturating_sub(start_time.elapsed())
                .min(Duration::from_millis(100)),
        )?;
    }
}
