use crate::{
    base::Failure,
    desktop::{
        i18n::{translate, Lang, Table},
        model::AppState,
        theme::Tokened as _,
        ux::seams::Interjections,
    },
    research_workspace::{
        index::{Filter, Index, IndexEvent, Indexer, Op},
        manifest::{locate, LocateOptions, Located, ManifestSource},
        pdf::PdfiumProcess,
        read::Limits,
        seams::{PageRenderer, SnapshotConsumer, VaultActions},
        session::{self, ResearchRef, SessionPrefs, TabView},
        stages::{self, ResearchMap, StageState},
    },
    seams::{Clock, SystemClock},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        button::Button,
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    *,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex, OnceLock},
};

#[cfg(test)]
mod accept;
pub mod actions;
#[cfg(test)]
mod actions_accept;
mod dataset;
#[cfg(test)]
mod dataset_accept;
mod evidence;
#[cfg(test)]
mod evidence_accept;
mod exports;
#[cfg(test)]
mod exports_accept;
mod map;
#[cfg(test)]
mod map_accept;
mod sources;
#[cfg(test)]
mod sources_accept;

#[derive(Clone, Default)]
pub struct ResearchSeams {
    pub research_home: Option<PathBuf>,
    pub orch_home: Option<PathBuf>,
    pub goals_home: Option<PathBuf>,
    pub renderer: Option<Arc<dyn PageRenderer>>,
    pub snapshot_consumer: Option<Arc<dyn SnapshotConsumer>>,
    pub vault: Option<Arc<dyn VaultActions>>,
    pub page_cache_bytes: u64,
}
impl ResearchSeams {
    pub fn production(axiom_home: &Path) -> Self {
        let research_home = axiom_home.join("research");
        Self {
            renderer: Some(Arc::new(PdfiumProcess::from_env(&research_home))),
            research_home: Some(research_home),
            orch_home: Some(axiom_home.join("orchestrate")),
            goals_home: Some(axiom_home.to_path_buf()),
            snapshot_consumer: None,
            vault: None,
            page_cache_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Debug)]
pub enum LoadState {
    NoResearch,
    Opening,
    Indexing { done: usize, total: usize },
    Ready,
    Failed { kind: String, message: String },
}

enum WorkEvent {
    Located(Arc<Located>),
    Progress(usize, usize),
    Ready(Arc<Index>, ResearchMap),
    Failed(Failure),
}
// The worker owns the indexer: its blocking stop/join never runs on the UI thread.
fn index_research(
    r: ResearchRef,
    orch_home: Option<PathBuf>,
    now_ms: i64,
    out: mpsc::Sender<WorkEvent>,
) {
    let located = match locate(
        &r.root,
        &LocateOptions {
            store: r.store,
            orch_home,
        },
    ) {
        Ok(l) => Arc::new(l),
        Err(e) => {
            let _ = out.send(WorkEvent::Failed(e));
            return;
        }
    };
    let _ = out.send(WorkEvent::Located(located.clone()));
    let (tx, rx) = mpsc::channel();
    let progress = out.clone();
    let indexer = Indexer::start(
        located,
        Limits::default(),
        Box::new(move |event| {
            if let IndexEvent::Progress { done, total } = event {
                let _ = progress.send(WorkEvent::Progress(done, total));
            } else {
                let _ = tx.send(event);
            }
        }),
    );
    match rx.recv() {
        Ok(IndexEvent::Ready { .. }) => {
            if let Some(index) = indexer.latest() {
                let map = stages::research_map(&index, now_ms);
                let _ = out.send(WorkEvent::Ready(index, map));
            }
        }
        Ok(IndexEvent::Failed(e)) => {
            let _ = out.send(WorkEvent::Failed(e));
        }
        _ => {
            let _ = out.send(WorkEvent::Failed(Failure::new("index", "indexer stopped")));
        }
    }
}

// All instances for one home share an ordered queue, including reopen/load.
enum SessionCommand {
    Load(mpsc::Sender<SessionPrefs>),
    Save(SessionPrefs),
}
fn session_queue(home: &Path) -> mpsc::Sender<SessionCommand> {
    static QUEUES: OnceLock<Mutex<HashMap<PathBuf, mpsc::Sender<SessionCommand>>>> =
        OnceLock::new();
    let mut queues = QUEUES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    queues
        .entry(home.to_path_buf())
        .or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let home = home.to_path_buf();
            std::thread::spawn(move || {
                for command in rx {
                    match command {
                        SessionCommand::Load(reply) => {
                            let _ = reply.send(session::load(&home));
                        }
                        SessionCommand::Save(prefs) => {
                            let _ = session::save(&home, &prefs);
                        }
                    }
                }
            });
            tx
        })
        .clone()
}
fn restore_tab(prefs: &SessionPrefs, id: &str) -> Tab {
    prefs
        .views
        .get(id)
        .and_then(|v| Tab::from_key(&v.tab))
        .unwrap_or(Tab::Map)
}
fn update_view(prefs: &mut SessionPrefs, id: &str, tab: Tab, f: impl FnOnce(&mut TabView)) {
    f(prefs
        .views
        .entry(id.to_owned())
        .or_default()
        .tabs
        .entry(tab.key().into())
        .or_default());
}

pub struct Workbench {
    pub seams: ResearchSeams,
    pub interjections: Option<Arc<dyn Interjections>>,
    pub located: Option<Arc<Located>>,
    pub index: Option<Arc<Index>>,
    pub map: Option<ResearchMap>,
    pub load: LoadState,
    pub tab: Tab,
    pub prefs: SessionPrefs,
    pub clock: Arc<dyn Clock>,
    current: Option<ResearchRef>,
    pending: Option<ResearchRef>,
    loading_prefs: bool,
    generation: u64,
    saves: Option<mpsc::Sender<SessionCommand>>,
    notice_text: Option<String>,
}
impl Workbench {
    pub fn new(
        seams: ResearchSeams,
        interjections: Option<Arc<dyn Interjections>>,
        clock: Arc<dyn Clock>,
        cx: &mut Context<Self>,
    ) -> Self {
        let loading_prefs = seams.research_home.is_some();
        let saves = seams.research_home.as_deref().map(session_queue);
        if let Some(queue) = &saves {
            let (reply, response) = mpsc::channel();
            let _ = queue.send(SessionCommand::Load(reply));
            let task = cx.background_spawn(async move { response.recv().unwrap_or_default() });
            cx.spawn(async move |this, cx| {
                let prefs = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.prefs = prefs;
                    this.loading_prefs = false;
                    let next = this.pending.take().or_else(|| this.prefs.last.clone());
                    if let Some(r) = next {
                        this.open(r, cx);
                    } else {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        Self {
            seams,
            interjections,
            located: None,
            index: None,
            map: None,
            load: LoadState::NoResearch,
            tab: Tab::Map,
            prefs: SessionPrefs::default(),
            clock,
            current: None,
            pending: None,
            loading_prefs,
            generation: 0,
            saves,
            notice_text: None,
        }
    }
    fn save(&self) {
        if let Some(tx) = &self.saves {
            let _ = tx.send(SessionCommand::Save(self.prefs.clone()));
        }
    }
    pub fn open(&mut self, r: ResearchRef, cx: &mut Context<Self>) {
        self.notice_text = None;
        if self.loading_prefs {
            self.pending = Some(r);
            self.load = LoadState::Opening;
            cx.notify();
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.current = Some(r.clone());
        self.located = None;
        self.index = None;
        self.map = None;
        self.tab = Tab::Map;
        self.load = LoadState::Opening;
        session::remember(&mut self.prefs, r.clone());
        self.save();
        let (tx, rx) = mpsc::channel();
        let orch = self.seams.orch_home.clone();
        let now = self.clock.now_ms();
        std::thread::spawn(move || index_research(r, orch, now, tx));
        cx.spawn(async move |this, cx| {
            let mut rx = rx;
            loop {
                let task = cx.background_executor().spawn(async move {
                    let result = rx.recv();
                    (rx, result)
                });
                let (receiver, result) = task.await;
                rx = receiver;
                let Ok(event) = result else { break };
                let terminal = matches!(event, WorkEvent::Ready(..) | WorkEvent::Failed(..));
                let active = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        match event {
                            WorkEvent::Located(l) => {
                                this.tab = restore_tab(&this.prefs, &l.manifest.research_id);
                                this.located = Some(l);
                                this.load = LoadState::Indexing { done: 0, total: 0 };
                            }
                            WorkEvent::Progress(done, total) => {
                                this.load = LoadState::Indexing { done, total }
                            }
                            WorkEvent::Ready(index, map) => {
                                this.index = Some(index);
                                this.map = Some(map);
                                this.load = LoadState::Ready;
                            }
                            WorkEvent::Failed(e) => {
                                this.load = LoadState::Failed {
                                    kind: e.kind,
                                    message: e.message,
                                }
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if terminal || !active {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = self.current.clone() {
            self.open(r, cx);
        }
    }
    pub fn set_tab(&mut self, t: Tab, cx: &mut Context<Self>) {
        self.tab = t;
        if let Some(l) = &self.located {
            self.prefs
                .views
                .entry(l.manifest.research_id.clone())
                .or_default()
                .tab = t.key().into();
            self.save();
        }
        cx.notify();
    }
    pub fn view(&self, t: Tab) -> TabView {
        self.located
            .as_ref()
            .and_then(|l| self.prefs.views.get(&l.manifest.research_id))
            .and_then(|v| v.tabs.get(t.key()))
            .cloned()
            .unwrap_or_default()
    }
    pub fn set_view(&mut self, t: Tab, f: impl FnOnce(&mut TabView), cx: &mut Context<Self>) {
        if let Some(l) = &self.located {
            update_view(&mut self.prefs, &l.manifest.research_id, t, f);
            self.save();
            cx.notify();
        }
    }
    pub fn open_evidence(&mut self, evidence_id: &str, cx: &mut Context<Self>) {
        self.set_view(
            Tab::Evidence,
            |v| v.selection = Some(evidence_id.into()),
            cx,
        );
        self.set_tab(Tab::Evidence, cx);
    }
    pub fn show_rows(&mut self, filters: Vec<Filter>, cx: &mut Context<Self>) {
        self.set_view(
            Tab::Dataset,
            |v| {
                v.filters = filters;
                v.offset = 0;
            },
            cx,
        );
        self.set_tab(Tab::Dataset, cx);
    }
    pub fn notice(&mut self, text: String, cx: &mut Context<Self>) {
        self.notice_text = Some(text);
        cx.notify();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryAction {
    Open,
    Retry,
    Blocker,
    Freeze,
}
fn primary_action(load: &LoadState, has_map: bool, blocked: bool) -> PrimaryAction {
    if matches!(load, LoadState::Failed { .. }) {
        PrimaryAction::Retry
    } else if !matches!(load, LoadState::Ready) || !has_map {
        PrimaryAction::Open
    } else if blocked {
        PrimaryAction::Blocker
    } else {
        PrimaryAction::Freeze
    }
}
impl PrimaryAction {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open a research folder",
            Self::Retry => "Retry opening",
            Self::Blocker => "Show what is blocking",
            Self::Freeze => "Freeze a snapshot",
        }
    }
}

const T: Table = &[
    ("Research", "研究"),
    ("Back", "返回"),
    ("Research folder", "研究資料夾"),
    ("Enter a research folder", "請輸入研究資料夾"),
    ("Store folder (optional)", "儲存資料夾（選填）"),
    ("Open", "開啟"),
    ("Open a research folder", "開啟研究資料夾"),
    ("Show what is blocking", "顯示阻礙事項"),
    ("Retry opening", "重試開啟"),
    ("Freeze a snapshot", "凍結快照"),
    ("Reading the research...", "正在讀取研究…"),
    (
        "Nothing is blocking this research",
        "目前沒有阻礙這項研究的事項",
    ),
    ("Map", "地圖"),
    ("Sources", "來源"),
    ("Evidence", "證據"),
    ("Dataset", "資料集"),
    ("Exports", "匯出"),
    ("No research open", "尚未開啟研究"),
    ("Opening", "正在開啟"),
    ("Indexing", "正在建立索引"),
    ("indexer stopped", "索引器已停止"),
    ("Ready", "已就緒"),
    ("Failed", "失敗"),
    ("Manifest file", "資訊清單檔案"),
    ("Adapter", "介接器"),
    ("Not started", "尚未開始"),
    ("Paused", "已暫停"),
    ("Blocked", "受阻"),
    ("Rate limited", "達到速率限制"),
    ("Stalled", "已停滯"),
    ("Recent", "最近開啟"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}
fn load_text(load: &LoadState, lang: Lang) -> String {
    match load {
        LoadState::NoResearch => tr(lang, "No research open").into(),
        LoadState::Opening => tr(lang, "Opening").into(),
        LoadState::Indexing { done, total } => format!("{} {done}/{total}", tr(lang, "Indexing")),
        LoadState::Ready => tr(lang, "Ready").into(),
        LoadState::Failed { kind, message } => {
            let detail = if kind == "index" && message == "indexer stopped" {
                tr(lang, "indexer stopped")
            } else {
                message.as_str()
            };
            format!("{}: {kind} — {detail}", tr(lang, "Failed"))
        }
    }
}
fn view_state_text(view: &TabView, lang: Lang) -> Option<String> {
    if view.filters.is_empty() && view.offset == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if !view.filters.is_empty() {
        let filters = view
            .filters
            .iter()
            .map(|filter| {
                let value = match &filter.op {
                    Op::Eq(value) | Op::Contains(value) => value.clone(),
                    Op::In(values) => values.join(", "),
                    Op::Empty => match lang {
                        Lang::En => "empty",
                        Lang::ZhTw => "空白",
                    }
                    .into(),
                    Op::NotEmpty => match lang {
                        Lang::En => "not empty",
                        Lang::ZhTw => "非空白",
                    }
                    .into(),
                };
                format!("{} = {value}", filter.column)
            })
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(match lang {
            Lang::En => format!("{} filter(s): {filters}", view.filters.len()),
            Lang::ZhTw => format!("{} 個篩選條件：{filters}", view.filters.len()),
        });
    }
    if view.offset != 0 {
        let item = view.offset as u128 + 1;
        parts.push(match lang {
            Lang::En => format!("starting at item {item}"),
            Lang::ZhTw => format!("從第 {item} 項開始"),
        });
    }
    Some(match lang {
        Lang::En => format!("Restored view: {}", parts.join(" ; ")),
        Lang::ZhTw => format!("已還原檢視：{}", parts.join("；")),
    })
}
fn tab_label(t: Tab) -> &'static str {
    match t {
        Tab::Map => "Map",
        Tab::Sources => "Sources",
        Tab::Evidence => "Evidence",
        Tab::Dataset => "Dataset",
        Tab::Exports => "Exports",
    }
}
fn blocker_text(map: &ResearchMap, lang: Lang) -> Option<String> {
    let (id, state) = stages::blocker(map)?;
    let label = map
        .stages
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.label.as_str())
        .unwrap_or(&id);
    let detail = match state {
        StageState::NotStarted => tr(lang, "Not started").to_string(),
        StageState::Paused { reason } => format!("{}: {}", tr(lang, "Paused"), reason),
        StageState::Blocked { reason } => format!("{}: {}", tr(lang, "Blocked"), reason),
        StageState::Failed { reason } => format!("{}: {}", tr(lang, "Failed"), reason),
        StageState::RateLimited { .. } => tr(lang, "Rate limited").to_string(),
        StageState::Stalled { since } => format!("{}: {}", tr(lang, "Stalled"), since),
        StageState::Running | StageState::Done => return None,
    };
    Some(format!("{label}: {detail}"))
}
fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}

fn form_ref(root: &str, store: &str) -> Option<ResearchRef> {
    let root = root.trim();
    if root.is_empty() {
        return None;
    }
    let store = store.trim();
    Some(ResearchRef {
        root: root.into(),
        store: (!store.is_empty()).then(|| PathBuf::from(store)),
    })
}

pub struct ResearchPage {
    state: Entity<AppState>,
    wb: Entity<Workbench>,
    path: Entity<InputState>,
    store: Entity<InputState>,
    map: Entity<map::MapPanel>,
    sources: Entity<sources::SourcesPanel>,
    evidence: Entity<evidence::EvidencePanel>,
    dataset: Entity<dataset::DatasetPanel>,
    exports: Entity<exports::ExportsPanel>,
}
impl ResearchPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let seams = state.read(cx).ux.research.clone();
        let interjections = state.read(cx).ux.seams.interjections.clone();
        let wb = cx.new(|cx| Workbench::new(seams, interjections, Arc::new(SystemClock), cx));
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        let path = cx.new(|cx| InputState::new(window, cx));
        let store = cx.new(|cx| InputState::new(window, cx));
        let map = cx.new(|cx| map::MapPanel::new(wb.clone(), window, cx));
        let sources = cx.new(|cx| sources::SourcesPanel::new(wb.clone(), window, cx));
        let evidence = cx.new(|cx| evidence::EvidencePanel::new(wb.clone(), window, cx));
        let dataset = cx.new(|cx| dataset::DatasetPanel::new(wb.clone(), window, cx));
        let exports = cx.new(|cx| exports::ExportsPanel::new(wb.clone(), window, cx));
        Self {
            state,
            wb,
            path,
            store,
            map,
            sources,
            evidence,
            dataset,
            exports,
        }
    }
    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.path.read(cx).value().to_string();
        let store = self.store.read(cx).value().to_string();
        let Some(r) = form_ref(&root, &store) else {
            let message = tr(*cx.global::<Lang>(), "Enter a research folder").to_owned();
            self.wb.update(cx, |wb, cx| wb.notice(message, cx));
            self.path.read(cx).focus_handle(cx).focus(window, cx);
            return;
        };
        self.wb.update(cx, |wb, cx| wb.open(r, cx));
    }
}
impl Render for ResearchPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let colors = cx.tokens();
        let wb = self.wb.read(cx);
        let ready_map = if matches!(wb.load, LoadState::Ready) {
            wb.map.as_ref()
        } else {
            None
        };
        let blocked = ready_map.and_then(|map| blocker_text(map, lang));
        let primary = primary_action(&wb.load, ready_map.is_some(), blocked.is_some());
        let blocker = if ready_map.is_some() {
            blocked.unwrap_or_else(|| tr(lang, "Nothing is blocking this research").into())
        } else if matches!(
            wb.load,
            LoadState::Opening | LoadState::Indexing { .. } | LoadState::Ready
        ) {
            tr(lang, "Reading the research...").into()
        } else {
            load_text(&wb.load, lang)
        };
        let title = wb
            .located
            .as_ref()
            .map(|l| l.manifest.title.clone())
            .unwrap_or_else(|| tr(lang, "Research").into());
        let info = wb.located.as_ref().map(|l| {
            let source = match &l.source {
                ManifestSource::File { .. } => tr(lang, "Manifest file").to_string(),
                ManifestSource::Adapter { name } => format!("{}: {name}", tr(lang, "Adapter")),
            };
            format!("{} · {}", l.manifest.research_version, source)
        });
        let load = load_text(&wb.load, lang);
        let current = wb.tab;
        let view_state = view_state_text(&wb.view(current), lang);
        let recent = wb.prefs.recent.clone();
        let notice = wb.notice_text.clone();
        let panel: AnyElement = match current {
            Tab::Map => self.map.clone().into_any_element(),
            Tab::Sources => self.sources.clone().into_any_element(),
            Tab::Evidence => self.evidence.clone().into_any_element(),
            Tab::Dataset => self.dataset.clone().into_any_element(),
            Tab::Exports => self.exports.clone().into_any_element(),
        };
        let mut rail = h_flex().gap(px(8.)).border_b_1().border_color(colors.orbit);
        for tab in [
            Tab::Map,
            Tab::Sources,
            Tab::Evidence,
            Tab::Dataset,
            Tab::Exports,
        ] {
            let id = format!("research-tab-{}", tab.key());
            let handle = self.wb.clone();
            rail = rail.child(
                div()
                    .id(SharedString::from(id))
                    .test_support()
                    .cursor_pointer()
                    .p(px(8.))
                    .text_color(if current == tab {
                        colors.gold
                    } else {
                        colors.text
                    })
                    .when(current == tab, |d| d.border_b_2().border_color(colors.gold))
                    .child(tr(lang, tab_label(tab)).test_text(tr(lang, tab_label(tab))))
                    .on_click(move |_, _, cx| handle.update(cx, |wb, cx| wb.set_tab(tab, cx))),
            );
        }
        let mut list = v_flex();
        for (i, r) in recent.into_iter().enumerate() {
            let title = r.root.display().to_string();
            let handle = self.wb.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("research-recent-{i}")))
                    .test_support()
                    .cursor_pointer()
                    .child(title.clone().test_text(title))
                    .on_click(move |_, _, cx| handle.update(cx, |wb, cx| wb.open(r.clone(), cx))),
            );
        }
        v_flex()
            .id("research-page")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .gap(px(10.))
            .text_color(colors.text)
            .child(
                Button::new("research-back")
                    .label(tr(lang, "Back"))
                    .on_click({
                        let state = self.state.clone();
                        move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx))
                    }),
            )
            .child(title)
            .children(info)
            .child(
                div()
                    .when(ready_map.is_some(), |d| {
                        d.text_color(colors.gold).child("●")
                    })
                    .child(marked("research-blocker", blocker)),
            )
            .child(
                Button::new("research-primary")
                    .label(tr(lang, primary.label()))
                    .on_click(cx.listener(move |this, _, window, cx| match primary {
                        PrimaryAction::Retry => this.wb.update(cx, |wb, cx| wb.refresh(cx)),
                        PrimaryAction::Blocker => {
                            this.wb.update(cx, |wb, cx| wb.set_tab(Tab::Map, cx))
                        }
                        PrimaryAction::Freeze => {
                            this.wb.update(cx, |wb, cx| wb.set_tab(Tab::Exports, cx))
                        }
                        PrimaryAction::Open => {
                            this.path.read(cx).focus_handle(cx).focus(window, cx)
                        }
                    })),
            )
            .child(marked("research-load", load))
            .children(notice)
            .child(rail)
            .children(view_state.map(|text| marked("research-view-state", text)))
            .child(panel)
            .child(tr(lang, "Research folder"))
            .child(Input::new(&self.path).id("research-open-path"))
            .child(tr(lang, "Store folder (optional)"))
            .child(Input::new(&self.store).id("research-open-store"))
            .child(
                Button::new("research-open-button")
                    .label(tr(lang, "Open"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx))),
            )
            .child(tr(lang, "Recent"))
            .child(list)
    }
}
