pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}
impl<T: crate::seams::Clock> Clock for T {
    fn now_ms(&self) -> i64 {
        crate::seams::Clock::now_ms(self)
    }
}
pub(crate) mod actions;
pub(crate) mod dataset;
pub(crate) mod exports;
pub(crate) mod map_format;
use crate::research_workspace::{
    pdf::PdfiumProcess,
    seams::{PageRenderer, SnapshotConsumer, VaultActions},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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
            snapshot_consumer: crate::research_workspace::paper_handoff::PaperHandoff::production(
                axiom_home,
            )
            .ok()
            .map(|handoff| Arc::new(handoff) as Arc<dyn SnapshotConsumer>),
            vault: None,
            page_cache_bytes: 256 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod accept;

use crate::{
    base::{Failure, Res},
    desktop::{
        data::{Cursor, Delta, Source},
        ux::seams::research as dto,
    },
};
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex},
    time::Duration,
};

trait FeedValue: Clone + PartialEq + Send + 'static {
    fn append_delta(&self, newer: &Self) -> Option<Box<dyn FnOnce(&mut Self) + Send>>;
    fn capability(_error: &Failure) -> Option<Self> {
        None
    }
}
impl<T: Clone + PartialEq + Send + 'static> FeedValue for Vec<T> {
    fn append_delta(&self, newer: &Self) -> Option<Box<dyn FnOnce(&mut Self) + Send>> {
        newer.strip_prefix(self.as_slice()).map(|tail| {
            let tail = tail.to_vec();
            Box::new(move |rows: &mut Vec<T>| rows.extend(tail))
                as Box<dyn FnOnce(&mut Vec<T>) + Send>
        })
    }
}
macro_rules! display_feed {
    ($kind:ty, $rows:ident) => {
        impl FeedValue for $kind {
            fn capability(error: &Failure) -> Option<Self> {
                if error.kind != "not_capable"
                    && !(error.kind == "missing_artifact"
                        && error.message == "Measurements missing")
                {
                    return None;
                }
                let mut display = Self::default();
                display.notices.push(dto::ResearchNotice {
                    kind: error.kind.clone(),
                    message: error.message.clone(),
                });
                Some(display)
            }
            fn append_delta(&self, newer: &Self) -> Option<Box<dyn FnOnce(&mut Self) + Send>> {
                let mut new_meta = newer.clone();
                new_meta.$rows.clear();
                let tail = newer.$rows.strip_prefix(self.$rows.as_slice())?.to_vec();
                Some(Box::new(move |value: &mut Self| {
                    let mut rows = std::mem::take(&mut value.$rows);
                    rows.extend(tail);
                    *value = new_meta;
                    value.$rows = rows;
                }))
            }
        }
    };
}
display_feed!(dto::MapDisplay, stages);
display_feed!(dto::SourcesDisplay, rows);
display_feed!(dto::EvidenceDisplay, items);
display_feed!(dto::DatasetDisplay, rows);
display_feed!(dto::ExportsDisplay, results);

struct SlotInner<T> {
    version: u64,
    result: Res<T>,
}
struct Slot<T>(Arc<Mutex<SlotInner<T>>>);
impl<T> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: Default> Default for Slot<T> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(SlotInner {
            version: 1,
            result: Ok(T::default()),
        })))
    }
}
impl<T: FeedValue> Slot<T> {
    fn set(&self, result: Res<T>) {
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.result != result {
            inner.version = inner
                .version
                .checked_add(1)
                .expect("research cursor overflow");
            inner.result = result;
        }
    }
    fn replace(&self, value: T) {
        self.set(Ok(value));
    }
    fn fail(&self, kind: &str, message: &str) {
        self.set(Err(Failure::new(kind, message)));
    }
    fn snapshot(&self) -> Res<T> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .result
            .clone()
    }
    fn source(&self) -> SlotSource<T> {
        SlotSource {
            slot: self.clone(),
            previous: None,
        }
    }
}
impl<T: Clone + PartialEq + Send + 'static> Slot<Vec<T>> {
    #[cfg(test)]
    fn append(&self, tail: Vec<T>) {
        let mut rows = self.snapshot().unwrap();
        rows.extend(tail);
        self.replace(rows);
    }
}
struct SlotSource<T> {
    slot: Slot<T>,
    previous: Option<(Cursor, T)>,
}
impl<T: FeedValue> Source<T> for SlotSource<T> {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<T>, Cursor)> {
        let inner = self.slot.0.lock().unwrap_or_else(|e| e.into_inner());
        let next = Cursor(inner.version);
        let value = match inner.result.clone() {
            Ok(value) => value,
            Err(error) => T::capability(&error).ok_or(error)?,
        };
        if cursor == next {
            return Ok((Delta::Unchanged, next));
        }
        let delta = match &self.previous {
            Some((prior_cursor, prior)) if *prior_cursor == cursor => {
                prior.append_delta(&value).map(Delta::Patch)
            }
            _ => None,
        }
        .unwrap_or_else(|| Delta::Replace(value.clone()));
        self.previous = Some((next, value));
        Ok((delta, next))
    }
    fn missing_is_empty(&self) -> bool {
        true
    }
}
#[cfg(test)]
type SharedRows<T> = Slot<Vec<T>>;
fn spawn_worker(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

#[derive(Default)]
struct Outputs {
    page: Slot<Vec<()>>,
    map: Slot<dto::MapDisplay>,
    sources: Slot<dto::SourcesDisplay>,
    evidence: Slot<dto::EvidenceDisplay>,
    dataset: Slot<dto::DatasetDisplay>,
    exports: Slot<dto::ExportsDisplay>,
    load: Mutex<dto::LoadState>,
    views: Mutex<HashMap<dto::Tab, dto::TabView>>,
    preview_waiters: Mutex<Vec<MapSignal>>,
}

pub struct ResearchFeeds {
    pub page: crate::desktop::data::Feed<Vec<()>>,
    pub map: crate::desktop::data::Feed<dto::MapDisplay>,
    pub sources: crate::desktop::data::Feed<dto::SourcesDisplay>,
    pub evidence: crate::desktop::data::Feed<dto::EvidenceDisplay>,
    pub dataset: crate::desktop::data::Feed<dto::DatasetDisplay>,
    pub exports: crate::desktop::data::Feed<dto::ExportsDisplay>,
}

/// A single ordered background worker owns all engine operations.
#[derive(Clone)]
pub struct ResearchAdapter {
    sender: mpsc::Sender<Command>,
    outputs: Arc<Outputs>,
}
#[derive(Default)]
struct MapWaitState {
    ready: bool,
    deterministic: bool,
    waker: Option<std::task::Waker>,
}
struct MapSignal(Arc<Mutex<MapWaitState>>);
impl MapSignal {
    fn notify(self) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.ready = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}
pub(crate) struct MapWaiter(Arc<Mutex<MapWaitState>>);
impl std::future::Future for MapWaiter {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.ready {
            std::task::Poll::Ready(())
        } else {
            if state.deterministic {
                // The deterministic GPUI scheduler cannot be awakened from an OS worker.
                cx.waker().wake_by_ref();
            } else {
                state.waker = Some(cx.waker().clone());
            }
            std::task::Poll::Pending
        }
    }
}
enum Command {
    #[cfg(test)]
    Probe(Box<dyn FnOnce() + Send>),
    Open(
        PathBuf,
        Option<PathBuf>,
        Vec<MapSignal>,
        Option<(mpsc::Sender<dto::OpenEvent>, i64)>,
    ),
    Action(
        dto::ActionRequest,
        i64,
        mpsc::Sender<Res<dto::ActionSnapshot>>,
    ),
    MapReady(MapSignal),
    Refresh,
    Sample(String, usize, u64),
    Correct(dto::Correction),
    Export(exports::Action, PathBuf),
    Locate(String, bool),
    Entity(String),
    View(dto::Tab, dto::TabView),
    Filters(dto::Tab, Vec<dto::Filter>),
}
impl ResearchAdapter {
    pub fn new(
        seams: ResearchSeams,
        interjections: Option<Arc<dyn crate::desktop::ux::seams::Interjections>>,
    ) -> Self {
        let outputs = Arc::new(Outputs::default());
        let (sender, receiver) = mpsc::channel();
        let shared = outputs.clone();
        spawn_worker(move || Worker::new(seams, interjections, shared).run(receiver));
        Self { sender, outputs }
    }
    pub fn feeds(&self, cx: &mut gpui_kit::App) -> ResearchFeeds {
        const FEED_PERIOD_MS: u64 = 100;
        let period = Duration::from_millis(FEED_PERIOD_MS);
        ResearchFeeds {
            page: crate::desktop::data::Feed::spawn(
                "research.page",
                self.outputs.page.source(),
                period,
                cx,
            ),
            map: self.map_feed(cx),
            sources: self.sources_feed(cx),
            evidence: self.evidence_feed(cx),
            dataset: crate::desktop::data::Feed::spawn(
                "research.dataset.panel",
                self.outputs.dataset.source(),
                period,
                cx,
            ),
            exports: crate::desktop::data::Feed::spawn(
                "research.exports.panel",
                self.outputs.exports.source(),
                period,
                cx,
            ),
        }
    }
    pub(crate) fn dataset_feed(
        &self,
        cx: &mut gpui_kit::App,
    ) -> crate::desktop::data::Feed<dto::DatasetDisplay> {
        const FEED_PERIOD_MS: u64 = 100;
        crate::desktop::data::Feed::spawn(
            "research.dataset.panel",
            self.outputs.dataset.source(),
            Duration::from_millis(FEED_PERIOD_MS),
            cx,
        )
    }
    pub(crate) fn exports_feed(
        &self,
        cx: &mut gpui_kit::App,
    ) -> crate::desktop::data::Feed<dto::ExportsDisplay> {
        const FEED_PERIOD_MS: u64 = 100;
        crate::desktop::data::Feed::spawn(
            "research.exports.panel",
            self.outputs.exports.source(),
            Duration::from_millis(FEED_PERIOD_MS),
            cx,
        )
    }
    pub(crate) fn sources_feed(
        &self,
        cx: &mut gpui_kit::App,
    ) -> crate::desktop::data::Feed<dto::SourcesDisplay> {
        const FEED_PERIOD_MS: u64 = 100;
        crate::desktop::data::Feed::spawn(
            "research.sources.panel",
            self.outputs.sources.source(),
            Duration::from_millis(FEED_PERIOD_MS),
            cx,
        )
    }
    pub(crate) fn evidence_feed(
        &self,
        cx: &mut gpui_kit::App,
    ) -> crate::desktop::data::Feed<dto::EvidenceDisplay> {
        const FEED_PERIOD_MS: u64 = 100;
        crate::desktop::data::Feed::spawn(
            "research.evidence.panel",
            self.outputs.evidence.source(),
            Duration::from_millis(FEED_PERIOD_MS),
            cx,
        )
    }
    pub(crate) fn map_feed(
        &self,
        cx: &mut gpui_kit::App,
    ) -> crate::desktop::data::Feed<dto::MapDisplay> {
        const FEED_PERIOD_MS: u64 = 100;
        crate::desktop::data::Feed::spawn(
            "research.map.panel",
            self.outputs.map.source(),
            Duration::from_millis(FEED_PERIOD_MS),
            cx,
        )
    }
    pub(crate) fn after_preview(&self, deterministic: bool) -> MapWaiter {
        let state = Arc::new(Mutex::new(MapWaitState {
            deterministic,
            ..Default::default()
        }));
        self.outputs
            .preview_waiters
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(MapSignal(state.clone()));
        MapWaiter(state)
    }
    pub(crate) fn after_map(&self, deterministic: bool) -> MapWaiter {
        let state = Arc::new(Mutex::new(MapWaitState {
            deterministic,
            ..Default::default()
        }));
        self.send(Command::MapReady(MapSignal(state.clone())));
        MapWaiter(state)
    }
    fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }
    #[cfg(test)]
    pub(super) fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        self.send(Command::Probe(Box::new(move || {
            let _ = tx.send(());
        })));
        rx.recv_timeout(Duration::from_secs(30))
            .expect("research worker stopped");
    }
}
impl dto::ResearchPort for ResearchAdapter {
    fn open(&self, root: PathBuf, store: Option<PathBuf>) {
        let signals = std::mem::take(
            &mut *self
                .outputs
                .preview_waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        self.send(Command::Open(root, store, signals, None));
    }
    fn open_observed(
        &self,
        root: PathBuf,
        store: Option<PathBuf>,
        now: i64,
        events: mpsc::Sender<dto::OpenEvent>,
    ) {
        let signals = std::mem::take(
            &mut *self
                .outputs
                .preview_waiters
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        self.send(Command::Open(root, store, signals, Some((events, now))));
    }
    fn remember(&self, prefs: &mut dto::SessionPrefs, reference: dto::ResearchRef) {
        remember(prefs, reference);
    }
    fn action(
        &self,
        request: dto::ActionRequest,
        now_ms: i64,
        reply: mpsc::Sender<Res<dto::ActionSnapshot>>,
    ) {
        self.send(Command::Action(request, now_ms, reply));
    }
    fn refresh(&self) {
        self.send(Command::Refresh);
    }
    fn sample(&self, node: String, count: usize, seed: u64) {
        self.send(Command::Sample(node, count, seed));
    }
    fn correct(&self, c: dto::Correction) {
        self.send(Command::Correct(c));
    }
    fn export_csv(&self, dest: PathBuf) {
        self.send(Command::Export(exports::Action::Csv, dest));
    }
    fn export_bundle(&self, dest: PathBuf) {
        self.send(Command::Export(exports::Action::Bundle, dest));
    }
    fn export_snapshot(&self, dest: PathBuf) {
        self.send(Command::Export(exports::Action::Snapshot, dest));
    }
    fn select_entity(&self, entity: String) {
        self.send(Command::Entity(entity));
    }
    fn locate_page(&self, id: String) {
        self.send(Command::Locate(id, false));
    }
    fn render_page(&self, id: String) {
        self.send(Command::Locate(id, true));
    }
    fn set_view(&self, tab: dto::Tab, view: dto::TabView) {
        self.send(Command::View(tab, view));
    }
    fn set_filters(&self, tab: dto::Tab, filters: Vec<dto::Filter>) {
        self.send(Command::Filters(tab, filters));
    }
    fn load_state(&self) -> dto::LoadState {
        self.outputs
            .load
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn view(&self, tab: dto::Tab) -> dto::TabView {
        self.outputs
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tab)
            .cloned()
            .unwrap_or_default()
    }
    fn map(&self) -> Res<dto::MapDisplay> {
        self.outputs.map.snapshot()
    }
    fn sources(&self) -> Res<dto::SourcesDisplay> {
        self.outputs.sources.snapshot()
    }
    fn evidence(&self) -> Res<dto::EvidenceDisplay> {
        self.outputs.evidence.snapshot()
    }
    fn dataset(&self) -> Res<dto::DatasetDisplay> {
        self.outputs.dataset.snapshot()
    }
    fn exports(&self) -> Res<dto::ExportsDisplay> {
        self.outputs.exports.snapshot()
    }
}

use crate::research_workspace::{
    self as engine,
    dataset::DataLayer,
    evidence,
    funnel::{self, Node},
    index::{Index, Op},
    manifest::{locate, LocateOptions},
    read::Limits,
    session::{ResearchRef, SessionPrefs},
    stages,
};

struct Worker {
    seams: ResearchSeams,
    interjections: Option<Arc<dyn crate::desktop::ux::seams::Interjections>>,
    outputs: Arc<Outputs>,
    current: Option<ResearchRef>,
    index: Option<Arc<Index>>,
    prefs: SessionPrefs,
    located_items: HashMap<String, dto::EvidenceItem>,
    selected_entity: Option<String>,
    action_state: Option<actions::State>,
}
impl Worker {
    fn new(
        seams: ResearchSeams,
        interjections: Option<Arc<dyn crate::desktop::ux::seams::Interjections>>,
        outputs: Arc<Outputs>,
    ) -> Self {
        Self {
            seams,
            interjections,
            outputs,
            current: None,
            index: None,
            prefs: SessionPrefs::default(),
            located_items: HashMap::new(),
            selected_entity: None,
            action_state: None,
        }
    }
    fn run(mut self, rx: mpsc::Receiver<Command>) {
        for command in rx {
            match command {
                #[cfg(test)]
                Command::Probe(probe) => probe(),
                Command::Open(root, store, signals, observer) => {
                    self.located_items.clear();
                    self.selected_entity = None;
                    self.outputs.map.replace(Default::default());
                    self.outputs.sources.replace(Default::default());
                    self.outputs.evidence.replace(Default::default());
                    self.outputs.dataset.replace(Default::default());
                    self.outputs.exports.replace(Default::default());
                    self.current = Some(ResearchRef { root, store });
                    self.action_state = None;
                    self.open(signals, observer);
                }
                Command::MapReady(signal) => signal.notify(),
                Command::Action(request, now, reply) => {
                    let _ = reply.send(self.action(request, now));
                }
                Command::Refresh => self.open(Vec::new(), None),
                Command::View(tab, view) => {
                    let old = self.view(tab);
                    if tab == dto::Tab::Evidence
                        && (old.node != view.node || old.offset != view.offset)
                    {
                        self.selected_entity = None;
                    }
                    self.set_view(tab, view);
                    self.update_tab(tab);
                }
                Command::Filters(tab, filters) => {
                    let mut view = self.view(tab);
                    view.filters = filters;
                    view.offset = 0;
                    self.set_view(tab, view);
                    self.update_tab(tab);
                }
                Command::Sample(node, count, seed) => self.sample(node, count, seed),
                Command::Correct(c) => self.correct(c),
                Command::Export(action, dest) => self.export(action, dest),
                Command::Locate(id, render) => self.locate(id, render),
                Command::Entity(entity) => {
                    self.selected_entity = Some(entity);
                    self.update_tab(dto::Tab::Evidence);
                }
            }
        }
    }
    fn view(&self, tab: dto::Tab) -> dto::TabView {
        self.outputs
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tab)
            .cloned()
            .unwrap_or_default()
    }
    fn set_view(&mut self, tab: dto::Tab, view: dto::TabView) {
        self.outputs
            .views
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(tab, view.clone());
        if let (Some(ix), Some(home)) = (&self.index, &self.seams.research_home) {
            if safe_home(home, ix) {
                let (reply, response) = mpsc::channel();
                let _ = session_queue(home).send(SessionCommand::EngineLoad(reply));
                self.prefs = response.recv().unwrap_or_default();
                self.prefs
                    .views
                    .entry(ix.located().manifest.research_id.clone())
                    .or_default()
                    .tabs
                    .insert(tab.key().into(), engine_view(&view));
                let _ = session_queue(home).send(SessionCommand::EngineSave(self.prefs.clone()));
            }
        }
    }
    fn notify_preview(signals: &mut Vec<MapSignal>) {
        for signal in std::mem::take(signals) {
            signal.notify();
        }
    }
    fn open(
        &mut self,
        mut preview_signals: Vec<MapSignal>,
        observer: Option<(mpsc::Sender<dto::OpenEvent>, i64)>,
    ) {
        let Some(reference) = self.current.clone() else {
            return;
        };
        *self.outputs.load.lock().unwrap_or_else(|e| e.into_inner()) = dto::LoadState::Opening;
        self.index = None;
        let result = (|| -> Res<Arc<Index>> {
            let located = Arc::new(locate(
                &reference.root,
                &LocateOptions {
                    store: reference.store.clone(),
                    orch_home: self.seams.orch_home.clone(),
                },
            )?);
            if let Some((events, _)) = &observer {
                let source = match &located.source {
                    engine::manifest::ManifestSource::File { .. } => dto::ManifestSource::File,
                    engine::manifest::ManifestSource::Adapter { name } => {
                        dto::ManifestSource::Adapter { name: name.clone() }
                    }
                };
                let _ = events.send(dto::OpenEvent::Located(
                    dto::ResearchInfo {
                        id: located.manifest.research_id.clone(),
                        title: located.manifest.title.clone(),
                        version: located.manifest.research_version.clone(),
                        source,
                    },
                    located.manifest.artifacts.len(),
                ));
            }
            if let Some(artifact) = located
                .manifest
                .artifacts
                .iter()
                .find(|a| a.role == engine::manifest::Role::Measurements)
            {
                if let Ok(page) =
                    engine::read::read_first_rows(&located, &artifact.id, 200, &Limits::default())
                {
                    self.outputs.dataset.replace(preview_display(page.rows));
                }
            }
            Self::notify_preview(&mut preview_signals);
            if let Some((events, _)) = &observer {
                let preview = self.outputs.dataset.snapshot().unwrap_or_default();
                let _ = events.send(dto::OpenEvent::Preview(preview));
            }
            let progress = self.outputs.clone();
            let events = observer.as_ref().map(|(events, _)| events.clone());
            let index = Index::build(located, Limits::default(), &move |done, total| {
                *progress.load.lock().unwrap_or_else(|e| e.into_inner()) =
                    dto::LoadState::Indexing { done, total };
                if let Some(events) = &events {
                    let _ = events.send(dto::OpenEvent::Progress(done, total));
                }
            })?;
            Ok(Arc::new(index))
        })();
        Self::notify_preview(&mut preview_signals);
        match result {
            Err(e) => {
                if let Some((events, _)) = &observer {
                    let _ = events.send(dto::OpenEvent::Failed(e.clone()));
                }
                *self.outputs.load.lock().unwrap_or_else(|p| p.into_inner()) =
                    dto::LoadState::Failed {
                        kind: e.kind.clone(),
                        message: e.message.clone(),
                    };
                self.outputs.map.set(Err(e.clone()));
                self.outputs.sources.set(Err(e.clone()));
                self.outputs.evidence.set(Err(e.clone()));
                self.outputs.dataset.set(Err(e.clone()));
                let mut exports = unavailable_exports();
                exports.notices.push(dto::ResearchNotice {
                    kind: e.kind,
                    message: e.message,
                });
                self.outputs.exports.replace(exports);
            }
            Ok(ix) => {
                if let Some(home) = &self.seams.research_home {
                    if safe_home(home, &ix) {
                        let (reply, response) = mpsc::channel();
                        let queue = session_queue(home);
                        let _ = queue.send(SessionCommand::EngineLoad(reply));
                        self.prefs = response.recv().unwrap_or_default();
                        engine::session::remember(&mut self.prefs, reference);
                        let _ = session_queue(home)
                            .send(SessionCommand::EngineSave(self.prefs.clone()));
                    }
                }
                let mut views = self.outputs.views.lock().unwrap_or_else(|e| e.into_inner());
                views.clear();
                if let Some(saved) = self.prefs.views.get(&ix.located().manifest.research_id) {
                    for (key, view) in &saved.tabs {
                        if let Some(tab) = dto::Tab::from_key(key) {
                            views.insert(tab, display_view(view));
                        }
                    }
                }
                drop(views);
                if let Some((events, now)) = &observer {
                    let map = stages::research_map(&ix, *now);
                    let safe = self
                        .seams
                        .research_home
                        .as_ref()
                        .is_some_and(|home| safe_home(home, &ix));
                    let _ = events.send(dto::OpenEvent::Ready {
                        blocker_en: blocker_text(&map, Lang::En),
                        blocker_zh: blocker_text(&map, Lang::ZhTw),
                        safe,
                    });
                }
                self.index = Some(ix);
                *self.outputs.load.lock().unwrap_or_else(|e| e.into_inner()) =
                    dto::LoadState::Ready;
                for tab in [
                    dto::Tab::Map,
                    dto::Tab::Sources,
                    dto::Tab::Evidence,
                    dto::Tab::Dataset,
                    dto::Tab::Exports,
                ] {
                    self.update_tab(tab);
                }
            }
        }
    }
    fn action(&mut self, request: dto::ActionRequest, now: i64) -> Res<dto::ActionSnapshot> {
        let ix = self
            .index
            .as_ref()
            .ok_or_else(|| Failure::new("index", "research is not ready"))?;
        if self.action_state.is_none() {
            let home = self
                .seams
                .research_home
                .as_ref()
                .ok_or_else(|| Failure::new("no_home", "research home unavailable"))?;
            if !safe_home(home, ix) {
                return Err(Failure::new(
                    "invalid_argument",
                    "research home overlaps source",
                ));
            }
            let store = engine::annotations::AnnotationStore::open(
                home,
                &ix.located().manifest.research_id,
            )?;
            let session = self.interjections.as_ref().and_then(|_| {
                actions::resolve_session(ix.located(), self.seams.goals_home.as_deref())
            });
            self.action_state = Some(actions::State {
                store,
                session,
                bridge: self.interjections.clone(),
            });
        }
        let state = self.action_state.as_mut().unwrap();
        let clock = crate::seams::FakeClock::new(now);
        match request {
            dto::ActionRequest::Open => {}
            dto::ActionRequest::Spot {
                target,
                node,
                verdict,
            } => {
                let verdict = match verdict {
                    dto::SpotVerdict::Confirm => engine::annotations::Verdict::Confirm,
                    dto::SpotVerdict::Dispute { reason } => {
                        engine::annotations::Verdict::Dispute { reason }
                    }
                };
                state.spot(ix, &target, node.as_deref(), verdict, &clock)?;
            }
            dto::ActionRequest::Correct(c) => state.correct(
                ix,
                engine::annotations::CorrectionDraft {
                    target: c.target,
                    field: c.field,
                    after: c.after,
                    who: c.who,
                    why: c.why,
                },
                &clock,
            )?,
            dto::ActionRequest::Retry(id) => state.retry(&id)?,
            dto::ActionRequest::Poll => state.poll()?,
        }
        Ok(dto::ActionSnapshot {
            session: state.session.is_some(),
            corrections: state
                .store
                .corrections(None)
                .into_iter()
                .map(|c| dto::CorrectionStatus {
                    id: c.id,
                    who: c.who,
                    why: c.why,
                    research_version: c.research_version,
                    reverify: match c.reverify {
                        engine::annotations::Reverify::Pending => dto::Reverify::Pending,
                        engine::annotations::Reverify::Passed => dto::Reverify::Passed,
                        engine::annotations::Reverify::Failed { reason } => {
                            dto::Reverify::Failed { reason }
                        }
                    },
                    invalidates: c.invalidates,
                    receipt: c.receipt.map(|r| dto::CorrectionReceipt {
                        stage: r.stage,
                        detail: r.detail,
                    }),
                    draft: c.draft,
                })
                .collect(),
        })
    }
    fn update_tab(&mut self, tab: dto::Tab) {
        let Some(ix) = self.index.as_ref() else {
            return;
        };
        let view = self.view(tab);
        match tab {
            dto::Tab::Map => self.outputs.map.set(map_display(ix)),
            dto::Tab::Sources => {
                let previous = self.outputs.sources.snapshot().ok();
                self.outputs
                    .sources
                    .set(sources_display(ix, &view).map(|mut display| {
                        if let Some(previous) = previous {
                            if display.sample.is_empty() {
                                display.sample = previous.sample;
                            }
                            for row in &mut display.rows {
                                if let Some(old) = previous.rows.iter().find(|old| old.id == row.id)
                                {
                                    row.page = old.page;
                                    row.locate_state = old.locate_state.clone();
                                }
                            }
                        }
                        display
                    }));
            }
            dto::Tab::Evidence => {
                self.outputs.evidence.set(
                    evidence_display(ix, &view, self.selected_entity.as_deref()).map(
                        |mut display| {
                            for item in &mut display.items {
                                if let Some(old) = self.located_items.get(&item.id) {
                                    item.locator = old.locator.clone();
                                    item.target = old.target.clone();
                                    item.image = old.image.clone();
                                    item.notice = old.notice.clone();
                                }
                            }
                            display
                        },
                    ),
                );
            }
            dto::Tab::Dataset => self.outputs.dataset.set(
                dataset_display(ix, &view)
                    .map_err(|e| Failure::new(&e.kind, format!("{}: {}", e.kind, e.message))),
            ),
            dto::Tab::Exports => {
                let mut display = unavailable_exports();
                let status = engine::repro::guard(ix).status;
                display.repro_en = exports::repro_text(&status, crate::desktop::i18n::Lang::En);
                display.repro_zh = exports::repro_text(&status, crate::desktop::i18n::Lang::ZhTw);
                if let Ok(previous) = self.outputs.exports.snapshot() {
                    display.results = previous.results;
                }
                self.outputs.exports.replace(display);
            }
        }
    }
    fn sample(&mut self, node: String, count: usize, seed: u64) {
        if let Some(ix) = &self.index {
            let result = (|| {
                let mut display = self.outputs.sources.snapshot()?;
                display.sample = funnel::sample(
                    ix,
                    &Node::from_key(&node).unwrap_or(Node::Total),
                    count,
                    seed,
                )?;
                Ok(display)
            })();
            self.outputs.sources.set(result);
        }
    }
    fn correct(&mut self, c: dto::Correction) {
        let Some(ix) = &self.index else { return };
        let result = (|| -> Res<()> {
            let home = self
                .seams
                .research_home
                .as_ref()
                .ok_or_else(|| Failure::new("no_home", "research home unavailable"))?;
            if !safe_home(home, ix) {
                return Err(Failure::new(
                    "invalid_argument",
                    "research home overlaps source",
                ));
            }
            let store = engine::annotations::AnnotationStore::open(
                home,
                &ix.located().manifest.research_id,
            )?;
            let session = self.interjections.as_ref().and_then(|_| {
                actions::resolve_session(ix.located(), self.seams.goals_home.as_deref())
            });
            let mut state = actions::State {
                store,
                session,
                bridge: self.interjections.clone(),
            };
            state.correct(
                ix,
                engine::annotations::CorrectionDraft {
                    target: c.target,
                    field: c.field,
                    after: c.after,
                    who: c.who,
                    why: c.why,
                },
                &crate::seams::SystemClock,
            )?;
            Ok(())
        })();
        match result {
            Err(e) => self.outputs.evidence.set(Err(e)),
            Ok(()) => {
                self.update_tab(dto::Tab::Evidence);
                self.update_tab(dto::Tab::Dataset);
            }
        }
    }
    fn export(&mut self, action: exports::Action, dest: PathBuf) {
        let Some(ix) = &self.index else { return };
        let view = engine_view(&self.view(dto::Tab::Dataset));
        let q = exports::query(&view);
        if matches!(action, exports::Action::Snapshot)
            && self
                .seams
                .research_home
                .as_ref()
                .is_some_and(|home| !safe_home(home, ix))
        {
            self.outputs
                .exports
                .fail("invalid_argument", "research home overlaps source");
            return;
        }
        let action_name = match action {
            exports::Action::Csv => "csv",
            exports::Action::Bundle => "bundle",
            exports::Action::Snapshot => "snapshot",
        };
        let mut display = self.outputs.exports.snapshot().unwrap_or_default();
        display.results.push(dto::ExportStatus {
            action: action_name.into(),
            busy: true,
            ..Default::default()
        });
        self.outputs.exports.replace(display);
        let result = exports::perform_action(
            action,
            ix,
            ix.located(),
            &q,
            &self.seams,
            &crate::seams::SystemClock,
            &dest,
        );
        let status = match result {
            Ok(exports::Outcome::Export(item)) => dto::ExportStatus {
                action: action_name.into(),
                path: Some(item.path),
                rows: item.rows,
                sha256: Some(item.sha256),
                ..Default::default()
            },
            Ok(exports::Outcome::Snapshot(item, handoff)) => dto::ExportStatus {
                action: "snapshot".into(),
                path: Some(item.path),
                rows: item.rows,
                sha256: Some(item.sha256),
                handoff: Some({
                    let paper = exports::delivery_text(
                        &handoff.consumer,
                        "paper",
                        crate::desktop::i18n::Lang::En,
                    );
                    if self.seams.vault.is_some() {
                        format!(
                            "{} · {}",
                            paper,
                            exports::delivery_text(
                                &handoff.vault,
                                "vault",
                                crate::desktop::i18n::Lang::En
                            )
                        )
                    } else {
                        paper
                    }
                }),
                handoff_zh: Some({
                    let paper = exports::delivery_text(
                        &handoff.consumer,
                        "paper",
                        crate::desktop::i18n::Lang::ZhTw,
                    );
                    if self.seams.vault.is_some() {
                        format!(
                            "{} · {}",
                            paper,
                            exports::delivery_text(
                                &handoff.vault,
                                "vault",
                                crate::desktop::i18n::Lang::ZhTw
                            )
                        )
                    } else {
                        paper
                    }
                }),
                ..Default::default()
            },
            Ok(exports::Outcome::Empty) => dto::ExportStatus {
                notice: Some(dto::ResearchNotice {
                    kind: "empty".into(),
                    message: "No evidence in the current dataset filter".into(),
                }),
                ..Default::default()
            },
            Ok(exports::Outcome::NoHome) => dto::ExportStatus {
                notice: Some(dto::ResearchNotice {
                    kind: "no_home".into(),
                    message: "Research home is not configured".into(),
                }),
                ..Default::default()
            },
            Err(e) => dto::ExportStatus {
                notice: Some(dto::ResearchNotice {
                    kind: e.kind,
                    message: e.message,
                }),
                ..Default::default()
            },
        };
        let mut display = self.outputs.exports.snapshot().unwrap_or_default();
        if status.notice.is_some() {
            display.results.pop();
            display
                .notices
                .retain(|notice| notice.kind == "unavailable");
            display.notices.push(status.notice.unwrap());
        } else if let Some(last) = display.results.last_mut() {
            *last = status;
            display
                .notices
                .retain(|notice| notice.kind == "unavailable");
        }
        self.outputs.exports.replace(display);
    }
    fn locate(&mut self, id: String, render: bool) {
        let Some(ix) = &self.index else { return };
        let result = (|| -> Res<dto::EvidenceDisplay> {
            let resolved = evidence::resolve(ix, &id, &Limits::default())?;
            let image = if render && matches!(resolved.target, evidence::Target::PdfPage { .. }) {
                let renderer = self
                    .seams
                    .renderer
                    .as_ref()
                    .ok_or_else(|| Failure::new("renderer_missing", "renderer unavailable"))?;
                let home = self
                    .seams
                    .research_home
                    .as_ref()
                    .ok_or_else(|| Failure::new("no_home", "research home unavailable"))?;
                if !safe_home(home, ix) {
                    return Err(Failure::new(
                        "invalid_argument",
                        "research home overlaps source",
                    ));
                }
                let cache = engine::pdf::PageCache::open(
                    &home.join("cache/pages"),
                    self.seams.page_cache_bytes,
                )?;
                Some(engine::pdf::page_image(ix, renderer.as_ref(), &cache, &resolved, 1.0)?.path)
            } else {
                None
            };
            if let evidence::Locator::PdfPage {
                sha256,
                physical_page,
                ..
            } = &resolved.item.locator
            {
                if let Ok(mut sources) = self.outputs.sources.snapshot() {
                    for row in &mut sources.rows {
                        if row.sha256.eq_ignore_ascii_case(sha256) {
                            row.page = Some(*physical_page);
                            row.locate_state = if image.is_some() {
                                "rendered"
                            } else {
                                "located"
                            }
                            .into();
                        }
                    }
                    self.outputs.sources.replace(sources);
                }
            }
            let mut display = self.outputs.evidence.snapshot()?;
            let item = dto::EvidenceItem {
                id: resolved.item.evidence_id,
                record_id: resolved.item.record_id,
                layer: format!("{:?}", resolved.item.layer),
                quote: resolved.item.quote,
                locator: format!("{:?}", resolved.target),
                target: Some(match &resolved.target {
                    evidence::Target::PdfPage {
                        physical_page,
                        printed_page,
                        highlight,
                        ..
                    } => dto::EvidenceLocator::PdfPage {
                        physical_page: *physical_page,
                        printed_page: printed_page.clone(),
                        highlight: highlight.is_some(),
                    },
                    evidence::Target::RawField {
                        file, row, column, ..
                    } => dto::EvidenceLocator::RawField {
                        file: file.clone(),
                        row: *row,
                        column: column.clone(),
                    },
                }),
                image: image.or_else(|| {
                    display
                        .items
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| item.image.clone())
                }),
                notice: None,
            };
            if let Some(previous) = display
                .items
                .iter_mut()
                .find(|previous| previous.id == item.id)
            {
                *previous = item;
            } else {
                display.items.push(item);
            }
            Ok(display)
        })();
        match result {
            Ok(display) => {
                if let Some(item) = display.items.iter().find(|item| item.id == id) {
                    self.located_items.insert(id, item.clone());
                }
                self.outputs.evidence.replace(display);
            }
            Err(error) => {
                if let Ok(mut display) = self.outputs.evidence.snapshot() {
                    let notice = dto::ResearchNotice {
                        kind: error.kind,
                        message: error.message,
                    };
                    if let Some(item) = display.items.iter_mut().find(|item| item.id == id) {
                        item.notice = Some(notice);
                        self.located_items.insert(id, item.clone());
                    } else {
                        let item = dto::EvidenceItem {
                            id,
                            notice: Some(notice),
                            ..Default::default()
                        };
                        self.located_items.insert(item.id.clone(), item.clone());
                        display.items.push(item);
                    }
                    self.outputs.evidence.replace(display);
                }
            }
        }
    }
}

fn unavailable_exports() -> dto::ExportsDisplay {
    dto::ExportsDisplay {
        results: vec![],
        notices: vec![dto::ResearchNotice {
            kind: "unavailable".into(),
            message: "Vault and memory are not in this version".into(),
        }],
        ..Default::default()
    }
}

pub(crate) fn safe_home(home: &Path, ix: &Index) -> bool {
    if !home.is_absolute()
        || home
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return false;
    }
    let mut ancestor = home;
    while !ancestor.exists() {
        let Some(parent) = ancestor.parent() else {
            return false;
        };
        ancestor = parent;
    }
    let (Ok(base), Ok(rest)) = (ancestor.canonicalize(), home.strip_prefix(ancestor)) else {
        return false;
    };
    let target = base.join(rest);
    !ix.located().roots().iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|r| target.starts_with(&r) || r.starts_with(&target))
    })
}
pub(crate) fn engine_view(view: &dto::TabView) -> engine::session::TabView {
    engine::session::TabView {
        offset: view.offset,
        selection: view.selection.clone(),
        node: view.node.clone(),
        filters: view
            .filters
            .iter()
            .map(|filter| engine::index::Filter {
                column: filter.column.clone(),
                op: match &filter.op {
                    dto::FilterOp::Eq(v) => Op::Eq(v.clone()),
                    dto::FilterOp::Contains(v) => Op::Contains(v.clone()),
                    dto::FilterOp::In(v) => Op::In(v.clone()),
                    dto::FilterOp::Empty => Op::Empty,
                    dto::FilterOp::NotEmpty => Op::NotEmpty,
                },
            })
            .collect(),
    }
}
pub(crate) fn display_filter(filter: &engine::index::Filter) -> dto::Filter {
    display_view(&engine::session::TabView {
        filters: vec![filter.clone()],
        ..Default::default()
    })
    .filters
    .remove(0)
}
pub(crate) fn display_view(view: &engine::session::TabView) -> dto::TabView {
    dto::TabView {
        offset: view.offset,
        selection: view.selection.clone(),
        node: view.node.clone(),
        filters: view
            .filters
            .iter()
            .map(|filter| dto::Filter {
                column: filter.column.clone(),
                op: match &filter.op {
                    Op::Eq(v) => dto::FilterOp::Eq(v.clone()),
                    Op::Contains(v) => dto::FilterOp::Contains(v.clone()),
                    Op::In(v) => dto::FilterOp::In(v.clone()),
                    Op::Empty => dto::FilterOp::Empty,
                    Op::NotEmpty => dto::FilterOp::NotEmpty,
                },
            })
            .collect(),
    }
}
fn funnel_nodes(f: &funnel::Funnel) -> Vec<dto::FunnelNode> {
    let mut nodes = vec![];
    for (key, label, count) in [
        ("total", "Total", f.total),
        ("deduplicated", "Deduplicated", f.deduplicated),
        ("duplicates", "Duplicates", f.duplicates),
        ("screened_in", "Screened in", f.screened_in),
        ("screened_out", "Screened out", f.screened_out),
        ("pending", "Pending", f.pending),
        ("unprocessed", "Unprocessed", f.unprocessed),
        ("read", "Read", f.read),
        ("read_failed", "Read failed", f.read_failed),
    ] {
        nodes.push(dto::FunnelNode {
            key: key.into(),
            label: label.into(),
            count,
        });
    }
    for (reason, count) in &f.out_reasons {
        nodes.push(dto::FunnelNode {
            key: format!(
                "screened_out:{}",
                if reason == "(none)" { "" } else { reason }
            ),
            label: reason.clone(),
            count: *count,
        });
    }
    for (reason, count) in &f.fail_reasons {
        nodes.push(dto::FunnelNode {
            key: format!(
                "read_failed:{}",
                if reason == "(none)" { "" } else { reason }
            ),
            label: reason.clone(),
            count: *count,
        });
    }
    nodes
}
fn map_display(ix: &Index) -> Res<dto::MapDisplay> {
    let map = stages::research_map(ix, crate::seams::Clock::now_ms(&crate::seams::SystemClock));
    let (funnel, mut notices) = match funnel::funnel(ix) {
        Ok(funnel) => (funnel_nodes(&funnel), vec![]),
        Err(error) => (
            vec![],
            vec![dto::ResearchNotice {
                kind: error.kind,
                message: error.message,
            }],
        ),
    };
    if let Some(store) = &ix.located().store {
        let path = store.join("units.tsv");
        if let Err(error) = std::fs::read_to_string(&path) {
            notices.push(dto::ResearchNotice {
                kind: "store_unavailable".into(),
                message: format!("{}: {error}", path.display()),
            });
        }
    }
    use crate::desktop::i18n::Lang;
    let en = map_format::map_lines(&map, Lang::En);
    let zh = map_format::map_lines(&map, Lang::ZhTw);
    let current = map
        .stages
        .iter()
        .position(|s| !matches!(s.state, stages::StageState::Done));
    let render = |line: &map_format::MapLine| {
        let mut text = format!("{} — {}", line.label, line.state_text);
        if let Some(progress) = &line.progress_text {
            text.push_str(&format!(" · {progress}"));
        }
        text.push_str(&format!(" · {}", line.gate_text));
        for stale in &line.stale_text {
            text.push_str(&format!(" · {stale}"));
        }
        text
    };
    Ok(dto::MapDisplay {
        title: ix.located().manifest.title.clone(),
        status: map_format::status_text(&map.status, Lang::En),
        status_zh: map_format::status_text(&map.status, Lang::ZhTw),
        engineering_en: map
            .engineering
            .as_ref()
            .map(|e| map_format::engineering_text(e, Lang::En)),
        engineering_zh: map
            .engineering
            .as_ref()
            .map(|e| map_format::engineering_text(e, Lang::ZhTw)),
        repro_en: map
            .repro
            .as_ref()
            .map(|r| map_format::repro_text(r, Lang::En)),
        repro_zh: map
            .repro
            .as_ref()
            .map(|r| map_format::repro_text(r, Lang::ZhTw)),
        stale_en: map
            .stale
            .iter()
            .map(|s| map_format::stale_text(s, Lang::En))
            .collect(),
        stale_zh: map
            .stale
            .iter()
            .map(|s| map_format::stale_text(s, Lang::ZhTw))
            .collect(),
        stages: map
            .stages
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let output = ix
                    .located()
                    .manifest
                    .stages
                    .iter()
                    .find(|stage| stage.id == s.id)
                    .and_then(|stage| stage.outputs.first())
                    .map(|id| match ix.located().artifact(id).map(|a| &a.role) {
                        Some(
                            engine::manifest::Role::Measurements
                            | engine::manifest::Role::RawMeasurements,
                        ) => dto::Tab::Dataset,
                        Some(engine::manifest::Role::Evidence) => dto::Tab::Evidence,
                        _ => dto::Tab::Sources,
                    });
                dto::MapStage {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    state: format!("{:?}", s.state),
                    gate: format!("{:?}", s.gate),
                    progress: en[i].progress_text.clone(),
                    stale: en[i].stale_text.clone(),
                    text_en: render(&en[i]),
                    text_zh: render(&zh[i]),
                    tone: format!("{:?}", en[i].tone),
                    current: current == Some(i),
                    output,
                }
            })
            .collect(),
        funnel,
        notices,
    })
}
fn sources_display(ix: &Index, view: &dto::TabView) -> Res<dto::SourcesDisplay> {
    let funnel = funnel::funnel(ix)?;
    let node = view
        .node
        .as_deref()
        .and_then(Node::from_key)
        .unwrap_or(Node::Total);
    let (rows, total) = funnel::rows(ix, &node, view.offset, 200)?;
    Ok(dto::SourcesDisplay {
        nodes: funnel_nodes(&funnel),
        rows: rows
            .into_iter()
            .map(|r| dto::SourceRow {
                id: r.record_id,
                path: r.path,
                sha256: r.sha256,
                title: r.title,
                doi: r.doi,
                group: r.duplicate_group,
                decision: match r.decision {
                    funnel::Decision::Included => "Included".into(),
                    funnel::Decision::Excluded { reason } => format!("Excluded: {reason}"),
                    funnel::Decision::Pending { reason } => format!("Pending: {reason}"),
                    funnel::Decision::Failed { reason } => format!("Failed: {reason}"),
                    funnel::Decision::Duplicate { of_group } => format!("Duplicate: {of_group}"),
                    funnel::Decision::Unprocessed => "Unprocessed".into(),
                },
                page: None,
                locate_state: "not selected".into(),
            })
            .collect(),
        total,
        offset: view.offset,
        sample: match view
            .selection
            .as_deref()
            .and_then(|s| s.strip_prefix("sample:"))
            .and_then(|s| s.parse().ok())
        {
            Some(seed) => funnel::sample(ix, &node, 5, seed)?,
            None => vec![],
        },
        notices: vec![],
    })
}
pub(crate) fn evidence_display(
    ix: &Index,
    view: &dto::TabView,
    selected_entity: Option<&str>,
) -> Res<dto::EvidenceDisplay> {
    let layer = if view.node.as_deref() == Some("raw") {
        evidence::Layer::Raw
    } else {
        evidence::Layer::Literature
    };
    let (entities, total) = evidence::entities(ix, layer.clone(), view.offset, 20)?;
    let mut items = Vec::new();
    let mut notices = Vec::new();
    let mut conditions = Vec::new();
    let mut contradictions = Vec::new();
    if let Some(entity) = selected_entity
        .filter(|selected| entities.iter().any(|name| name == selected))
        .map(str::to_owned)
        .or_else(|| entities.first().cloned())
        .as_ref()
    {
        let group = evidence::group(ix, entity, layer)?;
        conditions = group
            .conditions
            .iter()
            .map(|condition| dto::EvidenceCondition {
                id: condition.condition_id.clone(),
                rows: condition
                    .rows
                    .iter()
                    .map(|row| dto::EvidenceRow {
                        text: format!(
                            "{}: {} {}",
                            row.variable, row.value_original, row.unit_original
                        ),
                        evidence: row.evidence.clone(),
                    })
                    .collect(),
            })
            .collect();
        contradictions = group
            .contradictions
            .iter()
            .map(|c| dto::EvidenceContradiction {
                text: format!("{} · {}", c.condition_id, c.variable),
                a: format!("{} {}", c.a.value_original, c.a.unit_original),
                b: format!("{} {}", c.b.value_original, c.b.unit_original),
            })
            .collect();
        let ids: std::collections::BTreeSet<_> = group
            .conditions
            .into_iter()
            .flat_map(|condition| condition.rows)
            .flat_map(|row| row.evidence)
            .collect();
        for id in ids {
            match evidence::item(ix, &id) {
                Ok(item) => items.push(dto::EvidenceItem {
                    id: item.evidence_id,
                    record_id: item.record_id,
                    layer: format!("{:?}", item.layer),
                    quote: item.quote,
                    locator: format!("{:?}", item.locator),
                    target: None,
                    image: None,
                    notice: None,
                }),
                Err(error) => notices.push(dto::ResearchNotice {
                    kind: error.kind,
                    message: error.message,
                }),
            }
        }
    }
    Ok(dto::EvidenceDisplay {
        layers: vec!["literature".into(), "raw".into()],
        entities,
        conditions,
        contradictions,
        pdf_available: ix.located().has(engine::manifest::Capability::PdfPages),
        items,
        total,
        offset: view.offset,
        notices,
    })
}
pub(crate) fn dataset_display(ix: &Index, view: &dto::TabView) -> Res<dto::DatasetDisplay> {
    let layer = if view.node.as_deref() == Some("raw") {
        DataLayer::Raw
    } else {
        DataLayer::Extracted
    };
    let engine_view = engine_view(view);
    let (page, validation, missing) =
        dataset::load_page(ix, layer, &engine_view.filters, view.offset)?;
    let names = [
        "paper_or_source",
        "condition_id",
        "variable",
        "entity",
        "original",
        "unit_original",
        "canonical",
        "unit_canonical",
        "conversion",
        "quality_grade",
    ];
    let rows = page
        .rows
        .into_iter()
        .map(|r| dto::DatasetRow {
            text_en: dataset::row_text(&r, crate::desktop::i18n::Lang::En),
            text_zh: dataset::row_text(&r, crate::desktop::i18n::Lang::ZhTw),
            layer: if r.layer == DataLayer::Raw {
                "raw"
            } else {
                "extracted"
            }
            .into(),
            id: r.record_id,
            cells: vec![
                r.paper_or_source,
                r.condition_id,
                r.variable,
                r.entity,
                format!("{:?}", r.original),
                r.unit_original,
                format!("{:?}", r.canonical),
                r.unit_canonical,
                format!("{:?}", r.conversion),
                r.quality_grade.unwrap_or_default(),
            ],
            evidence: r.evidence,
            flags: r.flags,
        })
        .collect();
    Ok(dto::DatasetDisplay {
        validation_en: dataset::validation_text(&validation, crate::desktop::i18n::Lang::En),
        validation_zh: dataset::validation_text(&validation, crate::desktop::i18n::Lang::ZhTw),
        missing: missing
            .iter()
            .map(|m| format!("{} {}/{}", m.column, m.missing, m.total))
            .collect(),
        columns: names
            .iter()
            .map(|n| dto::DatasetColumn {
                key: (*n).into(),
                label: (*n).into(),
            })
            .collect(),
        rows,
        total: page.total,
        offset: page.offset,
        preview: false,
        filters: view.filters.clone(),
        notices: vec![dto::ResearchNotice {
            kind: "validation".into(),
            message: format!("{:?}", validation),
        }],
    })
}

fn preview_display(rows: Vec<engine::read::Row>) -> dto::DatasetDisplay {
    let columns: Vec<_> = rows
        .iter()
        .flat_map(|row| row.fields.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    dto::DatasetDisplay {
        columns: columns
            .iter()
            .map(|key| dto::DatasetColumn {
                key: key.clone(),
                label: key.clone(),
            })
            .collect(),
        total: rows.len(),
        preview: true,
        rows: rows
            .into_iter()
            .map(|row| dto::DatasetRow {
                id: row.number.to_string(),
                cells: columns
                    .iter()
                    .map(|key| row.fields.get(key).cloned().unwrap_or_default())
                    .collect(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

#[derive(Default)]
struct ResearchTarget {
    pending: Option<dto::Tab>,
    active: Option<gpui_kit::WeakEntity<super::Workbench>>,
}
impl gpui_kit::Global for ResearchTarget {}

pub(super) fn compose(
    state: &gpui_kit::Entity<crate::desktop::model::AppState>,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<super::ResearchPage>,
) -> (
    gpui_kit::Entity<super::Workbench>,
    gpui_kit::Entity<super::map::MapPanel>,
    gpui_kit::Entity<super::sources::SourcesPanel>,
    gpui_kit::Entity<super::evidence::EvidencePanel>,
    gpui_kit::Entity<super::dataset::DatasetPanel>,
    gpui_kit::Entity<super::exports::ExportsPanel>,
) {
    use gpui_kit::AppContext as _;
    let seams = state.read(cx).ux.research.clone();
    let interjections = state.read(cx).ux.seams.interjections.clone();
    let wb = cx.new(|cx| {
        super::Workbench::new(
            seams,
            interjections,
            Arc::new(crate::seams::SystemClock),
            cx,
        )
    });
    register_workbench(&wb, cx);
    let map = cx.new(|cx| super::map::MapPanel::new(wb.clone(), window, cx));
    let sources = cx.new(|cx| super::sources::SourcesPanel::new(wb.clone(), window, cx));
    let evidence = cx.new(|cx| super::evidence::EvidencePanel::new(wb.clone(), window, cx));
    let dataset = cx.new(|cx| super::dataset::DatasetPanel::new(wb.clone(), window, cx));
    let exports = cx.new(|cx| super::exports::ExportsPanel::new(wb.clone(), window, cx));
    (wb, map, sources, evidence, dataset, exports)
}

pub(crate) fn register_workbench(wb: &gpui_kit::Entity<super::Workbench>, cx: &mut gpui_kit::App) {
    if cx.try_global::<ResearchTarget>().is_none() {
        cx.set_global(ResearchTarget::default());
    }
    let target = cx.global_mut::<ResearchTarget>();
    target.active = Some(wb.downgrade());
    if let Some(tab) = target.pending.take() {
        wb.update(cx, |wb, cx| wb.select_target_tab(tab, cx));
    }
}

pub(crate) fn open_target(tab: Option<dto::Tab>, already_open: bool, cx: &mut gpui_kit::App) {
    if cx.try_global::<ResearchTarget>().is_none() {
        cx.set_global(ResearchTarget::default());
    }
    let target = cx.global_mut::<ResearchTarget>();
    let active = already_open
        .then(|| target.active.as_ref().and_then(|wb| wb.upgrade()))
        .flatten();
    target.pending = if active.is_some() { None } else { tab };
    if let (Some(wb), Some(tab)) = (active, tab) {
        wb.update(cx, |wb, cx| wb.select_target_tab(tab, cx));
    }
}

pub(crate) fn evidence_filter(id: &str) -> engine::index::Filter {
    engine::dataset::evidence_filter(id)
}

/// Interval for polling asynchronous correction receipts, not an animation duration.
pub(super) fn receipt_poll_interval() -> Duration {
    const RECEIPT_POLL_SECONDS: u64 = 1;
    Duration::from_secs(RECEIPT_POLL_SECONDS)
}

use crate::research_workspace::stages::ResearchMap;
use std::sync::OnceLock;

// All instances for one home share an ordered queue, including reopen/load.
pub(super) enum SessionCommand {
    Load(mpsc::Sender<dto::SessionPrefs>),
    Save(dto::SessionPrefs),
    EngineLoad(mpsc::Sender<SessionPrefs>),
    EngineSave(SessionPrefs),
}
pub(super) fn session_queue(home: &Path) -> mpsc::Sender<SessionCommand> {
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
                        SessionCommand::EngineLoad(reply) => {
                            let _ = reply.send(engine::session::load(&home));
                        }
                        SessionCommand::EngineSave(prefs) => {
                            let _ = engine::session::save(&home, &prefs);
                        }
                        SessionCommand::Load(reply) => {
                            let _ = reply.send(display_prefs(&engine::session::load(&home)));
                        }
                        SessionCommand::Save(prefs) => {
                            let _ = engine::session::save(&home, &engine_prefs(&prefs));
                        }
                    }
                }
            });
            tx
        })
        .clone()
}

fn display_ref(r: &ResearchRef) -> dto::ResearchRef {
    dto::ResearchRef {
        root: r.root.clone(),
        store: r.store.clone(),
    }
}
fn engine_ref(r: &dto::ResearchRef) -> ResearchRef {
    ResearchRef {
        root: r.root.clone(),
        store: r.store.clone(),
    }
}
fn display_prefs(p: &SessionPrefs) -> dto::SessionPrefs {
    dto::SessionPrefs {
        last: p.last.as_ref().map(display_ref),
        recent: p.recent.iter().map(display_ref).collect(),
        views: p
            .views
            .iter()
            .map(|(id, v)| {
                (
                    id.clone(),
                    dto::ViewState {
                        tab: v.tab.clone(),
                        tabs: v
                            .tabs
                            .iter()
                            .map(|(k, v)| (k.clone(), display_view(v)))
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}
fn engine_prefs(p: &dto::SessionPrefs) -> SessionPrefs {
    SessionPrefs {
        last: p.last.as_ref().map(engine_ref),
        recent: p.recent.iter().map(engine_ref).collect(),
        views: p
            .views
            .iter()
            .map(|(id, v)| {
                (
                    id.clone(),
                    engine::session::ViewState {
                        tab: v.tab.clone(),
                        tabs: v
                            .tabs
                            .iter()
                            .map(|(k, v)| (k.clone(), engine_view(v)))
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}
pub(super) fn remember(p: &mut dto::SessionPrefs, r: dto::ResearchRef) {
    let mut engine = engine_prefs(p);
    engine::session::remember(&mut engine, engine_ref(&r));
    *p = display_prefs(&engine);
}

use crate::desktop::i18n::Lang;
use crate::research_workspace::stages::StageState;

pub(super) fn blocker_text(map: &ResearchMap, lang: Lang) -> Option<String> {
    let (id, state) = stages::blocker(map)?;
    let label = map
        .stages
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.label.as_str())
        .unwrap_or(&id);
    let detail = match state {
        StageState::NotStarted => super::tr(lang, "Not started").to_string(),
        StageState::Paused { reason } => format!("{}: {}", super::tr(lang, "Paused"), reason),
        StageState::Blocked { reason } => format!("{}: {}", super::tr(lang, "Blocked"), reason),
        StageState::Failed { reason } => format!("{}: {}", super::tr(lang, "Failed"), reason),
        StageState::RateLimited { .. } => super::tr(lang, "Rate limited").to_string(),
        StageState::Stalled { since } => format!("{}: {}", super::tr(lang, "Stalled"), since),
        StageState::Running | StageState::Done => return None,
    };
    Some(format!("{label}: {detail}"))
}
