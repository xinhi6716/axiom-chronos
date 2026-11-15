use crate::base::{Failure, Res};
use std::{collections::VecDeque, sync::Mutex};
/// The language a proposal is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalLang {
    ZhTw,
    En,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub goal: String,
    pub scope: String,
    pub limits: String,
    pub acceptance: String,
    pub ambiguities: Vec<String>,
}
pub trait OptimizerRunner: Send + Sync + 'static {
    fn propose(&self, original: &str, lang: ProposalLang) -> Res<Proposal>;
}
#[derive(Default)]
pub struct FakeOptimizer {
    state: Mutex<(VecDeque<Res<Proposal>>, usize)>,
}
impl FakeOptimizer {
    pub fn new(script: Vec<Res<Proposal>>) -> Self {
        Self {
            state: Mutex::new((script.into(), 0)),
        }
    }
    pub fn calls(&self) -> usize {
        self.state.lock().unwrap().1
    }
}
impl OptimizerRunner for FakeOptimizer {
    fn propose(&self, _: &str, _: ProposalLang) -> Res<Proposal> {
        let mut s = self.state.lock().unwrap();
        s.1 += 1;
        s.0.pop_front()
            .unwrap_or_else(|| Err(Failure::new("empty", "No scripted proposal")))
    }
}
