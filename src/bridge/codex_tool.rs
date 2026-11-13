use super::server::Bridge;
use crate::orch::summary::{self, Next, Outcome, ReviewItem};
use crate::{
    base::{self, Failure, Res},
    codex::{self, RunRequest},
    engine::{
        budgets,
        policy::{self, ReviewerMode},
    },
    goal, graph, orch,
    review::roles as review_roles,
    usage::price::{self, CodexUsage},
};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

fn invalid(message: &str) -> Failure {
    Failure::new("invalid_argument", message)
}
fn string<'a>(a: &'a Value, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}
pub fn codex_run(b: &Bridge, a: &Value) -> Value {
    let cycle = a.get("cycle") == Some(&Value::Bool(true)) && string(a, "unit").is_some();
    let root = if cycle {
        string(a, "project").and_then(|p| b.home().ok().and_then(|h| orch::store_for(h, p).ok()))
    } else { None };
    let before = root.as_ref().and_then(|r| summary::cycle_ready(r).ok()).unwrap_or_default();
    let mut result = match run(b, a) {
        Ok(v) => v,
        Err(e) => json!({"status":"bridge_error","error":e.to_json()}),
    };
    if cycle {
        let ready = root.as_ref().and_then(|r| summary::cycle_ready(r).ok()).unwrap_or_default();
        result["ready"] = json!(ready.into_iter().filter(|id| !before.contains(id)).collect::<Vec<_>>());
        result["core"] = summary::cycle_core(&result);
        result["handoff"] = json!(summary::cycle_handoff(&result));
    }
    result
}
pub(super) fn destination(root: &Path, path: &str) -> Res<PathBuf> {
    let dest = Path::new(path);
    if !dest.is_absolute() {
        return Err(invalid("save_to must be absolute and inside store"));
    }
    let relative = dest
        .strip_prefix(root)
        .map_err(|_| invalid("save_to must be inside store"))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid("save_to must be inside store"));
    }
    std::fs::create_dir_all(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let mut parent = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for c in &components[..components.len() - 1] {
        parent.push(c.as_os_str());
        std::fs::create_dir_all(&parent).map_err(|e| Failure::new("io", e.to_string()))?;
        if !std::fs::canonicalize(&parent)
            .map_err(|e| Failure::new("io", e.to_string()))?
            .starts_with(&canonical_root)
        {
            return Err(invalid("save_to must be inside store"));
        }
    }
    if dest.is_symlink() {
        return Err(invalid("save_to must be inside store"));
    }
    Ok(dest.to_path_buf())
}
/// Registers `unit` under `track` from the brief at `brief_file`, so the
/// dispatcher's register-then-dispatch pair is one `codex_run` call.
/// Returns false when the unit is already registered under that track.
fn register(store: &Path, unit: &str, track: &str, brief_file: &str) -> Res<bool> {
    if let Some(existing) = orch::read_units(store)?.iter().find(|u| u.id == unit) {
        if existing.track != track {
            return Err(invalid(&format!(
                "unit {unit} is registered under track {}, not {track}",
                existing.track
            )));
        }
        return Ok(false);
    }
    let args: Vec<String> = ["unit", "add", unit, "--track", track, "--brief", brief_file]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let out = orch::run(store, &args, false, false);
    if out.exit_code != 0 {
        let text = if out.stderr.is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        return Err(Failure::new("orch_failed", text.trim().to_string()));
    }
    Ok(true)
}

fn ensure_review_requirements(root: &Path, unit: &str) -> Res<()> {
    let policy = policy::load(root)?;
    if policy.reviewer == ReviewerMode::Claude && !policy.pm_qa {
        return Ok(());
    }
    let rows = orch::evidence(root, unit)?;
    let Some(verify) = rows
        .iter()
        .rposition(|r| r.kind == "verify" && r.exit_code == 0)
    else {
        return Ok(());
    };
    let tree = &rows[verify].commit;
    let mut required = Vec::new();
    if matches!(policy.reviewer, ReviewerMode::Claude | ReviewerMode::Both) {
        required.push("chronos:reviewer");
    }
    if matches!(policy.reviewer, ReviewerMode::AstraMax | ReviewerMode::Both) {
        required.push("engine-astra:");
    }
    if policy.pm_qa {
        required.push("chronos:pm-qa");
    }
    for reviewer in required {
        if !rows[verify + 1..]
            .iter()
            .any(|r| r.kind == "require" && r.command == reviewer && r.commit == *tree)
        {
            orch::append_evidence(
                root,
                unit,
                &orch::Evidence {
                    kind: "require".into(),
                    command: reviewer.into(),
                    exit_code: 0,
                    commit: tree.clone(),
                    ts: base::now_iso(),
                },
            )?;
        }
    }
    Ok(())
}
struct UnitGuard(Option<(PathBuf, String, String)>);
fn resumable_cut(kind: &str) -> bool {
    matches!(kind, "interrupted" | "timeout" | "token_cap" | "usage_limit" | "usageLimitExceeded")
}
impl Drop for UnitGuard {
    fn drop(&mut self) {
        if let Some((store, u, attempt_id)) = &self.0 {
            orch::abort_unit(store, u, attempt_id);
        }
    }
}
pub(crate) fn crash_point(name: &str) {
    if cfg!(debug_assertions) && std::env::var("AXIOM_TEST_CRASH_AT").ok().as_deref() == Some(name) {
        std::process::abort();
    }
}

#[allow(clippy::too_many_arguments)]
fn unit_reply(
    b: &Bridge,
    project: &str,
    root: &Path,
    unit: &str,
    repo: &Path,
    registered: bool,
    model: &str,
    attempt: u32,
    attempt_id: Value,
    report: &Value,
    finished: orch::Finished,
    cycle: bool,
    reused: bool,
) -> Res<Value> {
    let built = finished.state == "built" || reused;
    if !reused && built && finished.verify_exit == Some(0) {
        if !orch::unit_brief(root, unit)?.test_matrix.is_empty() {
            let id = attempt_id.as_str().map(str::to_owned)
                .or(codex::routing::read_snapshot(root, unit)?.map(|s|s.attempt_id))
                .ok_or_else(|| Failure::new("red_invalid", "matrix row has no attempt"))?;
            orch::lifecycle::final_matrix(root, unit, repo, &id)?;
        }
    }
    if !reused
        && built
        && finished.verify_exit == Some(0)
        && !matches!(
            orch::selfcheck_state(root, unit)?,
            orch::SelfCheckState::NotRequired | orch::SelfCheckState::Passed
        )
    {
        let id = attempt_id
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                codex::routing::read_snapshot(root, unit)
                    .ok()
                    .flatten()
                    .map(|s| s.attempt_id)
            })
            .unwrap_or_default();
        let check = crate::engine::self_check::run(
            b.codex()?.as_ref(),
            b.home().ok(),
            root,
            unit,
            &id,
            repo,
        )?;
        if check.changed {
            let verify = orch::record_verify(root, unit, repo)?;
            if verify.exit_code != 0 {
                return Err(Failure::new(
                    "verify_failed",
                    "verification after self-check failed",
                ));
            }
            orch::lifecycle::final_matrix(root, unit, repo, &id)?;
        }
        if check.status != "pass" {
            return Err(Failure::new("selfcheck_required", "self-check failed"));
        }
    }
    if built && finished.verify_exit.is_some() {
        if let Some(id) = attempt_id.as_str() { orch::lifecycle::checkpoint_stage(root, unit, id, "selfchecked", repo)?; }
        crash_point("after_selfcheck");
    }
    let mut accept = json!({"ran":false,"reason":if reused {"reused"} else {"not_built"},"detail":"","tests":null,"failing":[],"output_file":null});
    if !reused && built && finished.verify_exit == Some(0) {
        let result = goal::accept_hold(b.home()?, project).and_then(|hold| match hold {
            Some(reason) => Ok(orch::AutoAccept::Skipped {
                reason,
                detail: String::new(),
            }),
            None => orch::accept_if_ready(root, unit, repo),
        });
        match result {
            Ok(orch::AutoAccept::Ran {
                module,
                exit_code,
                output_file,
                states,
                ..
            }) => {
                let output = std::fs::read_to_string(&output_file).unwrap_or_default();
                let sum = summary::summary(Some(exit_code), &output, Some(&output_file), None);
                let states: serde_json::Map<String, Value> =
                    states.into_iter().map(|(u, s)| (u, json!(s))).collect();
                accept = json!({"ran":true,"module":module,"exit_code":exit_code,"states":states,"tests":sum["tests"],"failing":sum["failing"],"output_file":sum["output_file"]});
            }
            Ok(orch::AutoAccept::Skipped { reason, detail }) => {
                accept["reason"] = json!(reason);
                accept["detail"] = json!(summary::one_line(&detail));
            }
            Err(e) => {
                accept["reason"] = json!("error");
                accept["detail"] = json!(summary::one_line(&e.message));
            }
        }
    }
    let mut state = if reused {
        "verified".to_string()
    } else {
        orch::derive_state(
            &orch::evidence(root, unit)?,
            orch::needs_accept(root, unit)?,
        )
    };
    let mut error = if built {
        Value::Null
    } else if report["status"] != "completed" {
        report["error"].clone()
    } else if !finished.outside_scope.is_empty() {
        json!({"kind":"outside_scope","message":"changes outside unit scope"})
    } else if finished.verify_exit.is_some_and(|code| code != 0) {
        json!({"kind":"verify_failed","message":"unit verification failed"})
    } else {
        json!({"kind":"unit_not_built","message":"unit was not built"})
    };
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        error["message"] = json!(summary::one_line(message));
    }
    let output = finished
        .verify_output
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let sum = summary::summary(
        finished.verify_exit,
        &output,
        finished.verify_output.as_deref(),
        Some(&finished.changed),
    );
    let mut result = json!({"status":if built {"completed"} else {"failed"},"error":error,"unit":unit,"state":state,"registered":registered,"model":model,"attempt":attempt,"attempt_id":attempt_id,"verify_exit":finished.verify_exit,"verify_output":sum["output_file"],"tests":sum["tests"],"failing":sum["failing"],"changed":sum["changed"],"outside_scope":finished.outside_scope.iter().take(summary::MAX_PATH_SAMPLE).collect::<Vec<_>>(),"outside_scope_count":finished.outside_scope.len(),"accept":accept,"report":root.join("reports").join(format!("{unit}.md")),"report_json":root.join("reports").join(format!("{unit}.json")),"pack":finished.pack,"usage":report["usage"],"slot_wait_ms":report["slot_wait_ms"],"slot_wait":report["slot_wait"]});
    result["scope_ignored"] = json!(finished
        .ignored
        .iter()
        .take(summary::MAX_PATH_SAMPLE)
        .map(|(path, reason)| json!({"path":path,"reason":reason}))
        .collect::<Vec<_>>());
    result["scope_ignored_count"] = json!(finished.ignored.len());
    result["unattributed"] = json!(finished
        .unattributed
        .iter()
        .take(summary::MAX_PATH_SAMPLE)
        .collect::<Vec<_>>());
    result["unattributed_count"] = json!(finished.unattributed.len());
    if reused {
        result["reused"] = json!(true);
    }
    if let Some(e) = finished.pack_error {
        result["pack_error"] = json!(summary::one_line(&e));
    }
    if cycle {
        let mut next = if reused {
            Next::Done
        } else {
            summary::next_for(root, unit)?
        };
        let accept_reason = accept["reason"].as_str().unwrap_or("");
        let reason = if accept["ran"] == false
            && !matches!(
                accept_reason,
                "not_built" | "not_module" | "already_accepted" | "reused"
            ) {
            if matches!(
                accept_reason,
                "units_pending" | "claimed" | "amend_pending" | "stopped"
            ) || accept_reason.starts_with("goal_")
            {
                next = Next::Wait;
            }
            format!(
                "{}: {}",
                accept_reason,
                accept["detail"].as_str().unwrap_or("")
            )
        } else if !error.is_null() {
            error["message"].as_str().unwrap_or("").to_owned()
        } else if accept["exit_code"].as_i64().is_some_and(|v| v != 0) {
            "accept failed".into()
        } else {
            String::new()
        };
        let mut review = Vec::new();
        let mut review_reply = Vec::new();
        let mut reviewed_primary = false;
        if next == Next::Review || (accept["ran"] == true && accept["exit_code"] == 0) {
            let units = orch::read_units(root)?;
            let track = units
                .iter()
                .find(|u| u.id == unit)
                .map(|u| u.track.as_str())
                .unwrap_or("");
            for u in units.iter().filter(|u| {
                u.superseded_by().is_none()
                    && (u.id == unit
                        || (accept["ran"] == true && accept["exit_code"] == 0 && u.track == track))
            }) {
                if summary::next_for(root, &u.id)? == Next::Review {
                    ensure_review_requirements(root, &u.id)?;
                    if matches!(
                        policy::load(root)?.reviewer,
                        ReviewerMode::AstraMax | ReviewerMode::Both
                    ) {
                        review_roles::engine_review_with_home(b.codex()?.as_ref(), b.home().ok(), root, &u.id, repo)?;
                        if u.id == unit {
                            if let Some(id) = attempt_id.as_str() { orch::lifecycle::checkpoint_stage(root, unit, id, "reviewed", repo)?; }
                        }
                        crash_point("after_review");
                        reviewed_primary |= u.id == unit;
                    }
                    for item in review_roles::review_items(root, &u.id)? {
                        review_reply.push(
                            json!({"unit":item.unit,"reviewer":item.reviewer,"pack":item.pack}),
                        );
                    }
                    let path = orch::pack_path(root, &u.id);
                    review.push(ReviewItem {
                        unit: u.id.clone(),
                        pack: path.is_file().then_some(path),
                    });
                }
            }
        }
        if reviewed_primary {
            state = orch::derive_state(
                &orch::evidence(root, unit)?,
                orch::needs_accept(root, unit)?,
            );
            let pending_primary = review_reply.iter().any(|item| item["unit"] == unit);
            if next == Next::Review {
                next = if pending_primary && (state == "built" || state == "verified") {
                    Next::Review
                } else if state == "verified" && !pending_primary {
                    Next::Done
                } else {
                    summary::next_for(root, unit)?
                };
            }
        }
        let mut paths = std::collections::BTreeMap::new();
        paths.insert(
            "report".into(),
            root.join("reports").join(format!("{unit}.md")),
        );
        paths.insert(
            "report_json".into(),
            root.join("reports").join(format!("{unit}.json")),
        );
        if let Some(path) = &finished.verify_output {
            paths.insert("verify_output".into(), path.clone());
        }
        if let Some(path) = &finished.pack {
            paths.insert("pack".into(), path.clone());
        }
        if let Some(path) = accept["output_file"].as_str() {
            paths.insert("accept_output".into(), path.into());
        }
        let outcome = Outcome {
            unit: unit.into(),
            state,
            next,
            reason: summary::one_line(&reason),
            verdict: None,
            review,
            paths,
            module_done: None,
        };
        for (key, value) in serde_json::to_value(outcome).unwrap().as_object().unwrap() {
            result[key] = value.clone();
        }
        result["review"] = json!(review_reply);

    }
    // Preserve the ignored-scope sample when a cycle reply's additional paths would
    // otherwise force the shared reply limiter to trim both samples equally.
    let ignored_len = result["scope_ignored"].as_array().map_or(0, Vec::len);
    while ignored_len > 0
        && super::tools::bound_reply(result.clone())["scope_ignored"]
            .as_array()
            .map_or(0, Vec::len)
            < ignored_len
    {
        let Some(sample) = result["changed"]["sample"].as_array_mut() else {
            break;
        };
        if sample.pop().is_none() {
            break;
        }
        result["truncated"] = json!(true);
    }
    Ok(result)
}
fn run(b: &Bridge, a: &Value) -> Res<Value> {
    let role = string(a, "role");
    let cycle = a.get("cycle") == Some(&Value::Bool(true));
    if cycle && string(a, "unit").is_none() {
        return Err(invalid("cycle needs unit mode"));
    }
    if string(a, "unit").is_some()
        && (!matches!(role, Some("terra" | "sol"))
            || a.get("brief").is_some()
            || a.get("save_to").is_some()
            || string(a, "sandbox").is_some_and(|s| s != "workspace-write"))
    {
        return Err(invalid(
            "unit mode takes exactly project, unit, role (terra or sol) and cwd; brief and save_to belong to astra brief mode",
        ));
    }
    if string(a, "unit").is_some() && a.get("effort").is_some() {
        return Err(invalid(
            "a registered unit takes its effort from EFFORT in its brief, not from an effort argument",
        ));
    }
    if a.get("role").is_some() && !matches!(role, Some("terra" | "sol" | "astra")) {
        return Err(invalid("invalid role"));
    }
    let explicit = string(a, "model").filter(|s| !s.is_empty());
    if let (Some(r), Some(m)) = (role, explicit) {
        if !codex::role_matches(r, m) {
            return Err(invalid("model does not match role"));
        }
    }
    if role.is_none() && explicit.is_none() {
        return Err(invalid("model or role is required"));
    }
    if let Some(r) = role {
        crate::laya::router::observe_routing(
            crate::laya::home_from_env().as_deref(),
            None,
            string(a, "unit"),
            r,
            string(a, "brief").unwrap_or(""),
        );
    } // laya-shadow
    let project = string(a, "project");
    let unit = string(a, "unit");
    let save = string(a, "save_to");
    let track = string(a, "track");
    let brief_file = string(a, "brief_file");
    if (track.is_some() || brief_file.is_some()) && unit.is_none() {
        return Err(invalid(
            "track and brief_file register a unit; they need unit mode",
        ));
    }
    if track.is_some() != brief_file.is_some() {
        return Err(invalid("track and brief_file go together"));
    }
    if unit.is_none() && string(a, "brief").is_none() {
        return Err(invalid("brief is required outside unit mode"));
    }
    if a.get("unit").is_some() && (project.is_none() || unit.is_none()) {
        return Err(invalid("unit requires project"));
    }
    if save.is_some() && (project.is_none() || role != Some("astra") || unit.is_some()) {
        return Err(invalid("save_to requires project and astra role"));
    }
    if unit.is_none() && string(a, "sandbox").unwrap_or("workspace-write") != "read-only" {
        return Err(Failure::new("unbound_write", "a writing codex_run needs project and unit; register the unit with orch and dispatch it by id"));
    }
    let store = project.map(|p| orch::store_for(b.home()?, p)).transpose()?;
    let budget_role = budgets::role_for_codex(role.unwrap_or(""));
    if let Some(root) = &store {
        if let Err(e) = budgets::check(root, budget_role, "codex_run") {
            if e.kind == "budget_exceeded" {
                return Ok(json!({"status":"refused","error":e.to_json()}));
            }
            return Err(e);
        }
    }
    let dest = match (save, &store) {
        (Some(s), Some(root)) => Some(destination(root, s)?),
        _ => None,
    };
    let cwd = string(a, "cwd").unwrap_or("");
    base::require_directory(cwd)?;
    let plugins = match a.get("keep_plugins") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("keep_plugins must contain strings"))
            })
            .collect::<Res<Vec<_>>>()?,
        _ => return Err(invalid("keep_plugins must be an array")),
    };
    let mut brief = string(a, "brief").unwrap_or("").to_owned();
    let mut effort = string(a, "effort").map(str::to_owned);
    let mut tier = None;
    let mut routing_snapshot = None;
    let mut attempt = 0;
    let mut attempt_id = String::new();
    let mut guard = UnitGuard(None);
    let mut registered = false;
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        if let (Some(t), Some(file)) = (track, brief_file) {
            registered = register(root, u, t, file)?;
        }
        let active = goal::find_active(b.home()?, project.unwrap())?;
        if active
            .as_ref()
            .is_some_and(|(_, g)| goal::creative_requested(g))
            && !root.join("modules.md").exists()
        {
            return Err(Failure::new(
                "architect_required",
                "creative goal requires modules.md",
            ));
        }
        let required = active.is_some();
        let (begin_result, refused_owner) =
            orch::lifecycle::begin_dispatch(root, u, r, Path::new(cwd), required, cycle);
        let begin = match begin_result {
            Ok(begin) => begin,
            Err(e) if e.kind == "already_running" || e.kind == "already_verified" => {
                let rows = orch::evidence(root, u)?;
                let state = orch::derive_state(&rows, orch::needs_accept(root, u)?);
                let owner = refused_owner.expect("refused begin records its owner under the lock");
                let mut result = json!({"status":"refused", "error":e.to_json(), "unit":u, "state":state, "owner":owner});
                if cycle {
                    result["next"] = json!(if e.kind == "already_running" {
                        "wait"
                    } else {
                        "done"
                    });
                }
                return Ok(result);
            }
            Err(e) => return Err(e),
        };
        match begin {
            None => {
                let rows = orch::evidence(root, u)?;
                let verify = rows.iter().rev().find(|r| r.kind == "verify");
                let saved: Value = std::fs::read(root.join("reports").join(format!("{u}.json")))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .unwrap_or_else(|| json!({}));
                let verify_path = root.join("reports").join(format!("{u}.verify.txt"));
                let pack_path = orch::pack_path(root, u);
                let mut pack = pack_path.is_file().then_some(pack_path.clone());
                let mut pack_error = None;
                if pack.is_none()
                    || std::fs::metadata(&pack_path)
                        .and_then(|m| m.modified())
                        .ok()
                        < std::fs::metadata(&verify_path)
                            .and_then(|m| m.modified())
                            .ok()
                {
                    match orch::review_pack(root, u, Path::new(cwd)) {
                        Ok(info) => pack = Some(info.path),
                        Err(e) => {
                            pack = None;
                            pack_error = Some(e.message);
                        }
                    }
                }
                let scope: Value =
                    std::fs::read(root.join("reports").join(format!("{u}.scope.json")))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                        .unwrap_or_else(|| json!({}));
                let finished = orch::Finished {
                    state: "built".into(),
                    verify_exit: verify.map(|v| v.exit_code),
                    verify_tail: String::new(),
                    verify_output: verify_path.is_file().then_some(verify_path),
                    outside_scope: vec![],
                    ignored: scope["ignored"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| {
                            Some((
                                item["path"].as_str()?.to_owned(),
                                item["reason"].as_str()?.to_owned(),
                            ))
                        })
                        .collect(),
                    unattributed: scope["unattributed"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect(),
                    changed: std::fs::read(root.join("reports").join(format!("{u}.changed.json")))
                        .ok()
                        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                        .unwrap_or_else(|| {
                            let mut paths = orch::lifecycle::report_paths(&saved, Path::new(cwd));
                            paths.sort();
                            paths.dedup();
                            paths
                        }),
                    pack,
                    pack_error,
                };
                let model = rows
                    .iter()
                    .rev()
                    .find(|r| r.kind == "run")
                    .map(|r| r.command.as_str())
                    .unwrap_or("");
                return unit_reply(
                    b,
                    project.unwrap(),
                    root,
                    u,
                    Path::new(cwd),
                    registered,
                    model,
                    rows.iter().filter(|r| r.kind == "run").count() as u32,
                    Value::Null,
                    &saved,
                    finished,
                    cycle,
                    false,
                );
            }
            Some(orch::Begin::Run {
                brief: value,
                attempt: n,
                attempt_id: id,
            }) => {
                attempt = n;
                attempt_id = id;
                guard.0 = Some((root.to_path_buf(), u.to_owned(), attempt_id.clone()));
                if value.tier.as_deref() == Some("luna") && (r != "terra" || explicit.is_some()) {
                    return Err(invalid(
                        "TIER in a brief applies only to a terra unit dispatched without model",
                    ));
                }
                let server = b.codex()?;
                let snapshot = codex::routing::for_attempt(
                    &server,
                    b.home().ok(),
                    root,
                    u,
                    &attempt_id,
                    r,
                    &value,
                    explicit,
                )?;
                orch::lifecycle::checkpoint_routing(root, u, &snapshot)?;
                effort = snapshot
                    .effort
                    .map(|e| format!("{e:?}").to_lowercase())
                    .or(value.effort);
                tier = value.tier;
                brief = value.text;
                routing_snapshot = Some(snapshot);
                brief = graph::constraints::prepend(
                    &brief,
                    &graph::constraints::load(
                        Path::new(cwd),
                        Some(root),
                        graph::constraints::DEFAULT_BUDGET,
                    )?,
                );
            }
            Some(_) => unreachable!("begin never reuses a verified unit"),
        }
    }
    if let Ok(home) = b.home() {
        if let Ok(graph_store) = graph::GraphStore::open(home) {
            let options = graph::code::SyncOptions {
                deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
                ..Default::default()
            };
            let _ = graph::code::sync(&graph_store, Path::new(cwd), None, &options);
            if let Some(root) = &store {
                let _ = graph::ingest::sync_orch(&graph_store, root, Path::new(cwd));
            }
        }
    }
    let mut resolved = explicit.unwrap_or("").to_owned();
    let result = (|| -> Res<(String, Value)> {
        let server = b.codex()?;
        let model = if let Some(snapshot) = &routing_snapshot {
            snapshot.model.clone()
        } else {
            match explicit {
                Some(m) => m.to_owned(),
                None => server.resolve_model(tier.as_deref().or(role).unwrap())?,
            }
        };
        resolved = model.clone();
        let mut req = RunRequest {
            brief,
            cwd: cwd.into(),
            model: model.clone(),
            sandbox: string(a, "sandbox").unwrap_or("workspace-write").into(),
            effort,
            output_schema: a.get("output_schema").filter(|v| !v.is_null()).cloned(),
            developer_instructions: string(a, "developer_instructions").map(str::to_owned),
            timeout_s: a
                .get("timeout_s")
                .and_then(Value::as_u64)
                .filter(|x| *x > 0)
                .unwrap_or(1800),
            lean: a.get("lean") != Some(&Value::Bool(false)),
            keep_plugins: plugins,
            web_search: a.get("web_search") == Some(&Value::Bool(true)),
        };
        let saved_worker = match (&store, unit) {
            (Some(root), Some(u)) => orch::lifecycle::checkpoint_worker(root, u, &attempt_id, None)?,
            _ => None,
        };
        if let (Some(root), Some(u)) = (&store, unit) {
            let matrix = orch::unit_brief(root, u)?;
            if !matrix.test_matrix.is_empty() && !orch::lifecycle::validate_red(root, u, &attempt_id, Path::new(cwd))? {
                let implement_brief = std::mem::take(&mut req.brief);
                let implement_schema = req.output_schema.take();
                req.brief = format!("TESTS-FIRST\nWrite only these matrix test files: {}. Use stubbed signatures so tests fail on assertions, not compile errors. Do not implement production code.\nCONTRACT\n{}\nTEST_MATRIX\n{}",
                    matrix.test_matrix.iter().map(|r|r.file.as_str()).collect::<Vec<_>>().join(", "),
                    matrix.contract.join("\n"),
                    matrix.test_matrix.iter().map(|r|format!("{} | {} | {}::{} | {} | {}",r.id,r.kind,r.file,r.case,r.assertion,r.command)).collect::<Vec<_>>().join("\n"));
                let red_worker = server.run(&req)?;
                req.brief = implement_brief;
                req.output_schema = implement_schema;
                if red_worker["status"] != "completed" { return Err(Failure::new("red_invalid", "matrix tests-first worker did not complete")); }
                orch::lifecycle::record_red(root, u, &attempt_id, Path::new(cwd))?;
            }
        }
        let report = if let Some(report) = saved_worker {
            report
        } else if let Some(root) = &store {
            let path = if let Some(u) = unit {
                orch::scratch_dir(root, &[u, &attempt_id])
            } else {
                orch::scratch_dir(root, &["astra", &base::unique()])
            };
            let (home, owner) = orch::store::storage_context(root);
            crate::storage::ensure_dir(&home, &path, crate::storage::Class::Scratch, &owner)?;
            let _ = crate::storage::ensure_dir(
                &home,
                &root.join("receipts"),
                crate::storage::Class::Evidence,
                &owner,
            );
            let lease = crate::storage::Ledger::open(&home)?.lease(&path)?;
            let result = (|| {
                let mut env = base::proc::scratch_env(&path)
                    .map_err(|e| Failure::new("io", e.to_string()))?;
                let mut roots = if unit.is_some() {
                    vec![path.clone()]
                } else {
                    vec![]
                };
                if unit.is_some() {
                    crate::storage::worktree::register(&home, Path::new(cwd), &owner)?;
                    let worker_env =
                        crate::storage::worktree::worker_env(&home, Path::new(cwd), &owner)?;
                    roots.extend(
                        worker_env
                            .iter()
                            .filter(|(key, _)| key == "CARGO_TARGET_DIR")
                            .map(|(_, value)| PathBuf::from(value)),
                    );
                    env.extend(worker_env);
                }
                server.run_with_scratch(&req, &env, &roots)
            })();
            drop(lease);
            if unit.is_none() || result.is_err() {
                let cleanup = crate::storage::retention::remove_scratch(
                    &home,
                    &path,
                    &owner,
                    "unit_ended",
                    false,
                );
                if cleanup.outcome != "done" {
                    eprintln!("scratch cleanup: {}", cleanup.outcome);
                }
            }
            result?
        } else {
            server.run(&req)?
        };
        Ok((model, report))
    })();
    let (model, mut report) = match result {
        Ok(r) => r,
        Err(e) => {
            if resumable_cut(&e.kind) {
                if let Some((root, u, id)) = &guard.0 {
                    orch::lifecycle::suspend_claim(root, u, id)?;
                    guard.0 = None;
                }
            }
            if dest.is_some() {
                let _ = orch::record_stage(store.as_ref().unwrap(), "astra", &resolved, false);
            }
            return Err(e);
        }
    };
    if unit.is_some() && (resumable_cut(report["status"].as_str().unwrap_or(""))
        || resumable_cut(report["error"]["kind"].as_str().unwrap_or("")))
    {
        // Keep the existing interruption timing receipt for diagnostics. This is
        // not counting run evidence and never drives fix limits or unit state.
        if report["status"] == "interrupted" {
            if let (Some(root), Some(u), Some(started_ms), Some(ended_ms)) = (
                &store, unit, report["run_started_ms"].as_i64(), report["run_ended_ms"].as_i64(),
            ) {
                if orch::receipts::read_runs(root).is_ok_and(|rows| !rows.iter().any(|r| r.unit == u && r.attempt == attempt_id)) {
                    let _ = orch::receipts::record_run(root, &orch::receipts::RunRecord {
                        unit: u.into(), attempt: attempt_id.clone(), started_ms, ended_ms,
                        status: "interrupted".into(),
                    });
                }
            }
        }
        if let Some((root, u, id)) = &guard.0 {
            orch::lifecycle::suspend_claim(root, u, id)?;
            guard.0 = None;
        }
        return Ok(json!({"status":"interrupted","error":report["error"],"unit":unit,
            "attempt":attempt,"attempt_id":attempt_id,"model":model,"next":"wait",
            "usage":report["usage"]}));
    }
    if let (Some(root), Some(u)) = (&store, unit) {
        if !orch::lifecycle::validate_red(root, u, &attempt_id, Path::new(cwd))? {
            return Err(Failure::new("red_invalid", "matrix row has no engine RED"));
        }
        orch::lifecycle::checkpoint_worker(root, u, &attempt_id, Some(&report))?;
        crash_point("after_worker");
    }
    if let (Some(root), Some(snapshot)) = (&store, &routing_snapshot) {
        let name = |v: serde_json::Value| v.as_str().map(str::to_owned);
        let record = orch::receipts::RoutingReceipt {
            unit: snapshot.unit.clone(),
            attempt: snapshot.attempt_id.clone(),
            tier: name(json!(snapshot.tier)),
            effort: name(json!(snapshot.effort)),
            risk: name(json!(snapshot.risk)),
            model_assigned: snapshot.model.clone(),
            model_used: Some(
                report["model"]
                    .as_str()
                    .unwrap_or(&snapshot.model)
                    .to_owned(),
            ),
            rule: name(json!(snapshot.rule)).unwrap_or_default(),
            reason: snapshot.reason.clone(),
            settings_version: snapshot.settings_version.clone(),
        };
        if orch::receipts::read_routing(root).is_ok_and(|rows| {
            !rows
                .iter()
                .any(|r| r.unit == record.unit && r.attempt == record.attempt)
        }) {
            let _ = orch::receipts::record_routing(root, &record);
        }
    }
    if let Some(root) = &store {
        let usage = &report["usage"];
        let input = usage["inputTokens"].as_u64();
        let cached = usage["cachedInputTokens"].as_u64();
        let output = usage["outputTokens"].as_u64();
        let usd = match (input, cached, output) {
            (Some(input), Some(cached_input), Some(output)) => price::codex_usd(
                &model,
                &CodexUsage {
                    input,
                    cached_input,
                    output,
                },
            ),
            _ => None,
        };
        orch::store::with_lock(root, false, || {
            let recorded = unit.is_some() && orch::events::read_events(root, 0)?.iter().any(|e|
                e.kind == crate::engine::kinds::ROLE_SPEND && e.unit.as_deref() == unit
                    && e.data["attempt_id"] == attempt_id && e.data["call"] == "codex_run");
            if !recorded {
                orch::events::append_event(root, crate::engine::kinds::ROLE_SPEND, None, unit,
                    json!({"role":budget_role.name(),"model":model,"usd":usd,"input_tokens":input.unwrap_or(0),"output_tokens":output.unwrap_or(0),"call":"codex_run","attempt_id":attempt_id}))?;
            }
            Ok(())
        })?;
    }
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        if let (Some(started_ms), Some(ended_ms), Some(status)) = (
            report["run_started_ms"].as_i64(),
            report["run_ended_ms"].as_i64(),
            report["status"].as_str(),
        ) {
            if orch::receipts::read_runs(root).is_ok_and(|rows| !rows.iter().any(|row| row.unit == u && row.attempt == attempt_id)) {
            let _ = orch::receipts::record_run(
                root,
                &orch::receipts::RunRecord {
                    unit: u.to_owned(),
                    attempt: attempt_id.clone(),
                    started_ms,
                    ended_ms,
                    status: status.to_owned(),
                },
            );
            }
        }
        let finished = orch::lifecycle::finish_checkpointed(root, u, &attempt_id, Path::new(cwd), r, &model, &report)?;
        if finished.verify_exit.is_some() { crash_point("after_verify"); }
        let graph_sync = match graph::GraphStore::open(b.home()?) {
            Ok(store) => {
                let options = graph::code::SyncOptions {
                    deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(5)),
                    ..Default::default()
                };
                let code = graph::code::sync(&store, Path::new(cwd), None, &options);
                let orch = graph::ingest::sync_orch(&store, root, Path::new(cwd));
                json!({"code":match code { Ok(report) => json!({"partial":report.partial,"partial_reason":report.partial_reason,"parse_failures":report.parse_failures}), Err(e) => json!({"error":e.to_json()}) }, "orch":match orch { Ok(report) => json!({"nodes_written":report.nodes_written,"edges_written":report.edges_written,"skipped":report.skipped}), Err(e) => json!({"error":e.to_json()}) }})
            }
            Err(e) => json!({"error":e.to_json()}),
        };
        let reply = unit_reply(
            b,
            project.unwrap(),
            root,
            u,
            Path::new(cwd),
            registered,
            &model,
            attempt,
            json!(attempt_id),
            &report,
            finished,
            cycle,
            false,
        );
        if let Err(e) = &reply {
            if resumable_cut(&e.kind) {
                orch::lifecycle::suspend_claim(root, u, &attempt_id)?;
                guard.0 = None;
            }
        }
        let mut reply = reply?;
        reply["graph_sync"] = graph_sync;
        return Ok(reply);
    }
    if let Some(dest) = dest {
        let valid = report["status"] == "completed" && !report["structured_output"].is_null();
        if valid {
            let bytes = serde_json::to_vec_pretty(&report["structured_output"])
                .map_err(|e| Failure::new("io", e.to_string()))?;
            base::atomic_write(&dest, &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
        } else if report["status"] == "completed" {
            report["status"] = json!("failed");
            report["error"] = json!({"kind":"invalid_structured_output","message":"structured output is required"});
        }
        orch::record_stage(store.as_ref().unwrap(), "astra", &model, valid)?;
        let coverage = report["structured_output"].get("coverage").cloned();
        let obj = report.as_object_mut().unwrap();
        obj.remove("structured_output");
        obj.remove("final_message");
        if valid {
            obj.insert("saved_to".into(), json!(dest));
        }
        if let Some(v) = coverage {
            obj.insert("coverage".into(), v);
        }
    }
    Ok(report)
}
