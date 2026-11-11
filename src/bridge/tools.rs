use super::server::Bridge;
use crate::{
    base::{Failure, Res},
    goal, linux,
    memory::{Memory, Record},
    orch,
};
use serde_json::{json, Value};
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
fn batch(a: &Value) -> Res<Option<goal::Batch>> {
    let given = ["progress_file", "marker_file", "pid"]
        .iter()
        .filter(|k| a.get(**k).is_some_and(|v| !v.is_null()))
        .count();
    if given == 0 {
        return Ok(None);
    }
    let pid = a
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    match (opt(a, "progress_file"), opt(a, "marker_file"), pid) {
        (Some(progress_file), Some(marker_file), Some(pid)) => Ok(Some(goal::Batch {
            progress_file: progress_file.into(),
            marker_file: marker_file.into(),
            pid,
        })),
        _ => Err(Failure::new(
            "invalid_argument",
            "progress_file, marker_file and pid go together: two paths and a process id",
        )),
    }
}
pub fn dispatch(b: &Bridge, n: &str, a: &Value) -> Res<Value> {
    match n {
        "codex_run" => Ok(super::codex_tool::codex_run(b, a)),
        "council" => super::council_tool::council(b, a),
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
            b.record_provenance(home, session)?;
            let g = if let Some(v) = opt(a, "status") {
                goal::set_status_with_batch(home, session, v, opt(a, "note"), batch(a)?)?["goal"]
                    .clone()
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
            let mut reply = json!({"status":g["status"],"mode":g["mode"],"reason":g["reason"]});
            for key in ["batch", "provenance"] {
                if !g[key].is_null() {
                    reply[key] = g[key].clone();
                }
            }
            Ok(json!({"status":"completed","goal":reply}))
        }
        "bridge_status" => {
            let mut result = json!({"status":"completed","codex_app_server_running":b.running()});
            for (key, cmd) in [("codex", &b.config.codex)] {
                result[key] = match cmd {
                    Err(e) => json!({"error":e.to_json()}),
                    Ok(c) => match crate::codex::probe_version(c) {
                        Ok(p) => {
                            json!({"command":c,"version":p.last_line.map(|x|vec![x]),"exit_code":p.exit_code,"version_check":p.check})
                        }
                        Err(e) => json!({"error":e.to_json()}),
                    },
                };
            }
            Ok(result)
        }
        _ => Err(Failure::new("unknown_tool", format!("unknown tool {n}"))),
    }
}
