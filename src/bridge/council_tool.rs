//! The `council` tool. It exists only in a session launched with `--advanced-research`.
use super::{codex_tool::destination, server::Bridge};
use crate::{
    base::{self, Failure, Res},
    codex::RunRequest,
    council::{self, CrossReply, CrossRequest, Status},
    orch,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

const HEARTBEAT_TICK: std::time::Duration = std::time::Duration::from_millis(200);
const HEARTBEAT_EVERY: u32 = 75;

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message.into())
}
fn text<'a>(a: &'a Value, key: &str) -> Res<&'a str> {
    a.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(format!("{key} is required")))
}

struct Running<'a>(&'a AtomicBool);
impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn cross(b: &Bridge, cwd: &str, request: &CrossRequest) -> CrossReply {
    let failed = |e: Failure| CrossReply {
        status: Status::Failed,
        output: None,
        tokens: 0,
        model: None,
        detail: Some(e.message),
    };
    let server = match b.codex() {
        Ok(server) => server,
        Err(e) => return failed(e),
    };
    let model = match server.resolve_model("astra") {
        Ok(model) => model,
        Err(e) => return failed(e),
    };
    let run = RunRequest {
        brief: request.prompt.clone(),
        cwd: cwd.into(),
        model: model.clone(),
        sandbox: "read-only".into(),
        effort: Some("high".into()),
        output_schema: Some(request.schema.clone()),
        developer_instructions: Some(
            "Answer from the text you are given. Do not run commands or read files.".into(),
        ),
        timeout_s: request.timeout_s,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    };
    let report = match server.run_capped(&run, Some(request.token_cap)) {
        Ok(report) => report,
        Err(e) => return failed(e),
    };
    let kind = report["error"]["kind"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    let status = match report["status"].as_str() {
        Some("completed") if !report["structured_output"].is_null() => Status::Ok,
        Some("token_cap") => Status::BudgetCut,
        Some("timeout") => Status::Timeout,
        _ if kind.contains("usagelimit") => Status::Quota,
        _ if kind == "invalid_structured_output" => Status::Malformed,
        _ => Status::Failed,
    };
    CrossReply {
        status,
        output: (status == Status::Ok).then(|| report["structured_output"].clone()),
        tokens: report["usage"]["totalTokens"].as_u64().unwrap_or(0),
        model: Some(model),
        detail: (status != Status::Ok).then(|| report["error"]["message"].to_string()),
    }
}

pub fn council(b: &Bridge, a: &Value) -> Res<Value> {
    let Some(setup) = b.council_setup() else {
        return Err(Failure::new("unknown_tool", "unknown tool council"));
    };
    let store = orch::store_for(b.home()?, text(a, "project")?)?;
    let plan_path = destination(&store, text(a, "plan_path")?)?;
    match a.get("action").and_then(Value::as_str).unwrap_or("run") {
        "run" => {
            let cwd = text(a, "cwd")?;
            base::require_directory(cwd)?;
            let request = council::Request {
                brief: text(a, "brief")?.to_owned(),
                cwd: cwd.into(),
                plan_path,
                schema_path: PathBuf::from(text(a, "schema_path")?),
            };
            let (running, stop) = b.council_flags();
            if running.swap(true, Ordering::SeqCst) {
                return Err(Failure::new("busy", "a council is already running"));
            }
            let _running = Running(running);
            stop.store(false, Ordering::SeqCst);
            // Members run on their own threads; this keeps the client from timing the call out.
            let progress = base::progress::current();
            let done = AtomicBool::new(false);
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    let mut ticks = 0;
                    while !done.load(Ordering::SeqCst) {
                        std::thread::sleep(HEARTBEAT_TICK);
                        ticks += 1;
                        if ticks % HEARTBEAT_EVERY == 0 {
                            if let Some(progress) = &progress {
                                progress.emit("council");
                            }
                        }
                    }
                });
                let result = council::run(setup, &request, &|r| cross(b, cwd, r), stop);
                done.store(true, Ordering::SeqCst);
                result
            })
        }
        "rule" => {
            let rulings = a
                .get("rulings")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("rulings must be an array"))?
                .iter()
                .map(|r| {
                    Ok((
                        text(r, "id")?.to_owned(),
                        text(r, "decision")?.to_owned(),
                        text(r, "reason")?.to_owned(),
                    ))
                })
                .collect::<Res<Vec<_>>>()?;
            council::rule(&plan_path, &rulings)
        }
        other => Err(invalid(format!("unknown action {other}"))),
    }
}

pub fn descriptor() -> Value {
    json!({
        "name": "council",
        "description": "Advanced research, available only because this session opted in. After Astra has written its plan, `run` puts the same brief to the two other members, cross-examines all positions once and settles the claims mechanically. The reply is a bounded digest; do not read the position files. `rule` records your ruling on each open claim.",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["run", "rule"], "default": "run"},
            "project": {"type": "string", "description": "Program slug."},
            "plan_path": {"type": "string", "description": "Absolute path of the plan Astra saved in the store."},
            "cwd": {"type": "string", "description": "Absolute project directory (run)."},
            "brief": {"type": "string", "description": "The exact brief Astra was given (run)."},
            "schema_path": {"type": "string", "description": "Absolute path of the schema Astra answered (run)."},
            "rulings": {"type": "array", "description": "One entry per open claim (rule).", "items": {"type": "object", "properties": {
                "id": {"type": "string"},
                "decision": {"type": "string", "enum": ["upheld", "rejected", "narrowed"]},
                "reason": {"type": "string"}
            }, "required": ["id", "decision", "reason"]}}
        }, "required": ["project", "plan_path"]}
    })
}
