use crate::base::{Failure, Res};
use std::{collections::HashMap, sync::Mutex};
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitEvidence {
    pub diff: Option<String>,
    pub review: Option<String>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<u64>,
    pub failure: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum UnitActionKind {
    Retry,
    Waive {
        reason: String,
        finding: Option<usize>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ActionReceipt {
    pub key: String,
    pub accepted: bool,
    pub message: String,
}
pub trait UnitActions: Send + Sync + 'static {
    fn evidence(&self, session: &str, unit: &str) -> Res<UnitEvidence>;
    fn act(
        &self,
        session: &str,
        unit: &str,
        kind: &UnitActionKind,
        key: &str,
    ) -> Res<ActionReceipt>;
}
#[derive(Default)]
struct State {
    evidence: HashMap<(String, String), UnitEvidence>,
    receipts: HashMap<String, ActionReceipt>,
    counts: HashMap<(String, String), u32>,
}
#[derive(Default)]
pub struct FakeUnitActions {
    state: Mutex<State>,
}
impl FakeUnitActions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_evidence(&self, session: &str, unit: &str, evidence: UnitEvidence) {
        self.state
            .lock()
            .unwrap()
            .evidence
            .insert((session.into(), unit.into()), evidence);
    }
    pub fn count(&self, unit: &str, kind: &str) -> u32 {
        *self
            .state
            .lock()
            .unwrap()
            .counts
            .get(&(unit.into(), kind.into()))
            .unwrap_or(&0)
    }
}
impl UnitActions for FakeUnitActions {
    fn evidence(&self, session: &str, unit: &str) -> Res<UnitEvidence> {
        self.state
            .lock()
            .unwrap()
            .evidence
            .get(&(session.into(), unit.into()))
            .cloned()
            .ok_or_else(|| Failure::new("not_found", "Unit not found"))
    }
    fn act(
        &self,
        session: &str,
        unit: &str,
        kind: &UnitActionKind,
        key: &str,
    ) -> Res<ActionReceipt> {
        let mut s = self.state.lock().unwrap();
        if let Some(r) = s.receipts.get(key) {
            return Ok(r.clone());
        }
        if !s.evidence.contains_key(&(session.into(), unit.into())) {
            return Err(Failure::new("not_found", "Unit not found"));
        }
        let action = match kind {
            UnitActionKind::Retry => "retry",
            UnitActionKind::Waive { reason, .. } if reason.trim().is_empty() => {
                return Err(Failure::new("reason_required", "A reason is required"))
            }
            UnitActionKind::Waive { .. } => "waive",
        };
        *s.counts.entry((unit.into(), action.into())).or_default() += 1;
        let r = ActionReceipt {
            key: key.into(),
            accepted: true,
            message: format!("{action} accepted"),
        };
        s.receipts.insert(key.into(), r.clone());
        Ok(r)
    }
}
