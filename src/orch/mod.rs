pub mod commands;
pub mod contracts;
mod evidence;
pub mod lifecycle;
pub mod parse;
pub mod store;

pub use commands::*;
pub use contracts::*;
pub use evidence::*;
pub use lifecycle::*;
pub use parse::*;
pub use store::*;

pub mod wait;
pub use wait::*;

// Keep all public acceptance/reuse paths on the same persisted release gate.
pub fn current_verified_unit(
    store: &std::path::Path,
    unit: &str,
    repo: &std::path::Path,
) -> crate::base::Res<bool> {
    let state = wait_state(store)?;
    current_verified_unit_in_state(store, &state, unit, repo)
}
// Caller has replay-validated state; product commits use this under the same
// store lock so they cannot select a weaker engineering acceptance path.
pub(crate) fn current_verified_unit_in_state(
    store: &std::path::Path,
    state: &WaitState,
    unit: &str,
    repo: &std::path::Path,
) -> crate::base::Res<bool> {
    if (state.engineering_enrolled || store.join("wait-required").exists())
        && !store.join("units.tsv").is_file()
    {
        return Err(crate::base::Failure::new(
            "invalid_argument",
            "opted-in workflow requires a registered unit store",
        ));
    }
    if store.join("wait-required").exists() && current_checkpoint(state, unit).is_none() {
        return Ok(false);
    }
    if !task_released(state, unit) || !evidence::current_verified_unit(store, unit, repo)? {
        return Ok(false);
    }
    if let Some(c) = state
        .checkpoints
        .iter()
        .filter(|c| c.task == unit)
        .max_by_key(|c| c.round)
    {
        if !wait::package_current(state, unit) {
            return Ok(false);
        }
        if c.contract != contract_identity(store, unit)?
            || std::fs::canonicalize(&c.repo).ok() != std::fs::canonicalize(repo).ok()
        {
            return Ok(false);
        }
        let identity = snapshot(repo, &unit_brief(store, unit)?.scope);
        if !state.events.iter().any(|e| {
            e.task == unit && e.round == c.round && e.kind == "release" && e.artifact == identity
        }) {
            return Ok(false);
        }
    }
    if store.join("modules.md").exists() {
        let map = parse_map(
            &std::fs::read_to_string(store.join("modules.md"))
                .map_err(|e| crate::base::Failure::new("io", e.to_string()))?,
        )
        .map_err(|e| crate::base::Failure::new("invalid_argument", e))?;
        let units = read_units(store)?;
        if let Some(u) = units.iter().find(|u| u.id == unit) {
            if let Some(m) = map.modules.iter().find(|m| m.id == u.track) {
                for d in units.iter().filter(|u| m.depends.contains(&u.track)) {
                    if !current_verified_unit_in_state(store, state, &d.id, repo)? {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
}
pub fn done_gate(store: &std::path::Path, change_mode: bool) -> crate::base::Res<()> {
    release_gate(store)?;
    product_done_gate(store)?;
    evidence::done_gate(store, change_mode)
}
pub fn done_gate_in_repo(
    store: &std::path::Path,
    change_mode: bool,
    repo: Option<&std::path::Path>,
) -> crate::base::Res<()> {
    release_gate(store)?;
    product_done_gate(store)?;
    evidence::done_gate_in_repo(store, change_mode, repo)?;
    if let Some(repo) = repo {
        if store.join("units.tsv").exists() {
            for u in read_units(store)? {
                if !current_verified_unit(store, &u.id, repo)? {
                    return Err(crate::base::Failure::new(
                        "not_done",
                        format!("unreleased or stale unit: {}", u.id),
                    ));
                }
            }
        }
    }
    Ok(())
}
fn product_done_gate(store: &std::path::Path) -> crate::base::Res<()> {
    // An absent store has no product journal. Do not bypass locking for an
    // existing (including partially initialized) store.
    match std::fs::symlink_metadata(store) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(crate::base::Failure::new("io", e.to_string())),
        Ok(_) => {}
    }
    let state = wait_state(store)?;
    let mut unfinished = state.product_runs.is_empty();
    for run in &state.product_runs {
        unfinished |= !wait::product_policy_finished_in_store(store, &state, &run.id)?;
    }
    if (state.engineering_enrolled || store.join("wait-required").exists() || !state.product_runs.is_empty()) && unfinished {
        return Err(crate::base::Failure::new("not_done", "product rounds await completion"));
    }
    Ok(())
}
pub fn record_review(
    store: &std::path::Path,
    units: &[String],
    reviewer: &str,
    pass: bool,
    repo: &std::path::Path,
) -> crate::base::Res<Vec<(String, String)>> {
    evidence::record_review(store, units, reviewer, pass, repo)?;
    let rows = read_units(store)?;
    Ok(units
        .iter()
        .map(|id| {
            (
                id.clone(),
                rows.iter().find(|u| u.id == *id).unwrap().state.clone(),
            )
        })
        .collect())
}

/// One user-requested round, independent of the legacy multi-round controller.
/// Unit release, source-bound reviews and full tests remain mandatory.
pub(crate) fn round_evidence_gate(store: &std::path::Path, repo: &std::path::Path) -> crate::base::Res<()> {
    release_gate(store)?;
    // Presentation rewrite is optional; it cannot replace engineering evidence.
    evidence::tested_gate_in_repo(store,true,Some(repo),false)?;
    for unit in read_units(store)? {
        if !current_verified_unit(store,&unit.id,repo)? {
            return Err(crate::base::Failure::new("not_done",format!("unverified unit: {}",unit.id)));
        }
    }
    Ok(())
}
