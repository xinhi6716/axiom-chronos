use super::{
    faults::{self, Fault},
    Cursor, Delta, Snapshot, Source,
};
use crate::{
    base::{Failure, Res},
    desktop::components::panel::PanelState,
};
use gpui_kit::{App, AppContext, AsyncApp, Context, Entity};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Inner<T> {
    generation: u64,
    applied: u64,
    cursor: Cursor,
    value: Option<Arc<T>>,
    version: u64,
    state: PanelState,
    updated_ms: Option<i64>,
    missing_is_empty: bool,
    stale_after: Option<Duration>,
}
pub struct FeedCore<T> {
    inner: Arc<Mutex<Inner<T>>>,
}
impl<T> Clone for FeedCore<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
impl<T: Clone> FeedCore<T> {
    pub fn new(missing_is_empty: bool, stale_after: Option<Duration>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                generation: 0,
                applied: 0,
                cursor: Cursor(0),
                value: None,
                version: 0,
                state: PanelState::Loading,
                updated_ms: None,
                missing_is_empty,
                stale_after,
            })),
        }
    }
    pub fn begin_poll(&self) -> u64 {
        let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        i.generation = i
            .generation
            .checked_add(1)
            .expect("feed generation overflow");
        i.generation
    }
    pub fn cursor(&self) -> Cursor {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).cursor
    }
    pub fn snapshot(&self) -> Snapshot<T> {
        let i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Snapshot {
            value: i.value.clone(),
            version: i.version,
            state: i.state.clone(),
            updated_ms: i.updated_ms,
        }
    }
    /// Returns false when a newer reply has already won.
    pub fn apply(
        &self,
        generation: u64,
        result: Res<(Delta<T>, Cursor)>,
        now_ms: i64,
        fault: Option<Fault>,
    ) -> bool {
        let (value, updated, missing_is_empty, stale_after) = {
            let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if generation <= i.applied {
                return false;
            }
            i.applied = generation;
            (
                i.value.clone(),
                i.updated_ms,
                i.missing_is_empty,
                i.stale_after,
            )
        };
        let forced_offline = matches!(fault, Some(Fault::Offline));
        let stale_since = match &fault {
            Some(Fault::Stale { age_ms }) => Some(now_ms.saturating_sub(*age_ms)),
            _ => None,
        };
        let fault = match fault {
            Some(Fault::Corrupt) => Some(Err(Failure::new("corrupt", "injected corrupt"))),
            Some(Fault::Missing) => Some(Err(Failure::new("missing", "injected missing"))),
            Some(Fault::Offline) => Some(Err(Failure::new("offline", "injected offline"))),
            _ => None,
        };
        let result = fault.unwrap_or(result);
        let (new_value, state, updated_ms, cursor) = if let Some(since_ms) = stale_since {
            (value, PanelState::Stale { since_ms }, updated, None)
        } else {
            match result {
                Ok((delta, cursor)) => {
                    let new_value = match delta {
                        Delta::Unchanged => value,
                        Delta::Replace(v) => Some(Arc::new(v)),
                        Delta::Patch(patch) => value.map(|v| {
                            let mut v = (*v).clone();
                            patch(&mut v);
                            Arc::new(v)
                        }),
                    };
                    let state = if new_value.is_some() {
                        PanelState::Ready
                    } else {
                        PanelState::Empty
                    };
                    let updated_ms = if new_value.is_some() {
                        Some(now_ms)
                    } else {
                        updated
                    };
                    (new_value, state, updated_ms, Some(cursor))
                }
                Err(e) => {
                    let kind = e.kind.as_str();
                    let expired = updated.zip(stale_after).is_some_and(|(then, limit)| {
                        now_ms.saturating_sub(then) > limit.as_millis().min(i64::MAX as u128) as i64
                    });
                    let state = match kind {
                        "missing" | "not_found" => {
                            if missing_is_empty {
                                PanelState::Empty
                            } else {
                                PanelState::Failed { reason: e.message }
                            }
                        }
                        "corrupt" | "corrupt_artifact" => PanelState::Corrupt { reason: e.message },
                        _ if expired && !forced_offline => PanelState::Stale {
                            since_ms: updated.unwrap(),
                        },
                        "offline" | "unreachable" => PanelState::Offline { reason: e.message },
                        _ => PanelState::Failed { reason: e.message },
                    };
                    (value, state, updated, None)
                }
            }
        };
        let mut i = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if i.applied != generation {
            return false;
        }
        let previous_value = std::mem::replace(&mut i.value, new_value);
        i.state = state;
        i.updated_ms = updated_ms;
        if let Some(cursor) = cursor {
            i.cursor = cursor;
        }
        i.version = i.version.checked_add(1).expect("feed version overflow");
        drop(i);
        drop(previous_value);
        true
    }
}

struct Watcher;

pub(super) fn slow_delay(fault: &Option<Fault>) -> Option<Duration> {
    match fault {
        Some(Fault::Slow { ms }) => Some(Duration::from_millis(*ms)),
        _ => None,
    }
}
pub struct Feed<T> {
    core: FeedCore<T>,
    watcher: Entity<Watcher>,
    retry: Arc<AtomicBool>,
}
impl<T> Clone for Feed<T> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
            watcher: self.watcher.clone(),
            retry: self.retry.clone(),
        }
    }
}
impl<T: Clone + Send + Sync + 'static> Feed<T> {
    pub fn spawn(
        panel: &'static str,
        mut source: impl Source<T>,
        every: Duration,
        cx: &mut App,
    ) -> Self {
        let core = FeedCore::new(source.missing_is_empty(), source.stale_after());
        let weak_core = Arc::downgrade(&core.inner);
        let watcher = cx.new(|_| Watcher);
        let weak_watcher = watcher.downgrade();
        let retry = Arc::new(AtomicBool::new(false));
        let retry_worker = retry.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut next = Instant::now();
            loop {
                let Some(inner) = weak_core.upgrade() else {
                    break;
                };
                let core = FeedCore { inner };
                if Instant::now() < next && !retry_worker.swap(false, Ordering::AcqRel) {
                    drop(core);
                    cx.background_executor()
                        .timer(
                            next.saturating_duration_since(Instant::now())
                                .min(Duration::from_millis(50)),
                        )
                        .await;
                    continue;
                }
                let generation = core.begin_poll();
                let cursor = core.cursor();
                let fault = cx.update(|cx| faults::active(panel, cx));
                if let Some(delay) = slow_delay(&fault) {
                    cx.background_executor().timer(delay).await;
                }
                let (returned, applied) = cx
                    .background_spawn(async move {
                        let result = source.poll(cursor);
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                            .min(i64::MAX as u128) as i64;
                        (source, core.apply(generation, result, now, fault))
                    })
                    .await;
                source = returned;
                if applied {
                    if let Some(watcher) = weak_watcher.upgrade() {
                        let _ = cx.update(|cx| watcher.update(cx, |_, cx| cx.notify()));
                    }
                }
                next = Instant::now() + every.max(Duration::from_millis(1));
            }
        })
        .detach();
        Self {
            core,
            watcher,
            retry,
        }
    }
    pub fn snapshot(&self) -> Snapshot<T> {
        self.core.snapshot()
    }
    pub fn retry(&self) {
        self.retry.store(true, Ordering::Release);
    }
    pub fn observe<V: 'static>(&self, cx: &mut Context<V>) {
        cx.observe(&self.watcher, |_, _, cx| cx.notify()).detach();
    }
}
