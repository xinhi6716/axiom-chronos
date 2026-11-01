use super::server::Bridge;
use crate::{
    base::{Failure, Res},
    goal, linux,
    memory::{Memory, Record},
    orch,
};
use serde_json::{json, Value};
use std::{
    path::{Component, Path},
    process::Stdio,
};

fn astra_alias(b: &Bridge, a: &Value, review: bool) -> Res<Value> {
    let server = b.codex()?;
    let model = match opt(a, "model").filter(|v| !v.is_empty()) {
        Some(model) if crate::codex::role_matches("astra", model) => model.to_owned(),
        Some(_) => return Err(Failure::new("invalid_argument", "model must be Astra")),
        None => server.resolve_model("astra")?,
    };
    let effort = if review { "high" } else { "medium" };
    if !server
        .model_capabilities()?
        .iter()
        .any(|c| c.id == model && c.supported_efforts.iter().any(|e| e == effort))
    {
        return Err(Failure::new(
            "unsupported_effort",
            "requested Astra effort is not advertised",
        ));
    }
    let cwd = if review {
        s(a, "project_dir")
    } else {
        opt(a, "cwd").unwrap_or("")
    };
    let cwd = if cwd.is_empty() {
        b.home()?
    } else {
        Path::new(cwd)
    };
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(Failure::new(
            "invalid_argument",
            "an absolute existing directory is required",
        ));
    }
    let (brief, schema) = if review {
        let paths = arr(a, "paths")?;
        let canonical_cwd = std::fs::canonicalize(cwd)
            .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
        if paths.is_empty()
            || s(a, "contract").trim().is_empty()
            || paths.iter().any(|p| {
                let p = Path::new(p);
                p.components().any(|c| !matches!(c, Component::Normal(_)))
                    || !cwd.join(p).is_file()
                    || std::fs::canonicalize(cwd.join(p))
                        .map_or(true, |resolved| !resolved.starts_with(&canonical_cwd))
            })
        {
            return Err(Failure::new(
                "invalid_argument",
                "review requires a contract and relative existing files",
            ));
        }
        (format!("Independently review these files against the contract. Report findings and verdict; fail for blocker or major findings.\nFiles: {}\nContract:\n{}", paths.join(", "), s(a, "contract")), Some(json!({"type":"object","required":["verdict","findings"],"properties":{"verdict":{"type":"string","enum":["pass","fail"]},"findings":{"type":"array","items":{"type":"object"}}}})))
    } else {
        if s(a, "prompt").trim().is_empty() {
            return Err(Failure::new("invalid_argument", "prompt is required"));
        }
        (
            s(a, "prompt").to_owned(),
            a.get("json_schema").filter(|v| !v.is_null()).cloned(),
        )
    };
    let req = crate::codex::RunRequest {
        brief,
        cwd: cwd.to_string_lossy().into_owned(),
        model: model.clone(),
        sandbox: "read-only".into(),
        effort: Some(effort.into()),
        output_schema: schema,
        developer_instructions: None,
        timeout_s: timeout(a, if review { 900 } else { 600 }),
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    };
    let mut result = server.run(&req)?;
    if review && result["status"] == "completed" {
        let output = result["structured_output"].clone();
        let serious = output["findings"].as_array().is_some_and(|fs| {
            fs.iter()
                .any(|f| matches!(f["severity"].as_str(), Some("blocker" | "major")))
        });
        result["verdict"] = if serious {
            json!("fail")
        } else {
            output["verdict"].clone()
        };
        result["findings"] = output["findings"].clone();
        result["reviewer"] = json!(model);
        if let (Some(project), Some(_)) = (opt(a, "project"), a.get("units")) {
            let states = orch::record_review(
                &store(b, project)?,
                &arr(a, "units")?,
                &format!(
                    "{}#{}",
                    model,
                    result["thread_id"].as_str().unwrap_or("unknown")
                ),
                result["verdict"] == "pass",
                cwd,
            )?;
            result["states"] = json!(states
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>());
        }
    }
    if !review {
        result["response"] = result["final_message"].clone();
    }
    Ok(result)
}
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
    match n {
        "dispatch" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                project: String,
                repo_root: std::path::PathBuf,
                request_id: String,
                action: super::dispatch::DispatchAction,
            }
            let args: Args = serde_json::from_value(a.clone())
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            if !args.repo_root.is_absolute() {
                return Err(Failure::new(
                    "invalid_argument",
                    "repo_root must be absolute",
                ));
            }
            let root = super::dispatch::RootContext {
                project: args.project,
                repo_root: std::fs::canonicalize(args.repo_root)
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?,
            };
            let request = super::dispatch::DispatchRequest {
                request_id: args.request_id,
                action: args.action,
            };
            let result = b.dispatch(&root, &request, &Default::default(), None)?;
            serde_json::to_value(result).map_err(|e| Failure::new("serialization", e.to_string()))
        }
        "codex_run" => Ok(super::codex_tool::codex_run(b, a)),
        "gemini_preview" => {
            let stage = opt(a, "stage");
            if stage.is_some_and(|s| !["second_review", "rewrite"].contains(&s)) {
                return Err(Failure::new("invalid_argument", "invalid stage"));
            }
            let root = opt(a, "project").map(|p| store(b, p)).transpose()?;
            let dest = opt(a, "save_to")
                .map(|path| {
                    super::codex_tool::destination(
                        root.as_deref().ok_or_else(|| {
                            Failure::new("invalid_argument", "save_to requires project")
                        })?,
                        path,
                    )
                })
                .transpose()?;
            if let (Some(root), Some(_)) = (&root, stage) {
                std::fs::create_dir_all(root).map_err(|e| Failure::new("io", e.to_string()))?;
            }
            let mut result = match astra_alias(b, a, false) {
                Ok(result) => result,
                Err(e) => {
                    if let (Some(root), Some(stage)) = (&root, stage) {
                        orch::record_stage(root, stage, "codex-astra", false)?;
                    }
                    return Err(e);
                }
            };
            let mut completed = result["status"] == "completed";
            if let Some(dest) = dest {
                if completed && !result["structured_output"].is_null() {
                    let bytes = serde_json::to_vec_pretty(&result["structured_output"])
                        .map_err(|e| Failure::new("io", e.to_string()))?;
                    crate::base::atomic_write(&dest, &bytes)
                        .map_err(|e| Failure::new("io", e.to_string()))?;
                } else if completed {
                    completed = false;
                    result["status"] = json!("failed");
                    result["error"] = json!({"kind":"invalid_structured_output","message":"structured output is required"});
                }
            }
            if let (Some(root), Some(stage)) = (&root, stage) {
                orch::record_stage(
                    root,
                    stage,
                    result["model"].as_str().unwrap_or(""),
                    completed,
                )?;
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
        "review_code" => astra_alias(b, a, true),
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
        "memory_search" => Memory::open(b.home()?)?.search(
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
            for (key, cmd) in [("codex", &b.config.codex), ("agy", &b.config.agy)] {
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
            Ok(result)
        }
        _ => Err(Failure::new("unknown_tool", format!("unknown tool {n}"))),
    }
}
