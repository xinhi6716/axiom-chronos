//! The real extreme-mode policy: one setting in `codex::extreme` at the slots home, shared by the
//! desktop toggles and `axiom extreme`.
use crate::{
    base::Res,
    codex::extreme,
    desktop::ux::seams::{FastCost, TurnPolicy, TurnSettings},
};
use std::path::PathBuf;

pub struct CodexTurnPolicy {
    slots_home: PathBuf,
}

impl CodexTurnPolicy {
    pub fn new(slots_home: PathBuf) -> Self {
        Self { slots_home }
    }
    pub fn slots_home(&self) -> &std::path::Path {
        &self.slots_home
    }
    fn settings(on: bool) -> TurnSettings {
        // The persisted setting has no revision counter; use its value as a stable version.
        TurnSettings {
            extreme: on,
            revision: u64::from(on),
        }
    }
}

impl TurnPolicy for CodexTurnPolicy {
    fn current(&self) -> TurnSettings {
        Self::settings(extreme::load(&self.slots_home).on)
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
        Ok(Self::settings(saved.on))
    }
    fn begin_turn(&self) -> TurnSettings {
        self.current()
    }
    fn fast_cost(&self) -> Option<FastCost> {
        None
    }
}
