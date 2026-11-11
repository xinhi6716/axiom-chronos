//! Quota I/O and refresh policy, independent of the UI.
use crate::{base::{self, Failure, Res}, codex::AppServer};
use serde_json::Value;
use std::{sync::{Arc, Mutex, mpsc::Receiver}, time::{Duration, Instant}};

pub(super) trait RateSource: Send + Sync {
    fn read(&self) -> Res<Value>;
    fn generation(&self) -> u64;
    fn events(&self) -> Res<Receiver<Value>>;
}

impl RateSource for AppServer {
    fn read(&self) -> Res<Value> { self.read_rate_limits() }
    fn generation(&self) -> u64 { AppServer::generation(self) }
    fn events(&self) -> Res<Receiver<Value>> { Ok(self.account_events()) }
}

#[derive(Default)]
pub(super) struct SharedCodex {
    inner: Mutex<(Option<Arc<AppServer>>, bool)>,
    #[cfg(test)]
    pub(super) resolve_override: Mutex<Option<Res<Vec<String>>>>,
    #[cfg(test)]
    pub(super) on_close: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl SharedCodex {
    pub(super) fn server(&self) -> Res<Arc<AppServer>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.1 { return Err(Failure::new("engine_stopped", "the engine worker stopped")); }
        if inner.0.is_none() {
            #[cfg(test)]
            let command = self.resolve_override.lock().unwrap().clone().unwrap_or_else(|| base::resolve_command("AXIOM_CODEX_CMD", "codex"))?;
            #[cfg(not(test))]
            let command = base::resolve_command("AXIOM_CODEX_CMD", "codex")?;
            inner.0 = Some(Arc::new(AppServer::new(command)));
        }
        Ok(inner.0.as_ref().unwrap().clone())
    }

    pub(super) fn close(&self) {
        #[cfg(test)]
        if let Some(hook) = self.on_close.lock().unwrap().as_ref() { hook(); }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.1 = true;
        if let Some(server) = &inner.0 { server.close(); }
    }

    #[cfg(test)]
    pub(super) fn created(&self) -> bool { self.inner.lock().unwrap().0.is_some() }
}

impl RateSource for SharedCodex {
    fn read(&self) -> Res<Value> { self.server()?.read_rate_limits() }
    fn generation(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).0.as_ref().map_or(0, |s| s.generation())
    }
    fn events(&self) -> Res<Receiver<Value>> { Ok(self.server()?.account_events()) }
}

/// `last` records the start, generation and runs revision of the previous read.
pub(super) fn due(now: Instant, started: Instant, last: Option<(Instant, u64, u64)>, generation: u64, revision: u64, active: bool) -> bool {
    match last {
        None => active || now.saturating_duration_since(started) >= Duration::from_secs(180),
        Some((at, old_generation, old_revision)) => {
            let elapsed = now.saturating_duration_since(at);
            generation != old_generation || elapsed >= Duration::from_secs(180)
                || (active && revision != old_revision && elapsed >= Duration::from_secs(60))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_boundaries() {
        let start = Instant::now();
        let last = Some((start, 1, 2));
        let at = |s| start + Duration::from_secs(s);
        assert!(!due(at(0), start, None, 0, 0, false));
        assert!(due(at(0), start, None, 0, 0, true));
        assert!(!due(at(179), start, None, 0, 0, false));
        assert!(due(at(180), start, None, 0, 0, false));
        assert!(!due(at(59), start, last, 1, 3, true));
        assert!(due(at(60), start, last, 1, 3, true));
        assert!(!due(at(60), start, last, 1, 3, false));
        assert!(!due(at(179), start, last, 1, 2, true));
        assert!(due(at(180), start, last, 1, 2, false));
        assert!(due(at(0), start, last, 2, 2, false));
    }
}
