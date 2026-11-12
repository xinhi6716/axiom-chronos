//! The real extreme-mode policy: one setting in `codex::extreme` at the slots home, shared by the
//! desktop toggles and `axiom extreme`.
use crate::{
    base::Res,
    codex::extreme,
    desktop::ux::seams::{FastCost, TurnPolicy, TurnSettings},
};
use std::{path::PathBuf, sync::Mutex};

pub struct CodexTurnPolicy {
    slots_home: PathBuf,
    seen: Mutex<TurnSettings>,
}

impl CodexTurnPolicy {
    pub fn new(slots_home: PathBuf) -> Self {
        let on = extreme::load(&slots_home).on;
        Self {
            slots_home,
            seen: Mutex::new(TurnSettings {
                extreme: on,
                revision: 0,
            }),
        }
    }
    pub fn slots_home(&self) -> &std::path::Path {
        &self.slots_home
    }
    fn observe(&self, on: bool) -> TurnSettings {
        let mut seen = self.seen.lock().unwrap();
        if seen.extreme != on {
            seen.extreme = on;
            seen.revision += 1;
        }
        *seen
    }
}

impl TurnPolicy for CodexTurnPolicy {
    fn current(&self) -> TurnSettings {
        self.observe(extreme::load(&self.slots_home).on)
    }
    fn set_extreme(&self, on: bool) -> TurnSettings {
        match self.try_set_extreme(on) {
            Ok(settings) => settings,
            Err(e) => {
                eprintln!("could not save extreme mode: {}", e.message);
                self.current()
            }
        }
    }
    fn try_set_extreme(&self, on: bool) -> Res<TurnSettings> {
        let saved = extreme::set(&self.slots_home, on)?;
        Ok(self.observe(saved.on))
    }
    fn begin_turn(&self) -> TurnSettings {
        self.current()
    }
    fn fast_cost(&self) -> Option<FastCost> {
        None
    }
}
