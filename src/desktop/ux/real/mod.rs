pub mod approvals;
pub mod background;
pub mod dual_core;
pub mod extreme;
pub mod interjections;
pub mod modes;
pub mod storage;
pub mod unit_actions;
pub mod wiring;

#[cfg(test)]
mod approvals_accept;
#[cfg(test)]
mod background_accept;
#[cfg(test)]
mod dual_core_accept;
#[cfg(test)]
mod extreme_accept;
#[cfg(test)]
mod foundation_accept;
#[cfg(test)]
mod interjections_accept;
#[cfg(test)]
mod modes_accept;
#[cfg(test)]
mod storage_accept;
#[cfg(test)]
mod unit_actions_accept;
#[cfg(test)]
mod wiring_accept;

use crate::base::{Failure, Res};
use std::path::{Path, PathBuf};

pub(crate) fn session_store(home: &Path, session: &str) -> Res<PathBuf> {
    let goal = crate::goal::load(home, session)?
        .ok_or_else(|| Failure::new("not_found", format!("no goal for session {session}")))?;
    crate::orch::store_for_goal(home, &goal)
}
