use super::server::Bridge;
use crate::{
    base::{Failure, Res},
    goal, linux,
    memory::{Memory, Record},
    orch,
};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio};
fn s<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).unwrap_or("")
}
fn opt<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}
fn arr(a: &Value, k: &str) -> Res<Vec<String>> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str().map(str::to_owned).ok_or_else(|| {
                    Failure::new("invalid_argument", format!("{k} must contain strings"))
                })
            })
            .collect(),
        _ => Err(Failure::new(
            "invalid_argument",
            format!("{k} must be an array"),
        )),
    }
}
fn require_reviewed_scope(
    repo: &Path,
    scope: &str,
    reviewed: &std::collections::HashSet<std::path::PathBuf>,
) -> Res<()> {
    let target = std::fs::canonicalize(repo.join(scope))
        .map_err(|e| Failure::new("review_scope", e.to_string()))?;
    if !target.starts_with(repo) {
        return Err(Failure::new("review_scope", "scope escapes project"));
    }
    let mut pending = vec![target];
    let mut count = 0usize;
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in
                std::fs::read_dir(&path).map_err(|e| Failure::new("review_scope", e.to_string()))?
            {
                let entry = entry.map_err(|e| Failure::new("review_scope", e.to_string()))?;
                if entry.file_name() == ".git" {
                    continue;
                }
                if entry
                    .file_type()
                    .map_err(|e| Failure::new("review_scope", e.to_string()))?
                    .is_symlink()
                {
                    return Err(Failure::new("review_scope", "symlink in review scope"));
                }
                pending.push(entry.path());
            }
        } else if path.is_file() {
            count += 1;
            if !reviewed.contains(&path) {
                return Err(Failure::new(
                    "review_scope",
                    format!("unreviewed scope file: {}", path.display()),
                ));
            }
        } else {
            return Err(Failure::new("review_scope", "unsupported scope entry"));
        }
    }
    if count == 0 {
        return Err(Failure::new("review_scope", "scope contains no files"));
    }
    Ok(())
}
fn timeout(a: &Value, d: u64) -> u64 {
    a.get("timeout_s")
        .and_then(Value::as_u64)
        .filter(|x| *x > 0)
        .unwrap_or(d)
}
fn store(b: &Bridge, project: &str) -> Res<std::path::PathBuf> {
    orch::store_for(b.home()?, project)
}
pub fn dispatch(b: &Bridge, n: &str, a: &Value) -> Res<Value> {
    if crate::base::proc::current_cancellation().is_some_and(|c| c.cancelled()) {
        return Err(Failure::new("interrupted", "request cancelled"));
    }
    if crate::base::proc::current_cancellation().is_some() {
        dispatch_inner(b, n, a)
    } else {
        b.verification.scope(|| dispatch_inner(b, n, a))
    }
}
fn dispatch_inner(b: &Bridge, n: &str, a: &Value) -> Res<Value> {
    let verification = n == "run_on_linux"
        || (n == "orch"
            && a["args"].as_array().is_some_and(|args| {
                args.iter()
                    .any(|v| matches!(v.as_str(), Some("accept" | "fulltest")))
                    || orch::wait_mutation(
                        &args
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                    )
            }));
    if verification {
        let project = if n == "orch" {
            opt(a, "project").map(str::to_owned)
        } else {
            opt(a, "project_dir").map(|p| crate::base::slug_for_dir(Path::new(p)))
        };
        if let Some(project) = project {
            if let Some((_, goal)) = goal::find_active(b.home()?, &project)? {
                if goal.interjection.is_some() {
                    return Err(Failure::new(
                        "pending_interjection",
                        "classify the pending interjection before verification",
                    ));
                }
                if goal.mode.as_deref() != Some("change") {
                    return Err(Failure::new(
                        "goal_mode",
                        "verification commands require change mode",
                    ));
                }
            }
        }
    }
    match n {
        "session" => match s(a, "action") {
            "begin" | "resume" => {
                let mut slot = b.attachment.lock().unwrap();
                if slot.is_some() {
                    return Err(Failure::new("busy", "caller already attached"));
                }
                if s(a, "action") == "resume" && opt(a, "session").is_none() {
                    return Err(Failure::new("invalid_argument", "session is required"));
                }
                if s(a, "action") == "begin" && opt(a, "session").is_some() {
                    return Err(Failure::new(
                        "invalid_argument",
                        "begin cannot select a session",
                    ));
                }
                let config = crate::session::Config {
                    home: b.home()?.to_path_buf(),
                    codex: b.config.codex.clone(),
                    bin: std::env::current_exe().unwrap_or_default(),
                };
                let (host, result) = crate::session::Attachment::open(&config, a)?;
                *slot = Some(std::sync::Arc::new(host));
                Ok(result)
            }
            "call" | "status" | "stop" | "interject" => {
                let host = b
                    .attachment
                    .lock()
                    .unwrap()
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| Failure::new("not_found", "no caller attached"))?;
                match s(a, "action") {
                    "call" => host.call(s(a, "tool"), a.get("arguments").unwrap_or(&Value::Null)),
                    "status" => host.status(),
                    "interject" => host.interject(s(a, "text")),
                    _ => {
                        host.stop();
                        b.attachment.lock().unwrap().take();
                        Ok(json!({"status":"completed"}))
                    }
                }
            }
            _ => Err(Failure::new("invalid_argument", "invalid session action")),
        },
        "codex_run" => {
            if a.get("unit").and_then(Value::as_str).is_some() && b.codex()?.has_unconfirmed() {
                return Err(Failure::new(
                    "turn_unconfirmed",
                    "a prior turn has unconfirmed termination; unit retry is blocked",
                ));
            }
            Ok(super::codex_tool::codex_run(b, a))
        }
        "gemini_preview" => {
            let project = opt(a, "project");
            let stage = opt(a, "stage");
            if let Some(t) = stage {
                if !["second_review", "rewrite"].contains(&t) {
                    return Err(Failure::new("invalid_argument", "invalid stage"));
                }
            }
            let root = project.map(|p| store(b, p)).transpose()?;
            if let Some(root) = &root {
                std::fs::create_dir_all(root).map_err(|e| Failure::new("io", e.to_string()))?;
            }
            let dest = match opt(a, "save_to") {
                Some(path) => Some(super::codex_tool::destination(
                    root.as_deref().ok_or_else(|| {
                        Failure::new("invalid_argument", "save_to requires project")
                    })?,
                    path,
                )?),
                None => None,
            };
            let role = if stage == Some("second_review") {
                "sol"
            } else {
                "luna"
            };
            let model = match super::review_run::model(b, opt(a, "model"), role) {
                Ok(model) => model,
                Err(e) => {
                    if let (Some(root), Some(t)) = (&root, stage) {
                        let _ = orch::record_stage(root, t, "", false);
                    }
                    return Err(e);
                }
            };
            let result = super::review_run::run(
                b,
                s(a, "prompt"),
                opt(a, "cwd").map(Path::new),
                &model,
                a.get("json_schema").filter(|v| !v.is_null()).cloned(),
                timeout(a, 600),
            );
            let mut result = match result {
                Ok(result) => result,
                Err(e) => {
                    if let (Some(root), Some(t)) = (&root, stage) {
                        let _ = orch::record_stage(root, t, &model, false);
                    }
                    return Err(e);
                }
            };
            let mut completed = result["status"] == "completed";
            if completed {
                if let Some(dest) = dest {
                    if let Some(v) = result.get("structured_output").filter(|v| !v.is_null()) {
                        let text = if let Some(s) = v.as_str() {
                            s.to_owned()
                        } else {
                            serde_json::to_string_pretty(v).unwrap()
                        };
                        crate::base::atomic_write(&dest, text.as_bytes())
                            .map_err(|e| Failure::new("io", e.to_string()))?;
                    } else {
                        completed = false;
                        result["status"] = json!("failed");
                        result["error"] = json!({"kind":"invalid_structured_output","message":"structured output is required"});
                    }
                }
            }
            if let (Some(root), Some(t)) = (&root, stage) {
                orch::record_stage(root, t, result["model"].as_str().unwrap_or(""), completed)?;
            }
            Ok(result)
        }
        "orch" => {
            let path = store(b, s(a, "project"))?;
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| Failure::new("io", e.to_string()))?;
            let mut args = arr(a, "args")?;
            let json_flag = args.first().map_or(false, |x| x == "--json");
            if json_flag {
                args.remove(0);
            }
            let force = args.iter().any(|x| x == "--force");
            args.retain(|x| x != "--force");
            let r = orch::run(&path, &args, json_flag, force);
            let err = if r.exit_code == 0 {
                Value::Null
            } else {
                json!({"kind":"orch_failed","message":if r.stderr.is_empty(){&r.stdout}else{&r.stderr}.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>()})
            };
            Ok(
                json!({"status":if r.exit_code==0 {"completed"}else{"failed"},"store":path,"exit_code":r.exit_code,"stdout":r.stdout,"stderr":r.stderr,"error":err}),
            )
        }
        "review_code" => {
            let units = arr(a, "units")?;
            let mut identities = Vec::new();
            if let Some(project) = opt(a, "project") {
                let root = store(b, project)?;
                let repo = std::fs::canonicalize(s(a, "project_dir"))
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                let reviewed = arr(a, "paths")?
                    .iter()
                    .map(|p| {
                        std::fs::canonicalize(repo.join(p))
                            .map_err(|e| Failure::new("review_scope", e.to_string()))
                    })
                    .collect::<Res<std::collections::HashSet<_>>>()?;
                for unit in &units {
                    let brief = orch::unit_brief(&root, unit)?;
                    for scope in &brief.scope {
                        require_reviewed_scope(&repo, scope, &reviewed)?;
                    }
                    if !s(a, "contract").contains(&brief.text) {
                        return Err(Failure::new(
                            "review_contract",
                            "review contract must include the complete unit brief",
                        ));
                    }
                    let snapshot = orch::snapshot(&repo, &brief.scope);
                    if snapshot.is_empty() {
                        return Err(Failure::new("review_scope", "cannot snapshot review scope"));
                    }
                    identities.push((
                        unit.clone(),
                        snapshot,
                        orch::contract_identity(&root, unit)?,
                    ));
                }
            }
            let role = super::review_run::role(b, opt(a, "project"), &units)?;
            let model = super::review_run::model(b, opt(a, "model"), role)?;
            let mut r = super::review::review(
                s(a, "project_dir"),
                &arr(a, "paths")?,
                s(a, "contract"),
                &model,
                timeout(a, 900),
                b,
            )?;
            if r["status"] == "completed" && a.get("units").is_some() && opt(a, "project").is_some()
            {
                let root = store(b, s(a, "project"))?;
                let repo = std::path::Path::new(s(a, "project_dir"));
                for (unit, snapshot, contract) in &identities {
                    let brief = orch::unit_brief(&root, unit)?;
                    if orch::snapshot(repo, &brief.scope) != *snapshot
                        || orch::contract_identity(&root, unit)? != *contract
                    {
                        r["status"] = json!("failed");
                        r["error"] = json!({"kind":"review_identity_changed", "message":"artifact or contract changed during review"});
                        return Ok(r);
                    }
                }
                let states = match orch::record_review(
                    &store(b, s(a, "project"))?,
                    &units,
                    r["reviewer"].as_str().unwrap_or(""),
                    r["verdict"] == "pass",
                    std::path::Path::new(s(a, "project_dir")),
                ) {
                    Ok(states) => states,
                    Err(e) => {
                        r["status"] = json!("failed");
                        r["error"] = e.to_json();
                        return Ok(r);
                    }
                };
                r["states"] = json!(states
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>());
            }
            Ok(r)
        }
        "run_on_linux" => linux::run(
            b.config.wsl.clone(),
            b.config.on_windows,
            s(a, "project_dir"),
            &arr(a, "args")?,
            timeout(a, 1800),
        ),
        "memory_record" => Memory::open(b.home()?)?.record(&Record {
            project: s(a, "project").into(),
            project_dir: s(a, "project_dir").into(),
            unit: s(a, "unit").into(),
            title: s(a, "title").into(),
            summary: s(a, "summary").into(),
            body: s(a, "body").into(),
            symbols: arr(a, "symbols")?,
            files: arr(a, "files")?,
            wish: s(a, "wish").into(),
        }),
        "memory_search" => Memory::open(b.home()?)?.search_fresh(
            s(a, "query"),
            opt(a, "project"),
            a["limit"].as_i64().unwrap_or(5),
        ),
        "memory_get" => Memory::open(b.home()?)?.get(
            a["id"]
                .as_i64()
                .ok_or_else(|| Failure::new("invalid_argument", "id must be an integer"))?,
        ),
        "memory_digest" => Memory::open(b.home()?)?.digest(s(a, "project"), opt(a, "add")),
        "goal" => {
            let keys = ["status", "classify", "mode"];
            if keys
                .iter()
                .filter(|k| a.get(**k).is_some_and(|v| !v.is_null()))
                .count()
                != 1
            {
                return Err(Failure::new(
                    "invalid_argument",
                    "exactly one of status, classify, mode is required",
                ));
            }
            let home = b.home()?;
            let session = s(a, "session");
            let g = if let Some(v) = opt(a, "status") {
                goal::set_status(home, session, v, opt(a, "note"))?["goal"].clone()
            } else if let Some(v) = opt(a, "mode") {
                json!(goal::set_mode(home, session, v)?)
            } else if let Some(v) = opt(a, "classify") {
                let g = goal::classify(home, session, v)?;
                if v == "stop" {
                    b.cancel();
                }
                json!(g)
            } else {
                return Err(Failure::new("invalid_argument", "invalid goal operation"));
            };
            Ok(json!({"status":"completed","goal":g}))
        }
        "bridge_status" => {
            let mut result = json!({"status":"completed","codex_app_server_running":b.running()});
            for (key, cmd) in [("codex", &b.config.codex)] {
                result[key] = match cmd {
                    Err(e) => json!({"error":e.to_json()}),
                    Ok(c) => match crate::base::proc::command(c)
                        .arg("--version")
                        .stdin(Stdio::null())
                        .output()
                    {
                        Ok(o) => {
                            let text = if o.stdout.is_empty() {
                                &o.stderr
                            } else {
                                &o.stdout
                            };
                            let text = String::from_utf8_lossy(text);
                            json!({"command":c,"version":text.trim().lines().last().map(|x|vec![x]),"exit_code":o.status.code()})
                        }
                        Err(e) => json!({"error":{"kind":"probe_failed","message":e.to_string()}}),
                    },
                };
            }
            result["agy"] = json!({"status":"disabled"});
            Ok(result)
        }
        _ => Err(Failure::new("unknown_tool", format!("unknown tool {n}"))),
    }
}
