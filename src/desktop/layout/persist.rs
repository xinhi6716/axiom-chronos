use crate::desktop::settings::{self, ColumnWidths};
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    time::Duration,
};

/// How long the writer waits after the last size change before it writes the file once.
const WRITE_DEBOUNCE_MS: u64 = 300;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelSizes {
    pub version: u32,
    pub panels: BTreeMap<String, Vec<f32>>,
}
impl Default for PanelSizes {
    fn default() -> Self {
        Self {
            version: 1,
            panels: BTreeMap::new(),
        }
    }
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}
fn valid_sizes(sizes: &[f32]) -> bool {
    !sizes.is_empty() && sizes.iter().all(|v| v.is_finite() && *v >= 0.)
}
pub fn load(home: &Path) -> PanelSizes {
    std::fs::read(home.join("desktop/layout.json"))
        .ok()
        .and_then(|data| serde_json::from_slice::<PanelSizes>(&data).ok())
        .filter(|s| {
            s.version == 1
                && s.panels
                    .iter()
                    .all(|(id, v)| valid_id(id) && valid_sizes(v))
        })
        .unwrap_or_default()
}
pub fn migrate_run_columns(store: &mut PanelSizes, widths: ColumnWidths) {
    if valid_sizes(&[widths.checklist, widths.details]) {
        store
            .panels
            .entry("run.columns".into())
            .or_insert_with(|| vec![widths.checklist, widths.details]);
    }
}
pub(super) enum Message {
    Write(PanelSizes),
    Flush(Sender<()>),
}
pub(crate) struct LayoutState {
    data: PanelSizes,
    sender: Sender<Message>,
}
impl Global for LayoutState {}

pub(crate) fn init_state(home: &Path, legacy: Option<ColumnWidths>) -> LayoutState {
    let mut data = load(home);
    if let Some(widths) = legacy {
        migrate_run_columns(&mut data, widths);
    }
    LayoutState {
        data,
        sender: worker(home.to_path_buf()),
    }
}

pub(crate) fn remember_state(state: &mut LayoutState, id: &str, sizes: Vec<f32>) -> bool {
    if !valid_id(id) || !valid_sizes(&sizes) {
        return false;
    }
    state.data.panels.insert(id.into(), sizes);
    let _ = state.sender.send(Message::Write(state.data.clone()));
    true
}

pub(crate) fn sizes_of(state: &LayoutState, id: &str) -> Option<Vec<f32>> {
    state.data.panels.get(id).cloned()
}

pub(crate) fn flush_state(state: &LayoutState) {
    flush_sender(&state.sender);
}

pub(super) fn worker(home: PathBuf) -> Sender<Message> {
    let (sender, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut pending: Option<PanelSizes> = None;
        loop {
            let msg = if pending.is_some() {
                rx.recv_timeout(Duration::from_millis(WRITE_DEBOUNCE_MS))
            } else {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            match msg {
                Ok(Message::Write(data)) => pending = Some(data),
                Ok(Message::Flush(ack)) => {
                    if let Some(data) = pending.take() {
                        let _ = write(&home, &data);
                    }
                    let _ = ack.send(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(data) = pending.take() {
                        let _ = write(&home, &data);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(data) = pending.take() {
                        let _ = write(&home, &data);
                    }
                    break;
                }
            }
        }
    });
    sender
}
fn write(home: &Path, data: &PanelSizes) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let dir = home.join("desktop");
    std::fs::create_dir_all(&dir)?;
    let temp = dir.join(format!(
        ".layout.{}.{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        serde_json::to_writer(&mut f, data)?;
        f.flush()?;
        drop(f);
        std::fs::rename(&temp, dir.join("layout.json"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
/// Bind a layout store to a home directory before use. Existing legacy run widths are migrated in memory.
pub fn init(home: &Path, cx: &mut App) {
    let legacy = cx
        .try_global::<settings::Store>()
        .and_then(|s| s.settings.columns);
    cx.set_global(init_state(home, legacy));
}
fn default_home() -> PathBuf {
    std::env::var_os("AXIOM_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .unwrap_or_default(),
            )
            .join(".axiom")
        })
}
pub fn remember(id: &'static str, sizes: Vec<f32>, cx: &mut App) {
    if cx.try_global::<LayoutState>().is_none() {
        init(&default_home(), cx);
    }
    remember_state(cx.global_mut::<LayoutState>(), id, sizes);
}
pub fn sizes(id: &str, cx: &App) -> Option<Vec<f32>> {
    cx.try_global::<LayoutState>()
        .and_then(|state| sizes_of(state, id))
}
/// Wait for the background worker to finish its latest queued write.
pub fn flush(cx: &App) {
    if let Some(state) = cx.try_global::<LayoutState>() {
        flush_state(state);
    }
}
pub(super) fn flush_sender(sender: &Sender<Message>) {
    let (tx, rx) = mpsc::channel();
    if sender.send(Message::Flush(tx)).is_ok() {
        let _ = rx.recv();
    }
}
