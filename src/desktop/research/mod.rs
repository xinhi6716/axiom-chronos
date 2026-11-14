use crate::desktop::{
    components::{Button, TextField},
    i18n::{translate, Lang, Table},
    model::AppState,
    theme::{self, Density, Step, Tokened as _},
    ux::seams::Interjections,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::input::InputState,
    *,
};
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
};

#[cfg(test)]
use crate::base::Failure;
#[cfg(test)]
use crate::research_workspace::{
    manifest::{locate, LocateOptions},
    read::Limits,
};
#[cfg(test)]
use std::{path::Path, sync::Mutex};

pub mod adapter;

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

pub use crate::desktop::ux::seams::research::{
    Filter, LoadState, ManifestSource, ResearchRef, SessionPrefs, Tab, TabView,
};
pub use adapter::ResearchSeams;

#[cfg(test)]
mod production_seams_tests {
    use super::ResearchSeams;
    use std::{ffi::OsString, sync::Mutex};

    static CURL_ENV: Mutex<()> = Mutex::new(());

    struct RestoreEnv(Option<OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("AXIOM_CURL_CMD", value),
                None => std::env::remove_var("AXIOM_CURL_CMD"),
            }
        }
    }

    #[test]
    fn production_seams_install_paper_without_vault() {
        let _lock = CURL_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = RestoreEnv(std::env::var_os("AXIOM_CURL_CMD"));
        let executable = std::env::current_exe().unwrap();
        std::env::set_var(
            "AXIOM_CURL_CMD",
            serde_json::to_string(&[executable]).unwrap(),
        );
        let tmp = tempfile::tempdir().unwrap();
        let seams = ResearchSeams::production(tmp.path());
        assert_eq!(seams.snapshot_consumer.unwrap().name(), "paper");
        assert!(seams.vault.is_none());
    }

    #[test]
    fn production_seams_skip_paper_when_curl_config_is_invalid() {
        let _lock = CURL_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = RestoreEnv(std::env::var_os("AXIOM_CURL_CMD"));
        std::env::set_var("AXIOM_CURL_CMD", "invalid-json");
        let tmp = tempfile::tempdir().unwrap();
        let seams = ResearchSeams::production(tmp.path());
        assert!(seams.snapshot_consumer.is_none());
        assert!(seams.vault.is_none());
    }

    #[test]
    fn production_seams_skip_paper_when_curl_command_is_missing() {
        let _lock = CURL_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let _restore = RestoreEnv(std::env::var_os("AXIOM_CURL_CMD"));
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nonexistent-curl-executable");
        std::env::set_var("AXIOM_CURL_CMD", serde_json::to_string(&[missing]).unwrap());
        let seams = ResearchSeams::production(tmp.path());
        assert!(seams.snapshot_consumer.is_none());
        assert!(seams.vault.is_none());
    }
}

use crate::desktop::ux::seams::research::OpenEvent;
#[cfg(test)]
use crate::research_workspace::index::Index;
use adapter::{session_queue, Clock, SessionCommand};

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
    pub port: Arc<dyn crate::desktop::ux::seams::research::ResearchPort>,
    map_adapter: adapter::ResearchAdapter,
    map_pending: bool,
    pub map_feed: crate::desktop::data::Feed<crate::desktop::ux::seams::research::MapDisplay>,
    pub sources_feed:
        crate::desktop::data::Feed<crate::desktop::ux::seams::research::SourcesDisplay>,
    pub evidence_feed:
        crate::desktop::data::Feed<crate::desktop::ux::seams::research::EvidenceDisplay>,
    pub dataset_feed:
        crate::desktop::data::Feed<crate::desktop::ux::seams::research::DatasetDisplay>,
    pub exports_feed:
        crate::desktop::data::Feed<crate::desktop::ux::seams::research::ExportsDisplay>,
    pub seams: ResearchSeams,
    pub interjections: Option<Arc<dyn Interjections>>,
    pub located: Option<crate::desktop::ux::seams::research::ResearchInfo>,
    blocker: Option<(Option<String>, Option<String>)>,
    persistence_safe: bool,
    pub load: LoadState,
    pub tab: Tab,
    target_tab: Option<Tab>,
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
        if cx
            .try_global::<crate::desktop::layout::persist::LayoutState>()
            .is_none()
        {
            if let Some(home) = seams.research_home.as_deref() {
                crate::desktop::layout::persist::init(home, cx);
            }
        }
        let adapter = adapter::ResearchAdapter::new(seams.clone(), interjections.clone());
        let feeds = adapter.feeds(cx);
        feeds.sources.observe(cx);
        feeds.evidence.observe(cx);
        feeds.dataset.observe(cx);
        feeds.exports.observe(cx);
        let port: Arc<dyn crate::desktop::ux::seams::research::ResearchPort> =
            Arc::new(adapter.clone());
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
            port,
            map_adapter: adapter,
            map_pending: false,
            map_feed: feeds.map,
            sources_feed: feeds.sources,
            evidence_feed: feeds.evidence,
            dataset_feed: feeds.dataset,
            exports_feed: feeds.exports,
            seams,
            interjections,
            located: None,
            blocker: None,
            persistence_safe: false,
            load: LoadState::NoResearch,
            tab: Tab::Map,
            target_tab: None,
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
    pub fn open_paths(&mut self, root: PathBuf, store: Option<PathBuf>, cx: &mut Context<Self>) {
        self.open(ResearchRef { root, store }, cx);
    }
    pub fn open(&mut self, r: ResearchRef, cx: &mut Context<Self>) {
        self.notice_text = None;
        self.map_pending = true;
        if self.loading_prefs {
            self.pending = Some(r);
            self.load = LoadState::Opening;
            cx.notify();
            return;
        }
        let preview_ready = self
            .map_adapter
            .after_preview(cx.background_executor().is_test());
        let (tx, rx) = mpsc::channel();
        self.port
            .open_observed(r.root.clone(), r.store.clone(), self.clock.now_ms(), tx);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            preview_ready.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    let dataset = this.map_adapter.dataset_feed(cx);
                    dataset.observe(cx);
                    this.dataset_feed = dataset;
                    cx.notify();
                }
            });
        })
        .detach();
        let ready = self
            .map_adapter
            .after_map(cx.background_executor().is_test());
        cx.spawn(async move |this, cx| {
            ready.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    let feed = this.map_adapter.map_feed(cx);
                    feed.observe(cx);
                    this.map_feed = feed;
                    let sources = this.map_adapter.sources_feed(cx);
                    let evidence = this.map_adapter.evidence_feed(cx);
                    sources.observe(cx);
                    evidence.observe(cx);
                    this.sources_feed = sources;
                    this.evidence_feed = evidence;
                    let dataset = this.map_adapter.dataset_feed(cx);
                    let exports = this.map_adapter.exports_feed(cx);
                    dataset.observe(cx);
                    exports.observe(cx);
                    this.dataset_feed = dataset;
                    this.exports_feed = exports;
                    this.map_pending = false;
                    cx.notify();
                }
            });
        })
        .detach();
        self.current = Some(r.clone());
        self.located = None;
        self.blocker = None;
        self.persistence_safe = false;
        self.tab = Tab::Map;
        self.load = LoadState::Opening;
        self.port.remember(&mut self.prefs, r.clone());
        self.save();
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
                let terminal = matches!(event, OpenEvent::Ready { .. } | OpenEvent::Failed(..));
                let active = this
                    .update(cx, |this, cx| {
                        if this.generation != generation {
                            return false;
                        }
                        match event {
                            OpenEvent::Located(l, total) => {
                                this.tab = this
                                    .target_tab
                                    .take()
                                    .unwrap_or_else(|| restore_tab(&this.prefs, &l.id));
                                this.located = Some(l);
                                this.load = LoadState::Indexing { done: 0, total };
                            }
                            OpenEvent::Progress(done, total) => {
                                this.load = LoadState::Indexing { done, total }
                            }
                            OpenEvent::Preview(_) => {}
                            OpenEvent::Ready {
                                blocker_en,
                                blocker_zh,
                                safe,
                            } => {
                                this.blocker = Some((blocker_en, blocker_zh));
                                this.persistence_safe = safe;
                                this.load = LoadState::Ready;
                            }
                            OpenEvent::Failed(e) => {
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
    pub(crate) fn select_target_tab(&mut self, t: Tab, cx: &mut Context<Self>) {
        self.set_tab(t, cx);
        if !matches!(self.load, LoadState::Ready) {
            self.target_tab = Some(t);
        }
    }
    pub fn set_tab(&mut self, t: Tab, cx: &mut Context<Self>) {
        self.target_tab = None;
        self.tab = t;
        if let Some(l) = &self.located {
            self.prefs.views.entry(l.id.clone()).or_default().tab = t.key().into();
            self.save();
        }
        cx.notify();
    }
    pub fn view(&self, t: Tab) -> TabView {
        self.located
            .as_ref()
            .and_then(|l| self.prefs.views.get(&l.id))
            .and_then(|v| v.tabs.get(t.key()))
            .cloned()
            .unwrap_or_default()
    }
    pub(crate) fn tab_view(&self, t: Tab) -> crate::desktop::ux::seams::research::TabView {
        self.view(t)
    }
    pub(crate) fn set_tab_view(
        &mut self,
        t: Tab,
        f: impl FnOnce(&mut crate::desktop::ux::seams::research::TabView),
        cx: &mut Context<Self>,
    ) {
        let mut view = self.tab_view(t);
        f(&mut view);
        self.set_view(t, |current| *current = view, cx);
    }
    pub fn set_view(&mut self, t: Tab, f: impl FnOnce(&mut TabView), cx: &mut Context<Self>) {
        if let Some(l) = &self.located {
            update_view(&mut self.prefs, &l.id, t, f);
            self.save();
            self.port.set_view(t, self.view(t));
            self.refresh_tab_feeds(cx);
            cx.notify();
        }
    }
    pub(crate) fn refresh_tab_feeds(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let ready = self
            .map_adapter
            .after_map(cx.background_executor().is_test());
        cx.spawn(async move |this, cx| {
            ready.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    let sources = this.map_adapter.sources_feed(cx);
                    let evidence = this.map_adapter.evidence_feed(cx);
                    sources.observe(cx);
                    evidence.observe(cx);
                    this.sources_feed = sources;
                    this.evidence_feed = evidence;
                    let dataset = this.map_adapter.dataset_feed(cx);
                    let exports = this.map_adapter.exports_feed(cx);
                    dataset.observe(cx);
                    exports.observe(cx);
                    this.dataset_feed = dataset;
                    this.exports_feed = exports;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub fn open_evidence(&mut self, evidence_id: &str, cx: &mut Context<Self>) {
        self.set_view(
            Tab::Evidence,
            |v| v.selection = Some(evidence_id.into()),
            cx,
        );
        self.set_tab(Tab::Evidence, cx);
    }
    pub(crate) fn layout_persistence_safe(&self) -> bool {
        self.persistence_safe
    }
    pub(crate) fn show_evidence_rows(&mut self, id: &str, cx: &mut Context<Self>) {
        self.show_rows(
            vec![adapter::display_filter(&adapter::evidence_filter(id))],
            cx,
        );
    }
    pub fn show_dataset(&mut self, cx: &mut Context<Self>) {
        self.show_rows(Vec::new(), cx);
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
fn view_state_text(
    view: &crate::desktop::ux::seams::research::TabView,
    lang: Lang,
) -> Option<String> {
    use crate::desktop::ux::seams::research::FilterOp as Op;
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
        let (wb, map, sources, evidence, dataset, exports) = adapter::compose(&state, window, cx);
        cx.observe(&wb, |_, _, cx| cx.notify()).detach();
        let path = cx.new(|cx| InputState::new(window, cx));
        let store = cx.new(|cx| InputState::new(window, cx));
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
            wb.blocker.as_ref()
        } else {
            None
        };
        let blocked = ready_map.and_then(|(en, zh)| match lang {
            Lang::En => en.clone(),
            Lang::ZhTw => zh.clone(),
        });
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
            .map(|l| l.title.clone())
            .unwrap_or_else(|| tr(lang, "Research").into());
        let info = wb.located.as_ref().map(|l| {
            let source = match &l.source {
                ManifestSource::File => tr(lang, "Manifest file").to_string(),
                ManifestSource::Adapter { name } => format!("{}: {name}", tr(lang, "Adapter")),
            };
            format!("{} · {}", l.version, source)
        });
        let load = load_text(&wb.load, lang);
        let current = wb.tab;
        let ready = matches!(wb.load, LoadState::Ready);
        let view_state = view_state_text(&wb.tab_view(current), lang);
        let recent = wb.prefs.recent.clone();
        let notice = wb.notice_text.clone();
        let panel: AnyElement = if matches!(wb.load, LoadState::Indexing { .. })
            && matches!(current, Tab::Sources | Tab::Evidence | Tab::Exports)
        {
            marked("research-indexing", load.clone()).into_any_element()
        } else {
            match current {
                Tab::Map => self.map.clone().into_any_element(),
                Tab::Sources => self.sources.clone().into_any_element(),
                Tab::Evidence => self.evidence.clone().into_any_element(),
                Tab::Dataset => self.dataset.clone().into_any_element(),
                Tab::Exports => self.exports.clone().into_any_element(),
            }
        };
        let mut rail = div()
            .flex()
            .gap(theme::space(Density::default(), Step::Sm))
            .border_b_1()
            .border_color(colors.orbit);
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
                    .id(SharedString::from(format!(
                        "action:research.{}.open",
                        tab.key()
                    )))
                    .test_support()
                    .child(
                        div()
                            .id(SharedString::from(id))
                            .test_support()
                            .cursor_pointer()
                            .p(theme::space(Density::default(), Step::Sm))
                            .text_color(if current == tab {
                                colors.gold
                            } else {
                                colors.text
                            })
                            .when(current == tab, |d| d.border_b_2().border_color(colors.gold))
                            .child(tr(lang, tab_label(tab)).test_text(tr(lang, tab_label(tab))))
                            .on_click(move |_, _, cx| {
                                handle.update(cx, |wb, cx| wb.set_tab(tab, cx))
                            }),
                    ),
            );
        }
        let mut list = div().flex().flex_col();
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
        div()
            .flex()
            .flex_col()
            .id("research-page")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .p(theme::space(Density::default(), Step::Xl))
            .gap(theme::space(Density::default(), Step::Md))
            .text_color(colors.text)
            .child(div().id("action:research.back").test_support().child(
                Button::new("research-back", tr(lang, "Back"), &colors).on_click({
                    let state = self.state.clone();
                    move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx))
                }),
            ))
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
                Button::new("research-primary", tr(lang, primary.label()), &colors).on_click(
                    cx.listener(move |this, _, window, cx| match primary {
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
                    }),
                ),
            )
            .child(marked("research-load", load))
            .children(notice)
            .child(rail)
            .children(if ready {
                view_state.map(|text| marked("research-view-state", text))
            } else {
                None
            })
            .child(
                div()
                    .id(SharedString::from(format!(
                        "research.{}.panel",
                        current.key()
                    )))
                    .test_support()
                    .w_full()
                    .child(panel),
            )
            .child(tr(lang, "Research folder"))
            .child({
                let path = self.path.clone();
                div()
                    .id("research-open-path")
                    .test_support()
                    .on_click(move |_, window, cx| path.read(cx).focus_handle(cx).focus(window, cx))
                    .child(TextField::new(
                        "research-open-path-field",
                        &self.path,
                        &colors,
                    ))
            })
            .child(tr(lang, "Store folder (optional)"))
            .child({
                let store = self.store.clone();
                div()
                    .id("research-open-store")
                    .test_support()
                    .on_click(move |_, window, cx| {
                        store.read(cx).focus_handle(cx).focus(window, cx)
                    })
                    .child(TextField::new(
                        "research-open-store-field",
                        &self.store,
                        &colors,
                    ))
            })
            .child(
                div().id("action:research.open").test_support().child(
                    Button::new("research-open-button", tr(lang, "Open"), &colors)
                        .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx))),
                ),
            )
            .child(tr(lang, "Recent"))
            .child(list)
    }
}
