use crate::base::{Failure, Res};
use crate::engine::kinds::*;
use crate::memory::{Memory, Record};
use crate::orch::events::{append_event, append_if, read_events, Event};
use crate::orch::store::wish_of_store;
use crate::orch::{store_for, unit_brief};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Action {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
}

pub fn digest(wish: &str, action: &Action) -> String {
    let canonical =
        json!({"wish": wish, "command": action.command, "args": action.args, "cwd": action.cwd});
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical).expect("JSON value serializes"))
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    Person,
    Model,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Requested {
    pub id: String,
    pub digest: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Executed {
    pub exit_code: i32,
    pub output_tail: String,
}

fn failure(kind: &str, message: &str) -> Failure {
    Failure::new(kind, message)
}
fn has_id(event: &Event, id: &str) -> bool {
    event.data["id"].as_str() == Some(id)
}
fn requested<'a>(events: &'a [Event], id: &str) -> Res<&'a Event> {
    events
        .iter()
        .find(|e| e.kind == APPROVAL_REQUESTED && has_id(e, id))
        .ok_or_else(|| failure("not_found", "approval not found"))
}
fn checked_action(event: &Event, wish: &str) -> Res<Action> {
    let action: Action = serde_json::from_value(event.data["action"].clone())
        .map_err(|_| failure("tampered", "invalid stored action"))?;
    let stored = event.data["digest"]
        .as_str()
        .ok_or_else(|| failure("tampered", "missing digest"))?;
    if event.wish != wish || digest(wish, &action) != stored {
        return Err(failure("tampered", "stored action or digest changed"));
    }
    Ok(action)
}

pub fn request(store: &Path, action: Action, reason: &str, origin: Origin) -> Res<Requested> {
    let wish = wish_of_store(store);
    let hash = digest(&wish, &action);
    loop {
        let events = read_events(store, 0)?;
        let n = events
            .iter()
            .filter(|e| e.kind == APPROVAL_REQUESTED)
            .filter_map(|e| {
                e.data["id"]
                    .as_str()?
                    .strip_prefix("ap-")?
                    .parse::<u64>()
                    .ok()
            })
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| failure("invalid_argument", "approval id overflow"))?;
        let id = format!("ap-{n}");
        let result = append_if(
            store,
            APPROVAL_REQUESTED,
            None,
            None,
            json!({"id": id, "action": action, "digest": hash, "reason": reason,
                "requested_by": match origin { Origin::Person => "person", Origin::Model => "model" }}),
            |events| {
                if events
                    .iter()
                    .any(|e| e.kind == APPROVAL_REQUESTED && has_id(e, &id))
                {
                    Err(failure("id_conflict", "approval id already exists"))
                } else {
                    Ok(())
                }
            },
        );
        match result {
            Ok(_) => return Ok(Requested { id, digest: hash }),
            Err(e) if e.kind == "id_conflict" => continue,
            Err(e) => return Err(e),
        }
    }
}

pub fn reply(
    home: &Path,
    store: &Path,
    id: &str,
    approve: bool,
    hash: &str,
    wish: &str,
    reason: Option<&str>,
    origin: Origin,
) -> Res<Event> {
    if origin == Origin::Model {
        return Err(failure("person_only", "only a person may reply"));
    }
    if wish != wish_of_store(store) {
        return Err(failure("wrong_wish", "wish does not match store"));
    }
    let reason = reason.unwrap_or("");
    if !approve && reason.trim().is_empty() {
        return Err(failure("invalid_argument", "denial reason is required"));
    }
    // Load the action for the denial event; verify the same value again under the lock.
    let events = read_events(store, 0)?;
    let initial = requested(&events, id)?;
    let action = checked_action(initial, wish)?;
    let kind = if approve {
        APPROVAL_APPROVED
    } else {
        APPROVAL_DENIED
    };
    append_if(
        store,
        kind,
        None,
        None,
        json!({"id": id, "digest": hash, "reason": reason, "action": action}),
        |events| {
            let req = requested(events, id)?;
            let current = checked_action(req, wish)?;
            if current != action || req.data["digest"].as_str() != Some(hash) {
                return Err(failure("tampered", "digest does not match request"));
            }
            if events.iter().any(|e| {
                has_id(e, id) && (e.kind == APPROVAL_APPROVED || e.kind == APPROVAL_DENIED)
            }) {
                return Err(failure("already_replied", "approval already has a reply"));
            }
            if !approve {
                let project = store
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|p| p.to_str())
                    .ok_or_else(|| failure("invalid_argument", "store has no project"))?;
                let project_dir = std::fs::canonicalize(&action.cwd)
                    .unwrap_or_else(|_| action.cwd.clone().into());
                let project_dir = project_dir.to_string_lossy();
                #[cfg(windows)]
                let project_dir = if let Some(path) = project_dir.strip_prefix(r"\\?\UNC\") {
                    format!(r"\\{path}")
                } else {
                    project_dir
                        .strip_prefix(r"\\?\")
                        .unwrap_or(&project_dir)
                        .to_owned()
                };
                #[cfg(not(windows))]
                let project_dir = project_dir.into_owned();
                // Memory requires a verified unit to cite a file from its scope.
                let files = store_for(home, project)
                    .and_then(|s| unit_brief(&s, id))
                    .ok()
                    .and_then(|brief| {
                        brief.scope.into_iter().find(|p| {
                            let path = Path::new(p);
                            !path.is_absolute()
                                && !path
                                    .components()
                                    .any(|c| matches!(c, std::path::Component::ParentDir))
                                && Path::new(&project_dir).join(path).is_file()
                        })
                    })
                    .into_iter()
                    .collect();
                let record = Record {
                    project: project.into(),
                    project_dir,
                    unit: id.into(),
                    title: format!("correction: approval {id} denied"),
                    summary: reason.into(),
                    body: serde_json::to_string(&action).expect("action serializes"),
                    symbols: vec![],
                    files,
                    wish: wish.into(),
                };
                match Memory::open(home)?.record(&record) {
                    Ok(_) => (),
                    Err(e) if e.kind == "not_verified" => (),
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        },
    )
}

pub fn execute(store: &Path, id: &str) -> Res<Executed> {
    let wish = wish_of_store(store);
    let mut action = None;
    append_if(
        store,
        APPROVAL_EXECUTING,
        None,
        None,
        json!({"id": id}),
        |events| {
            if events
                .iter()
                .any(|e| e.kind == APPROVAL_EXECUTING && has_id(e, id))
            {
                return Err(failure("already_executed", "approval already claimed"));
            }
            if !events
                .iter()
                .any(|e| e.kind == APPROVAL_APPROVED && has_id(e, id))
                || events
                    .iter()
                    .any(|e| e.kind == APPROVAL_DENIED && has_id(e, id))
            {
                return Err(failure("not_approved", "approval is not approved"));
            }
            let req = requested(events, id)?;
            let checked = checked_action(req, &wish)?;
            let approved = events
                .iter()
                .find(|e| e.kind == APPROVAL_APPROVED && has_id(e, id))
                .unwrap();
            if approved.data["digest"] != req.data["digest"] {
                return Err(failure("tampered", "approved digest changed"));
            }
            action = Some(checked);
            Ok(())
        },
    )?;
    let action = action.expect("claim captures action");
    let output = crate::base::proc::cmd(&action.command)
        .args(&action.args)
        .current_dir(&action.cwd)
        .output();
    let output = match output {
        Ok(output) => output,
        Err(e) => {
            let error = Failure::new("io", e.to_string());
            append_event(
                store,
                APPROVAL_EXECUTED,
                None,
                None,
                json!({"id": id, "exit_code": -1, "output_tail": error.message}),
            )?;
            return Err(error);
        }
    };
    let exit_code = output.status.code().unwrap_or(-1);
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    let output_tail: String = combined
        .chars()
        .rev()
        .take(2000)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    append_event(
        store,
        APPROVAL_EXECUTED,
        None,
        None,
        json!({"id": id, "exit_code": exit_code, "output_tail": output_tail}),
    )?;
    Ok(Executed {
        exit_code,
        output_tail,
    })
}

pub fn status(store: &Path, id: &str) -> Res<Value> {
    let events = read_events(store, 0)?;
    requested(&events, id)?;
    let state = events
        .iter()
        .rev()
        .find_map(|e| {
            if !has_id(e, id) {
                return None;
            }
            match e.kind.as_str() {
                APPROVAL_REQUESTED => Some("requested"),
                APPROVAL_APPROVED => Some("approved"),
                APPROVAL_DENIED => Some("denied"),
                APPROVAL_EXECUTING => Some("executing"),
                APPROVAL_EXECUTED => Some("executed"),
                _ => None,
            }
        })
        .unwrap_or("requested");
    Ok(json!({"status": state}))
}
