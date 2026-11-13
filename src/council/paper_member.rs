//! Paper-specific research members, isolated from council calls.
use super::{
    member::{ask_agy, ask_agy_stdin, ask_fable_model, Answer, Ask, Meter},
    Status, AGY_PROMPT_LIMIT, FABLE_MODEL,
};
use crate::{
    base::Res,
    paper::members::PaperMember,
    research_program::investigation::{MemberCall, MemberReply, ResearchMember},
    usage::pools::HYDROGEN_XHIGH,
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Mutex, OnceLock},
    time::Duration,
};

pub struct PaperFable {
    command: Res<Vec<String>>,
    ledger: PathBuf,
    cap_usd: f64,
}
impl PaperFable {
    pub fn new(command: Res<Vec<String>>, ledger: PathBuf, cap_usd: f64) -> Self {
        Self {
            command,
            ledger,
            cap_usd,
        }
    }
}
pub struct PaperGemini {
    command: Res<Vec<String>>,
}
impl PaperGemini {
    pub fn new(command: Res<Vec<String>>) -> Self {
        Self { command }
    }
}

fn failed(detail: String) -> Answer {
    Answer {
        status: Status::Failed,
        output: None,
        raw: None,
        tokens: 0,
        detail: Some(detail),
    }
}
fn reply(mut answer: Answer, provider: &str, model: &str, usd: Option<f64>) -> MemberReply {
    if answer.raw.as_deref().is_some_and(|raw| {
        raw.lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|v| {
                v.get("model")
                    .and_then(|v| v.as_str())
                    .or_else(|| v.pointer("/message/model").and_then(|v| v.as_str()))
                    .is_some_and(|actual| actual != model)
            })
    }) {
        answer.status = Status::Malformed;
        answer.output = None;
        answer.detail = Some("wrong_model".into());
    }
    MemberReply {
        status: answer.status,
        output: answer.output,
        raw: answer.raw,
        tokens: answer.tokens,
        detail: answer.detail,
        provider: provider.into(),
        model: Some(model.into()),
        usd,
    }
}
fn prepare(call: &MemberCall) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&call.out_dir).map_err(|e| e.to_string())?;
    let path = call.out_dir.join("schema.json");
    std::fs::write(&path, call.schema.to_string()).map_err(|e| e.to_string())?;
    // The child runs in out_dir; relative schema paths must not be interpreted twice.
    path.canonicalize().map_err(|e| e.to_string())
}
fn ask<'a>(call: &'a MemberCall, stop: &'a AtomicBool) -> Ask<'a> {
    Ask {
        prompt: call.prompt.clone(),
        cwd: &call.out_dir,
        schema: &call.schema,
        timeout: Duration::from_secs(call.timeout_s),
        stop,
        raw_path: None,
    }
}
impl ResearchMember for PaperFable {
    fn name(&self) -> &str {
        "fable"
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        self.call_model(call, FABLE_MODEL, stop)
    }
}
impl PaperMember for PaperFable {
    fn call_model(&self, call: &MemberCall, model: &str, stop: &AtomicBool) -> MemberReply {
        // Serialize ledger read/modify/write across paper calls within this process.
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut meter = match Meter::open(&self.ledger, "paper", self.cap_usd) {
            Ok(meter) => meter,
            Err(e) => return reply(failed(e.message), "fable", model, None),
        };
        if let Err(e) = prepare(call) {
            return reply(failed(e), "fable", model, None);
        }
        let before = meter.spent_usd();
        let mut answer = ask_fable_model(&self.command, ask(call, stop), "", &mut meter, model);
        if answer.status == Status::Ok && !meter.observed() {
            answer.status = Status::BudgetCut;
            answer.output = None;
            answer.detail = Some("no usage was reported, so its cost cannot be bounded".into());
        }
        let usd = (meter.observed() && meter.unpriced_tokens() == 0)
            .then_some(meter.spent_usd() - before);
        reply(answer, "fable", model, usd)
    }
}
impl ResearchMember for PaperGemini {
    fn name(&self) -> &str {
        "gemini"
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        self.call_model(call, HYDROGEN_XHIGH, stop)
    }
}
impl PaperMember for PaperGemini {
    fn call_model(&self, call: &MemberCall, model: &str, stop: &AtomicBool) -> MemberReply {
        let path = match prepare(call) {
            Ok(path) => path,
            Err(e) => return reply(failed(e), "gemini", model, None),
        };
        let answer = if call.prompt.encode_utf16().count() <= AGY_PROMPT_LIMIT {
            ask_agy(&self.command, ask(call, stop), &path, model)
        } else {
            ask_agy_stdin(&self.command, ask(call, stop), &path, model)
        };
        // If the CLI echoes no model, argv proves the requested model, so it is the reply model.
        reply(answer, "gemini", model, None)
    }
}
