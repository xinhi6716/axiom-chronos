use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionKey {
    pub generation: u64,
    pub thread_id: String,
    pub turn_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(Clone, Debug)]
pub struct ToolCall {
    pub execution: ExecutionKey,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Debug)]
pub struct ToolReply {
    pub success: bool,
    pub content_items: Vec<Value>,
}
pub trait ToolHandler: Send + Sync {
    fn handle(&self, call: ToolCall) -> Res<ToolReply>;
}
#[derive(Clone, Debug)]
pub enum ThreadMode {
    FreshPersistent,
    Resume { thread_id: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportTerminal {
    Completed,
    Failed,
    Cancelled,
    OutcomeUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReport {
    pub execution: ExecutionKey,
    pub terminal: TransportTerminal,
    pub requested_effort: Option<String>,
    pub confirmed_effort: Option<String>,
    pub report: Value,
}
#[derive(Clone, Debug)]
pub enum RunEventKind {
    BeforeTurn,
    TurnStarted,
    Terminal { report: RunReport },
    ConnectionLost { reason: String },
}
#[derive(Clone, Debug)]
pub struct RunEvent {
    pub attempt_id: String,
    pub sequence: u64,
    pub execution: ExecutionKey,
    pub kind: RunEventKind,
}
pub trait RunObserver: Send + Sync {
    fn on_event(&self, event: &RunEvent) -> Res<()>;
}
pub struct RunOptions {
    pub attempt_id: String,
    pub thread: ThreadMode,
    pub tools: Vec<DynamicTool>,
    pub handler: Option<Arc<dyn ToolHandler>>,
    pub observer: Option<Arc<dyn RunObserver>>,
    pub cancel: Arc<AtomicBool>,
    pub cancel_grace_ms: u64,
    pub skill_paths: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCapability {
    pub id: String,
    pub supported_efforts: Vec<String>,
}
#[derive(Clone, Debug)]
pub enum AttemptRead {
    Terminal(RunReport),
    Running { execution: ExecutionKey },
    Unknown { reason: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub read_only_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub timeout_s: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub policy_applied: bool,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub duration_ms: Option<u64>,
}

impl super::AppServer {
    pub fn run_with_options(&self, req: &super::RunRequest, options: RunOptions) -> Res<RunReport> {
        use std::{
            collections::HashSet,
            sync::{atomic::Ordering, mpsc},
            time::{Duration, Instant},
        };
        crate::base::require_directory(&req.cwd)?;
        if !super::SANDBOXES.contains(&req.sandbox.as_str())
            || req.brief.trim().is_empty()
            || req.model.trim().is_empty()
            || options.attempt_id.trim().is_empty()
            || req.timeout_s == 0
        {
            return Err(Failure::new("invalid_argument", "invalid run request"));
        }
        if !options.tools.is_empty() && options.handler.is_none() {
            return Err(Failure::new("invalid_argument", "tools require a handler"));
        }
        let mut names = HashSet::new();
        if options
            .tools
            .iter()
            .any(|t| t.name.trim().is_empty() || !names.insert(t.name.clone()))
        {
            return Err(Failure::new(
                "invalid_argument",
                "invalid dynamic tool names",
            ));
        }
        if let Some(effort) = req.effort.as_ref().filter(|v| !v.is_empty()) {
            let caps = self.model_capabilities()?;
            if !caps
                .iter()
                .any(|c| c.id == req.model && c.supported_efforts.contains(effort))
            {
                return Err(Failure::new(
                    "unsupported_effort",
                    "requested model effort is not advertised",
                ));
            }
        }
        let mut input = vec![json!({"type":"text","text":req.brief})];
        for path in &options.skill_paths {
            let path = std::fs::canonicalize(path)
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            let name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .ok_or_else(|| Failure::new("invalid_argument", "invalid skill path"))?;
            input.push(json!({"type":"skill","name":name,"path":path}));
        }
        let tools: Vec<Value> = options
            .tools
            .iter()
            .map(
                |t| json!({"name":t.name,"description":t.description,"inputSchema":t.input_schema}),
            )
            .collect();
        let mut thread_params = json!({"model":req.model,"ephemeral":false,"sandbox":req.sandbox,"approvalPolicy":"never","cwd":req.cwd,"dynamicTools":tools});
        if let Some(instructions) = &req.developer_instructions {
            thread_params["developerInstructions"] = json!(instructions);
        }
        let disabled = self.lean_policy(req, &mut thread_params)?;
        if req.lean {
            thread_params["config"]["features"]["multi_agent"] = json!(false);
        }
        let started = match &options.thread {
            ThreadMode::FreshPersistent => self.request("thread/start", thread_params)?,
            ThreadMode::Resume { thread_id } if !thread_id.trim().is_empty() => {
                thread_params["threadId"] = json!(thread_id);
                self.request("thread/resume", thread_params)?
            }
            _ => return Err(Failure::new("invalid_argument", "empty thread id")),
        };
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Failure::new("codex_rpc", "missing thread id"))?
            .to_owned();
        let events = self.subscribe(&thread_id);
        struct Guard<'a> {
            server: &'a super::AppServer,
            thread: String,
            turn: Option<String>,
            generation: u64,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if self.server.generation() != self.generation {
                    return;
                }
                if let Some(t) = &self.turn {
                    self.server.unregister_turn(&self.thread, t);
                }
                self.server.unsubscribe(&self.thread);
            }
        }
        let mut guard = Guard {
            server: self,
            thread: thread_id.clone(),
            turn: None,
            generation: self.generation(),
        };
        let mut key = ExecutionKey {
            generation: self.generation(),
            thread_id: thread_id.clone(),
            turn_id: None,
        };
        let mut sequence = 0u64;
        let notify = |sequence: &mut u64, key: &ExecutionKey, kind| -> Res<()> {
            *sequence += 1;
            if let Some(observer) = &options.observer {
                observer.on_event(&RunEvent {
                    attempt_id: options.attempt_id.clone(),
                    sequence: *sequence,
                    execution: key.clone(),
                    kind,
                })?;
            }
            Ok(())
        };
        notify(&mut sequence, &key, RunEventKind::BeforeTurn)?;
        if options.cancel.load(Ordering::Acquire) {
            return Err(Failure::new("cancelled", "cancelled before turn dispatch"));
        }
        let mut turn = json!({"threadId":thread_id,"input":input});
        if let Some(e) = &req.effort {
            turn["effort"] = json!(e);
        }
        if let Some(schema) = &req.output_schema {
            turn["outputSchema"] = schema.clone();
        }
        if !disabled.is_null() {
            turn["disabledPluginIds"] = disabled["plugins"].clone();
        }
        let deadline = Instant::now() + Duration::from_secs(req.timeout_s);
        let response = match self.request_cancellable(
            "turn/start",
            turn,
            &options.cancel,
            deadline,
            options.cancel_grace_ms,
        ) {
            Ok(response) => response,
            Err(e) if e.kind == "outcome_unknown" || e.kind == "codex_exited" => {
                notify(
                    &mut sequence,
                    &key,
                    RunEventKind::ConnectionLost {
                        reason: e.message.clone(),
                    },
                )?;
                return Ok(RunReport {
                    execution: key,
                    terminal: TransportTerminal::OutcomeUnknown,
                    requested_effort: req.effort.clone(),
                    confirmed_effort: None,
                    report: json!({"status":"outcome_unknown","error":{"kind":e.kind,"message":e.message}}),
                });
            }
            Err(e) => return Err(e),
        };
        let turn_id = response
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Failure::new("codex_rpc", "missing turn id"))?
            .to_owned();
        key.turn_id = Some(turn_id.clone());
        self.register_turn(&thread_id, &turn_id);
        guard.turn = Some(turn_id.clone());
        let mut observer_error = notify(&mut sequence, &key, RunEventKind::TurnStarted).err();
        let (done_tx, done_rx) = mpsc::channel();
        let responder = self
            .responder()
            .ok_or_else(|| Failure::new("codex_exited", "connection lost"))?;
        let mut calls = HashSet::new();
        let mut active = 0usize;
        let mut cancel_deadline = None;
        let mut terminal: Option<TransportTerminal> = None;
        let mut handler_failed = false;
        let mut report = json!({"status":null,"thread_id":thread_id,"turn_id":turn_id,"model":req.model,"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"refused_requests":[],"usage":null,"disabled":disabled,"started_mcp_servers":[],"error":null});
        loop {
            while let Ok(failed) = done_rx.try_recv() {
                active = active.saturating_sub(1);
                handler_failed |= failed;
            }
            // The reader delivers requests and the terminal in wire order. Only
            // callbacks received before that terminal belong to this execution.
            if terminal.is_some() && active == 0 {
                break;
            }
            if cancel_deadline.is_none()
                && (options.cancel.load(Ordering::Acquire)
                    || observer_error.is_some()
                    || Instant::now() >= deadline)
            {
                self.interrupt_nowait(&thread_id, &turn_id);
                cancel_deadline =
                    Some(Instant::now() + Duration::from_millis(options.cancel_grace_ms));
            }
            if cancel_deadline.is_some_and(|d| Instant::now() >= d) {
                terminal = Some(TransportTerminal::OutcomeUnknown);
                break;
            }
            match events.recv_timeout(Duration::from_millis(20)) {
                Ok(event) => {
                    let method = event["method"].as_str().unwrap_or("");
                    let params = &event["params"];
                    if method == "item/tool/call" && event.get("id").is_some() {
                        let id = event["id"].clone();
                        let cid = params["callId"].as_str().unwrap_or("");
                        let name = params["tool"].as_str().unwrap_or("");
                        let valid = params["threadId"] == thread_id
                            && params["turnId"] == turn_id
                            && !cid.is_empty()
                            && names.contains(name)
                            && calls.insert(cid.to_owned())
                            && terminal.is_none()
                            && cancel_deadline.is_none();
                        if !valid {
                            responder(
                                json!({"id":id,"error":{"code":-32602,"message":"invalid or duplicate tool call"}}),
                            );
                            continue;
                        }
                        let call = ToolCall {
                            execution: key.clone(),
                            call_id: cid.into(),
                            name: name.into(),
                            arguments: params["arguments"].clone(),
                        };
                        let handler = options.handler.as_ref().unwrap().clone();
                        let reply = responder.clone();
                        let done = done_tx.clone();
                        active += 1;
                        std::thread::spawn(move || {
                            let result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    handler.handle(call)
                                }));
                            let failed = !matches!(&result, Ok(Ok(r)) if r.success);
                            let response = match result {
                                Ok(Ok(r)) => {
                                    json!({"id":id,"result":{"success":r.success,"contentItems":r.content_items}})
                                }
                                Ok(Err(e)) => {
                                    json!({"id":id,"error":{"code":-32603,"message":e.message}})
                                }
                                Err(_) => {
                                    json!({"id":id,"error":{"code":-32603,"message":"handler panicked"}})
                                }
                            };
                            reply(response);
                            let _ = done.send(failed);
                        });
                    } else if event.get("id").is_some() {
                        report["refused_requests"]
                            .as_array_mut()
                            .unwrap()
                            .push(json!(method));
                    } else {
                        if params
                            .get("turnId")
                            .and_then(Value::as_str)
                            .is_some_and(|id| id != turn_id)
                        {
                            continue;
                        }
                        match method {
                            "mcpServer/startupStatus/updated" if params["status"] == "ready" => {
                                report["started_mcp_servers"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(params["name"].clone());
                            }
                            "error" if params["willRetry"] != true => {
                                report["error"] = super::run::turn_error(&params["error"]);
                            }
                            "thread/tokenUsage/updated" => {
                                report["usage"] = params["tokenUsage"]["total"].clone()
                            }
                            "item/completed" => {
                                super::run::record_item(&mut report, &params["item"])
                            }
                            "turn/completed" if params["turn"]["id"] == turn_id => {
                                let status = params["turn"]["status"].as_str().unwrap_or("");
                                report["status"] = json!(status);
                                if !params["turn"]["error"].is_null() {
                                    report["error"] =
                                        super::run::turn_error(&params["turn"]["error"]);
                                }
                                terminal = Some(if handler_failed {
                                    TransportTerminal::Failed
                                } else {
                                    match status {
                                        "interrupted" | "cancelled" => TransportTerminal::Cancelled,
                                        "completed" if cancel_deadline.is_none() => {
                                            TransportTerminal::Completed
                                        }
                                        "failed" if cancel_deadline.is_none() => {
                                            TransportTerminal::Failed
                                        }
                                        _ => TransportTerminal::OutcomeUnknown,
                                    }
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Err(e) = notify(
                        &mut sequence,
                        &key,
                        RunEventKind::ConnectionLost {
                            reason: "app-server connection lost".into(),
                        },
                    ) {
                        observer_error.get_or_insert(e);
                    }
                    terminal = Some(TransportTerminal::OutcomeUnknown);
                    break;
                }
            }
        }
        if handler_failed
            && matches!(
                terminal,
                Some(TransportTerminal::Completed | TransportTerminal::Failed)
            )
        {
            terminal = Some(TransportTerminal::Failed);
        }
        if matches!(terminal, Some(TransportTerminal::Completed)) {
            report["error"] = Value::Null;
            if let Some(schema) = &req.output_schema {
                let parsed = report["final_message"]
                    .as_str()
                    .ok_or_else(|| "final message is missing".to_string())
                    .and_then(|s| {
                        serde_json::from_str::<Value>(s)
                            .map_err(|e| format!("final message is not JSON: {e}"))
                    })
                    .and_then(|v| {
                        crate::base::schema::validate(schema, &v)
                            .map(|_| v)
                            .map_err(|e| format!("schema validation failed: {e}"))
                    });
                match parsed {
                    Ok(v) => {
                        report["structured_output"] = v;
                        report["final_message"] = Value::Null;
                    }
                    Err(message) => {
                        report["status"] = json!("failed");
                        report["error"] =
                            json!({"kind":"invalid_structured_output","message":message});
                        terminal = Some(TransportTerminal::Failed);
                    }
                }
            }
        } else if handler_failed {
            report["status"] = json!("failed");
            report["error"] =
                json!({"kind":"tool_handler_failed","message":"dynamic tool handler failed"});
        }
        let result = RunReport {
            execution: key.clone(),
            terminal: terminal.unwrap_or(TransportTerminal::OutcomeUnknown),
            requested_effort: req.effort.clone(),
            confirmed_effort: None,
            report,
        };
        if let Some(e) = observer_error.take() {
            return Err(e);
        }
        notify(
            &mut sequence,
            &key,
            RunEventKind::Terminal {
                report: result.clone(),
            },
        )?;
        Ok(result)
    }
    pub fn read_attempt(&self, thread_id: &str, turn_id: Option<&str>) -> Res<AttemptRead> {
        if thread_id.trim().is_empty() || turn_id.is_some_and(|id| id.trim().is_empty()) {
            return Err(Failure::new("invalid_argument", "empty thread or turn id"));
        }
        let history = self.request(
            "thread/read",
            json!({"threadId":thread_id,"includeTurns":true}),
        )?;
        let turns = history.pointer("/thread/turns").and_then(Value::as_array);
        let turn = turns.and_then(|turns| match turn_id {
            Some(id) => turns.iter().find(|t| t["id"] == id),
            None => turns.last(),
        });
        let Some(turn) = turn else {
            return Ok(AttemptRead::Unknown {
                reason: "turn absent from history".into(),
            });
        };
        let Some(id) = turn["id"].as_str().filter(|s| !s.is_empty()) else {
            return Ok(AttemptRead::Unknown {
                reason: "history has no turn id".into(),
            });
        };
        let execution = ExecutionKey {
            generation: self.generation(),
            thread_id: thread_id.into(),
            turn_id: Some(id.into()),
        };
        let terminal = match turn["status"].as_str() {
            Some("completed") => TransportTerminal::Completed,
            Some("failed") => TransportTerminal::Failed,
            Some("interrupted") | Some("cancelled") => TransportTerminal::Cancelled,
            Some("inProgress") | Some("in_progress") => {
                return Ok(AttemptRead::Running { execution })
            }
            _ => {
                return Ok(AttemptRead::Unknown {
                    reason: "unrecognized history status".into(),
                })
            }
        };
        let mut report = json!({"status":turn["status"],"thread_id":thread_id,"turn_id":id,"model":history.pointer("/thread/model").cloned().unwrap_or(Value::Null),"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"error":turn["error"]});
        for item in turn["items"].as_array().into_iter().flatten() {
            super::run::record_item(&mut report, item);
        }
        Ok(AttemptRead::Terminal(RunReport {
            execution,
            terminal,
            requested_effort: None,
            confirmed_effort: None,
            report,
        }))
    }
    pub fn interrupt_attempt(&self, key: &ExecutionKey, grace_ms: u64) -> Res<AttemptRead> {
        use std::time::{Duration, Instant};
        let Some(turn_id) = key.turn_id.as_deref().filter(|id| !id.trim().is_empty()) else {
            return Err(Failure::new("invalid_argument", "interrupt needs turn id"));
        };
        if !self.alive() || self.generation() != key.generation {
            return Ok(AttemptRead::Unknown {
                reason: "execution connection is not live".into(),
            });
        }
        self.interrupt_nowait(&key.thread_id, turn_id);
        let deadline = Instant::now() + Duration::from_millis(grace_ms);
        while Instant::now() < deadline && self.alive() && self.generation() == key.generation {
            let cancel = AtomicBool::new(false);
            let history = self.request_cancellable(
                "thread/read",
                json!({"threadId":key.thread_id,"includeTurns":true}),
                &cancel,
                deadline,
                0,
            );
            if let Ok(value) = history {
                if let Some(turn) = value
                    .pointer("/thread/turns")
                    .and_then(Value::as_array)
                    .and_then(|v| v.iter().find(|t| t["id"] == turn_id))
                {
                    let terminal = match turn["status"].as_str() {
                        Some("completed") => Some(TransportTerminal::Completed),
                        Some("failed") => Some(TransportTerminal::Failed),
                        Some("interrupted" | "cancelled") => Some(TransportTerminal::Cancelled),
                        _ => None,
                    };
                    if let Some(terminal) = terminal {
                        return Ok(AttemptRead::Terminal(RunReport {
                            execution: key.clone(),
                            terminal,
                            requested_effort: None,
                            confirmed_effort: None,
                            report: turn.clone(),
                        }));
                    }
                }
            }
            std::thread::sleep(
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
        Ok(AttemptRead::Unknown {
            reason: "interrupt sent; terminal outcome not observed within grace".into(),
        })
    }
    pub fn exec_check(&self, spec: &CommandSpec) -> Res<CommandResult> {
        use std::time::Instant;
        if spec.program.trim().is_empty() || spec.timeout_s == 0 || spec.writable_roots.len() != 1 {
            return Err(Failure::new(
                "blocked_capability",
                "command requires exactly one scratch writable root and a finite timeout",
            ));
        }
        let canon = |p: &std::path::Path| {
            std::fs::canonicalize(p).map_err(|e| Failure::new("invalid_argument", e.to_string()))
        };
        let scratch = canon(&spec.writable_roots[0])?;
        let cwd = canon(&spec.cwd)?;
        let output = canon(&spec.output_dir)?;
        if !cwd.starts_with(&scratch)
            || !output.starts_with(&scratch)
            || spec.read_only_roots.iter().any(|p| {
                canon(p).map_or(true, |r| r.starts_with(&scratch) || scratch.starts_with(r))
            })
        {
            return Err(Failure::new(
                "blocked_capability",
                "scratch, output and read-only roots overlap or cwd is outside scratch",
            ));
        }
        let server = self.check_server(scratch.clone(), spec.timeout_s);
        let mut command = vec![spec.program.clone()];
        command.extend(spec.args.iter().cloned());
        let start = Instant::now();
        let result = server.request("command/exec", json!({"command":command,"cwd":cwd,"env":spec.env,"sandboxPolicy":{"type":"workspaceWrite","writableRoots":[scratch],"networkAccess":false,"excludeTmpdirEnvVar":true,"excludeSlashTmp":true},"timeoutMs":spec.timeout_s.saturating_mul(1000)}))?;
        // Stop the dedicated runner tree before the host persists command logs.
        // Do not follow output symlinks planted by a scratch-writing test.
        server.close();
        if canon(&spec.output_dir)? != output {
            return Err(Failure::new(
                "blocked_capability",
                "command changed the output directory boundary",
            ));
        }
        let stdout = result["stdout"]
            .as_str()
            .ok_or_else(|| Failure::new("blocked_capability", "command/exec missing stdout"))?;
        let stderr = result["stderr"]
            .as_str()
            .ok_or_else(|| Failure::new("blocked_capability", "command/exec missing stderr"))?;
        if result["exitCode"].as_i64().is_none() && result["timedOut"] != true {
            return Err(Failure::new(
                "blocked_capability",
                "command/exec missing terminal status",
            ));
        }
        let stdout_path = output.join("stdout.txt");
        let stderr_path = output.join("stderr.txt");
        crate::base::atomic_write(&stdout_path, stdout.as_bytes())
            .map_err(|e| Failure::new("io_error", e.to_string()))?;
        crate::base::atomic_write(&stderr_path, stderr.as_bytes())
            .map_err(|e| Failure::new("io_error", e.to_string()))?;
        Ok(CommandResult {
            exit_code: result["exitCode"]
                .as_i64()
                .and_then(|x| i32::try_from(x).ok()),
            timed_out: result["timedOut"].as_bool().unwrap_or(false),
            policy_applied: true,
            stdout_path,
            stderr_path,
            duration_ms: Some(start.elapsed().as_millis().min(u64::MAX as u128) as u64),
        })
    }
    pub fn model_capabilities(&self) -> Res<Vec<ModelCapability>> {
        let mut result = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = std::collections::HashSet::new();
        loop {
            let page = self.request(
                "model/list",
                cursor
                    .as_ref()
                    .map_or_else(|| json!({}), |c| json!({"cursor":c})),
            )?;
            let data = page["data"]
                .as_array()
                .ok_or_else(|| Failure::new("codex_rpc", "model/list: missing data"))?;
            for model in data {
                if let Some(id) = model["id"]
                    .as_str()
                    .or_else(|| model["model"].as_str())
                    .filter(|s| !s.is_empty())
                {
                    let supported_efforts = model["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e["reasoningEffort"].as_str().or_else(|| e.as_str()))
                        .map(str::to_owned)
                        .collect();
                    result.push(ModelCapability {
                        id: id.into(),
                        supported_efforts,
                    });
                }
            }
            cursor = page["nextCursor"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            match &cursor {
                Some(c) if seen.insert(c.clone()) => {}
                Some(_) => return Err(Failure::new("codex_rpc", "model/list: repeated cursor")),
                None => break,
            }
        }
        Ok(result)
    }
}
