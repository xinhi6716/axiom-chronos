use super::Provider;
use crate::{
    base::{self, Failure, Res},
    codex,
};
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub struct ModelCall {
    pub provider: Provider,
    pub model: String,
    pub prompt: String,
    pub timeout_s: u64,
    pub cwd: PathBuf,
}
#[derive(Clone, Debug, Default)]
pub struct ModelReply {
    pub text: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub usd: Option<f64>,
}
pub trait ModelBackend: Send + Sync {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply>;
}
pub trait ModelSlots: Send + Sync {
    fn acquire(&self, provider: Provider, model: &str, cancel: &AtomicBool) -> Res<Box<dyn Send>>;
}
pub fn cancelled(cancel: &AtomicBool) -> Res<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(Failure::new("cancelled", "module cancelled"))
    } else {
        Ok(())
    }
}
/// Use with backends that do not acquire the global Codex slot themselves.
pub struct GlobalSlots {
    pub home: PathBuf,
    pub codex_limit: u64,
}
/// Use with CodexBackend: it acquires the global slot in run_cancellable.
/// Workshop CLI wiring should pair this adapter with CodexBackend.
pub struct BackendHeldSlots;
impl ModelSlots for BackendHeldSlots {
    fn acquire(&self, _: Provider, _: &str, cancel: &AtomicBool) -> Res<Box<dyn Send>> {
        cancelled(cancel)?;
        Ok(Box::new(()))
    }
}
impl ModelSlots for GlobalSlots {
    fn acquire(&self, provider: Provider, model: &str, cancel: &AtomicBool) -> Res<Box<dyn Send>> {
        cancelled(cancel)?;
        if provider == Provider::Claude {
            return Ok(Box::new(()));
        }
        if self.codex_limit == 0 {
            return Err(Failure::new(
                "invalid_argument",
                "codex_limit must be positive",
            ));
        }
        let slot = codex::slots::acquire(
            &self.home,
            self.codex_limit,
            &mut Default::default(),
            || cancelled(cancel),
        )?;
        slot.model(model)?;
        Ok(Box::new(slot))
    }
}
#[derive(Clone, Debug)]
pub struct DecisionRequest {
    pub question: String,
    pub options: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct DecisionReply {
    pub choice: String,
    pub reason: String,
}
pub trait DecisionRouter: Send + Sync {
    fn route(&self, module: &str, request: &DecisionRequest) -> Res<DecisionReply>;
}
pub struct NoRouter;
impl DecisionRouter for NoRouter {
    fn route(&self, _: &str, _: &DecisionRequest) -> Res<DecisionReply> {
        Err(Failure::new("unavailable", "decision router unavailable"))
    }
}
#[derive(Default)]
pub struct FakeRouter;
static FAKE_ROUTER_CALLS: Mutex<Vec<(String, DecisionRequest)>> = Mutex::new(Vec::new());
impl FakeRouter {
    pub fn calls() -> Vec<(String, DecisionRequest)> {
        FAKE_ROUTER_CALLS.lock().unwrap().clone()
    }
}
impl DecisionRouter for FakeRouter {
    fn route(&self, module: &str, request: &DecisionRequest) -> Res<DecisionReply> {
        FAKE_ROUTER_CALLS
            .lock()
            .unwrap()
            .push((module.into(), request.clone()));
        Ok(DecisionReply {
            choice: request
                .options
                .first()
                .cloned()
                .ok_or_else(|| Failure::new("invalid_argument", "options must not be empty"))?,
            reason: "first option".into(),
        })
    }
}
pub trait NetworkService: Send + Sync {
    fn request(&self, module: &str, params: &Value) -> Res<Value>;
}
pub struct NoNetwork;
impl NetworkService for NoNetwork {
    fn request(&self, _: &str, _: &Value) -> Res<Value> {
        Err(Failure::new("unsupported", "network service unsupported"))
    }
}
#[derive(Default)]
pub struct FakeBackend {
    pub calls: Mutex<Vec<String>>,
    pub reply: Mutex<ModelReply>,
}
impl ModelBackend for FakeBackend {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        cancelled(cancel)?;
        self.calls.lock().unwrap().push(call.model.clone());
        Ok(self.reply.lock().unwrap().clone())
    }
}
#[derive(Default)]
pub struct FakeSlots {
    pub calls: Mutex<Vec<(Provider, String)>>,
}
impl ModelSlots for FakeSlots {
    fn acquire(&self, provider: Provider, model: &str, cancel: &AtomicBool) -> Res<Box<dyn Send>> {
        cancelled(cancel)?;
        self.calls.lock().unwrap().push((provider, model.into()));
        Ok(Box::new(()))
    }
}
pub struct CodexBackend {
    pub server: Arc<codex::AppServer>,
}
impl ModelBackend for CodexBackend {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        cancelled(cancel)?;
        let req = codex::RunRequest {
            brief: call.prompt.clone(),
            cwd: call.cwd.to_string_lossy().into_owned(),
            model: call.model.clone(),
            sandbox: "read-only".into(),
            effort: None,
            output_schema: None,
            developer_instructions: None,
            timeout_s: call.timeout_s,
            lean: true,
            keep_plugins: vec![],
            web_search: false,
        };
        let v = self.server.run_cancellable(&req, None, Some(cancel))?;
        cancelled(cancel)?;
        if v["status"] != "completed" {
            return Err(Failure::new(
                "failed",
                v["error"]["message"]
                    .as_str()
                    .unwrap_or("codex call failed"),
            ));
        }
        Ok(ModelReply {
            text: v["final_message"].as_str().unwrap_or("").into(),
            input_tokens: v["usage"]["inputTokens"].as_u64().unwrap_or(0),
            cached_input_tokens: v["usage"]["cachedInputTokens"].as_u64().unwrap_or(0),
            output_tokens: v["usage"]["outputTokens"].as_u64().unwrap_or(0),
            usd: None,
        })
    }
}
pub struct ClaudeBackend {
    pub command: Vec<String>,
}
impl ModelBackend for ClaudeBackend {
    fn call(&self, call: &ModelCall, cancel: &AtomicBool) -> Res<ModelReply> {
        if self.command.is_empty() {
            return Err(Failure::new("invalid_argument", "empty command"));
        }
        let mut argv = self.command.clone();
        argv.extend([
            "-p".into(),
            call.prompt.clone(),
            "--model".into(),
            call.model.clone(),
            "--output-format".into(),
            "json".into(),
        ]);
        let mut cmd = base::proc::command(&argv);
        cmd.current_dir(&call.cwd);
        let result = run_process(&mut cmd, call.timeout_s, cancel)?;
        if result.code != 0 {
            return Err(Failure::new("failed", result.stderr));
        }
        let v: Value = serde_json::from_str(&result.stdout)
            .map_err(|e| Failure::new("failed", e.to_string()))?;
        Ok(ModelReply {
            text: v["result"].as_str().unwrap_or("").into(),
            input_tokens: v["usage"]["input_tokens"].as_u64().unwrap_or(0),
            cached_input_tokens: v["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0),
            output_tokens: v["usage"]["output_tokens"].as_u64().unwrap_or(0),
            usd: v["total_cost_usd"].as_f64(),
        })
    }
}
pub struct Services {
    pub engine_home: PathBuf,
    pub slots: Arc<dyn ModelSlots>,
    pub claude: Arc<dyn ModelBackend>,
    pub codex: Arc<dyn ModelBackend>,
    pub router: Arc<dyn DecisionRouter>,
    pub network: Arc<dyn NetworkService>,
}

pub(crate) struct ProcessOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub(crate) fn run_process(
    cmd: &mut std::process::Command,
    timeout: u64,
    cancel: &AtomicBool,
) -> Res<ProcessOutput> {
    cancelled(cancel)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        base::proc::spawn_group(cmd).map_err(|e| Failure::new("failed", e.to_string()))?;
    fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            while let Ok(n) = stream.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let keep = n.min(65536usize.saturating_sub(out.len()));
                out.extend_from_slice(&buf[..keep]);
            }
            out
        })
    }
    let out = drain(child.stdout.take().unwrap());
    let err = drain(child.stderr.take().unwrap());
    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::SeqCst) || start.elapsed() >= Duration::from_secs(timeout) {
            base::proc::kill_tree(&mut child);
            return if cancel.load(Ordering::SeqCst) {
                Err(Failure::new("cancelled", "module cancelled"))
            } else {
                Err(Failure::new("failed", "process timed out"))
            };
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                base::proc::kill_tree(&mut child);
                return Err(Failure::new("failed", e.to_string()));
            }
        }
    };
    // Descendants can retain the pipes after the parent exits; never wait indefinitely for them.
    let deadline = Instant::now() + Duration::from_millis(200);
    while (!out.is_finished() || !err.is_finished()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    if !out.is_finished() || !err.is_finished() {
        let _ = base::proc::kill_pid_tree(child.id());
    }
    let bytes = |h: std::thread::JoinHandle<Vec<u8>>| {
        if h.is_finished() {
            h.join().unwrap_or_default()
        } else {
            Vec::new()
        }
    };
    let bounded_text = |bytes: Vec<u8>| {
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() > 65536 {
            let mut end = 65536;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        text
    };
    Ok(ProcessOutput {
        code: status.code().unwrap_or(-1),
        stdout: bounded_text(bytes(out)),
        stderr: bounded_text(bytes(err)),
    })
}
