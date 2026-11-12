use crate::{
    base::Res,
    paper::{members::PaperMembers, JournalRecord, PaperEvent, PassRecord},
};
use std::{path::PathBuf, sync::atomic::AtomicBool};

pub struct RunConfig {
    pub store: PathBuf,
    pub home: PathBuf,
    pub project_root: PathBuf,
    pub timeout_s: u64,
    pub fable_cap_usd: f64,
}

pub fn run_passes(
    _cfg: &RunConfig,
    _m: &PaperMembers,
    _journal: &JournalRecord,
    _template_rules: &str,
    _stop: &AtomicBool,
    _on_event: &dyn Fn(&PaperEvent),
) -> Res<Vec<PassRecord>> {
    unimplemented!()
}
