use crate::{
    base::{self, Failure, Res},
    codex::AppServer,
    engine::{
        self,
        budgets::Role,
        calls::{self, EngineCall},
        finding_verification,
        policy::{self, ReviewerMode},
    },
    orch,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub struct ReviewItem {
    pub unit: String,
    pub reviewer: String,
    pub pack: PathBuf,
    pub verdict_file: PathBuf,
}

pub fn review_items(store: &Path, unit: &str) -> Res<Vec<ReviewItem>> {
    base::require_slug("unit", unit)?;
    let policy = policy::load(store)?;
    let rows = orch::evidence(store, unit)?;
    let rows = rows
        .iter()
        .rposition(|r| r.kind == "run")
        .map_or(&[][..], |i| &rows[i..]);
    let tree = rows
        .iter()
        .rev()
        .find(|r| r.kind == "verify")
        .map(|r| r.commit.as_str())
        .unwrap_or("");
    let mut names = Vec::new();
    if matches!(policy.reviewer, ReviewerMode::Claude | ReviewerMode::Both) {
        names.push("chronos:reviewer");
    }
    if policy.pm_qa {
        names.push("chronos:pm-qa");
    }
    Ok(names
        .into_iter()
        .filter(|name| {
            let matching: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.kind == "review"
                        && r.command
                            .split_once(orch::CARRIED_MARK)
                            .map_or(r.command.as_str(), |(s, _)| s)
                            == *name
                        && r.commit == tree
                })
                .collect();
            matching.is_empty()
                || matching
                    .iter()
                    .any(|(i, _)| !orch::evidence::review_passes(rows, *i))
        })
        .map(|name| ReviewItem {
            unit: unit.into(),
            reviewer: name.into(),
            pack: orch::pack_path(store, unit),
            verdict_file: orch::review_verdict_path(store, unit, name),
        })
        .collect())
}

pub fn engine_review(
    codex: &AppServer,
    store: &Path,
    unit: &str,
    repo: &Path,
) -> Res<Option<Value>> {
    if !matches!(
        policy::load(store)?.reviewer,
        ReviewerMode::AstraMax | ReviewerMode::Both
    ) {
        return Ok(None);
    }
    base::require_slug("unit", unit)?;
    let tree = orch::evidence(store, unit)?
        .into_iter()
        .rev()
        .find(|r| r.kind == "verify")
        .filter(|r| r.exit_code == 0)
        .map(|r| r.commit)
        .ok_or_else(|| Failure::new("not_built", "no passing verify"))?;
    let pack = std::fs::read_to_string(orch::pack_path(store, unit))
        .map_err(|e| Failure::new("no_review_pack", e.to_string()))?;
    if tree.is_empty() || !pack.contains(&tree) {
        return Err(Failure::new("no_review_pack", "review pack is not current"));
    }
    let model = codex.resolve_model("astra")?;
    let call = EngineCall {
        role: Role::Reviewer,
        case: "review",
        cwd: repo,
        timeout_s: 900,
        pack: format!(
            "{}\n\n{}",
            engine::asset("roles/astra-reviewer.md").unwrap_or(""),
            pack
        ),
        schema: json!({"type":"object","properties":{"verdict":{"type":"string"},"findings":{"type":"array","items":{"type":"object","properties":{"severity":{"type":"string"},"location":{"type":"string"},"check":{"type":"string"},"evidence":{"type":"string"},"fix":{"type":"string"},"needs":{"type":["string","null"]}}}}},"required":["verdict","findings"]}),
    };
    let mut value = calls::astra_max(codex, store, &call)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| Failure::new("invalid_argument", "review verdict must be an object"))?;
    object.insert("unit".into(), json!(unit));
    object.insert("tree".into(), json!(tree));
    let text = serde_json::to_string_pretty(&value)
        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
    let verdict = orch::parse_verdict(&text)?;
    let reviewer = format!("engine-astra:{model}");
    let path = orch::review_verdict_path(store, unit, &reviewer);
    base::atomic_write(&path, text.as_bytes()).map_err(|e| Failure::new("io", e.to_string()))?;
    orch::record_review(
        store,
        &[unit.into()],
        &reviewer,
        orch::passes(&verdict, &BTreeSet::new()),
        repo,
    )?;
    finding_verification::verify_review(codex, store, unit, repo)?;
    orch::evidence::update_unit_state(store, unit)?;
    Ok(Some(value))
}
