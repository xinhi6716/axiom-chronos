//! UI-facing research data and non-blocking commands.
use crate::base::Res;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Map,
    Sources,
    Evidence,
    Dataset,
    Exports,
}
impl Tab {
    pub fn key(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Sources => "sources",
            Self::Evidence => "evidence",
            Self::Dataset => "dataset",
            Self::Exports => "exports",
        }
    }
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "map" => Some(Self::Map),
            "sources" => Some(Self::Sources),
            "evidence" => Some(Self::Evidence),
            "dataset" => Some(Self::Dataset),
            "exports" => Some(Self::Exports),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    NoResearch,
    Opening,
    Indexing { done: usize, total: usize },
    Ready,
    Failed { kind: String, message: String },
}
impl Default for LoadState {
    fn default() -> Self {
        Self::NoResearch
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterOp {
    Eq(String),
    Contains(String),
    In(Vec<String>),
    Empty,
    NotEmpty,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabView {
    pub offset: usize,
    pub filters: Vec<Filter>,
    pub selection: Option<String>,
    pub node: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResearchRef {
    pub root: PathBuf,
    pub store: Option<PathBuf>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    pub tab: String,
    pub tabs: BTreeMap<String, TabView>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionPrefs {
    pub last: Option<ResearchRef>,
    pub recent: Vec<ResearchRef>,
    pub views: BTreeMap<String, ViewState>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestSource {
    File,
    Adapter { name: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchInfo {
    pub id: String,
    pub title: String,
    pub version: String,
    pub source: ManifestSource,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpotVerdict {
    Confirm,
    Dispute { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reverify {
    Pending,
    Passed,
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrectionReceipt {
    pub stage: String,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrectionStatus {
    pub id: String,
    pub who: String,
    pub why: String,
    pub research_version: String,
    pub reverify: Reverify,
    pub invalidates: Vec<String>,
    pub receipt: Option<CorrectionReceipt>,
    pub draft: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResearchNotice {
    pub kind: String,
    pub message: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunnelNode {
    pub key: String,
    pub label: String,
    pub count: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapStage {
    pub id: String,
    pub label: String,
    pub state: String,
    pub gate: String,
    pub progress: Option<String>,
    pub stale: Vec<String>,
    pub text_en: String,
    pub text_zh: String,
    pub tone: String,
    pub current: bool,
    pub output: Option<Tab>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapDisplay {
    pub title: String,
    pub status: String,
    pub stages: Vec<MapStage>,
    pub funnel: Vec<FunnelNode>,
    pub notices: Vec<ResearchNotice>,
    pub status_zh: String,
    pub engineering_en: Option<String>,
    pub engineering_zh: Option<String>,
    pub repro_en: Option<String>,
    pub repro_zh: Option<String>,
    pub stale_en: Vec<String>,
    pub stale_zh: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceRow {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub title: String,
    pub doi: String,
    pub group: String,
    pub decision: String,
    pub page: Option<u32>,
    pub locate_state: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourcesDisplay {
    pub nodes: Vec<FunnelNode>,
    pub rows: Vec<SourceRow>,
    pub total: usize,
    pub offset: usize,
    pub sample: Vec<String>,
    pub notices: Vec<ResearchNotice>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvidenceLocator {
    PdfPage {
        physical_page: u32,
        printed_page: Option<String>,
        highlight: bool,
    },
    RawField {
        file: String,
        row: u64,
        column: String,
    },
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceRow {
    pub text: String,
    pub evidence: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceCondition {
    pub id: String,
    pub rows: Vec<EvidenceRow>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceContradiction {
    pub text: String,
    pub a: String,
    pub b: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceItem {
    pub id: String,
    pub record_id: String,
    pub layer: String,
    pub quote: String,
    pub locator: String,
    pub target: Option<EvidenceLocator>,
    pub image: Option<PathBuf>,
    pub notice: Option<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceDisplay {
    pub layers: Vec<String>,
    pub entities: Vec<String>,
    pub conditions: Vec<EvidenceCondition>,
    pub contradictions: Vec<EvidenceContradiction>,
    pub pdf_available: bool,
    pub items: Vec<EvidenceItem>,
    pub total: usize,
    pub offset: usize,
    pub notices: Vec<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatasetColumn {
    pub key: String,
    pub label: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatasetRow {
    pub id: String,
    pub layer: String,
    pub text_en: String,
    pub text_zh: String,
    pub cells: Vec<String>,
    pub evidence: Vec<String>,
    pub flags: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatasetDisplay {
    pub validation_en: String,
    pub validation_zh: String,
    pub missing: Vec<String>,
    pub columns: Vec<DatasetColumn>,
    pub rows: Vec<DatasetRow>,
    pub total: usize,
    pub offset: usize,
    pub preview: bool,
    pub filters: Vec<Filter>,
    pub notices: Vec<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportStatus {
    pub action: String,
    pub path: Option<PathBuf>,
    pub rows: usize,
    pub sha256: Option<String>,
    pub busy: bool,
    pub handoff: Option<String>,
    pub handoff_zh: Option<String>,
    pub notice: Option<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportsDisplay {
    pub repro_en: String,
    pub repro_zh: String,
    pub results: Vec<ExportStatus>,
    pub notices: Vec<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Correction {
    pub target: String,
    pub field: String,
    pub after: String,
    pub who: String,
    pub why: String,
}

#[derive(Clone, Debug)]
pub enum OpenEvent {
    Located(ResearchInfo, usize),
    Preview(DatasetDisplay),
    Progress(usize, usize),
    Ready {
        blocker_en: Option<String>,
        blocker_zh: Option<String>,
        safe: bool,
    },
    Failed(crate::base::Failure),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionSnapshot {
    pub session: bool,
    pub corrections: Vec<CorrectionStatus>,
}
#[derive(Clone, Debug)]
pub enum ActionRequest {
    Open,
    Spot {
        target: String,
        node: Option<String>,
        verdict: SpotVerdict,
    },
    Correct(Correction),
    Retry(String),
    Poll,
}

/// Implementations enqueue commands; disk, indexing and rendering never run in the caller.
pub trait ResearchPort: Send + Sync {
    fn open(&self, root: PathBuf, store: Option<PathBuf>);
    fn open_observed(
        &self,
        root: PathBuf,
        store: Option<PathBuf>,
        now_ms: i64,
        events: std::sync::mpsc::Sender<OpenEvent>,
    );
    fn remember(&self, prefs: &mut SessionPrefs, reference: ResearchRef);
    fn action(
        &self,
        request: ActionRequest,
        now_ms: i64,
        reply: std::sync::mpsc::Sender<Res<ActionSnapshot>>,
    );
    fn refresh(&self);
    fn sample(&self, node: String, count: usize, seed: u64);
    fn correct(&self, correction: Correction);
    fn export_csv(&self, destination: PathBuf);
    fn export_bundle(&self, destination: PathBuf);
    fn export_snapshot(&self, destination: PathBuf);
    fn select_entity(&self, entity: String);
    fn locate_page(&self, evidence_id: String);
    fn render_page(&self, evidence_id: String);
    fn set_view(&self, tab: Tab, view: TabView);
    fn set_filters(&self, tab: Tab, filters: Vec<Filter>);
    fn load_state(&self) -> LoadState;
    fn view(&self, tab: Tab) -> TabView;
    fn map(&self) -> Res<MapDisplay>;
    fn sources(&self) -> Res<SourcesDisplay>;
    fn evidence(&self) -> Res<EvidenceDisplay>;
    fn dataset(&self) -> Res<DatasetDisplay>;
    fn exports(&self) -> Res<ExportsDisplay>;
}
