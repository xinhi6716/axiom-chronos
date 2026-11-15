use crate::{
    base::{self, Failure, Res},
    codex::{AppServer, RunRequest},
    engine::{
        self,
        budgets::{self, Role},
        calls::EngineCall,
        finding_verification,
        policy::{self, ReviewerMode},
    },
    orch,
    usage::price::{self, CodexUsage},
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
    engine_review_with_home(codex, None, store, unit, repo)
}

pub fn engine_review_with_home(
    codex: &AppServer,
    home: Option<&Path>,
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
    if !matches!(
        orch::selfcheck_state(store, unit)?,
        orch::SelfCheckState::NotRequired | orch::SelfCheckState::Passed
    ) {
        return Err(Failure::new(
            "selfcheck_required",
            format!("self-check required for {unit}"),
        ));
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
    let brief = orch::unit_brief(store, unit)?;
    if tree.is_empty() || !pack.contains(&tree) || orch::snapshot(repo, &brief.scope) != tree {
        return Err(Failure::new("no_review_pack", "review pack is not current"));
    }
    let stage = policy::effective_models(home, store)?.models.reviewer;
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
    budgets::check(store, call.role, call.case)?;
    let model = codex.resolve_model(&stage.tier)?;
    let report = codex.run(&RunRequest {
        model: model.clone(),
        effort: Some(stage.effort),
        sandbox: "read-only".into(),
        developer_instructions: None,
        lean: true,
        web_search: false,
        output_schema: Some(call.schema.clone()),
        brief: format!("AXIOM-ENGINE reviewer review\n{}", call.pack),
        cwd: call.cwd.to_string_lossy().into_owned(),
        timeout_s: call.timeout_s,
        keep_plugins: vec![],
    })?;
    let usage = &report["usage"];
    let input = usage["inputTokens"].as_u64();
    let cached = usage["cachedInputTokens"].as_u64();
    let output = usage["outputTokens"].as_u64();
    let usd = match (input, cached, output) {
        (Some(input), Some(cached_input), Some(output)) => price::codex_usd(
            &model, &CodexUsage { input, cached_input, output },
        ),
        _ => None,
    };
    budgets::record(store, call.role, &model, usd, input.unwrap_or(0), output.unwrap_or(0), call.case)?;
    if report["status"] != "completed" {
        return Err(Failure::new(
            report["error"]["kind"].as_str().unwrap_or("codex_call"),
            report["error"]["message"].as_str().unwrap_or("Codex call failed"),
        ));
    }
    let mut value = report["structured_output"].clone();
    let object = value
        .as_object_mut()
        .ok_or_else(|| Failure::new("invalid_argument", "review verdict must be an object"))?;
    object.insert("unit".into(), json!(unit));
    object.insert("tree".into(), json!(tree));
    // Preserve supplied thread evidence without inventing an id when it is absent.
    object.remove("thread_id");
    if let Some(thread) = report["thread_id"].as_str() {
        object.insert("thread_id".into(), json!(thread));
    }
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
