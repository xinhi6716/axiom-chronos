use crate::{
    base::{self, Failure, Res},
    modes::{
        providers::{Member, Reply},
        readonly::READ_TOOLS,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Budget {
    pub max_calls: u32,
    pub max_usd: Option<f64>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Budgets(pub BTreeMap<Member, Budget>);
#[derive(Default, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Spend {
    pub calls: u32,
    pub usd: f64,
    pub unpriced_calls: u32,
}
pub fn grant() -> Vec<String> {
    READ_TOOLS.iter().map(|s| (*s).to_owned()).collect()
}
fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn dir(home: &Path, session: &str) -> Res<PathBuf> {
    base::require_slug("session", session)?;
    Ok(home.join("modes").join("sessions").join(session))
}
pub(crate) fn save(home: &Path, session: &str, budgets: &Budgets) -> Res<()> {
    base::atomic_write(
        &dir(home, session)?.join("budgets.json"),
        &serde_json::to_vec(budgets).map_err(error)?,
    )
    .map_err(error)
}
pub(crate) fn load(home: &Path, session: &str) -> Res<Budgets> {
    serde_json::from_slice(&fs::read(dir(home, session)?.join("budgets.json")).map_err(error)?)
        .map_err(error)
}
pub fn spend(home: &Path, session: &str) -> Res<BTreeMap<Member, Spend>> {
    match fs::read(dir(home, session)?.join("spend.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(error),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(error(e)),
    }
}
pub fn check(home: &Path, session: &str, member: Member) -> Res<()> {
    let budgets = load(home, session)?;
    let spent = spend(home, session)?
        .get(&member)
        .cloned()
        .unwrap_or_default();
    let allowed = budgets.0.get(&member).is_some_and(|b| {
        spent.calls < b.max_calls && b.max_usd.is_none_or(|limit| spent.usd < limit)
    });
    if allowed {
        Ok(())
    } else {
        Err(Failure::new(
            "budget",
            format!("{} is over budget", member.name()),
        ))
    }
}
pub fn settle(home: &Path, session: &str, member: Member, reply: &Reply) -> Res<Spend> {
    let mut all = spend(home, session)?;
    let entry = all.entry(member).or_default();
    entry.calls = entry.calls.saturating_add(1);
    match reply.cost_usd {
        Some(cost) => entry.usd += cost,
        None => entry.unpriced_calls = entry.unpriced_calls.saturating_add(1),
    }
    let result = entry.clone();
    base::atomic_write(
        &dir(home, session)?.join("spend.json"),
        &serde_json::to_vec(&all).map_err(error)?,
    )
    .map_err(error)?;
    Ok(result)
}
