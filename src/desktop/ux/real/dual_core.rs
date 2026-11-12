//! Real DualCoreSource, projected from the session store's `dual_core` events; owned by module engine-views-seam.
use super::session_store;
use crate::{
    desktop::ux::{celestial::DualCore, seams::dual_core::DualCoreSource},
    engine::{kinds, policy},
    orch::events,
};
use std::path::{Path, PathBuf};

pub struct EventDualCore {
    home: PathBuf,
}
impl EventDualCore {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}

/// The projection is a pure function of policy.json and events.jsonl, so it replays after a restart.
pub fn project(store: &Path) -> DualCore {
    match policy::load(store) {
        Ok(p) if p.dual_core => {}
        _ => return DualCore::Off,
    }
    let Ok(all) = events::read_events(store, 0) else {
        return DualCore::Off;
    };
    let Some(last) = all.iter().rfind(|e| e.kind == kinds::DUAL_CORE) else {
        return DualCore::Off;
    };
    let text = |key: &str| last.data.get(key).and_then(|v| v.as_str()).unwrap_or("");
    let topic: gpui_kit::SharedString = text("kind").to_owned().into();
    let agree = last.data.get("agree").and_then(|v| v.as_bool()).unwrap_or(false);
    match text("step") {
        "co_core" if agree => DualCore::Agreed { topic },
        // Disagreement with no arbiter step yet: the decision is pending.
        "co_core" => DualCore::Deciding { topic },
        "arbiter" => {
            let id = text("id");
            let to_person = all.iter().any(|e| {
                e.kind == kinds::PERSON_REQUEST
                    && e.seq > last.seq
                    && e.data.get("id").and_then(|v| v.as_str()) == Some(id)
            });
            if to_person {
                DualCore::Split { topic }
            } else {
                DualCore::Adjudicated {
                    topic,
                    by: "arbiter".into(),
                }
            }
        }
        _ => DualCore::Off,
    }
}

impl DualCoreSource for EventDualCore {
    fn state(&self, session: &str) -> DualCore {
        match session_store(&self.home, session) {
            Ok(store) => project(&store),
            Err(_) => DualCore::Off,
        }
    }
}
