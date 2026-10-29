use super::server::Bridge;
use crate::{
    agy,
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
        "codex_run" => Ok(super::codex_tool::codex_run(b, a)),
        "gemini_preview" => {
            let model = opt(a, "model")
                .filter(|x| !x.is_empty())
                .unwrap_or("gemini-4-argon-medium");
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
            let result = b.config.agy.clone().and_then(|command| {
                agy::run(
                    &command,
                    s(a, "prompt"),
                    opt(a, "cwd").map(Path::new),
                    model,
                    a.get("json_schema"),
                    timeout(a, 600),
                )
            });
            let mut result = match result {
                Ok(result) => result,
                Err(e) => {
                    if let (Some(root), Some(t)) = (&root, stage) {
                        let _ = orch::record_stage(root, t, model, false);
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
                orch::record_stage(root, t, model, completed)?;
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
            let model = opt(a, "model")
                .filter(|x| !x.is_empty())
                .unwrap_or("gemini-4-argon-high");
            let mut r = agy::review(
                s(a, "project_dir"),
                &arr(a, "paths")?,
                s(a, "contract"),
                model,
                timeout(a, 900),
                &b.config.agy.clone()?,
            )?;
            if r["status"] == "completed" && a.get("units").is_some() && opt(a, "project").is_some()
            {
                let states = orch::record_review(
                    &store(b, s(a, "project"))?,
                    &arr(a, "units")?,
                    model,
                    r["verdict"] == "pass",
                )?;
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
                    agy::cancel_all();
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
