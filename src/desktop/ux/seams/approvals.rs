use crate::base::{Failure, Res};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalCard {
    pub action_id: String,
    pub version: u64,
    pub session: String,
    pub action: String,
    pub target: String,
    pub scope: String,
    pub cost_usd: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Approve,
    Reject { reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub action_id: String,
    pub payload_version: u64,
    pub key: String,
    pub verdict: Verdict,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalOutcome {
    Executed { summary: String },
    Rejected,
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalReceipt {
    pub action_id: String,
    pub key: String,
    pub outcome: ApprovalOutcome,
}
pub trait ApprovalService: Send + Sync + 'static {
    fn pending(&self) -> Vec<ApprovalCard>;
    fn decide(&self, d: &Decision) -> Res<ApprovalReceipt>;
}
#[derive(Default)]
pub struct Book {
    pub keys: HashMap<(String, u64), String>,
    pub in_flight: HashSet<String>,
    pub receipts: HashMap<String, ApprovalReceipt>,
    pub errors: HashMap<String, String>,
    pub next: u64,
}
#[derive(Default)]
struct State {
    cards: Vec<ApprovalCard>,
    keys: HashMap<String, ApprovalReceipt>,
    actions: HashMap<String, ApprovalReceipt>,
    executions: HashMap<String, u32>,
}
#[derive(Default)]
pub struct FakeApprovals {
    state: Mutex<State>,
}
impl FakeApprovals {
    pub fn new(cards: Vec<ApprovalCard>) -> Self {
        Self {
            state: Mutex::new(State {
                cards,
                ..Default::default()
            }),
        }
    }
    pub fn executions(&self, id: &str) -> u32 {
        *self.state.lock().unwrap().executions.get(id).unwrap_or(&0)
    }
    pub fn bump_version(&self, id: &str) {
        if let Some(c) = self
            .state
            .lock()
            .unwrap()
            .cards
            .iter_mut()
            .find(|c| c.action_id == id)
        {
            c.version += 1;
        }
    }
}
impl ApprovalService for FakeApprovals {
    fn pending(&self) -> Vec<ApprovalCard> {
        let s = self.state.lock().unwrap();
        s.cards
            .iter()
            .filter(|c| !s.actions.contains_key(&c.action_id))
            .cloned()
            .collect()
    }
    fn decide(&self, d: &Decision) -> Res<ApprovalReceipt> {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.keys.get(&d.key) {
            return Ok(r.clone());
        }
        if let Some(r) = s.actions.get(&d.action_id) {
            return Ok(r.clone());
        }
        let card = s
            .cards
            .iter()
            .find(|c| c.action_id == d.action_id)
            .cloned()
            .ok_or_else(|| Failure::new("not_found", "Approval not found"))?;
        if card.version != d.payload_version {
            return Err(Failure::new("stale", "Approval changed"));
        }
        let outcome = match &d.verdict {
            Verdict::Approve => {
                *s.executions.entry(d.action_id.clone()).or_default() += 1;
                ApprovalOutcome::Executed {
                    summary: format!("{}: {}", card.action, card.target),
                }
            }
            Verdict::Reject { .. } => ApprovalOutcome::Rejected,
        };
        let r = ApprovalReceipt {
            action_id: d.action_id.clone(),
            key: d.key.clone(),
            outcome,
        };
        s.keys.insert(d.key.clone(), r.clone());
        s.actions.insert(d.action_id.clone(), r.clone());
        Ok(r)
    }
}
