use crate::{
    base::{Failure, Res},
    modes::providers::Member,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pick {
    pub member: Member,
    pub reason: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    pub picks: Vec<Pick>,
    #[serde(default)]
    pub note: String,
}
pub trait Router {
    fn route(&self, text: &str, context: &str) -> Res<Routing>;
}
pub fn parse_routing(v: &serde_json::Value) -> Res<Routing> {
    let routing: Routing = serde_json::from_value(v.clone())
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    validate(&routing)?;
    Ok(routing)
}
pub fn validate(r: &Routing) -> Res<()> {
    let mut members = BTreeSet::new();
    if r.picks.len() > 3
        || r.picks
            .iter()
            .any(|p| p.reason.trim().is_empty() || !members.insert(p.member))
    {
        return Err(Failure::new(
            "invalid_argument",
            "routing requires at most three distinct members with non-empty reasons",
        ));
    }
    Ok(())
}
