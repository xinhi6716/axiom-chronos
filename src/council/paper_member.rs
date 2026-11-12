//! Paper-specific research members, isolated from council calls.
use super::{
    member::{ask_argon, ask_argon_stdin, ask_fable, Answer, Ask, Meter},
    Status, AGY_PROMPT_LIMIT, ARGON_MODEL, FABLE_MODEL,
};
use crate::{
    base::Res,
    research_program::investigation::{MemberCall, MemberReply, ResearchMember},
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
pub struct PaperArgon {
    command: Res<Vec<String>>,
}
impl PaperArgon {
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
fn reply(answer: Answer, provider: &str, model: &str, usd: Option<f64>) -> MemberReply {
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
        // Serialize ledger read/modify/write across paper calls within this process.
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut meter = match Meter::open(&self.ledger, "paper", self.cap_usd) {
            Ok(meter) => meter,
            Err(e) => return reply(failed(e.message), "fable", FABLE_MODEL, None),
        };
        if let Err(e) = prepare(call) {
            return reply(failed(e), "fable", FABLE_MODEL, None);
        }
        let before = meter.spent_usd();
        let mut answer = ask_fable(&self.command, ask(call, stop), "", &mut meter);
        if answer.status == Status::Ok && !meter.observed() {
            answer.status = Status::BudgetCut;
            answer.output = None;
            answer.detail = Some("no usage was reported, so its cost cannot be bounded".into());
        }
        let usd = (meter.observed() && meter.unpriced_tokens() == 0)
            .then_some(meter.spent_usd() - before);
        reply(answer, "fable", FABLE_MODEL, usd)
    }
}
impl ResearchMember for PaperArgon {
    fn name(&self) -> &str {
        "argon"
    }
    fn call(&self, call: &MemberCall, stop: &AtomicBool) -> MemberReply {
        let path = match prepare(call) {
            Ok(path) => path,
            Err(e) => return reply(failed(e), "argon", ARGON_MODEL, None),
        };
        let answer = if call.prompt.encode_utf16().count() <= AGY_PROMPT_LIMIT {
            ask_argon(&self.command, ask(call, stop), &path)
        } else {
            ask_argon_stdin(&self.command, ask(call, stop), &path)
        };
        reply(answer, "argon", ARGON_MODEL, None)
    }
}
