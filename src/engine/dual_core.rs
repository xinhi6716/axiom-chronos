use super::{
    budgets::{self, Role, Spend},
    calls::{self, EngineCall},
    kinds, policy,
};
use crate::{
    base::{Failure, Res},
    codex::AppServer,
    council,
    orch::events,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    PlanAcceptance,
    CouncilRuling,
    ModuleMap,
    FixRounds,
    ScopeReduction,
    GoalClose,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::PlanAcceptance => "plan_acceptance",
            Self::CouncilRuling => "council_ruling",
            Self::ModuleMap => "module_map",
            Self::FixRounds => "fix_rounds",
            Self::ScopeReduction => "scope_reduction",
            Self::GoalClose => "goal_close",
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub kind: Kind,
    pub id: String,
    pub question: String,
    pub inputs: Vec<PathBuf>,
    pub core_choice: String,
    pub core_reasoning: String,
    pub irreversible: bool,
    pub reserved: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Off,
    Agree,
    Upheld { reason: String },
    Overruled { reason: String },
    ToPerson { reason: String },
    Reserved,
}

fn delta(before: Spend, after: Spend) -> Option<f64> {
    if before.unknown || after.unknown {
        None
    } else {
        Some(after.usd - before.usd)
    }
}
fn field<'a>(value: &'a Value, key: &str) -> Res<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Failure::new("invalid_response", format!("missing or invalid {key}")))
}
pub fn judge(codex: &AppServer, store: &Path, cwd: &Path, d: &Decision) -> Res<Verdict> {
    if !policy::load(store)?.dual_core {
        return Ok(Verdict::Off);
    }
    if d.reserved {
        return Ok(Verdict::Reserved);
    }
    let inputs = d
        .inputs
        .iter()
        .map(|path| {
            fs::read_to_string(cwd.join(path))
                .map_err(|e| Failure::new("io", format!("{}: {e}", path.display())))
        })
        .collect::<Res<Vec<_>>>()?;
    let pack = json!({"question": d.question, "inputs": inputs, "core_choice": d.core_choice});
    let co_call = EngineCall {
        role: Role::CoCore,
        case: d.kind.name(),
        pack: pack.to_string(),
        cwd,
        schema: json!({"type":"object","properties":{"choice":{"type":"string"},"reason":{"type":"string"}},"required":["choice","reason"],"additionalProperties":false}),
        timeout_s: 600,
    };
    let before = budgets::spent(store, Role::CoCore)?;
    let result = calls::astra_max(codex, store, &co_call)?;
    let after = budgets::spent(store, Role::CoCore)?;
    let choice = field(&result, "choice")?;
    let reason = field(&result, "reason")?;
    let agree = choice.trim().to_lowercase() == d.core_choice.trim().to_lowercase();
    let core = budgets::spent(store, Role::Core)?;
    events::append_event(
        store,
        kinds::DUAL_CORE,
        None,
        None,
        json!({
            "id":d.id,"kind":d.kind.name(),"step":"co_core","choice":choice,"agree":agree,
            "usd":delta(before, after),"core_usd":if core.unknown { None } else { Some(core.usd) }
        }),
    )?;
    if agree {
        return Ok(Verdict::Agree);
    }
    let arbiter_pack = json!({"question":d.question,"inputs":inputs,"core_choice":d.core_choice,"co_core_choice":choice,"co_core_reason":reason});
    let arbiter_call = EngineCall {
        role: Role::Arbiter,
        case: d.kind.name(),
        pack: arbiter_pack.to_string(),
        cwd,
        schema: json!({"type":"object","properties":{"uphold":{"type":"string","enum":["core","co_core"]},"reason":{"type":"string"}},"required":["uphold","reason"],"additionalProperties":false}),
        timeout_s: 600,
    };
    let before = budgets::spent(store, Role::Arbiter)?;
    let result = calls::claude_quick(store, &arbiter_call, council::FABLE_MODEL)?;
    let after = budgets::spent(store, Role::Arbiter)?;
    let upheld = field(&result, "uphold")?;
    if !matches!(upheld, "core" | "co_core") {
        return Err(Failure::new("invalid_response", "invalid uphold"));
    }
    let reason = field(&result, "reason")?.to_owned();
    events::append_event(
        store,
        kinds::DUAL_CORE,
        None,
        None,
        json!({
            "id":d.id,"kind":d.kind.name(),"step":"arbiter","choice":upheld,"agree":upheld == "core","usd":delta(before, after)
        }),
    )?;
    if upheld == "core" {
        return Ok(Verdict::Upheld { reason });
    }
    if d.irreversible {
        events::append_event(
            store,
            kinds::PERSON_REQUEST,
            None,
            None,
            json!({"id":d.id,"kind":d.kind.name(),"reason":reason}),
        )?;
        Ok(Verdict::ToPerson { reason })
    } else {
        Ok(Verdict::Overruled { reason })
    }
}
