use super::{
    store::{LaunchRecord, Outcome, Store},
    Cause, Event, LaunchRequest, Routine, WishLauncher,
};
use crate::{
    base::{Failure, Res},
    seams::{sha256_hex, ApprovalRequest, Clock, Decision, PersonApproval},
};

pub fn due(routine: &Routine, last_due_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    let schedule = routine.schedule.as_ref()?;
    if schedule.every_minutes == 0 || now_ms < schedule.start_ms {
        return None;
    }
    let start = i128::from(schedule.start_ms);
    let interval = i128::from(schedule.every_minutes) * 60_000;
    let latest = start + ((i128::from(now_ms) - start) / interval) * interval;
    let latest = i64::try_from(latest).ok()?;
    if last_due_ms.is_some_and(|last| latest <= last) {
        None
    } else {
        Some(latest)
    }
}
pub fn tick(
    store: &Store,
    clock: &dyn Clock,
    events: &[Event],
    launcher: &dyn WishLauncher,
    approval: &dyn PersonApproval,
) -> Res<Vec<LaunchRecord>> {
    let now = clock.now_ms();
    let mut records = Vec::new();
    for routine in store.routines()?.into_iter().filter(|r| r.enabled) {
        let scheduled = store.update_state(|state| {
            let slot = due(&routine, state.last_due_ms.get(&routine.id).copied(), now)?;
            state.last_due_ms.insert(routine.id.clone(), slot);
            Some(slot)
        })?;
        if let Some(due_ms) = scheduled {
            let request = LaunchRequest {
                routine: routine.id.clone(),
                project_dir: routine.project_dir.clone(),
                wish: routine.wish.clone(),
                cause: Cause::Schedule { due_ms },
            };
            records.push(run(store, now, request, launcher, approval)?);
        }
        for event in events.iter().filter(|event| {
            routine.triggers.iter().any(|trigger| {
                trigger.kind == event.kind && trigger.repo.eq_ignore_ascii_case(&event.repo)
            })
        }) {
            let key = format!("{}/{}", routine.id, event.id);
            if !store.update_state(|state| state.seen.insert(key))? {
                continue;
            }
            let request = LaunchRequest {
                routine: routine.id.clone(),
                project_dir: routine.project_dir.clone(),
                wish: format!("{}\n\n{}\n{}", routine.wish, event.title, event.url),
                cause: Cause::Event {
                    id: event.id.clone(),
                    kind: event.kind.clone(),
                    url: event.url.clone(),
                },
            };
            records.push(run(store, now, request, launcher, approval)?);
        }
    }
    Ok(records)
}
fn run(
    store: &Store,
    now: i64,
    request: LaunchRequest,
    launcher: &dyn WishLauncher,
    approval: &dyn PersonApproval,
) -> Res<LaunchRecord> {
    let serialized =
        serde_json::to_vec(&request).map_err(|e| Failure::new("json", e.to_string()))?;
    let approval_request = ApprovalRequest {
        subject: format!("routine:{}", request.routine),
        action: "launch_wish".into(),
        summary: request.wish.clone(),
        digest: sha256_hex(&serialized),
    };
    let outcome = match approval.decide(&approval_request) {
        Decision::Denied => Outcome::Denied,
        Decision::Approved => match launcher.launch(&request) {
            Ok(id) => Outcome::Launched { id },
            Err(error) => Outcome::Failed {
                message: error.to_string(),
            },
        },
    };
    let record = LaunchRecord {
        routine: request.routine,
        cause: request.cause,
        outcome,
        at_ms: now,
    };
    store.update_state(|state| state.log.push(record.clone()))?;
    Ok(record)
}
