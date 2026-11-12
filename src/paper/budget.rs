use crate::base::{Failure, Res};
use crate::paper::SpendEstimate;
use std::path::Path;

pub const PAPER_PROGRAM_TYPE: &str = "paper_four_pass";

pub fn estimate(_home: &Path, _models: &[String]) -> Res<SpendEstimate> {
    Err(Failure::new("not_implemented", ""))
}

#[derive(Debug)]
pub struct PaperBudget {
    _private: (),
}

impl PaperBudget {
    pub fn open(_store: &Path, _run_id: &str, _fable_cap_usd: f64) -> Res<Self> {
        Err(Failure::new("not_implemented", ""))
    }

    pub fn before_call(&self, _store: &Path, _member: &str) -> Res<()> {
        Err(Failure::new("not_implemented", ""))
    }

    pub fn charge(
        &mut self,
        _store: &Path,
        _member: &str,
        _model: &str,
        _usd: Option<f64>,
        _tokens: u64,
    ) -> Res<()> {
        Err(Failure::new("not_implemented", ""))
    }

    pub fn fable_spent(&self) -> f64 {
        0.0
    }

    pub fn finish(&self, _home: &Path, _models: &[String], _completed: bool) -> Res<()> {
        Err(Failure::new("not_implemented", ""))
    }
}
