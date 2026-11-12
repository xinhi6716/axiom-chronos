//! service; owned by module paper-service.
use super::{
    cache::FetchMode, evidence::BundleInputs, http::Transport, members::PaperMembers,
    readiness::ReadyStatus, EvidenceBundle, JournalChoice, PaperEvent, PassRecord, Requirements,
    Shortlist, SpendEstimate, Step,
};
use crate::{base::Res, seams::Clock};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Debug, Clone)]
pub struct PaperStatus {
    pub steps: Vec<(Step, bool, Option<String>)>,
    pub evidence: Option<String>,
    pub requirements: Option<Requirements>,
    pub shortlist: Option<Shortlist>,
    pub choice: Option<JournalChoice>,
    pub estimate: Option<SpendEstimate>,
    pub passes: Vec<PassRecord>,
    pub ready: Option<ReadyStatus>,
    pub pdf: Option<PathBuf>,
    pub running: bool,
    pub last_error: Option<(String, String)>,
}

pub struct PaperSession {
    pub store: PathBuf,
    pub home: PathBuf,
    pub project_root: PathBuf,
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub timeout_s: u64,
    pub fable_cap_usd: f64,
}

#[allow(unused_variables)]
impl PaperSession {
    pub fn open(
        store: &Path,
        data_home: &Path,
        project_root: &Path,
        transport: Arc<dyn Transport>,
        clock: Arc<dyn Clock>,
    ) -> Res<PaperSession> {
        unimplemented!()
    }
    pub fn status(&self) -> Res<PaperStatus> {
        unimplemented!()
    }
    pub fn build_evidence(&self, inputs: &BundleInputs) -> Res<EvidenceBundle> {
        unimplemented!()
    }
    pub fn shortlist(
        &self,
        requirements: &Requirements,
        jcr: Option<&Path>,
        mode: FetchMode,
    ) -> Res<Shortlist> {
        unimplemented!()
    }
    pub fn choose_journal(&self, journal_id: &str, shortlist_hash: &str) -> Res<JournalChoice> {
        unimplemented!()
    }
    pub fn estimate(&self) -> Res<SpendEstimate> {
        unimplemented!()
    }
    pub fn write(
        &self,
        m: &PaperMembers,
        estimate_digest: &str,
        stop: &AtomicBool,
        on_event: &dyn Fn(&PaperEvent),
    ) -> Res<ReadyStatus> {
        unimplemented!()
    }
}
