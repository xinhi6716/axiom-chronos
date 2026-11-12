use super::super::celestial::DualCore;
use std::{collections::HashMap, sync::Mutex};
pub trait DualCoreSource: Send + Sync + 'static {
    fn state(&self, session: &str) -> DualCore;
}
#[derive(Default)]
pub struct FakeDualCore {
    states: Mutex<HashMap<String, DualCore>>,
}
impl FakeDualCore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, session: &str, value: DualCore) {
        self.states.lock().unwrap().insert(session.into(), value);
    }
}
impl DualCoreSource for FakeDualCore {
    fn state(&self, session: &str) -> DualCore {
        self.states
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or(DualCore::Off)
    }
}
