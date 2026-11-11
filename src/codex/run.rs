use super::{slots, AppServer, SANDBOXES};
use crate::{
    base::{self, Failure, Res},
    codegraph,
};
use serde_json::{json, Map, Value};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub struct RunRequest {
    pub brief: String,
    pub cwd: String,
    pub model: String,
    pub sandbox: String,
    pub effort: Option<String>,
    pub output_schema: Option<Value>,
    pub developer_instructions: Option<String>,
    pub timeout_s: u64,
    pub lean: bool,
    pub keep_plugins: Vec<String>,
    pub web_search: bool,
}

impl AppServer {
    pub fn run(&self, req: &RunRequest) -> Res<Value> {
        self.run_capped(req, None)
    }
    /// A run that is interrupted, with status `token_cap`, once its total tokens reach the cap.
    pub fn run_capped(&self, req: &RunRequest, token_cap: Option<u64>) -> Res<Value> {
        self.run_cancellable(req, token_cap, None)
    }
    pub fn run_cancellable(
        &self,
        req: &RunRequest,
        token_cap: Option<u64>,
        stop: Option<&AtomicBool>,
    ) -> Res<Value> {
        base::require_directory(&req.cwd)?;
        if !SANDBOXES.contains(&req.sandbox.as_str())
            || req.brief.trim().is_empty()
            || req.model.trim().is_empty()
        {
            return Err(Failure::new(
                "invalid_argument",
                "invalid sandbox, empty brief, or empty model",
            ));
        }
        let limit = slots::from_env()?;
        let cancellation = self.cancellation.load(Ordering::Acquire);
        let cancelled = || {
            self.cancellation.load(Ordering::Acquire) != cancellation
                || stop.is_some_and(|s| s.load(Ordering::Acquire))
        };
        let deadline = Instant::now().checked_add(Duration::from_secs(req.timeout_s));
        let mut wait = slots::Wait::default();
        let stopped = || {
            if cancelled() {
                Err(Failure::new(
                    "interrupted",
                    "cancelled while waiting for a Codex slot",
                ))
            } else if deadline.is_some_and(|d| Instant::now() >= d) {
                Err(Failure::new(
                    "timeout",
                    "timed out waiting for a Codex slot",
                ))
            } else {
                Ok(())
            }
        };
        let slot = slots::acquire(&self.slots_home, limit, &mut wait, || stopped());
        let _slot = match slot {
            Ok(slot) => slot,
            Err(e) if matches!(e.kind.as_str(), "interrupted" | "timeout") => {
                let mut report =
                    report_error(req, "", "", Value::Null, &e.kind, &e.kind, &e.message);
                slot_wait(&mut report, &wait, limit);
                return Ok(report);
            }
            Err(e) => return Err(e),
        };
        _slot.model(&req.model)?;
        let with_wait = |mut e: Failure| {
            if let Some(note) = wait.note(limit) {
                e.message.push_str(&format!("; {note}"));
            }
            e
        };
        let mut thread = json!({"model":req.model,"ephemeral":true,"sandbox":req.sandbox,"approvalPolicy":"never","cwd":req.cwd});
        if let Some(s) = req
            .developer_instructions
            .as_ref()
            .filter(|s| !s.is_empty())
        {
            thread["developerInstructions"] = json!(s);
        }
        let graph = codegraph::enabled(Path::new(&req.cwd));
        let disabled = if req.lean {
            let installed = self
                .request("plugin/installed", json!({"cwds":[req.cwd]}))
                .map_err(&with_wait)?;
            let mut plugins: Vec<String> = installed["marketplaces"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|m| m["plugins"].as_array().into_iter().flatten())
                .filter(|p| p["enabled"] == true)
                .filter_map(|p| p["id"].as_str())
                .filter(|id| !req.keep_plugins.iter().any(|k| k == id))
                .map(str::to_owned)
                .collect();
            plugins.sort();
            let config = self
                .request("config/read", json!({"cwd":req.cwd}))
                .map_err(&with_wait)?;
            let mut servers: Vec<String> = config["config"]["mcp_servers"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().cloned())
                .filter(|name| !graph || name != codegraph::NAME)
                .collect();
            servers.sort();
            let plugin_config: Map<String, Value> = plugins
                .iter()
                .map(|p| (p.clone(), json!({"enabled":false})))
                .collect();
            let server_config: Map<String, Value> = servers
                .iter()
                .map(|p| (p.clone(), json!({"enabled":false})))
                .collect();
            let mut config = json!({"features":{"apps":false,"goals":false},"plugins":plugin_config,"mcp_servers":server_config,"skills":{"max_context_tokens":1},"personality":"none"});
            let mut sections = vec!["apps", "goals", "skills_catalog", "personality"];
            if !req.web_search {
                config["web_search"] = json!("disabled");
                sections.push("web_search");
            }
            thread["config"] = config;
            json!({"sections":sections,"plugins":plugins,"mcp_servers":servers})
        } else {
            Value::Null
        };
        if graph {
            thread["config"]["mcp_servers"][codegraph::NAME] = codegraph::codex_server(&req.cwd);
        }
        if let Err(e) = stopped() {
            let mut report = report_error(req, "", "", disabled, &e.kind, &e.kind, &e.message);
            slot_wait(&mut report, &wait, limit);
            return Ok(report);
        }
        let started = self.request("thread/start", thread).map_err(&with_wait)?;
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| with_wait(self.protocol_failure("thread/start: missing thread id")))?
            .to_owned();
        let events = self.subscribe(&thread_id);
        let mut turn = json!({"threadId":thread_id,"input":[{"type":"text","text":req.brief}]});
        if let Some(s) = req.effort.as_ref().filter(|s| !s.is_empty()) {
            turn["effort"] = json!(s);
        }
        if let Some(schema) = &req.output_schema {
            turn["outputSchema"] = schema.clone();
        }
        if !disabled.is_null() {
            turn["disabledPluginIds"] = disabled["plugins"].clone();
        }
        if let Err(e) = stopped() {
            self.unsubscribe(&thread_id);
            let mut report =
                report_error(req, &thread_id, "", disabled, &e.kind, &e.kind, &e.message);
            slot_wait(&mut report, &wait, limit);
            return Ok(report);
        }
        let result = self.request("turn/start", turn).map_err(&with_wait);
        let started = match result {
            Ok(v) => v,
            Err(e) => {
                self.unsubscribe(&thread_id);
                self.close();
                return if e.kind == "codex_exited" {
                    let mut report = report_error(
                        req,
                        &thread_id,
                        "",
                        disabled,
                        "bridge_error",
                        &e.kind,
                        &e.message,
                    );
                    slot_wait(&mut report, &wait, limit);
                    Ok(report)
                } else {
                    Err(e)
                };
            }
        };
        let Some(turn_id) = started.pointer("/turn/id").and_then(Value::as_str) else {
            self.unsubscribe(&thread_id);
            self.close();
            return Err(with_wait(
                self.protocol_failure("turn/start: missing turn id"),
            ));
        };
        self.register_turn(&thread_id, turn_id);
        let mut report = json!({"status":null,"thread_id":thread_id,"turn_id":turn_id,"model":req.model,"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"refused_requests":[],"usage":null,"disabled":disabled,"started_mcp_servers":[],"error":null});
        slot_wait(&mut report, &wait, limit);
        let mut ending: Option<Instant> = None;
        loop {
            let remaining = deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::MAX);
            if ending.is_none() && (remaining.is_zero() || cancelled()) {
                self.interrupt(&thread_id, turn_id);
                let status = if cancelled() {
                    "interrupted"
                } else {
                    "timeout"
                };
                report["status"] = json!(status);
                let message = if cancelled() {
                    "turn was cancelled".into()
                } else {
                    format!("turn exceeded {}s and was interrupted", req.timeout_s)
                };
                report["error"] =
                    json!({"kind":status,"message":self.note_untested_version(&message)});
                ending = Some(Instant::now());
            }
            // Keep the slot until completion; an unresponsive interrupted server must die first.
            if ending.is_some_and(|at| at.elapsed() >= Duration::from_secs(30)) {
                self.close();
                break;
            }
            let event = match events.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => event,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    report["status"] = json!("bridge_error");
                    report["error"] =
                        json!({"kind":"codex_exited","message":"codex app-server exited mid-turn"});
                    break;
                }
            };
            let method = event["method"].as_str().unwrap_or("");
            let params = &event["params"];
            if event.get("id").is_some() {
                report["refused_requests"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(method));
                continue;
            }
            if method == "mcpServer/startupStatus/updated" && params["status"] == "ready" {
                report["started_mcp_servers"]
                    .as_array_mut()
                    .unwrap()
                    .push(params["name"].clone());
                continue;
            }
            if params
                .get("turnId")
                .and_then(Value::as_str)
                .is_some_and(|id| id != turn_id)
            {
                continue;
            }
            if method != "thread/started"
                && method != "mcpServer/startupStatus/updated"
                && (method != "turn/completed" || params["turn"]["id"] == turn_id)
            {
                if let Some(progress) = base::progress::current() {
                    progress.emit(method);
                }
            }
            match method {
                "thread/tokenUsage/updated" => {
                    report["usage"] = params["tokenUsage"]["total"].clone();
                    let total = report["usage"]["totalTokens"].as_u64().unwrap_or(0);
                    if ending.is_none() && token_cap.is_some_and(|cap| total >= cap) {
                        self.interrupt(&thread_id, turn_id);
                        report["status"] = json!("token_cap");
                        report["error"] = json!({"kind":"token_cap","message":format!("turn reached {total} tokens and was interrupted")});
                        ending = Some(Instant::now());
                    }
                }
                "item/completed" => record_item(&mut report, &params["item"]),
                "error" if params["willRetry"] != true => {
                    report["error"] = turn_error(&params["error"])
                }
                "turn/completed" if params["turn"]["id"] == turn_id => {
                    if ending.is_some() {
                        break;
                    }
                    report["status"] = params["turn"]["status"].clone();
                    if !params["turn"]["error"].is_null() {
                        report["error"] = turn_error(&params["turn"]["error"]);
                    }
                    if !matches!(
                        report["status"].as_str(),
                        Some("completed" | "failed" | "interrupted")
                    ) {
                        let status = report["status"].take();
                        report["status"] = json!("failed");
                        if report["error"].is_null() {
                            report["error"] = self
                                .protocol_failure(&format!(
                                    "turn/completed: unexpected status {status}"
                                ))
                                .to_json();
                        }
                    }
                    if report["status"] == "completed" {
                        report["error"] = Value::Null;
                        if let Some(schema) = &req.output_schema {
                            let parsed = report["final_message"]
                                .as_str()
                                .ok_or_else(|| "final message is missing".to_owned())
                                .and_then(|s| {
                                    serde_json::from_str::<Value>(s)
                                        .map_err(|e| format!("final message is not JSON: {e}"))
                                })
                                .and_then(|v| {
                                    base::schema::validate(schema, &v)
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
                                    report["error"] = json!({"kind":"invalid_structured_output","message":message});
                                }
                            }
                        }
                    } else if report["error"].is_null() {
                        report["error"] = json!({"kind":"codex_error","message":format!("turn {}", report["status"].as_str().unwrap_or("unknown"))});
                    }
                    break;
                }
                _ => {}
            }
        }
        self.unregister_turn(&thread_id, turn_id);
        self.unsubscribe(&thread_id);
        Ok(report)
    }
}

fn slot_wait(report: &mut Value, wait: &slots::Wait, limit: u64) {
    report["slot_wait_ms"] = json!(wait.elapsed.as_millis().min(u64::MAX as u128) as u64);
    report["slot_wait"] = json!(wait.note(limit));
}

fn report_error(
    req: &RunRequest,
    thread: &str,
    turn: &str,
    disabled: Value,
    status: &str,
    kind: &str,
    message: &str,
) -> Value {
    json!({"status":status,"thread_id":thread,"turn_id":turn,"model":req.model,"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"refused_requests":[],"usage":null,"disabled":disabled,"started_mcp_servers":[],"error":{"kind":kind,"message":message}})
}

fn turn_error(error: &Value) -> Value {
    let info = &error["codexErrorInfo"];
    let mut kind = info
        .as_str()
        .or_else(|| {
            info.as_object()
                .and_then(|o| o.keys().next().map(String::as_str))
        })
        .unwrap_or("codex_error")
        .to_owned();
    let message = error["message"].as_str().unwrap_or("");
    if kind == "other" {
        if let Ok(v) = serde_json::from_str::<Value>(message) {
            if let Some(code) = v
                .pointer("/error/code")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                kind = code.to_owned();
            }
        }
    }
    let lower = kind.to_lowercase();
    let mut result = json!({"kind":kind,"message":message});
    if lower.contains("usagelimit") || lower.contains("auth") {
        result["user_action"] = json!(true);
    }
    result
}

fn record_item(report: &mut Value, item: &Value) {
    match item["type"].as_str() {
        Some("agentMessage") => report["final_message"] = item["text"].clone(),
        Some("commandExecution") => report["commands"].as_array_mut().unwrap().push(
            json!({"command":item["command"],"status":item["status"],"exit_code":item["exitCode"]}),
        ),
        Some("fileChange") => {
            for change in item["changes"].as_array().into_iter().flatten() {
                report["file_changes"].as_array_mut().unwrap().push(json!({"path":change["path"],"kind":change["kind"]["type"],"status":item["status"]}));
            }
        }
        _ => {}
    }
}
