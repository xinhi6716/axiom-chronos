//! review; owned by module paper-review.
use super::{budget::PaperBudget, runner::RunConfig};
use crate::{base::Res, research_program::investigation::ResearchMember};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReviewRecord {
    pub verdict: String,
    pub findings: Vec<serde_json::Value>,
    pub input_hash: String,
    pub model: Option<String>,
    pub usd: Option<f64>,
    pub tokens: u64,
    pub at: String,
}
pub fn input_files(_store: &Path) -> Res<Vec<(String, PathBuf)>> {
    unimplemented!()
}
pub fn current_input_hash(_store: &Path) -> Res<String> {
    unimplemented!()
}
pub fn review(
    _cfg: &RunConfig,
    _reviewer: &dyn ResearchMember,
    _budget: &mut PaperBudget,
    _stop: &AtomicBool,
) -> Res<ReviewRecord> {
    unimplemented!()
}
