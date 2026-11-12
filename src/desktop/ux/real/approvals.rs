//! Real ApprovalService over `engine::approvals`; owned by module approvals-seam.
use crate::base::{Failure, Res};
use crate::desktop::ux::seams::{
    ApprovalCard, ApprovalOutcome, ApprovalReceipt, ApprovalService, Decision, Verdict,
};
use crate::engine::approvals::{self, Origin};
use crate::engine::kinds::*;
use crate::orch::events::{append_event, read_events, Event};
use crate::orch::store::{is_wish_id, wish_of_store, wish_stores};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// Where a card lives: its store, wish and the digest the request was bound to.
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub store: PathBuf,
    pub wish: String,
    pub digest: String,
    pub card: ApprovalCard,
    valid_action: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Requested,
    Approved,
    Denied,
    Executing,
    Executed { exit_code: i64, tail: String },
}

pub struct EngineApprovals {
    home: PathBuf,
    pub(crate) bound: Mutex<Vec<Binding>>,
}

fn version_of(digest: &str) -> u64 {
    let head: String = digest.chars().take(16).collect();
    u64::from_str_radix(&head, 16).unwrap_or(0)
}

fn state_of(events: &[Event], id: &str) -> State {
    let mut state = State::Requested;
    for e in events.iter().filter(|e| e.data["id"].as_str() == Some(id)) {
        match e.kind.as_str() {
            APPROVAL_APPROVED => state = State::Approved,
            APPROVAL_DENIED => state = State::Denied,
            APPROVAL_EXECUTING => state = State::Executing,
            APPROVAL_EXECUTED => {
                state = State::Executed {
                    exit_code: e.data["exit_code"].as_i64().unwrap_or(-1),
                    tail: e.data["output_tail"].as_str().unwrap_or("").to_owned(),
                }
            }
            _ => {}
        }
    }
    state
}

fn session_of(home: &Path, wish: &str) -> String {
    std::fs::read_dir(home.join("goals"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .find_map(|e| {
            let goal: crate::goal::Goal =
                serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok()?;
            if goal.wish_id.as_deref() == Some(wish) {
                e.path().file_stem()?.to_str().map(str::to_owned)
            } else {
                None
            }
        })
        .unwrap_or_else(|| wish.to_owned())
}

impl EngineApprovals {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            bound: Mutex::new(vec![]),
        }
    }

    /// Every approval request in every wish store, with its persisted state.
    fn scan(&self) -> Vec<(Binding, State)> {
        let mut out = vec![];
        let mut projects: Vec<PathBuf> = std::fs::read_dir(self.home.join("orchestrate"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|k| k.is_dir()))
            .map(|e| e.path())
            .collect();
        projects.sort();
        for project in projects {
            for store in wish_stores(&project) {
                let wish = wish_of_store(&store);
                if !is_wish_id(&wish) {
                    continue;
                }
                let Ok(events) = read_events(&store, 0) else {
                    continue;
                };
                let session = session_of(&self.home, &wish);
                for e in events.iter().filter(|e| e.kind == APPROVAL_REQUESTED) {
                    let Some(id) = e.data["id"].as_str() else {
                        continue;
                    };
                    let digest = e.data["digest"].as_str().unwrap_or("");
                    let action =
                        serde_json::from_value::<approvals::Action>(e.data["action"].clone());
                    let valid_action = action.is_ok() && !digest.is_empty();
                    let action = action.unwrap_or(approvals::Action {
                        command: String::new(),
                        args: vec![],
                        cwd: String::new(),
                    });
                    let card = ApprovalCard {
                        action_id: id.to_owned(),
                        version: version_of(digest),
                        session: session.clone(),
                        action: action.command.clone(),
                        target: action.args.join(" "),
                        scope: action.cwd.clone(),
                        cost_usd: None,
                    };
                    out.push((
                        Binding {
                            store: store.clone(),
                            wish: wish.clone(),
                            digest: digest.to_owned(),
                            card,
                            valid_action,
                        },
                        state_of(&events, id),
                    ));
                }
            }
        }
        out
    }

    fn receipt(d: &Decision, state: &State) -> Option<ApprovalReceipt> {
        let outcome = match state {
            State::Denied => ApprovalOutcome::Rejected,
            State::Executed { exit_code: 0, tail } => ApprovalOutcome::Executed {
                summary: tail.trim().to_owned(),
            },
            State::Executed { exit_code, tail } => ApprovalOutcome::Failed {
                reason: format!("exit code {exit_code}: {}", tail.trim()),
            },
            _ => return None,
        };
        Some(ApprovalReceipt {
            action_id: d.action_id.clone(),
            key: d.key.clone(),
            outcome,
        })
    }

    fn checked_state(b: &Binding) -> Res<State> {
        if wish_of_store(&b.store) != b.wish {
            return Err(Failure::new("wrong_wish", "Wish does not match store"));
        }
        let events = read_events(&b.store, 0)?;
        let request = events
            .iter()
            .find(|e| {
                e.kind == APPROVAL_REQUESTED && e.data["id"].as_str() == Some(&b.card.action_id)
            })
            .ok_or_else(|| Failure::new("wrong_wish", "Approval not found"))?;
        let action: approvals::Action = serde_json::from_value(request.data["action"].clone())
            .map_err(|_| Failure::new("tampered", "Invalid stored action"))?;
        if request.wish != b.wish
            || request.data["digest"].as_str() != Some(&b.digest)
            || approvals::digest(&b.wish, &action) != b.digest
        {
            return Err(Failure::new("tampered", "Stored action or digest changed"));
        }
        Ok(state_of(&events, &b.card.action_id))
    }

    /// Wait for another decider (thread or process) to finish, then replay its result.
    fn settle(&self, b: &Binding, d: &Decision) -> Res<ApprovalReceipt> {
        self.settle_until(b, d, Instant::now() + Duration::from_secs(60))
    }

    pub(super) fn settle_until(
        &self,
        b: &Binding,
        d: &Decision,
        deadline: Instant,
    ) -> Res<ApprovalReceipt> {
        let id = &b.card.action_id;
        loop {
            let state = Self::checked_state(b)?;
            if let Some(r) = Self::receipt(d, &state) {
                return Ok(r);
            }
            if state == State::Approved {
                match approvals::execute(&b.store, id) {
                    Ok(_) => continue,
                    Err(e) if e.kind == "already_executed" => {}
                    Err(e)
                        if e.kind == "io"
                            && matches!(Self::checked_state(b)?, State::Executed { .. }) =>
                    {
                        continue
                    }
                    Err(e) => return Err(e),
                }
            }
            if Instant::now() > deadline {
                return Err(Failure::new("timeout", "Execution did not finish"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl ApprovalService for EngineApprovals {
    fn pending(&self) -> Vec<ApprovalCard> {
        let pending: Vec<Binding> = self
            .scan()
            .into_iter()
            .filter(|(b, s)| *s == State::Requested && b.valid_action)
            .map(|(b, _)| b)
            .collect();
        let cards = pending.iter().map(|b| b.card.clone()).collect();
        let mut bound = self.bound.lock().unwrap();
        for b in pending {
            if !bound
                .iter()
                .any(|old| old.store == b.store && old.card.action_id == b.card.action_id)
            {
                bound.push(b);
            }
        }
        cards
    }

    fn decide(&self, d: &Decision) -> Res<ApprovalReceipt> {
        // Serialize key claims across adapter instances and processes. The key index
        // stores only bindings; outcomes always come from the engine's events.
        let registry = self.home.join("approvals-seam");
        std::fs::create_dir_all(&registry).map_err(|e| Failure::new("io", e.to_string()))?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(registry.join("decide.lock"))
            .map_err(|e| Failure::new("io", e.to_string()))?;
        lock.lock().map_err(|e| Failure::new("io", e.to_string()))?;
        let mut candidates: Vec<Binding> = self
            .bound
            .lock()
            .unwrap()
            .iter()
            .filter(|b| b.card.action_id == d.action_id)
            .cloned()
            .collect();
        for (b, _) in self.scan() {
            if b.card.action_id == d.action_id && !candidates.iter().any(|old| old.store == b.store)
            {
                candidates.push(b);
            }
        }
        if candidates.is_empty() {
            return Err(Failure::new("wrong_wish", "Approval not found"));
        }
        if candidates.iter().any(|b| b.digest.is_empty()) {
            return Err(Failure::new("tampered", "Missing stored digest"));
        }
        let mut matching = candidates
            .into_iter()
            .filter(|b| b.card.version == d.payload_version);
        let b = matching
            .next()
            .ok_or_else(|| Failure::new("stale", "Approval changed"))?;
        if matching.next().is_some() {
            return Err(Failure::new("wrong_wish", "Ambiguous approval binding"));
        }
        let id = b.card.action_id.clone();
        let state = Self::checked_state(&b)?;
        if let Some(event) = read_events(&registry, 0)?
            .into_iter()
            .find(|e| e.kind == "approval_key" && e.data["key"].as_str() == Some(&d.key))
        {
            let store: PathBuf = serde_json::from_value(event.data["store"].clone())
                .map_err(|e| Failure::new("invalid_json", e.to_string()))?;
            let wish = event.data["wish"]
                .as_str()
                .ok_or_else(|| Failure::new("invalid_json", "Missing wish"))?
                .to_owned();
            let digest = event.data["digest"]
                .as_str()
                .ok_or_else(|| Failure::new("invalid_json", "Missing digest"))?
                .to_owned();
            let original_id = event.data["id"]
                .as_str()
                .ok_or_else(|| Failure::new("invalid_json", "Missing id"))?
                .to_owned();
            let mut original = b.clone();
            original.store = store;
            original.wish = wish;
            original.digest = digest;
            original.card.action_id = original_id.clone();
            let mut replay = d.clone();
            replay.action_id = original_id;
            return self.settle(&original, &replay);
        }
        let (approve, reason) = match &d.verdict {
            Verdict::Approve => (true, None),
            Verdict::Reject { reason } => (false, Some(reason.as_str())),
        };
        if state == State::Requested {
            match approvals::reply(
                &self.home,
                &b.store,
                &id,
                approve,
                &b.digest,
                &b.wish,
                reason,
                Origin::Person,
            ) {
                Ok(_) => {}
                Err(e) if e.kind == "already_replied" => {}
                Err(e) => return Err(e),
            }
            if approve && matches!(Self::checked_state(&b)?, State::Approved) {
                match approvals::execute(&b.store, &id) {
                    Ok(_) => {}
                    Err(e) if e.kind == "already_executed" => {}
                    Err(e)
                        if e.kind == "io"
                            && matches!(Self::checked_state(&b)?, State::Executed { .. }) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        let receipt = self.settle(&b, d)?;
        append_event(
            &registry,
            "approval_key",
            None,
            None,
            serde_json::json!({
                "key": d.key, "id": id, "store": b.store, "wish": b.wish, "digest": b.digest
            }),
        )?;
        Ok(receipt)
    }
}
