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
        let value = inner.result.clone()?;
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
        false
    }
}
#[cfg(test)]
type SharedRows<T> = Slot<Vec<T>>;
fn spawn_worker(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

#[derive(Default)]
struct Outputs {
    map: Slot<dto::MapDisplay>,
    sources: Slot<dto::SourcesDisplay>,
    evidence: Slot<dto::EvidenceDisplay>,
    dataset: Slot<dto::DatasetDisplay>,
    exports: Slot<dto::ExportsDisplay>,
    load: Mutex<dto::LoadState>,
    views: Mutex<HashMap<dto::Tab, dto::TabView>>,
}

pub struct ResearchFeeds {
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
enum Command {
    #[cfg(test)]
    Probe(Box<dyn FnOnce() + Send>),
    Open(PathBuf, Option<PathBuf>),
    Refresh,
    Sample(String, usize, u64),
    Correct(dto::Correction),
    Export(super::exports::Action, PathBuf),
    Locate(String, bool),
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
        let period = Duration::from_millis(100);
        ResearchFeeds {
            map: crate::desktop::data::Feed::spawn(
                "research.map",
                self.outputs.map.source(),
                period,
                cx,
            ),
            sources: crate::desktop::data::Feed::spawn(
                "research.sources",
                self.outputs.sources.source(),
                period,
                cx,
            ),
            evidence: crate::desktop::data::Feed::spawn(
                "research.evidence",
                self.outputs.evidence.source(),
                period,
                cx,
            ),
            dataset: crate::desktop::data::Feed::spawn(
                "research.dataset",
                self.outputs.dataset.source(),
                period,
                cx,
            ),
            exports: crate::desktop::data::Feed::spawn(
                "research.exports",
                self.outputs.exports.source(),
                period,
                cx,
            ),
        }
    }
    fn send(&self, command: Command) {
        let _ = self.sender.send(command);
    }
    #[cfg(test)]
    fn flush(&self) {
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
        self.send(Command::Open(root, store));
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
        self.send(Command::Export(super::exports::Action::Csv, dest));
    }
    fn export_bundle(&self, dest: PathBuf) {
        self.send(Command::Export(super::exports::Action::Bundle, dest));
    }
    fn export_snapshot(&self, dest: PathBuf) {
        self.send(Command::Export(super::exports::Action::Snapshot, dest));
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
        }
    }
    fn run(mut self, rx: mpsc::Receiver<Command>) {
        for command in rx {
            match command {
                #[cfg(test)]
                Command::Probe(probe) => probe(),
                Command::Open(root, store) => {
                    self.located_items.clear();
                    self.outputs.sources.replace(Default::default());
                    self.outputs.evidence.replace(Default::default());
                    self.outputs.exports.replace(Default::default());
                    self.current = Some(ResearchRef { root, store });
                    self.open();
                }
                Command::Refresh => self.open(),
                Command::View(tab, view) => {
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
                self.prefs
                    .views
                    .entry(ix.located().manifest.research_id.clone())
                    .or_default()
                    .tabs
                    .insert(tab.key().into(), engine_view(&view));
                let _ = super::session_queue(home)
                    .send(super::SessionCommand::Save(self.prefs.clone()));
            }
        }
    }
    fn open(&mut self) {
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
            let progress = self.outputs.clone();
            let index = Index::build(located, Limits::default(), &move |done, total| {
                *progress.load.lock().unwrap_or_else(|e| e.into_inner()) =
                    dto::LoadState::Indexing { done, total };
            })?;
            Ok(Arc::new(index))
        })();
        match result {
            Err(e) => {
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
                        let queue = super::session_queue(home);
                        let _ = queue.send(super::SessionCommand::Load(reply));
                        self.prefs = response.recv().unwrap_or_default();
                        engine::session::remember(&mut self.prefs, reference);
                        let _ = super::session_queue(home)
                            .send(super::SessionCommand::Save(self.prefs.clone()));
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
                            display.sample = previous.sample;
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
                self.outputs
                    .evidence
                    .set(evidence_display(ix, &view).map(|mut display| {
                        for item in &mut display.items {
                            if let Some(old) = self.located_items.get(&item.id) {
                                item.locator = old.locator.clone();
                                item.image = old.image.clone();
                                item.notice = old.notice.clone();
                            }
                        }
                        display
                    }));
            }
            dto::Tab::Dataset => self.outputs.dataset.set(dataset_display(ix, &view)),
            dto::Tab::Exports => {
                let mut display = unavailable_exports();
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
                super::actions::resolve_session(ix.located(), self.seams.goals_home.as_deref())
            });
            let mut state = super::actions::State {
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
    fn export(&mut self, action: super::exports::Action, dest: PathBuf) {
        let Some(ix) = &self.index else { return };
        let view = engine_view(&self.view(dto::Tab::Dataset));
        let q = super::exports::query(&view);
        if matches!(action, super::exports::Action::Snapshot)
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
            super::exports::Action::Csv => "csv",
            super::exports::Action::Bundle => "bundle",
            super::exports::Action::Snapshot => "snapshot",
        };
        let mut display = self.outputs.exports.snapshot().unwrap_or_default();
        display.results.push(dto::ExportStatus {
            action: action_name.into(),
            busy: true,
            ..Default::default()
        });
        self.outputs.exports.replace(display);
        let result = super::exports::perform_action(
            action,
            ix,
            ix.located(),
            &q,
            &self.seams,
            &crate::seams::SystemClock,
            &dest,
        );
        let status = match result {
            Ok(super::exports::Outcome::Export(item)) => dto::ExportStatus {
                action: action_name.into(),
                path: Some(item.path),
                rows: item.rows,
                sha256: Some(item.sha256),
                ..Default::default()
            },
            Ok(super::exports::Outcome::Snapshot(item, handoff)) => dto::ExportStatus {
                action: "snapshot".into(),
                path: Some(item.path),
                rows: item.rows,
                sha256: Some(item.sha256),
                handoff: Some(format!("{:?}", handoff.consumer)),
                ..Default::default()
            },
            Ok(super::exports::Outcome::Empty) => dto::ExportStatus {
                notice: Some(dto::ResearchNotice {
                    kind: "empty".into(),
                    message: "No evidence in the current dataset filter".into(),
                }),
                ..Default::default()
            },
            Ok(super::exports::Outcome::NoHome) => dto::ExportStatus {
                notice: Some(dto::ResearchNotice {
                    kind: "no_home".into(),
                    message: "Research home is not configured".into(),
                }),
                ..Default::default()
            },
            Err(e) => {
                self.outputs.exports.set(Err(e));
                return;
            }
        };
        let mut display = self.outputs.exports.snapshot().unwrap_or_default();
        if let Some(last) = display.results.last_mut() {
            *last = status;
        }
        self.outputs.exports.replace(display);
    }
    fn locate(&mut self, id: String, render: bool) {
        let Some(ix) = &self.index else { return };
        let result = (|| -> Res<dto::EvidenceDisplay> {
            let resolved = evidence::resolve(ix, &id, &Limits::default())?;
            let image = if render {
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
    }
}

fn safe_home(home: &Path, ix: &Index) -> bool {
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
fn engine_view(view: &dto::TabView) -> engine::session::TabView {
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
fn display_view(view: &engine::session::TabView) -> dto::TabView {
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
    Ok(dto::MapDisplay {
        title: ix.located().manifest.title.clone(),
        status: format!("{:?}", map.status),
        stages: map
            .stages
            .into_iter()
            .map(|s| dto::MapStage {
                id: s.id,
                label: s.label,
                state: format!("{:?}", s.state),
                gate: format!("{:?}", s.gate),
                progress: s.progress.map(|p| format!("{}/{}", p.done, p.total)),
                stale: s.stale.into_iter().map(|s| format!("{:?}", s)).collect(),
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
                decision: format!("{:?}", r.decision),
                page: None,
                locate_state: "not selected".into(),
            })
            .collect(),
        total,
        offset: view.offset,
        sample: vec![],
        notices: vec![],
    })
}
fn evidence_display(ix: &Index, view: &dto::TabView) -> Res<dto::EvidenceDisplay> {
    let layer = if view.node.as_deref() == Some("raw") {
        evidence::Layer::Raw
    } else {
        evidence::Layer::Literature
    };
    let (entities, total) = evidence::entities(ix, layer.clone(), view.offset, 20)?;
    let mut items = Vec::new();
    let mut notices = Vec::new();
    if let Some(entity) = view
        .selection
        .as_ref()
        .filter(|selected| entities.contains(selected))
        .or_else(|| entities.first())
    {
        let group = evidence::group(ix, entity, layer)?;
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
        items,
        total,
        offset: view.offset,
        notices,
    })
}
fn dataset_display(ix: &Index, view: &dto::TabView) -> Res<dto::DatasetDisplay> {
    let layer = if view.node.as_deref() == Some("raw") {
        DataLayer::Raw
    } else {
        DataLayer::Extracted
    };
    let engine_view = engine_view(view);
    let (page, validation, _) =
        super::dataset::load_page(ix, layer, &engine_view.filters, view.offset)?;
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
