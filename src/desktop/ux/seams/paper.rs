use crate::base::{Failure, Res};
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PaperView {
    pub phase: String,
    pub passes: Vec<PassRow>,
    pub journal: Option<JournalRow>,
    pub shortlist: Vec<JournalRow>,
    pub estimate: Option<EstimateRow>,
    pub changelog: Vec<ChangeRow>,
    pub readiness: Vec<GateRow>,
    pub requirements: Option<Requirements>,
    pub evidence: Option<String>,
    pub pdf: Option<PathBuf>,
    pub running: bool,
    pub last_error: Option<(String, String)>,
    pub steps: Vec<GateRow>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PassRow {
    pub number: u8,
    pub status: String,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct JournalRow {
    pub id: String,
    pub title: String,
    pub hash: String,
    pub bucket: String,
    pub excluded: bool,
    pub metrics: Vec<MetricRow>,
    pub filters: Vec<FilterRow>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FilterRow {
    pub name: String,
    pub status: String,
    pub detail: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MetricRow {
    pub label: String,
    pub value: String,
    pub source: String,
    pub date: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeRow {
    pub pass: u8,
    pub member: String,
    pub location: String,
    pub kind: String,
    pub reason: String,
    pub validation: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct GateRow {
    pub label: String,
    pub ok: bool,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PassModels {
    pub astra_draft: String,
    pub fable_revise: String,
    pub gemini_revise: String,
    pub fable_style: String,
}
impl Default for PassModels {
    fn default() -> Self {
        Self {
            astra_draft: "gpt-6-astra".into(),
            fable_revise: "claude-fable-5-2".into(),
            gemini_revise: "gemini-4-hydrogen-xhigh".into(),
            fable_style: "claude-fable-5-2".into(),
        }
    }
}
impl PassModels {
    pub fn validate(&self) -> Res<()> {
        if [
            &self.astra_draft,
            &self.fable_revise,
            &self.gemini_revise,
            &self.fable_style,
        ]
        .iter()
        .any(|s| s.is_empty() || s.chars().any(char::is_whitespace))
        {
            return Err(Failure::new("invalid_model", "invalid model"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Quartile {
    Q1,
    Q2,
    Q3,
    Q4,
}
#[derive(Clone, Debug, PartialEq)]
pub enum QuartileScheme {
    Sjr,
    Jcr,
}
#[derive(Clone, Debug, PartialEq)]
pub enum AccessWanted {
    OpenAccess,
    Subscription,
    Either,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Money {
    pub amount_minor: u64,
    pub currency: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Requirements {
    pub abstract_text: String,
    pub keywords: Vec<String>,
    pub max_quartile: Option<Quartile>,
    pub tier_scheme: QuartileScheme,
    pub access: AccessWanted,
    pub apc_limit: Option<Money>,
    pub require_no_fee: bool,
    pub categories: Vec<String>,
    pub free_text: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShortlistView {
    pub rows: Vec<JournalRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShortlistInput {
    pub requirements: Requirements,
    pub jcr: Option<PathBuf>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StartInput {
    pub estimate_digest: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewInput {
    pub page: u32,
    pub width_px: u32,
}

/// Estimate values presented by the paper flow. Token counts are optional because
/// the engine does not currently estimate them.
#[derive(Clone, Debug, PartialEq)]
pub struct EstimateRow {
    pub digest: String,
    pub usd: Option<f64>,
    pub tokens: Option<u64>,
    pub stale: bool,
    pub line: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageImage {
    pub png: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
    pub page: u32,
    pub pages: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewView {
    Ready(PageImage),
    Unavailable { reason: String, pdf: PathBuf },
}
/// Compatibility name for callers that have not migrated to the display name.
pub type Preview = PreviewView;
pub trait PaperService: Send + Sync + 'static {
    fn status(&self, session: &str) -> Res<PaperView>;
    fn evidence_question(&self, _session: &str) -> Res<Option<String>> {
        Ok(None)
    }
    fn models(&self, session: &str) -> Res<PassModels>;
    fn set_models(&self, session: &str, m: PassModels) -> Res<()>;
    fn request_shortlist(&self, session: &str, input: ShortlistInput) -> Res<()>;
    fn choose_journal(&self, session: &str, journal_id: &str, shortlist_hash: &str) -> Res<()>;
    fn estimate(&self, session: &str) -> Res<EstimateRow>;
    /// Check the persisted estimate against the current inputs without rewriting it.
    fn estimate_is_fresh(&self, session: &str, digest: &str) -> Res<bool>;
    fn start(&self, session: &str, input: StartInput) -> Res<()>;
    fn cancel(&self, session: &str) -> Res<()>;
    fn preview(&self, session: &str, input: PreviewInput) -> Res<PreviewView>;
    fn open_pdf(&self, session: &str) -> Res<()>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaperCall {
    Status(String),
    Models(String),
    SetModels(String, PassModels),
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
    statuses: HashMap<String, PaperView>,
    shortlists: HashMap<String, ShortlistView>,
    estimates: HashMap<String, EstimateRow>,
    models: HashMap<String, PassModels>,
    previews: HashMap<String, PreviewView>,
    questions: HashMap<String, String>,
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
    pub fn set_status(&self, session: &str, status: impl Into<PaperView>) {
        self.state
            .lock()
            .unwrap()
            .statuses
            .insert(session.into(), status.into());
    }
    pub fn set_shortlist(&self, session: &str, shortlist: impl Into<ShortlistView>) {
        self.state
            .lock()
            .unwrap()
            .shortlists
            .insert(session.into(), shortlist.into());
    }
    pub fn set_estimate(&self, session: &str, estimate: impl Into<EstimateRow>) {
        self.state
            .lock()
            .unwrap()
            .estimates
            .insert(session.into(), estimate.into());
    }
    pub fn set_preview(&self, session: &str, preview: impl Into<PreviewView>) {
        self.state
            .lock()
            .unwrap()
            .previews
            .insert(session.into(), preview.into());
    }
    pub fn set_evidence_question(&self, session: &str, question: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .questions
            .insert(session.into(), question.into());
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
    fn evidence_question(&self, session: &str) -> Res<Option<String>> {
        Ok(self.state.lock().unwrap().questions.get(session).cloned())
    }
    fn status(&self, session: &str) -> Res<PaperView> {
        let mut result = None;
        self.record("status", PaperCall::Status(session.into()), |s| {
            result = s.statuses.get(session).cloned();
            Ok(())
        })?;
        result.ok_or_else(|| Failure::new("not_found", "no scripted status"))
    }
    fn models(&self, session: &str) -> Res<PassModels> {
        let mut result = None;
        self.record("models", PaperCall::Models(session.into()), |s| {
            result = Some(s.models.get(session).cloned().unwrap_or_default());
            Ok(())
        })?;
        Ok(result.unwrap())
    }
    fn set_models(&self, session: &str, m: PassModels) -> Res<()> {
        self.record(
            "set_models",
            PaperCall::SetModels(session.into(), m.clone()),
            |s| {
                m.validate()?;
                s.models.insert(session.into(), m);
                Ok(())
            },
        )
    }
    fn request_shortlist(&self, session: &str, input: ShortlistInput) -> Res<()> {
        self.record(
            "request_shortlist",
            PaperCall::RequestShortlist(session.into(), input.requirements, input.jcr),
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
                status.shortlist = list.rows;
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
    fn estimate(&self, session: &str) -> Res<EstimateRow> {
        let mut result = None;
        self.record("estimate", PaperCall::Estimate(session.into()), |s| {
            result = s.estimates.get(session).cloned();
            Ok(())
        })?;
        result.ok_or_else(|| Failure::new("not_found", "no scripted estimate"))
    }
    fn estimate_is_fresh(&self, session: &str, digest: &str) -> Res<bool> {
        let state = self.state.lock().unwrap();
        let current = state
            .estimates
            .get(session)
            .map(|estimate| estimate.digest.as_str())
            .or_else(|| {
                state
                    .statuses
                    .get(session)
                    .and_then(|status| status.estimate.as_ref())
                    .map(|estimate| estimate.digest.as_str())
            });
        Ok(current == Some(digest))
    }
    fn start(&self, session: &str, input: StartInput) -> Res<()> {
        self.record(
            "start",
            PaperCall::Start(session.into(), input.estimate_digest),
            |_| Ok(()),
        )
    }
    fn cancel(&self, session: &str) -> Res<()> {
        self.record("cancel", PaperCall::Cancel(session.into()), |_| Ok(()))
    }
    fn preview(&self, session: &str, input: PreviewInput) -> Res<Preview> {
        let mut result = None;
        self.record(
            "preview",
            PaperCall::Preview(session.into(), input.page, input.width_px),
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
