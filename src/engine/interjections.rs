use crate::{
    base::{Failure, Res},
    goal::{self, Goal},
    orch::{
        self,
        events::{self, Event},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Links {
    pub plan_revision: Option<String>,
    pub units: Vec<String>,
    pub scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub id: String,
    pub states: Vec<(String, u64)>,
}

fn invalid(message: &str) -> Failure {
    Failure::new("invalid_argument", message)
}

// Serialize wrappers across processes, including the goal update and event writes.
fn lock(home: &Path) -> Res<orch::StoreLock> {
    let dir = home.join("goals").join("interjections");
    std::fs::create_dir_all(&dir).map_err(|e| Failure::new("io", e.to_string()))?;
    orch::lock_store(&dir)
}

fn store(home: &Path, session: &str) -> Res<PathBuf> {
    let goal = goal::load(home, session)?
        .ok_or_else(|| Failure::new("not_found", format!("no goal for session {session}")))?;
    let store = orch::store_for_goal(home, &goal)?;
    std::fs::create_dir_all(&store).map_err(|e| Failure::new("io", e.to_string()))?;
    Ok(store)
}

pub fn submit(home: &Path, session: &str, prompt: &str) -> Res<Option<(String, Goal)>> {
    let _lock = lock(home)?;
    if goal::load(home, session)?.is_none() || prompt.trim_start().starts_with('<') {
        return Ok(None);
    }
    let store = store(home, session)?;
    let _store_lock = orch::lock_store(&store)?;
    let Some(goal) = goal::interject(home, session, prompt)? else {
        return Ok(None);
    };
    let id = format!("ij-{}", goal.interjection_counter);
    events::append_event(&store, "interjection_sent", None, None, json!({"id": id}))?;
    events::append_event(
        &store,
        "interjection_received",
        None,
        None,
        json!({
            "id": id, "chars": prompt.chars().count(),
            "digest": format!("{:x}", Sha256::digest(prompt.as_bytes()))
        }),
    )?;
    Ok(Some((id, goal)))
}

pub fn classify(home: &Path, session: &str, kind: &str) -> Res<Goal> {
    let _lock = lock(home)?;
    let store = store(home, session)?;
    let _store_lock = orch::lock_store(&store)?;
    let pending = goal::load(home, session)?.and_then(|g| g.interjection);
    let result = goal::classify(home, session, kind)?;
    if let Some(id) = pending.and_then(|i| i.id) {
        events::append_event(
            &store,
            "interjection_classified",
            None,
            None,
            json!({"id": id, "kind": kind}),
        )?;
        if kind == "answer" || kind == "stop" {
            events::append_event(
                &store,
                "interjection_applied",
                None,
                None,
                json!({"id": id, "links": {"status": result.status}}),
            )?;
        }
    }
    Ok(result)
}

fn classified_kind(events: &[Event], id: &str) -> Res<String> {
    let mut kind = None;
    for event in events.iter().filter(|e| e.data["id"].as_str() == Some(id)) {
        match event.kind.as_str() {
            "interjection_classified" => kind = event.data["kind"].as_str().map(str::to_owned),
            "interjection_applied" | "interjection_declined" => {
                return Err(invalid("interjection already resolved"))
            }
            _ => (),
        }
    }
    kind.ok_or_else(|| invalid("unknown or unclassified interjection"))
}

pub fn apply(home: &Path, session: &str, id: &str, links: Links) -> Res<Event> {
    let _lock = lock(home)?;
    let store = store(home, session)?;
    let _store_lock = orch::lock_store(&store)?;
    let kind = classified_kind(&events::read_events(&store, 0)?, id)?;
    if kind == "replan"
        && links
            .plan_revision
            .as_deref()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(invalid("replan requires a plan revision"));
    }
    events::append_event(
        &store,
        "interjection_applied",
        None,
        None,
        json!({"id": id, "links": links}),
    )
}

pub fn decline(home: &Path, session: &str, id: &str, reason: &str) -> Res<Event> {
    if reason.trim().is_empty() {
        return Err(invalid("decline requires a reason"));
    }
    let _lock = lock(home)?;
    let store = store(home, session)?;
    let _store_lock = orch::lock_store(&store)?;
    classified_kind(&events::read_events(&store, 0)?, id)?;
    events::append_event(
        &store,
        "interjection_declined",
        None,
        None,
        json!({"id": id, "reason": reason}),
    )
}

pub fn receipts(store: &Path) -> Res<Vec<Receipt>> {
    let mut receipts: Vec<Receipt> = Vec::new();
    for event in events::read_events(store, 0)? {
        if ![
            "interjection_sent",
            "interjection_received",
            "interjection_classified",
            "interjection_applied",
            "interjection_declined",
        ]
        .contains(&event.kind.as_str())
        {
            continue;
        }
        let Some(id) = event.data["id"].as_str() else {
            continue;
        };
        let index = match receipts.iter().position(|r| r.id == id) {
            Some(index) => index,
            None => {
                receipts.push(Receipt {
                    id: id.to_owned(),
                    states: Vec::new(),
                });
                receipts.len() - 1
            }
        };
        receipts[index].states.push((event.kind, event.seq));
    }
    Ok(receipts)
}

pub fn paused(store: &Path) -> Res<Option<String>> {
    let mut paused = Vec::new();
    for event in events::read_events(store, 0)? {
        let Some(id) = event.data["id"].as_str() else {
            continue;
        };
        match event.kind.as_str() {
            "interjection_classified" if event.data["kind"] == "pause_batch" => {
                paused.push(id.to_owned())
            }
            "interjection_applied" | "interjection_declined" => paused.retain(|p| p != id),
            _ => (),
        }
    }
    Ok(paused.into_iter().next())
}
