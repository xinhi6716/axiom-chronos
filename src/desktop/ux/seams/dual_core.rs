use gpui_kit::SharedString;
use std::{collections::HashMap, sync::Mutex};
#[derive(Clone, Debug, PartialEq)]
pub enum DualCore {
    Off,
    Deciding {
        topic: SharedString,
    },
    Agreed {
        topic: SharedString,
    },
    Split {
        topic: SharedString,
    },
    Adjudicated {
        topic: SharedString,
        by: SharedString,
    },
}
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
