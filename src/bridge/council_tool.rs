//! The `council` tool. It exists only in a session launched with `--advanced-research`.
use super::{codex_tool::destination, server::Bridge};
use crate::{
    base::{self, Failure, Res},
    codex::RunRequest,
    council::{self, CrossReply, CrossRequest, Status},
    engine::{
        adjudicator::{self, Case, Evidence, Outcome},
        budgets::{self, Role},
        dual_core::{self, Decision, Kind},
        policy,
    },
    goal, orch,
};
use serde_json::{json, Value};
use std::{
    io::Write,
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
        raw: None,
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
        output_schema: None,
        developer_instructions: Some(
            "Answer from the text you are given. Do not run commands or read files.".into(),
        ),
        timeout_s: request.timeout_s,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    };
    let report =
        match server.run_cancellable(&run, Some(request.token_cap), Some(b.council_flags().1)) {
            Ok(report) => report,
            Err(e) => return failed(e),
        };
    let kind = report["error"]["kind"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    let status = match report["status"].as_str() {
        Some("completed") if report["final_message"].is_string() => Status::Ok,
        Some("completed") => Status::Malformed,
        Some("token_cap") => Status::BudgetCut,
        Some("timeout") => Status::Timeout,
        _ if kind.contains("usagelimit") => Status::Quota,
        _ if kind == "invalid_structured_output" => Status::Malformed,
        _ => Status::Failed,
    };
    CrossReply {
        status,
        output: None,
        raw: report["final_message"].as_str().map(str::to_owned),
        tokens: report["usage"]["totalTokens"].as_u64().unwrap_or(0),
        model: Some(model),
        detail: if status == Status::Ok {
            report["slot_wait"].as_str().map(str::to_owned)
        } else {
            Some(format!(
                "{}{}",
                report["error"]["message"],
                report["slot_wait"]
                    .as_str()
                    .map(|s| format!("; {s}"))
                    .unwrap_or_default()
            ))
        },
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
            if let Err(e) = budgets::check(&store, Role::CouncilMember, "council") {
                if e.kind == "budget_exceeded" {
                    return Ok(json!({"status":"refused","error":e.to_json()}));
                }
                return Err(e);
            }
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
                let events_path = request
                    .plan_path
                    .parent()
                    .unwrap()
                    .join("council-events.jsonl");
                let event_error = std::sync::Mutex::new(None);
                let sink = |event: &Value| {
                    let message = event.to_string();
                    if let Some(progress) = &progress {
                        progress.emit(&message);
                    }
                    let write = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&events_path)
                        .and_then(|mut f| writeln!(f, "{message}"));
                    if let Err(e) = write {
                        *event_error.lock().unwrap() = Some(e.to_string());
                    }
                };
                let mut result =
                    council::run_with_events(setup, &request, &|r| cross(b, cwd, r), stop, &sink);
                if let Some(e) = event_error.into_inner().unwrap() {
                    result = Err(Failure::new("io", e));
                }
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
            let mut result = council::rule(&plan_path, &rulings)?;
            let policy = policy::load(&store)?;
            if !policy.dual_core && !policy.adjudicator.enabled {
                return Ok(result);
            }
            let project = text(a, "project")?;
            let cwd = goal::find_active(b.home()?, project)?
                .and_then(|(_, g)| g.cwd.map(PathBuf::from))
                .or_else(|| b.project_cwd(project))
                .unwrap_or_else(|| store.clone());
            let codex = b.codex()?;
            let path = plan_path.parent().unwrap().join("council.json");
            let council: council::Council = serde_json::from_slice(
                &std::fs::read(&path).map_err(|e| Failure::new("io", e.to_string()))?,
            )
            .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            if policy.dual_core {
                let mut verdicts = Vec::new();
                for (id, choice, reason) in &rulings {
                    let claim = council
                        .claims
                        .iter()
                        .find(|c| c.id == *id)
                        .ok_or_else(|| invalid(format!("unknown claim {id}")))?;
                    let detail = serde_json::to_string(claim)
                        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                    let d = Decision {
                        kind: Kind::CouncilRuling,
                        id: id.clone(),
                        question: format!(
                            "Council ruling for {id}: {detail}\nCore reason: {reason}"
                        ),
                        inputs: vec![],
                        core_choice: choice.clone(),
                        core_reasoning: reason.clone(),
                        irreversible: false,
                        reserved: adjudicator::is_reserved(&claim.text)
                            || adjudicator::is_reserved(reason),
                    };
                    let verdict = dual_core::judge(codex.as_ref(), &store, &cwd, &d)?;
                    verdicts.push(json!({"id":id,"verdict":format!("{verdict:?}")}));
                }
                result["dual_core"] = json!(verdicts);
            }
            if policy.adjudicator.enabled {
                let mut outcomes = Vec::new();
                for claim in council
                    .claims
                    .iter()
                    .filter(|c| c.status != council::ClaimStatus::Consensus && c.ruling.is_none())
                {
                    let detail = serde_json::to_string(claim)
                        .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
                    let case = Case::CouncilDispute {
                        claim: claim.text.clone(),
                        positions: vec![detail.clone()],
                    };
                    let evidence = vec![Evidence {
                        id: "E1".into(),
                        path: Some(path.clone()),
                        text: detail,
                    }];
                    let outcome = adjudicator::adjudicate(
                        codex.as_ref(),
                        &store,
                        &cwd,
                        &case,
                        evidence,
                        false,
                    )?;
                    match outcome {
                        Outcome::Ruled(ruling) => {
                            adjudicator::apply(&store, &case, &ruling)?;
                            outcomes.push(json!({"id":claim.id,"ruling_id":ruling.id,"decision":ruling.decision,"reason":ruling.reason,"evidence":ruling.evidence,"answer":ruling.answer}));
                        }
                        Outcome::Person { reason } => {
                            outcomes.push(json!({"id":claim.id,"person":reason}))
                        }
                    }
                }
                result["adjudications"] = json!(outcomes);
            }
            Ok(result)
        }
        other => Err(invalid(format!("unknown action {other}"))),
    }
}

pub fn descriptor() -> Value {
    json!({
        "name": "council",
        "description": "Advanced research, available only because this session opted in. After Astra has written its plan, `run` conducts an ordered Astra/Fable dialogue, one Argon review, and ordered Astra/Fable replies only to objections. Dialogue phases stop on agreement, budgets or their round cap; the digest identifies each phase and turn. The reply is a bounded digest; do not read the position files. `rule` records your rulings and its reply carries the next unruled claims until none remain.",
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
