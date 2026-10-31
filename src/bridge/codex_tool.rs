use super::server::Bridge;
use crate::{
    base::{self, Failure, Res},
    codex::{self, RunRequest},
    goal, orch,
};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

fn invalid(message: &str) -> Failure {
    Failure::new("invalid_argument", message)
}

// Inspect links themselves, including Windows junctions, rather than following
// a declared path through an ancestor outside the repository.
fn output_is_link(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    { meta.file_type().is_symlink() }
}

fn required_output_present(repo: &Path, output: &str) -> bool {
    let mut path = repo.to_path_buf();
    for part in Path::new(output).components() {
        if !matches!(part, Component::Normal(_)) { return false; }
        path.push(part.as_os_str());
        let Ok(meta) = std::fs::symlink_metadata(&path) else { return false; };
        if output_is_link(&meta) { return false; }
    }
    let Ok(meta) = std::fs::symlink_metadata(&path) else { return false; };
    if output.ends_with('/') {
        if !meta.is_dir() { return false; }
        // The same Git tree used for source deltas defines monitored files;
        // ignored files alone cannot satisfy a directory declaration.
        let tree = orch::snapshot(repo, &[".".into()]);
        let Ok(list) = std::process::Command::new("git").current_dir(repo)
            .args(["ls-tree", "-r", "--name-only", "-z", tree.trim_start_matches("tree:"), "--", output])
            .output() else { return false; };
        list.status.success() && list.stdout.split(|b| *b == 0).filter(|p| !p.is_empty())
            .filter_map(|p| std::str::from_utf8(p).ok())
            .any(|p| required_output_present(repo, p))
    } else {
        meta.is_file()
    }
}
fn string<'a>(a: &'a Value, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}
pub fn codex_run(b: &Bridge, a: &Value) -> Value {
    match run(b, a) {
        Ok(v) => v,
        Err(e) => json!({"status":"bridge_error","error":e.to_json()}),
    }
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
struct UnitGuard(Option<(PathBuf, String)>);
impl Drop for UnitGuard {
    fn drop(&mut self) {
        if let Some((store, u)) = &self.0 {
            orch::abort_unit(store, u);
        }
    }
}
fn run(b: &Bridge, a: &Value) -> Res<Value> {
    let role = string(a, "role");
    if a.get("role").is_some() && !matches!(role, Some("terra" | "sol" | "astra" | "astra-verify"))
    {
        return Err(invalid("invalid role"));
    }
    let verifier = role == Some("astra-verify");
    if role == Some("astra") && a.get("effort").is_some_and(|v| v.as_str() != Some("max")) {
        return Err(invalid("astra research requires max effort"));
    }
    if verifier
        && (string(a, "sandbox") != Some("read-only")
            || !matches!(string(a, "task_kind"), Some("verification" | "measurement"))
            || a.get("unit").is_some()
            || a.get("save_to").is_some()
            || a.get("effort").is_some_and(|v| v.as_str() != Some("high"))
            || a.get("lean").is_some_and(|v| v != &Value::Bool(true))
            || a.get("keep_plugins")
                .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
            || a.get("web_search")
                .is_some_and(|v| v != &Value::Bool(false))
            || a.get("developer_instructions").is_some())
    {
        return Err(invalid("astra-verify requires read-only, verification or measurement, high effort and isolated capabilities; no unit or save_to"));
    }
    let model_role = if verifier {
        "astra"
    } else {
        role.unwrap_or("")
    };
    let explicit = string(a, "model").filter(|s| !s.is_empty());
    if role.is_none() && explicit.is_some_and(|m| codex::role_matches("astra", m)) {
        return Err(invalid(
            "explicit Astra model requires astra or astra-verify role",
        ));
    }
    if explicit.is_some_and(|m| codex::role_matches("luna", m)) {
        return Err(invalid("luna is reserved for nonengineering content"));
    }
    if let (Some(r), Some(m)) = (role, explicit) {
        if !codex::role_matches(if r == "astra-verify" { "astra" } else { r }, m) {
            return Err(invalid("model does not match role"));
        }
    }
    if role.is_none() && explicit.is_none() {
        return Err(invalid("model or role is required"));
    }
    let project = string(a, "project");
    let unit = string(a, "unit");
    let save = string(a, "save_to");
    if unit.is_none() && string(a, "brief").is_none() {
        return Err(invalid("brief is required outside unit mode"));
    }
    if a.get("unit").is_some() && (project.is_none() || unit.is_none()) {
        return Err(invalid("unit requires project"));
    }
    if unit.is_some()
        && (!matches!(role, Some("terra" | "sol"))
            || a.get("brief").is_some()
            || save.is_some()
            || string(a, "sandbox").is_some_and(|s| s != "workspace-write"))
    {
        return Err(invalid(
            "unit mode requires terra or sol, no brief, and workspace-write",
        ));
    }
    if save.is_some() && (project.is_none() || role != Some("astra") || unit.is_some()) {
        return Err(invalid("save_to requires project and astra role"));
    }
    if unit.is_none() && string(a, "sandbox").unwrap_or("workspace-write") != "read-only" {
        return Err(Failure::new("unbound_write", "a writing codex_run needs project and unit; register the unit with orch and dispatch it by id"));
    }
    let store = project.map(|p| orch::store_for(b.home()?, p)).transpose()?;
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
    let mut attempt = 0;
    let mut guard = UnitGuard(None);
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        let active = goal::find_active(b.home()?, project.unwrap())?;
        if let Some((_, g)) = &active {
            if g.interjection.is_some() {
                return Err(Failure::new(
                    "pending_interjection",
                    "classify the pending interjection before starting a writing unit",
                ));
            }
            if g.mode.as_deref() != Some("change") {
                return Err(Failure::new(
                    "goal_mode",
                    "a writing unit requires change goal mode",
                ));
            }
        }
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
        let required = active.is_some_and(|(_, g)| goal::astra_requested(&g));
        if orch::current_checkpoint(&orch::wait_state(root)?, u).is_some() {
            if r != "terra" || explicit.is_some() || a.get("effort").is_some_and(|v| v != "high") {
                return Err(invalid("enrolled round must enter through Terra HIGH; host owns assembly and independent verification"));
            }
            return enrolled_round(b, root, u, Path::new(cwd), required, a);
        }
        match orch::begin_unit(root, u, r, Path::new(cwd), required)? {
            orch::Begin::Reused => {
                return Ok(json!({"status":"completed","unit":u,"state":"verified","reused":true}))
            }
            orch::Begin::Run {
                brief: value,
                attempt: n,
            } => {
                brief = value.text;
                attempt = n;
                guard.0 = Some((root.to_path_buf(), u.to_owned()));
            }
        }
    }
    let mut resolved = explicit.unwrap_or("").to_owned();
    let result = (|| -> Res<(String, Value)> {
        let server = b.codex()?;
        let model = match explicit {
            Some(m) => m.to_owned(),
            None => server.resolve_model(model_role)?,
        };
        resolved = model.clone();
        let req = RunRequest {
            brief,
            cwd: cwd.into(),
            model: model.clone(),
            sandbox: string(a, "sandbox").unwrap_or("workspace-write").into(),
            effort: Some(
                string(a, "effort")
                    .unwrap_or(if verifier {
                        crate::codex::policy::role("astra-verify")?.1
                    } else {
                        crate::codex::policy::worker_effort(&model)
                    })
                    .to_owned(),
            ),
            output_schema: a.get("output_schema").filter(|v| !v.is_null()).cloned(),
            developer_instructions: if verifier {
                Some("You are an independent read-only verifier. Do not run Cargo or any build, test, or other mutating command; inspect static files and Host receipts only. Do not modify anything. Report evidence and limitations.".into())
            } else {
                string(a, "developer_instructions").map(str::to_owned)
            },
            timeout_s: a
                .get("timeout_s")
                .and_then(Value::as_u64)
                .filter(|x| *x > 0)
                .unwrap_or(1800),
            lean: a.get("lean") != Some(&Value::Bool(false)),
            keep_plugins: plugins,
            web_search: a.get("web_search") == Some(&Value::Bool(true)),
        };
        Ok((model, server.run(&req)?))
    })();
    let (requested_model, mut report) = match result {
        Ok(r) => r,
        Err(e) => {
            if e.kind == "turn_unconfirmed" {
                if let (Some(root), Some(u)) = (&store, unit) {
                    orch::quarantine_unit(root, u)?;
                }
            }
            if dest.is_some() {
                let _ = orch::record_stage(store.as_ref().unwrap(), "astra", &resolved, false);
            }
            return Err(e);
        }
    };
    let model = report["model"]
        .as_str()
        .unwrap_or(&requested_model)
        .to_owned();
    if report["error"]["kind"] != "turn_unconfirmed"
        && role.is_some_and(|r| {
            !codex::role_matches(if r == "astra-verify" { "astra" } else { r }, &model)
        })
    {
        report["status"] = json!("failed");
        report["error"] = json!({"kind":"model_mismatch","message":"actual worker model does not match authorized role"});
    }
    if verifier {
        report["requested_effort"] = json!("high");
        // Only a server settings notification can establish actual effort.
        if report.get("actual_effort").is_none() {
            report["actual_effort"] = Value::Null;
        }
        report["task_kind"] = json!(string(a, "task_kind"));
    }
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        if report["error"]["kind"] == "turn_unconfirmed" {
            orch::quarantine_unit(root, u)?;
            return Ok(report);
        }
        let finished = orch::finish_unit(root, u, Path::new(cwd), r, &model, &report)?;
        let built = finished.state == "built";
        let error = if built {
            Value::Null
        } else if report["status"] != "completed" {
            report["error"].clone()
        } else if !finished.outside_scope.is_empty() {
            json!({"kind":"outside_scope","message":"changes outside unit scope"})
        } else {
            json!({"kind":"verify_failed","message":"unit verification failed"})
        };
        let mut result = json!({"status":if built {"completed"} else {"failed"},"unit":u,"state":finished.state,"model":model,"requested_model":requested_model,"effort":string(a,"effort").unwrap_or(crate::codex::policy::worker_effort(&requested_model)),"attempt":attempt,"verify_exit":finished.verify_exit,"outside_scope":finished.outside_scope,"file_changes":report["file_changes"],"usage":report["usage"],"report":root.join("reports").join(format!("{u}.md")),"error":error});
        result["thread_id"] = report["thread_id"].clone();
        result["turn_id"] = report["turn_id"].clone();
        if !built {
            result["verify_tail"] = json!(finished.verify_tail);
        }
        return Ok(result);
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


fn check_stage(report: &Value) -> Res<()> {
    if report["error"]["kind"] == "turn_unconfirmed" {
        return Err(Failure::new("turn_unconfirmed", "worker termination unconfirmed"));
    }
    if report["status"] != "completed" {
        if report["error"]["kind"] == "invalid_structured_output" {
            return Err(Failure::new("protocol_incomplete", "required typed stage result missing or truncated"));
        }
        let kind = match report["status"].as_str() {
            Some("timeout") => "timeout", Some("interrupted" | "cancelled") => "cancelled",
            _ => "failed",
        };
        return Err(Failure::new(kind, "worker did not complete; see durable Host stage report"));
    }
    if report["structured_output"]["outcome"] == "blocked" {
        return Err(Failure::new("blocked", format!("{}; next: {}; evidence: {}",
            report["structured_output"]["blocking_cause"].as_str().unwrap_or("missing blocking cause"),
            report["structured_output"]["next_action"].as_str().unwrap_or("unknown"), report["saved_to"])));
    }
    if report["structured_output"].get("outcome").is_some()
        && report["structured_output"]["outcome"] != "awaiting_core" {
        let kind = match report["structured_output"]["outcome"].as_str() {
            Some("cancelled") => "cancelled", _ => "failed",
        };
        return Err(Failure::new(kind, "typed worker outcome does not permit advancing the engineering chain"));
    }
    Ok(())
}

// `server.run` constructs these reports only after matching turn/completed;
// unknown interruption/cleanup instead returns turn_unconfirmed. This is Host
// evidence, not the model's typed outcome.
fn terminal_stage_report(report: &Value) -> bool {
    report["turn_id"].as_str().is_some_and(|id| !id.is_empty())
        && report["error"]["kind"] != "turn_unconfirmed"
        && matches!(report["status"].as_str(), Some("completed" | "failed" | "interrupted" | "cancelled" | "timeout"))
}

/// One durable attempt owns every writer, Rust command and independent verifier.
/// No caller-supplied report is accepted as host evidence.
fn enrolled_round(
    b: &Bridge,
    store: &Path,
    task: &str,
    repo: &Path,
    required: bool,
    args: &Value,
) -> Res<Value> {
    use sha2::{Digest, Sha256};
    use std::{process::Command, time::Duration};
    let server = b.codex()?;
    let state = orch::wait_state(store)?;
    let checkpoint =
        orch::current_checkpoint(&state, task).ok_or_else(|| invalid("missing checkpoint"))?;
    if orch::current_verified_unit(store, task, repo)? {
        let release = state.events.iter().rev().find(|e| e.task == task && e.round == checkpoint.round && e.kind == "release")
            .ok_or_else(|| invalid("verified checkpoint lacks release identity"))?;
        return Ok(json!({"status":"completed","transport_status":"completed","outcome":"verified","reused":true,"unit":task,"state":"verified",
            "operation_id":release.dispatch_id,"parent_id":state.operations.iter().find(|op| op.id == release.dispatch_id).map(|op| op.parent.as_str()).unwrap_or(task),
            "stage":"released","delivery_id":release.id,"round":checkpoint.round,"source_identity":orch::snapshot(repo, &[".".into()])}));
    }
    let brief = orch::unit_brief(store, task)?;
    if brief.delivery.is_none() || brief.outputs.as_ref() != Some(&checkpoint.outputs) {
        return Err(invalid("live engineering contract requires matching DELIVERY/OUTPUTS"));
    }
    let source = || orch::snapshot(repo, &[".".into()]);
    let artifact = || orch::snapshot(repo, &brief.scope);
    if source().is_empty() || artifact().is_empty() {
        return Err(invalid("source identity unavailable"));
    }
    let execute = |role: &str, effort: &str, text: String, schema: Option<Value>| -> Res<Value> {
        let read_only = role == "astra";
        let intent = orch::wait_state(store)?
            .events
            .into_iter()
            .rev()
            .find(|e| {
                e.task == task
                    && e.round == checkpoint.round
                    && ["dispatch", "review_intent"].contains(&e.kind.as_str())
            })
            .ok_or_else(|| invalid("missing durable host attempt"))?;
        let source_before = source();
        let schema = if read_only { schema } else { Some(json!({
            "type":"object","properties":{
                "outcome":{"type":"string","enum":["awaiting_core","blocked","failed","cancelled","unconfirmed"]},
                "operation_id":{"type":"string","enum":[intent.dispatch_id]},
                "task":{"type":"string","enum":[task]},"round":{"type":"integer","enum":[checkpoint.round]},
                "evidence":{"type":"string"},"blocking_cause":{"type":"string"},"next_action":{"type":"string"}},
            "required":["outcome","operation_id","task","round","evidence","blocking_cause","next_action"],"additionalProperties":false
        })) };
        let execution = (|| {
            server.run(&RunRequest {
            brief:format!("Host binding TASK {task} ROUND {}\nOPERATION {}\n{text}",checkpoint.round,intent.dispatch_id), cwd:repo.to_string_lossy().into_owned(), model:server.resolve_model(role)?,
            sandbox:if read_only { "read-only" } else { "workspace-write" }.into(),
            effort:Some(effort.into()), output_schema:schema,
            developer_instructions:Some(if read_only { "Independent read-only verification. Do not run Cargo or any build, test, or other mutating command; inspect static files and Host receipts only. Do not modify anything. Return observed issue identifiers, severity and evidence; never claim checks not run." } else { "Preserve all existing user edits. Stay inside the supplied SCOPE." }.into()),
            timeout_s:args["timeout_s"].as_u64().filter(|n| *n > 0).unwrap_or(7200),
            lean:true, keep_plugins:vec![], web_search:false,
        })
        })();
        let mut report = match &execution {
            Ok(report) => report.clone(),
            Err(e) => {
                json!({"status":"unknown","error":e.to_json(),"model":null,"actual_effort":null,"actual_sandbox":null})
            }
        };
        report["source_identity"] = json!(source());
        report["source_before"] = json!(source_before);
        report["binding"] = json!({"task":task,"round":checkpoint.round,"attempt":intent.dispatch_id,"issue":checkpoint.issue,"evidence_version":checkpoint.evidence_version,"contract":checkpoint.contract});
        report["task"] = json!(task);
        report["round"] = json!(checkpoint.round);
        report["requested_role"] = json!(role);
        let dest = store.join("reports").join(format!(
            "host-{task}-round{}-{role}-{effort}-{}.json",
            checkpoint.round,
            base::unique()
        ));
        std::fs::create_dir_all(dest.parent().unwrap())
            .map_err(|e| Failure::new("io", e.to_string()))?;
        report["saved_to"] = json!(dest);
        base::atomic_write(
            &dest,
            &serde_json::to_vec(&report).map_err(|e| invalid(&e.to_string()))?,
        )
        .map_err(|e| Failure::new("io", e.to_string()))?;
        execution?;
        Ok(report)
    };
    // A blocked worker result is terminal, unlike an intentionally saved
    // stage boundary. Do not turn its released lease into an implicit retry.
    if let Some(dispatch) = state.events.iter().rev().find(|e| e.task == task && e.round == checkpoint.round && e.kind == "dispatch") {
        if !state.events.iter().any(|e| e.task == task && e.round == checkpoint.round && e.kind == "recovery" && e.dispatch_id == dispatch.id) {
            if let Some(op) = state.operations.iter().find(|op| op.id == dispatch.id && op.outcome == "blocked" && !op.executing && op.terminal_failure.is_some()) {
                return Ok(op.reply());
            }
        }
    }
    // TODO: explicit new-operation identity/retry API after recovery.
    let mut operation = orch::wait::resume_operation(store, task, &source())?;
    if let Some(op) = &operation {
        if op.outcome != "running" { return Ok(op.reply()); }
    }
    if operation.is_none() && checkpoint.round >= 6 && !orch::pre_dispatch_review_ready(&state, task) {
        orch::wait::host_progress(
            store,
            task,
            "review_intent",
            &json!({"source_identity":source()}),
            &artifact(),
        )?;
        let before = source();
        let report = execute(
            "astra",
            "max",
            format!(
                "GOAL Review the next writing round before any writer starts using static inspection and Host receipts only. Do not run Cargo or any build/test command or mutate anything.\nGOAL {}\nSCOPE {}\nCONTEXT {}\nCheckpoint: {}",
                brief.goal,
                brief.scope.join(" "),
                brief.context.as_deref().unwrap_or(""),
                serde_json::to_string(&checkpoint).unwrap()
            ),
            None,
        )?;
        if before != source() {
            return Err(invalid("read-only pre-review source changed"));
        }
        orch::wait::host_progress(store, task, "start_review", &report, &artifact())?;
    }
    let attempt = match orch::begin_unit(store, task, "terra", repo, required)? {
        orch::Begin::Reused => return Ok(json!({"status":"completed","reused":true,"unit":task})),
        orch::Begin::Run { attempt, .. } => attempt,
    };
    let _guard = UnitGuard(Some((store.into(), task.into())));
    let mut op = match operation.take() {
        Some(op) => op,
        None => orch::wait::open_operation(store, task, string(args, "parent_id").unwrap_or(task),
            args["timeout_s"].as_u64().filter(|n| *n > 0).unwrap_or(7200), &source())?,
    };
    op.worker_lease = std::fs::read_to_string(store.join(".worker.claim"))
        .map_err(|e| Failure::new("io", e.to_string()))?;
    orch::wait::save_operation(store, &mut op, false, None)?;
    let progress = |kind: &str, report: &Value| -> Res<orch::Event> {
        let state = orch::wait_state(store)?;
        let id = state.events.iter().rev().find(|e| e.task == task && e.round == checkpoint.round && e.kind == "dispatch").map(|e| &e.id);
        if let Some(e) = state.events.iter().find(|e| e.task == task && e.round == checkpoint.round && Some(&e.dispatch_id) == id && e.kind == kind) {
            return Ok(e.clone());
        }
        orch::wait::host_progress(store, task, kind, report, &artifact())
    };
    let start_phase = |op: &mut orch::EngineeringOperation, phase: &str| -> Res<()> {
        if base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
            return Err(Failure::new("cancelled", "operation cancelled before next phase"));
        }
        if base::proc::unix_ms() >= op.deadline_ms {
            return Err(Failure::new("timeout", "shared operation deadline exhausted"));
        }
        op.phase = phase.into(); op.executing = true; op.outcome = "running".into();
        orch::wait::save_operation(store, op, false, None)
    };
    let finish_phase = |op: &mut orch::EngineeringOperation, phase: &str, next: &str, receipt: Value| -> Res<()> {
        if base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
            return Err(Failure::new("cancelled", "late phase completion after cancellation"));
        }
        if base::proc::unix_ms() >= op.deadline_ms {
            return Err(Failure::new("timeout", "late phase completion after operation deadline"));
        }
        let current_source = source();
        if matches!(phase, "build" | "assembly") {
            // A resumed lifecycle lease has a fresh dirty baseline. Preserve the
            // ORIGINAL operation tree as scope authority across every boundary.
            let diff = Command::new("git").current_dir(repo).args(["diff", "--no-renames", "--name-only", "-z",
                op.initial_source.trim_start_matches("tree:"), current_source.trim_start_matches("tree:"), "--"])
                .output().map_err(|e| Failure::new("io", e.to_string()))?;
            if !diff.status.success() { return Err(invalid("operation scope comparison failed")); }
            let outside: Vec<_> = diff.stdout.split(|b| *b == 0).filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned()).filter(|p| !orch::covers(&brief.scope, p)).collect();
            if !outside.is_empty() { return Err(Failure::new("outside_scope", outside.join(", "))); }
        }
        op.receipts.insert(phase.into(), receipt); op.phase = next.into();
        op.terminal_failure = None;
        op.source = current_source; op.executing = false; op.outcome = "blocked".into();
        orch::wait::save_operation(store, op, true, None)?;
        if args["stop_after"] == phase { return Err(Failure::new("stage_boundary", "saved stage boundary")); }
        Ok(())
    };
    let result = base::proc::deadline_scope(op.deadline_ms, || -> Res<Value> {
        let build = if let Some(saved) = op.receipts.get("build") { saved.clone() } else {
        start_phase(&mut op, "build")?;
        let build = execute("terra", "high", brief.text.clone(), None)?;
        if terminal_stage_report(&build) {
            op.executing = false;
            op.terminal_failure = Some(build.clone());
            orch::wait::save_operation(store, &mut op, false, None)?;
        }
        if build["error"]["kind"] == "turn_unconfirmed" {
            return Err(Failure::new(
                "turn_unconfirmed",
                "Terra termination unknown",
            ));
        }
        if let Err(e) = check_stage(&build) {
            return Err(e);
        }
        let mut build = build;
        if brief.delivery.as_deref() == Some("implementation") {
            let current = source();
            let diff = Command::new("git").current_dir(repo).args(["diff", "--no-renames", "--name-only", "-z",
                op.initial_source.trim_start_matches("tree:"), current.trim_start_matches("tree:"), "--"])
                .output().map_err(|e| Failure::new("io", e.to_string()))?;
            if current.is_empty() || !diff.status.success() { return Err(invalid("operation scope comparison failed")); }
            let paths: Vec<_> = diff.stdout.split(|b| *b == 0).filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned()).collect();
            let outside: Vec<_> = paths.iter().filter(|p| !orch::covers(&brief.scope, p)).collect();
            if !outside.is_empty() { return Err(Failure::new("outside_scope", format!("{outside:?}"))); }
            if paths.is_empty() || !paths.iter().any(|p| orch::covers(&checkpoint.outputs, p)) {
                return Err(Failure::new("blocked", "implementation has no actual deliverable source delta"));
            }
            if checkpoint.outputs.iter().any(|p| !required_output_present(repo, p)) {
                return Err(Failure::new("blocked", "required checkpoint output is missing or has the wrong type"));
            }
            let binding = json!({"operation_id":op.id,"task":task,"round":checkpoint.round,"contract":checkpoint.contract});
            let mut before_verify = || {
                op.executing = true;
                if let Err(e) = orch::wait::save_operation(store, &mut op, false, None) {
                    op.executing = false;
                    return Err(e);
                }
                Ok(())
            };
            let (run, receipt, _) = orch::lifecycle::host_verification_check(store, task, repo, &brief.verify, Some(&binding), Some(&mut before_verify))?;
            if run.timed_out || base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
                return Err(Failure::new("turn_unconfirmed", "implementation Host verification termination unconfirmed"));
            }
            op.executing = false;
            orch::wait::save_operation(store, &mut op, false, None)?;
            if run.exit_code != 0 || receipt["receipt"]["binding"] != binding
                || !orch::verification_receipt_current(&receipt, &brief.verify, &current) || source() != current {
                return Err(Failure::new("verify_failed", "implementation Host verification failed or receipt identity mismatched"));
            }
            build["delivery_receipt"] = receipt;
        }
        progress("build", &build)?;
        finish_phase(&mut op, "build", "assembly", build.clone())?;
        build
        };
        if brief.delivery.as_deref() == Some("implementation") {
            let binding = json!({"operation_id":op.id,"task":task,"round":checkpoint.round,"contract":checkpoint.contract});
            let receipt = &build["delivery_receipt"];
            if receipt["receipt"]["binding"] != binding
                || !orch::verification_receipt_current(receipt, &brief.verify, &source()) {
                return Err(Failure::new("verify_failed", "missing, stale or mismatched implementation receipt"));
            }
        }
        let assembly = if let Some(saved) = op.receipts.get("assembly") { saved.clone() } else {
        start_phase(&mut op, "assembly")?;
        progress("assembly_intent", &json!({"source_identity":source()}))?;
        let assembly = execute("sol","high",format!("GOAL Assemble and integrate the Terra result; preserve user work and do not widen scope.\n{}\nTerra evidence: {}",brief.text,build),None)?;
        if terminal_stage_report(&assembly) {
            op.executing = false;
            op.terminal_failure = Some(assembly.clone());
            orch::wait::save_operation(store, &mut op, false, None)?;
        }
        if assembly["error"]["kind"] == "turn_unconfirmed" {
            return Err(Failure::new("turn_unconfirmed", "Sol termination unknown"));
        }
        if let Err(e) = check_stage(&assembly) {
            return Err(e);
        }
        progress("assembly", &assembly)?;
        finish_phase(&mut op, "assembly", "host_test", assembly.clone())?;
        assembly
        };
        let receipt = if let Some(saved) = op.receipts.get("host_test") { saved.clone() } else {
        start_phase(&mut op, "host_test")?;
        op.executing = false;
        orch::wait::save_operation(store, &mut op, false, None)?;
        // Both reports participate in scope checks. Attribute the writing attempt
        // to Terra; Sol's actual identity remains in the assembly host event.
        let mut combined = build.clone();
        combined["file_changes"]
            .as_array_mut()
            .ok_or_else(|| invalid("missing file changes"))?
            .extend(
                assembly["file_changes"]
                    .as_array()
                    .ok_or_else(|| invalid("missing assembly changes"))?
                    .clone(),
            );
        let mut before_verify = || {
            op.executing = true;
            if let Err(e) = orch::wait::save_operation(store, &mut op, false, None) {
                op.executing = false;
                return Err(e);
            }
            Ok(())
        };
        let finished = orch::lifecycle::finish_unit_held(
            store,
            task,
            repo,
            "terra",
            build["model"].as_str().unwrap_or(""),
            &combined,
            &mut before_verify,
        )?;
        // A scope rejection never launched VERIFY; otherwise only a normal
        // process exit is proof that the Host-owned command has stopped.
        let host_terminal = !finished.outside_scope.is_empty()
            || (finished.verify_receipt.as_ref().is_some_and(|r| r["receipt"]["exit_code"].is_number() && r["receipt"]["timed_out"] == false)
                && !base::proc::current_cancellation().is_some_and(|c| c.cancelled()));
        if host_terminal {
            op.executing = false;
            orch::wait::save_operation(store, &mut op, false, None)?;
        } else if finished.verify_receipt.is_some() {
            return Err(Failure::new("turn_unconfirmed", "Host verification termination unconfirmed"));
        }
        if base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
            return Err(Failure::new("cancelled", "Host verification cancelled"));
        }
        if base::proc::remaining(Duration::from_secs(1)).is_zero() {
            return Err(Failure::new("timeout", "Host verification exhausted shared deadline"));
        }
        if finished.verify_exit != Some(0) || !finished.outside_scope.is_empty() {
            return Err(invalid("scope or unit verification failed"));
        }
        let receipt = finished.verify_receipt.ok_or_else(|| invalid("missing host verification receipt"))?;
        finish_phase(&mut op, "host_test", "package", receipt.clone())?;
        receipt
        };
        let package_source = source();
        let receipt_intact = || orch::verification_receipt_current(&receipt, &brief.verify, &package_source);
        if !receipt_intact() {
            return Err(invalid("stale or missing host verification receipt"));
        }
        let package_report = if let Some(saved) = op.receipts.get("package") { saved.clone() } else {
        start_phase(&mut op, "package")?;
        op.executing = false;
        orch::wait::save_operation(store, &mut op, false, None)?;
        progress("package_intent", &json!({"source_identity":package_source}))?;
        let mut commands = vec![];
        // An inherited global CARGO_TARGET_DIR may be shared by unrelated
        // products with the same package name. Never consume that mutable archive.
        let cargo_target = base::cargo_target(repo, &format!("package:{task}:{}:{}", checkpoint.round, base::unique()))?;
        for command in [
            "cargo build --locked --offline",
            "cargo package --locked --offline --allow-dirty",
        ] {
            let mut process = Command::new("cargo");
            process
                .args(command.split_whitespace().skip(1))
                .env("CARGO_TARGET_DIR", &cargo_target);
            op.executing = true;
            orch::wait::save_operation(store, &mut op, false, None)?;
            let run = base::proc::run_command(process, repo, Duration::from_secs(1800))?;
            if run.timed_out || base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
                return Err(Failure::new("turn_unconfirmed", "package command termination unconfirmed"));
            }
            op.executing = false;
            orch::wait::save_operation(store, &mut op, false, None)?;
            commands.push(json!({"command":command,"env":{"CARGO_TARGET_DIR":cargo_target},"exit_code":run.exit_code,"output":run.output}));
            if run.exit_code != 0 {
                return Err(Failure::new(
                    if run.timed_out { "timeout" } else { "package_failed" },
                    serde_json::to_string(&commands).unwrap(),
                ));
            }
        }
        if package_source != source() || !receipt_intact() {
            return Err(invalid("source or receipt changed during packaging"));
        }
        let mut metadata_command = Command::new("cargo");
        metadata_command
            .env("CARGO_TARGET_DIR", &cargo_target)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--locked",
                "--offline",
            ]);
        op.executing = true;
        orch::wait::save_operation(store, &mut op, false, None)?;
        let metadata = base::proc::run_command(metadata_command, repo, Duration::from_secs(120))?;
        if metadata.timed_out || base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
            return Err(Failure::new("turn_unconfirmed", "cargo metadata termination unconfirmed"));
        }
        op.executing = false;
        orch::wait::save_operation(store, &mut op, false, None)?;
        if metadata.exit_code != 0 {
            return Err(invalid("cargo metadata failed"));
        }
        let metadata: Value = serde_json::from_str(&metadata.output)
            .map_err(|_| invalid("invalid cargo metadata"))?;
        let package = metadata["packages"]
            .as_array()
            .filter(|p| p.len() == 1)
            .and_then(|p| p.first())
            .ok_or_else(|| invalid("package identity requires a single-package workspace"))?;
        let path = Path::new(
            metadata["target_directory"]
                .as_str()
                .ok_or_else(|| invalid("missing cargo target"))?,
        )
        .join("package")
        .join(format!(
            "{}-{}.crate",
            package["name"].as_str().unwrap_or(""),
            package["version"].as_str().unwrap_or("")
        ));
        let bytes = std::fs::read(&path).map_err(|e| Failure::new("io", e.to_string()))?;
        let hash = format!("sha256:{:x}", Sha256::digest(&bytes));
        let archive = store.join("reports").join(format!(
            "package-{task}-round{}-{}.crate",
            checkpoint.round,
            base::unique()
        ));
        base::atomic_write(&archive, &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
        let package_report = json!({"source_identity":package_source,"package_path":archive,"cargo_package_path":path,"package_hash":hash,"commands":commands,"verification_receipt":receipt});
        progress("package", &package_report)?;
        finish_phase(&mut op, "package", "verification", package_report.clone())?;
        package_report
        };
        let hash = package_report["package_hash"].clone();
        let verification = if let Some(saved) = op.receipts.get("verification") { saved.clone() } else {
        start_phase(&mut op, "verification")?;
        progress("verification_intent", &package_report)?;
        let schema = json!({"type":"object","properties":{"issues":{"type":"array","items":{"type":"string"}},"severity":{"type":"string","enum":["normal","severe","excellent"]},"evidence":{"type":"string"}},"required":["issues","severity","evidence"],"additionalProperties":false});
        let mut verification = execute("astra","high",format!("GOAL Independently verify the assembled source and Rust package using static inspection and Host receipts only. Do not run Cargo or any build/test command or mutate anything. Identify distinct valid issues; empty issues only when supported by evidence.\nGOAL {}\nSCOPE {}\nCONTEXT {}\nHost verification receipt: {}\nPackage evidence: {}",brief.goal,brief.scope.join(" "),brief.context.as_deref().unwrap_or(""),receipt,package_report),Some(schema))?;
        verification["package_hash"] = json!(hash);
        if terminal_stage_report(&verification) {
            op.executing = false;
            orch::wait::save_operation(store, &mut op, false, None)?;
        }
        if package_source != source() || !receipt_intact() {
            return Err(invalid("source or receipt changed during read-only verification"));
        }
        check_stage(&verification)?;
        progress("verification", &verification)?;
        finish_phase(&mut op, "verification", "acceptance", verification.clone())?;
        verification
        };
        start_phase(&mut op, "acceptance")?;
        op.executing = false;
        orch::wait::save_operation(store, &mut op, false, None)?;
        if orch::needs_accept(store, task)? {
            op.executing = true;
            orch::wait::save_operation(store, &mut op, false, None)?;
            let run = orch::record_accept(store, &checkpoint.module, repo)?;
            if run.timed_out || base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
                return Err(Failure::new("turn_unconfirmed", "acceptance termination unconfirmed"));
            }
            op.executing = false;
            orch::wait::save_operation(store, &mut op, false, None)?;
            if run.exit_code != 0 || package_source != source() || !receipt_intact() {
                return Err(invalid("acceptance failed or changed packaged source/receipt"));
            }
        }
        orch::record_review(
            store,
            &[task.into()],
            verification["model"].as_str().unwrap_or(""),
            verification["structured_output"]["issues"]
                .as_array()
                .is_some_and(|i| i.is_empty()),
            repo,
        )?;
        // Commit the deduplicated core outbox before returning any delivery.
        let notices = orch::wait::core_notices(store)?;
        if base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
            return Err(Failure::new("cancelled", "late operation completion after cancellation"));
        }
        if base::proc::remaining(Duration::from_secs(1)).is_zero() {
            return Err(Failure::new("timeout", "late operation completion after shared deadline"));
        }
        op.phase = "done".into(); op.executing = false; op.outcome = "awaiting_core".into();
        op.source = source();
        orch::wait::save_operation(store, &mut op, true, None)?;
        let mut reply = op.reply();
        reply["core_notices"] = json!(notices);
        reply["package_hash"] = hash;
        reply["attempt_number"] = json!(attempt);
        Ok(reply)
    });
    if let Err(e) = &result {
        if e.kind == "stage_boundary" { return Ok(op.reply()); }
        op.outcome = if op.executing { "unconfirmed" } else { match e.kind.as_str() {
            "turn_unconfirmed" | "codex_exited" => "unconfirmed",
            "timeout" => "timeout", "cancelled" | "interrupted" => "cancelled",
            "blocked" => "blocked", "protocol_incomplete" => "protocol_incomplete", _ => "failed",
        }}.into();
        let unknown = op.executing || matches!(e.kind.as_str(), "turn_unconfirmed" | "codex_exited");
        // Make the guard non-releasing before any journal or quarantine I/O.
        if unknown { orch::lifecycle::retain_unit(store, task)?; }
        if let Err(commit_error) = orch::wait::save_operation(store, &mut op, true, Some(e)) {
            // The in-memory outbox is not a committed delivery. Keep the lease
            // even for a known-stopped process until recovery can inspect disk.
            orch::lifecycle::retain_unit(store, task)?;
            return Err(commit_error);
        }
        if unknown { orch::quarantine_unit(store, task)?; }
    }
    if let Err(e) = &result {
        // Diagnostics must not mask a committed failure delivery.
        let _ = orch::wait::host_progress(
            store,
            task,
            "suspended",
            &json!({"source_identity":source(),"error":e.to_json(),"termination":if op.executing || matches!(e.kind.as_str(),"turn_unconfirmed" | "codex_exited") { "unknown" } else { "see actual stage report" }}),
            &artifact(),
        );
    }
    match result { Ok(v) => Ok(v), Err(_) => Ok(op.reply()) }
}
