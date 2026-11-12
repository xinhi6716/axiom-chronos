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

// BEGIN research-program bridge
struct ProgramMember<'a> {
    name: &'static str,
    bridge: &'a Bridge,
    setup: &'a council::Setup,
    cwd: &'a std::path::Path,
}

impl crate::research_program::investigation::ResearchMember for ProgramMember<'_> {
    fn name(&self) -> &str {
        self.name
    }

    fn call(
        &self,
        call: &crate::research_program::investigation::MemberCall,
        stop: &AtomicBool,
    ) -> crate::research_program::investigation::MemberReply {
        use crate::research_program::investigation::MemberReply;
        let schema_path = call.out_dir.join("schema.json");
        let mut reply = if let Err(e) = std::fs::write(&schema_path, call.schema.to_string()) {
            CrossReply {
                status: Status::Failed,
                output: None,
                raw: None,
                tokens: 0,
                model: None,
                detail: Some(e.to_string()),
            }
        } else {
            match self.name {
                "astra" => cross(
                    self.bridge,
                    &self.cwd.to_string_lossy(),
                    &CrossRequest {
                        prompt: format!(
                            "{}\nReturn only JSON matching this schema:\n{}",
                            call.prompt, call.schema
                        ),
                        schema: call.schema.clone(),
                        timeout_s: call.timeout_s,
                        token_cap: self.setup.astra_token_cap,
                    },
                ),
                "fable" => council::ask_fable_once(
                    self.setup,
                    &call.prompt,
                    self.cwd,
                    &call.schema,
                    &schema_path,
                    std::time::Duration::from_secs(call.timeout_s),
                    stop,
                ),
                _ => council::ask_agy(
                    &self.setup.argon,
                    &call.prompt,
                    self.cwd,
                    &call.schema,
                    &schema_path,
                    std::time::Duration::from_secs(call.timeout_s),
                    stop,
                ),
            }
        };
        if reply.status == Status::Ok && reply.output.is_none() {
            match reply.raw.as_deref().map(serde_json::from_str::<Value>) {
                Some(Ok(value)) => reply.output = Some(value),
                Some(Err(e)) => {
                    reply.status = Status::Malformed;
                    reply.detail = Some(e.to_string());
                }
                None => {
                    reply.status = Status::Malformed;
                    reply.detail = Some("missing output".into());
                }
            }
        }
        MemberReply {
            status: reply.status,
            output: reply.output,
            raw: reply.raw,
            model: reply.model,
            provider: self.name.into(),
            usd: None,
            tokens: reply.tokens,
            detail: reply.detail,
        }
    }
}

fn program(b: &Bridge, setup: &council::Setup, store: PathBuf, a: &Value) -> Res<Value> {
    use crate::research_program::{
        adjudication::MemberAdjudicator,
        runner::{self, CouncilRunner, Members, ProgramConfig},
    };
    let brief = text(a, "brief")?;
    let cwd = text(a, "cwd")?;
    base::require_directory(cwd)?;
    let sources = match a.get("sources") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("sources must be an array of strings"))
            })
            .collect::<Res<Vec<_>>>()?,
        Some(_) => return Err(invalid("sources must be an array of strings")),
    };
    let cfg = ProgramConfig {
        store,
        cwd: cwd.into(),
        question: brief.into(),
        sources,
        home: b.home()?.into(),
        initial_cap_usd: setup.fable_cap_usd,
    };
    let astra = ProgramMember {
        name: "astra",
        bridge: b,
        setup,
        cwd: &cfg.cwd,
    };
    let fable = ProgramMember {
        name: "fable",
        bridge: b,
        setup,
        cwd: &cfg.cwd,
    };
    let argon = ProgramMember {
        name: "argon",
        bridge: b,
        setup,
        cwd: &cfg.cwd,
    };
    let members = Members {
        investigators: vec![&astra, &fable, &argon],
        red_team: &argon,
        pre_mortem: &fable,
        synthesizer: &astra,
    };
    let astra_cross = |request: &CrossRequest| cross(b, cwd, request);
    let council = CouncilRunner {
        setup,
        astra: &astra_cross,
    };
    let adjudicator = MemberAdjudicator(&astra);
    let (running, stop) = b.council_flags();
    if running.swap(true, Ordering::SeqCst) {
        return Err(Failure::new("busy", "a council is already running"));
    }
    let _running = Running(running);
    stop.store(false, Ordering::SeqCst);
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
        let result = runner::run(&cfg, &members, &council, &adjudicator, stop);
        done.store(true, Ordering::SeqCst);
        result
    })
}
// END research-program bridge

pub fn council(b: &Bridge, a: &Value) -> Res<Value> {
    let Some(setup) = b.council_setup() else {
        return Err(Failure::new("unknown_tool", "unknown tool council"));
    };
    let store = orch::store_for(b.home()?, text(a, "project")?)?;
    // BEGIN research-program dispatch
    if a.get("action").and_then(Value::as_str) == Some("program") {
        return program(b, setup, store, a);
    }
    // END research-program dispatch
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
            council::rule(&plan_path, &rulings)
        }
        other => Err(invalid(format!("unknown action {other}"))),
    }
}

// BEGIN research-program descriptor
pub fn descriptor() -> Value {
    json!({
        "name": "council",
        "description": "Advanced research, available only because this session opted in. `program` runs the research program before planning and returns its bounded digest. After Astra has written its plan, `run` conducts an ordered Astra/Fable dialogue, one Argon review, and ordered Astra/Fable replies only to objections. Dialogue phases stop on agreement, budgets or their round cap; the digest identifies each phase and turn. The reply is a bounded digest; do not read the position files. `rule` records your rulings and its reply carries the next unruled claims until none remain.",
        "inputSchema": {"type": "object", "properties": {
            "action": {"type": "string", "enum": ["run", "rule", "program"], "default": "run"},
            "project": {"type": "string", "description": "Program slug."},
            "plan_path": {"type": "string", "description": "Absolute path of the plan Astra saved in the store."},
            "cwd": {"type": "string", "description": "Absolute project directory (run or program)."},
            "brief": {"type": "string", "description": "The exact brief (run or program)."},
            "sources": {"type": "array", "description": "Project-relative source paths (program).", "items": {"type": "string"}},
            "schema_path": {"type": "string", "description": "Absolute path of the schema Astra answered (run)."},
            "rulings": {"type": "array", "description": "One entry per open claim (rule).", "items": {"type": "object", "properties": {
                "id": {"type": "string"},
                "decision": {"type": "string", "enum": ["upheld", "rejected", "narrowed"]},
                "reason": {"type": "string"}
            }, "required": ["id", "decision", "reason"]}}
        }, "required": ["project"]}
    })
}

// END research-program descriptor

// BEGIN research-program bridge test
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::server::Config;

    #[test]
    fn program_is_offered_and_requires_brief() {
        assert!(descriptor()["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("program")));
        let bridge = Bridge::new(Config {
            data_home: Some(std::env::temp_dir().join(format!("council-test-{}", base::unique()))),
            codex: Ok(vec![]),
            wsl: None,
            on_windows: false,
        })
        .with_council(council::Setup::new(Ok(vec![]), Ok(vec![])));
        let error = council(&bridge, &json!({"action":"program", "project":"test"})).unwrap_err();
        assert_eq!(error.kind, "invalid_argument");
        assert_eq!(error.message, "brief is required");
    }
}
// END research-program bridge test
