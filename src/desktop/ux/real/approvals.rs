//! Real ApprovalService over `engine::approvals`; owned by module approvals-seam.
use crate::base::{Failure, Res};
use crate::desktop::ux::seams::{
    ApprovalCard, ApprovalOutcome, ApprovalReceipt, ApprovalService, Decision, Verdict,
};
use crate::engine::approvals::{self, Origin};
use crate::engine::kinds::*;
use crate::orch::events::{read_events, Event};
use crate::orch::store::{is_wish_id, wish_of_store, wish_stores};
use std::{
    collections::HashMap,
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
    gate: Mutex<()>,
    pub(crate) bound: Mutex<Vec<Binding>>,
    keys: Mutex<HashMap<String, ApprovalReceipt>>,
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
            gate: Mutex::new(()),
            bound: Mutex::new(vec![]),
            keys: Mutex::new(HashMap::new()),
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
                    let (Some(id), Some(digest)) =
                        (e.data["id"].as_str(), e.data["digest"].as_str())
                    else {
                        continue;
                    };
                    let Ok(action) =
                        serde_json::from_value::<approvals::Action>(e.data["action"].clone())
                    else {
                        continue;
                    };
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

    fn remember(&self, r: ApprovalReceipt) -> ApprovalReceipt {
        self.keys.lock().unwrap().insert(r.key.clone(), r.clone());
        r
    }

    /// Wait for another decider (thread or process) to finish, then replay its result.
    fn settle(&self, b: &Binding, d: &Decision) -> Res<ApprovalReceipt> {
        let id = &b.card.action_id;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let state = state_of(&read_events(&b.store, 0)?, id);
            if let Some(r) = Self::receipt(d, &state) {
                return Ok(self.remember(r));
            }
            if state == State::Approved && d.verdict == Verdict::Approve {
                match approvals::execute(&b.store, id) {
                    Ok(_) => continue,
                    Err(e) if e.kind == "already_executed" => {}
                    Err(e) if e.kind == "io" => continue,
                    Err(e) => return Err(e),
                }
            }
            if Instant::now() > deadline {
                return Ok(ApprovalReceipt {
                    action_id: id.clone(),
                    key: d.key.clone(),
                    outcome: ApprovalOutcome::Failed {
                        reason: "execution did not finish".into(),
                    },
                });
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
            .filter(|(_, s)| *s == State::Requested)
            .map(|(b, _)| b)
            .collect();
        let cards = pending.iter().map(|b| b.card.clone()).collect();
        *self.bound.lock().unwrap() = pending;
        cards
    }

    fn decide(&self, d: &Decision) -> Res<ApprovalReceipt> {
        let _gate = self.gate.lock().unwrap();
        if let Some(r) = self.keys.lock().unwrap().get(&d.key) {
            return Ok(r.clone());
        }
        let mut candidates: Vec<Binding> = self
            .bound
            .lock()
            .unwrap()
            .iter()
            .filter(|b| b.card.action_id == d.action_id)
            .cloned()
            .collect();
        if candidates.is_empty() {
            candidates = self
                .scan()
                .into_iter()
                .map(|(b, _)| b)
                .filter(|b| b.card.action_id == d.action_id)
                .collect();
        }
        if candidates.is_empty() {
            return Err(Failure::new("not_found", "Approval not found"));
        }
        let b = candidates
            .into_iter()
            .find(|b| b.card.version == d.payload_version)
            .ok_or_else(|| Failure::new("stale", "Approval changed"))?;
        let id = b.card.action_id.clone();
        let state = state_of(&read_events(&b.store, 0)?, &id);
        if state != State::Requested {
            return self.settle(&b, d);
        }
        let (approve, reason) = match &d.verdict {
            Verdict::Approve => (true, None),
            Verdict::Reject { reason } => (false, Some(reason.as_str())),
        };
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
            Err(e) if e.kind == "already_replied" => return self.settle(&b, d),
            Err(e) => return Err(e),
        }
        if approve {
            match approvals::execute(&b.store, &id) {
                Ok(_) => {}
                Err(e) if e.kind == "already_executed" || e.kind == "io" => {}
                Err(e) => return Err(e),
            }
        }
        self.settle(&b, d)
    }
}
