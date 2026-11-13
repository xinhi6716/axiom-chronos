use crate::{
    base::{Failure, Res},
    paper::{service::PaperStatus, Requirements, Shortlist, SpendEstimate},
};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

#[derive(Clone, Debug, PartialEq)]
pub struct PageImage {
    pub png: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub page: u32,
    pub pages: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Ready(PageImage),
    Unavailable { reason: String, pdf: PathBuf },
}
pub trait PaperService: Send + Sync + 'static {
    fn status(&self, session: &str) -> Res<PaperStatus>;
    fn request_shortlist(&self, session: &str, req: Requirements, jcr: Option<PathBuf>) -> Res<()>;
    fn choose_journal(&self, session: &str, journal_id: &str, shortlist_hash: &str) -> Res<()>;
    fn estimate(&self, session: &str) -> Res<SpendEstimate>;
    fn start(&self, session: &str, estimate_digest: &str) -> Res<()>;
    fn cancel(&self, session: &str) -> Res<()>;
    fn preview(&self, session: &str, page: u32, width_px: u32) -> Res<Preview>;
    fn open_pdf(&self, session: &str) -> Res<()>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperCall {
    Status(String),
    RequestShortlist(String, Requirements, Option<PathBuf>),
    ChooseJournal(String, String, String),
    Estimate(String),
    Start(String, String),
    Cancel(String),
    Preview(String, u32, u32),
    OpenPdf(String),
}
#[derive(Default)]
struct State {
    statuses: HashMap<String, PaperStatus>,
    shortlists: HashMap<String, Shortlist>,
    estimates: HashMap<String, SpendEstimate>,
    previews: HashMap<String, Preview>,
    failures: HashMap<String, Failure>,
    calls: Vec<PaperCall>,
}
#[derive(Default)]
pub struct FakePaper {
    state: Mutex<State>,
}
impl FakePaper {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_status(&self, session: &str, status: PaperStatus) {
        self.state
            .lock()
            .unwrap()
            .statuses
            .insert(session.into(), status);
    }
    pub fn set_shortlist(&self, session: &str, shortlist: Shortlist) {
        self.state
            .lock()
            .unwrap()
            .shortlists
            .insert(session.into(), shortlist);
    }
    pub fn set_estimate(&self, session: &str, estimate: SpendEstimate) {
        self.state
            .lock()
            .unwrap()
            .estimates
            .insert(session.into(), estimate);
    }
    pub fn set_preview(&self, session: &str, preview: Preview) {
        self.state
            .lock()
            .unwrap()
            .previews
            .insert(session.into(), preview);
    }
    pub fn fail(&self, method: &str, failure: Failure) {
        self.state
            .lock()
            .unwrap()
            .failures
            .insert(method.into(), failure);
    }
    pub fn clear_failure(&self, method: &str) {
        self.state.lock().unwrap().failures.remove(method);
    }
    pub fn calls(&self) -> Vec<PaperCall> {
        self.state.lock().unwrap().calls.clone()
    }
    fn record(
        &self,
        method: &str,
        call: PaperCall,
        f: impl FnOnce(&mut State) -> Res<()>,
    ) -> Res<()> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(call);
        if let Some(error) = state.failures.get(method) {
            return Err(error.clone());
        }
        f(&mut state)
    }
}
impl PaperService for FakePaper {
    fn status(&self, session: &str) -> Res<PaperStatus> {
        let mut result = None;
        self.record("status", PaperCall::Status(session.into()), |s| {
            result = s.statuses.get(session).cloned();
            Ok(())
        })?;
        result.ok_or_else(|| Failure::new("not_found", "no scripted status"))
    }
    fn request_shortlist(&self, session: &str, req: Requirements, jcr: Option<PathBuf>) -> Res<()> {
        self.record(
            "request_shortlist",
            PaperCall::RequestShortlist(session.into(), req, jcr),
            |s| {
                let list = s
                    .shortlists
                    .get(session)
                    .cloned()
                    .ok_or_else(|| Failure::new("not_found", "no scripted shortlist"))?;
                let status = s
                    .statuses
                    .get_mut(session)
                    .ok_or_else(|| Failure::new("not_found", "no scripted status"))?;
                status.shortlist = Some(list);
                Ok(())
            },
        )
    }
    fn choose_journal(&self, session: &str, id: &str, hash: &str) -> Res<()> {
        self.record(
            "choose_journal",
            PaperCall::ChooseJournal(session.into(), id.into(), hash.into()),
            |_| Ok(()),
        )
    }
    fn estimate(&self, session: &str) -> Res<SpendEstimate> {
        let mut result = None;
        self.record("estimate", PaperCall::Estimate(session.into()), |s| {
            result = s.estimates.get(session).cloned();
            Ok(())
        })?;
        result.ok_or_else(|| Failure::new("not_found", "no scripted estimate"))
    }
    fn start(&self, session: &str, digest: &str) -> Res<()> {
        self.record(
            "start",
            PaperCall::Start(session.into(), digest.into()),
            |_| Ok(()),
        )
    }
    fn cancel(&self, session: &str) -> Res<()> {
        self.record("cancel", PaperCall::Cancel(session.into()), |_| Ok(()))
    }
    fn preview(&self, session: &str, page: u32, width: u32) -> Res<Preview> {
        let mut result = None;
        self.record(
            "preview",
            PaperCall::Preview(session.into(), page, width),
            |s| {
                result = s.previews.get(session).cloned();
                Ok(())
            },
        )?;
        result.ok_or_else(|| Failure::new("not_found", "no scripted preview"))
    }
    fn open_pdf(&self, session: &str) -> Res<()> {
        self.record("open_pdf", PaperCall::OpenPdf(session.into()), |_| Ok(()))
    }
}
