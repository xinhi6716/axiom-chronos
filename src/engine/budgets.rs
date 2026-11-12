use super::{kinds, policy};
use crate::{
    base::{Failure, Res},
    orch::events::{self, Event},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, str::FromStr};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Core,
    Dispatcher,
    Reviewer,
    Verifier,
    Adjudicator,
    CouncilMember,
    CoCore,
    Arbiter,
}
impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Dispatcher => "dispatcher",
            Self::Reviewer => "reviewer",
            Self::Verifier => "verifier",
            Self::Adjudicator => "adjudicator",
            Self::CouncilMember => "council_member",
            Self::CoCore => "co_core",
            Self::Arbiter => "arbiter",
        }
    }
}
impl FromStr for Role {
    type Err = Failure;
    fn from_str(s: &str) -> Res<Self> {
        match s {
            "core" => Ok(Self::Core),
            "dispatcher" => Ok(Self::Dispatcher),
            "reviewer" => Ok(Self::Reviewer),
            "verifier" => Ok(Self::Verifier),
            "adjudicator" => Ok(Self::Adjudicator),
            "council_member" => Ok(Self::CouncilMember),
            "co_core" => Ok(Self::CoCore),
            "arbiter" => Ok(Self::Arbiter),
            _ => Err(Failure::new(
                "invalid_argument",
                format!("unknown role: {s}"),
            )),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spend {
    pub usd: f64,
    pub unknown: bool,
}
pub fn spent(store: &Path, role: Role) -> Res<Spend> {
    let mut spend = Spend {
        usd: 0.0,
        unknown: false,
    };
    for e in events::read_events(store, 0)? {
        if e.kind == kinds::ROLE_SPEND && e.data["role"] == role.name() {
            match e.data.get("usd") {
                Some(Value::Number(n)) => match n.as_f64() {
                    Some(v) if v.is_finite() && v >= 0.0 => {
                        spend.usd += v;
                        if !spend.usd.is_finite() {
                            spend.unknown = true;
                        }
                    }
                    _ => spend.unknown = true,
                },
                _ => spend.unknown = true,
            }
        }
    }
    Ok(spend)
}
pub fn check(store: &Path, role: Role, call: &str) -> Res<()> {
    let Some(limit) = policy::load(store)?.budgets.get(&role).copied() else {
        return Ok(());
    };
    let s = spent(store, role)?;
    if s.unknown || s.usd >= limit {
        let reason = if s.unknown { "unknown_spend" } else { "spent" };
        events::append_event(
            store,
            kinds::BUDGET_EXCEEDED,
            None,
            None,
            json!({"role":role.name(),"limit_usd":limit,"spent_usd":if s.unknown { None } else { Some(s.usd) },"reason":reason,"call":call}),
        )?;
        return Err(Failure::new(
            "budget_exceeded",
            format!("{} budget exceeded: {reason}", role.name()),
        ));
    }
    Ok(())
}
pub fn record(
    store: &Path,
    role: Role,
    model: &str,
    usd: Option<f64>,
    input_tokens: u64,
    output_tokens: u64,
    call: &str,
) -> Res<Event> {
    if usd.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(Failure::new("invalid_argument", "invalid spend"));
    }
    events::append_event(
        store,
        kinds::ROLE_SPEND,
        None,
        None,
        json!({"role":role.name(),"model":model,"usd":usd,"input_tokens":input_tokens,"output_tokens":output_tokens,"call":call}),
    )
}
pub fn role_for_agent(subagent: &str) -> Option<Role> {
    let name = subagent.strip_prefix("chronos:")?;
    Some(match name {
        "dispatcher" => Role::Dispatcher,
        "reviewer" | "pm-qa" => Role::Reviewer,
        _ => Role::Core,
    })
}
pub fn role_for_codex(role: &str) -> Role {
    match role {
        "terra" | "sol" => Role::Dispatcher,
        _ => Role::Core,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spend_limits_and_unknown_are_enforced() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        record(p, Role::Reviewer, "model", Some(0.5), 10, 2, "first").unwrap();
        assert_eq!(
            spent(p, Role::Reviewer).unwrap(),
            Spend {
                usd: 0.5,
                unknown: false
            }
        );
        assert!(check(p, Role::Reviewer, "x").is_ok());
        policy::set(p, "budget.reviewer", "0.5").unwrap();
        assert_eq!(
            check(p, Role::Reviewer, "second").unwrap_err().kind,
            "budget_exceeded"
        );
        let e = events::read_events(p, 0).unwrap();
        assert_eq!(
            e[1].data,
            json!({"role":"reviewer","limit_usd":0.5,"spent_usd":0.5,"reason":"spent","call":"second"})
        );
        policy::set(p, "budget.core", "2").unwrap();
        record(p, Role::Core, "unknown", None, 1, 1, "x").unwrap();
        assert!(spent(p, Role::Core).unwrap().unknown);
        assert_eq!(
            check(p, Role::Core, "y").unwrap_err().kind,
            "budget_exceeded"
        );
        let e = events::read_events(p, 0).unwrap();
        assert_eq!(e.last().unwrap().data["reason"], "unknown_spend");
        assert!(e.last().unwrap().data["spent_usd"].is_null());
        assert!(record(p, Role::Core, "bad", Some(f64::NAN), 0, 0, "x").is_err());
    }
    #[test]
    fn routing_and_names() {
        for r in [
            Role::Core,
            Role::Dispatcher,
            Role::Reviewer,
            Role::Verifier,
            Role::Adjudicator,
            Role::CouncilMember,
            Role::CoCore,
            Role::Arbiter,
        ] {
            assert_eq!(r.name().parse::<Role>().unwrap(), r);
        }
        assert!("unknown".parse::<Role>().is_err());
        assert_eq!(role_for_agent("other:reviewer"), None);
        assert_eq!(role_for_agent("chronos:pm-qa"), Some(Role::Reviewer));
        assert_eq!(role_for_agent("chronos:dispatcher"), Some(Role::Dispatcher));
        assert_eq!(role_for_agent("chronos:architect"), Some(Role::Core));
        assert_eq!(role_for_codex("sol"), Role::Dispatcher);
        assert_eq!(role_for_codex("terra"), Role::Dispatcher);
        assert_eq!(role_for_codex("astra"), Role::Core);
    }
}
