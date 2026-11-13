//! UI-facing research data and non-blocking commands.
use crate::base::Res;
use std::path::PathBuf;

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
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapDisplay {
    pub title: String,
    pub status: String,
    pub stages: Vec<MapStage>,
    pub funnel: Vec<FunnelNode>,
    pub notices: Vec<ResearchNotice>,
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceItem {
    pub id: String,
    pub record_id: String,
    pub layer: String,
    pub quote: String,
    pub locator: String,
    pub image: Option<PathBuf>,
    pub notice: Option<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EvidenceDisplay {
    pub layers: Vec<String>,
    pub entities: Vec<String>,
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
    pub cells: Vec<String>,
    pub evidence: Vec<String>,
    pub flags: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DatasetDisplay {
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
    pub notice: Option<ResearchNotice>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExportsDisplay {
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

/// Implementations enqueue commands; disk, indexing and rendering never run in the caller.
pub trait ResearchPort: Send + Sync {
    fn open(&self, root: PathBuf, store: Option<PathBuf>);
    fn refresh(&self);
    fn sample(&self, node: String, count: usize, seed: u64);
    fn correct(&self, correction: Correction);
    fn export_csv(&self, destination: PathBuf);
    fn export_bundle(&self, destination: PathBuf);
    fn export_snapshot(&self, destination: PathBuf);
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
