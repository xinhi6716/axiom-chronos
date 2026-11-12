use super::server::Bridge;
use crate::{
    base::{Failure, Res},
    goal, graph, linux,
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

fn protected(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "kind"
            | "unit"
            | "state"
            | "next"
            | "exit_code"
            | "path"
            | "pack"
            | "report"
            | "report_json"
            | "verify_output"
            | "output_file"
    ) || key.ends_with("_file")
}

fn contains_protected(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| protected(k) || contains_protected(v)),
        Value::Array(values) => values.iter().any(contains_protected),
        _ => false,
    }
}

fn shrink(value: &mut Value, limit: usize, keep: bool, level: u8) {
    match value {
        Value::String(s) if !keep && s.len() > limit => {
            let mut end = limit.min(s.len());
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        Value::Array(values) => {
            let count = limit / 16;
            values.truncate(count);
            for v in values {
                shrink(v, limit, keep, level);
            }
        }
        Value::Object(map) => {
            let count = limit / 16;
            let mut i = 0;
            map.retain(|k, v| {
                i += 1;
                i <= count || protected(k) || contains_protected(v)
            });
            for (k, v) in map {
                if k == "stdout" {
                    if let Some(s) = v.as_str() {
                        if let Ok(mut inner) = serde_json::from_str::<Value>(s) {
                            shrink(&mut inner, limit, false, level);
                            *v = json!(inner.to_string());
                            continue;
                        }
                    }
                }
                let keep = protected(k)
                    && (level == 0
                        || (level == 1
                            && matches!(
                                k.as_str(),
                                "status" | "kind" | "unit" | "state" | "next" | "exit_code"
                            )));
                shrink(v, limit, keep, level);
            }
        }
        _ => {}
    }
}

fn reply_size(payload: &Value) -> usize {
    let body = super::server::call_body(|| Ok(payload.clone()));
    serde_json::to_vec(&body).expect("JSON payload").len()
}

/// Bound the complete bridge response, including its duplicated pretty-printed content.
pub fn bound_reply(payload: Value) -> Value {
    use crate::orch::summary::MAX_REPLY_BYTES;
    if reply_size(&payload) <= MAX_REPLY_BYTES {
        return payload;
    }
    let payload = if payload.is_object() {
        payload
    } else {
        json!({"value":payload})
    };
    for level in 0..=2 {
        let mut low = 0;
        let mut high = MAX_REPLY_BYTES;
        let mut best = None;
        while low <= high {
            let middle = (low + high) / 2;
            let mut candidate = payload.clone();
            shrink(&mut candidate, middle, false, level);
            candidate["truncated"] = json!(true);
            if reply_size(&candidate) <= MAX_REPLY_BYTES {
                best = Some(candidate);
                low = middle + 1;
            } else if middle == 0 {
                break;
            } else {
                high = middle - 1;
            }
        }
        if let Some(best) = best {
            return best;
        }
    }
    let mut candidate = payload;
    shrink(&mut candidate, 0, false, 2);
    candidate["truncated"] = json!(true);
    candidate
}

fn unit_reply(mut value: Value, b: &Bridge, args: &Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.remove("verify_tail");
        if let Some(accept) = map.get_mut("accept").and_then(Value::as_object_mut) {
            if accept.remove("output").is_some() {
                if let (Ok(root), Some(module)) = (
                    store(b, s(args, "project")),
                    accept.get("module").and_then(Value::as_str),
                ) {
                    accept.insert(
                        "output_file".into(),
                        json!(orch::accept_output_path(&root, module)),
                    );
                }
            }
        }
    }
    bound_reply(value)
}
pub fn dispatch(b: &Bridge, n: &str, a: &Value) -> Res<Value> {
    match n {
        "codex_run" => {
            let result = super::codex_tool::codex_run(b, a);
            Ok(if a.get("unit").is_some() {
                unit_reply(result, b, a)
            } else {
                result
            })
        }
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
            let bounded = matches!(
                args.first().map(String::as_str),
                Some("verify" | "accept" | "schedule")
            );
            let force = args.iter().any(|x| x == "--force");
            args.retain(|x| x != "--force");
            let r = orch::run(&path, &args, json_flag, force);
            let schedule_kind = (args.first().map(String::as_str) == Some("schedule"))
                .then(|| {
                    r.stderr
                        .strip_prefix("error: ")
                        .and_then(|s| s.split_once(": "))
                })
                .flatten()
                .map(|(kind, _)| kind)
                .filter(|kind| {
                    [
                        "invalid_argument",
                        "not_ready",
                        "already_claimed",
                        "already_verified",
                        "held",
                        "not_found",
                        "not_claimed",
                        "no_map",
                    ]
                    .contains(kind)
                });
            let err = if r.exit_code == 0 {
                Value::Null
            } else {
                json!({"kind":schedule_kind.unwrap_or("orch_failed"),"message":if r.stderr.is_empty(){&r.stdout}else{&r.stderr}.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>()})
            };
            let result = json!({"status":if r.exit_code==0 {"completed"}else{"failed"},"store":path,"exit_code":r.exit_code,"stdout":r.stdout,"stderr":r.stderr,"error":err});
            Ok(if bounded { bound_reply(result) } else { result })
        }
        "run_on_linux" => linux::run(
            b.config.wsl.clone(),
            b.config.on_windows,
            s(a, "project_dir"),
            &arr(a, "args")?,
            timeout(a, 1800),
        ),
        "memory_record" => {
            if opt(a, "mode").unwrap_or("unit") == "source" {
                let source = a.get("source").ok_or_else(|| Failure::new("invalid_argument", "source is required"))?;
                let artifact = source.get("artifact").and_then(Value::as_str).ok_or_else(|| Failure::new("invalid_argument", "source.artifact is required"))?;
                let links: Vec<graph::ingest::SourceLink> = serde_json::from_value(source.get("links").cloned().unwrap_or(json!([])))
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                let report = graph::ingest::import_source(&graph::GraphStore::open(b.home()?)?, std::path::Path::new(s(a, "project_dir")), artifact, &links)?;
                return Ok(json!({"status":"completed","source":report}));
            }
            if opt(a, "mode").unwrap_or("unit") != "unit" {
                return Err(Failure::new("invalid_argument", "mode must be unit or source"));
            }
            let record = Record {
            project: s(a, "project").into(),
            project_dir: s(a, "project_dir").into(),
            unit: s(a, "unit").into(),
            title: s(a, "title").into(),
            summary: s(a, "summary").into(),
            body: s(a, "body").into(),
            symbols: arr(a, "symbols")?,
            files: arr(a, "files")?,
            wish: s(a, "wish").into(),
            };
            let memory = Memory::open(b.home()?)?;
            if a.get("nodes").is_some() || a.get("links").is_some() {
                let typed: graph::TypedWrite = serde_json::from_value(json!({"nodes":a.get("nodes").cloned().unwrap_or(json!([])),"links":a.get("links").cloned().unwrap_or(json!([]))}))
                    .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                memory.record_typed(&record, &typed)
            } else {
                memory.record(&record)
            }
        }
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
        "graph_explore" => {
            let req: graph::query::ExploreRequest = serde_json::from_value(a.clone())
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            graph::query::explore(b.home()?, &req)
        }
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
