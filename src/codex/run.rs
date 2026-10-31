use super::{AppServer, SANDBOXES};
use crate::base::{self, Failure, Res};
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

pub(crate) const LEAN_WORKER_BASE_INSTRUCTIONS: &str = "You are a scoped Chronos worker. Carry out the supplied GOAL within SCOPE and use VERIFY to establish completion. Follow developer instructions, the user request, project AGENTS conventions and enforced permissions. Preserve unrelated edits; other workers may share this workspace. Read only material needed for the task, batch independent inspections, and stop once the requested outcome is verified. Do not delegate or expand the task. When the prompt supplies all necessary material, return the requested output directly without tools. Report actual results and unresolved failures concisely; never claim checks you did not run. Ask for missing information only when it materially prevents correct completion.";

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
    pub(crate) fn thread_start_params(&self, req: &RunRequest) -> Res<(Value, Value)> {
        let mut thread = json!({"model":req.model,"ephemeral":true,"sandbox":req.sandbox,"approvalPolicy":"never","cwd":req.cwd});
        if let Some(s) = req
            .developer_instructions
            .as_ref()
            .filter(|s| !s.is_empty())
        {
            thread["developerInstructions"] = json!(s);
        }
        let disabled = if req.lean {
            let installed = self.request("plugin/installed", json!({"cwds":[req.cwd]}))?;
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
            let config = self.request("config/read", json!({"cwd":req.cwd}))?;
            let listed = self.request("skills/list", json!({"cwds":[req.cwd]}))?;
            let entries = listed["data"]
                .as_array()
                .ok_or_else(|| Failure::new("codex_rpc", "skills/list: missing data"))?;
            let mut skills = Vec::new();
            for entry in entries {
                if entry["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
                {
                    return Err(Failure::new("codex_rpc", "skills/list: discovery errors"));
                }
                let discovered = entry["skills"]
                    .as_array()
                    .ok_or_else(|| Failure::new("codex_rpc", "skills/list: missing skills"))?;
                for skill in discovered {
                    if skill["enabled"] == true
                        && !skill["pluginId"]
                            .as_str()
                            .is_some_and(|id| req.keep_plugins.iter().any(|k| k == id))
                    {
                        let path = skill["path"]
                            .as_str()
                            .filter(|p| !p.is_empty())
                            .ok_or_else(|| {
                                Failure::new("codex_rpc", "skills/list: missing skill path")
                            })?;
                        skills.push(path.to_owned());
                    }
                }
            }
            skills.sort();
            skills.dedup();
            let mut servers: Vec<String> = config["config"]["mcp_servers"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys().cloned())
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
            config["skills"]["config"] = json!(skills
                .iter()
                .map(|path| json!({"path":path,"enabled":false}))
                .collect::<Vec<_>>());
            if req.keep_plugins.is_empty() {
                config["features"]["plugins"] = json!(false);
                config["features"]["remote_plugin"] = json!(false);
            }
            let mut sections = vec!["apps", "goals", "skills_catalog", "personality"];
            if !req.web_search {
                config["web_search"] = json!("disabled");
                sections.push("web_search");
            }
            thread["config"] = config;
            json!({"sections":sections,"plugins":plugins,"mcp_servers":servers,"skills":skills})
        } else {
            Value::Null
        };
        // Bridge dispatch is the only delegation path, including non-lean workers.
        thread["config"]["features"]["multi_agent"] = json!(false);
        thread["config"]["features"]["multi_agent_v2"] = json!(false);
        thread["config"]["features"]["view_image"] = json!(true);
        if req.sandbox == "workspace-write" {
            // Workers may run Cargo before Cargo.toml even exists. Keep their
            // build outputs outside SCOPE without relying on generated .gitignore.
            let target=base::cargo_target(std::path::Path::new(&req.cwd),&format!("worker:{}",base::unique()))?;
            thread["config"]["shell_environment_policy"]["set"]["CARGO_TARGET_DIR"]=json!(target);
            thread["config"]["sandbox_workspace_write"]["writable_roots"]=json!([target]);
        }
        Ok((thread, disabled))
    }
    pub fn run(&self, req: &RunRequest) -> Res<Value> {
        let observed_start = Instant::now();
        let budget = base::proc::remaining(Duration::from_secs(req.timeout_s));
        if budget.is_zero() { return Err(Failure::new("timeout", "operation budget exhausted before worker launch")); }
        let started_at = base::now_iso();
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
        let (mut thread, disabled) = self.thread_start_params(req)?;
        if req.lean {
            thread["baseInstructions"] = json!(LEAN_WORKER_BASE_INSTRUCTIONS);
        }
        let started = self.request("thread/start", thread)?;
        let actual_model = started["model"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        let actual_sandbox = started["sandbox"]["type"].clone();
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| Failure::new("codex_rpc", "thread/start: missing thread id"))?
            .to_owned();
        let expected_type = if req.sandbox == "read-only" {
            "readOnly"
        } else {
            "workspaceWrite"
        };
        let model_ok = actual_model.as_deref().is_some_and(|actual| {
            ["astra", "terra", "sol", "luna"]
                .iter()
                .find(|role| super::role_matches(role, &req.model))
                .map_or(true, |role| super::role_matches(role, actual))
        });
        if !model_ok
            || started["sandbox"]["type"] != expected_type
            || started["approvalPolicy"] != "never"
        {
            self.unsubscribe(&thread_id);
            return Err(Failure::new("thread_policy", "thread/start did not confirm authorized model, sandbox and never approvals; turn not started"));
        }
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
        let result = self.request("turn/start", turn);
        let started = match result {
            Ok(v) => v,
            Err(e) => {
                self.unsubscribe(&thread_id);
                if matches!(
                    e.kind.as_str(),
                    "timeout" | "turn_unconfirmed" | "codex_exited"
                ) {
                    self.mark_unconfirmed();
                    self.close();
                    return Err(Failure::new("turn_unconfirmed", "turn/start response unconfirmed; app-server connection closed, affecting concurrent calls; turn termination is unconfirmed"));
                }
                return if e.kind == "codex_exited" {
                    Ok(report_error(
                        req,
                        actual_model.as_deref(),
                        &thread_id,
                        "",
                        disabled,
                        "bridge_error",
                        &e.kind,
                        &e.message,
                    ))
                } else {
                    Err(e)
                };
            }
        };
        let Some(turn_id) = started.pointer("/turn/id").and_then(Value::as_str) else {
            self.unsubscribe(&thread_id);
            self.mark_unconfirmed();
            self.close();
            return Err(Failure::new("turn_unconfirmed", "turn/start omitted turn id; connection closed, affecting concurrent calls; termination unconfirmed"));
        };
        self.register_turn(&thread_id, turn_id);
        let mut report = json!({"status":null,"thread_id":thread_id,"turn_id":turn_id,"model":actual_model,"requested_model":req.model,"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"refused_requests":[],"usage":null,"disabled":disabled,"started_mcp_servers":[],"error":null});
        report["actual_sandbox"] = actual_sandbox;
        report["actual_effort"] = Value::Null;
        let deadline = observed_start.checked_add(budget);
        let cancellation = base::proc::current_cancellation();
        let mut stopping: Option<Instant> = None;
        let mut timed_out = false;
        let mut confirmed = false;
        loop {
            let remaining = deadline
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::MAX);
            if stopping.is_none()
                && (remaining.is_zero() || cancellation.as_ref().is_some_and(|c| c.cancelled()))
            {
                timed_out = remaining.is_zero();
                let reason = if remaining.is_zero() {
                    "timeout"
                } else {
                    "cancelled"
                };
                if let Err(e) = self.interrupt(&thread_id, turn_id) {
                    report["status"] = json!("bridge_error");
                    report["error"] = json!({"kind":"turn_unconfirmed","message":format!("{reason}: interrupt failed ({}); turn may still be running", e.message)});
                    break;
                }
                stopping = Some(Instant::now() + Duration::from_secs(2));
            }
            if stopping.is_some_and(|d| Instant::now() >= d) {
                report["status"] = json!("bridge_error");
                report["error"] = json!({"kind":"turn_unconfirmed","message":"interrupt acknowledged but terminal event not confirmed; turn may still be running"});
                break;
            }
            let wait = stopping
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|| Duration::from_millis(50).min(remaining));
            let event = match events.recv_timeout(wait) {
                Ok(event) => event,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    report["status"] = json!("bridge_error");
                    report["error"] = json!({"kind":"turn_unconfirmed","message":"app-server exited mid-turn; terminal event unconfirmed"});
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
            match method {
                "thread/settings/updated" => {
                    let effort = params
                        .get("threadSettings")
                        .and_then(|settings| settings.get("effort"))
                        .or_else(|| params.get("effort"));
                    if let Some(effort) = effort.and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        report["actual_effort"] = json!(effort);
                    }
                }
                "thread/tokenUsage/updated" => {
                    report["usage"] = params["tokenUsage"]["total"].clone()
                }
                "item/completed" => record_item(&mut report, &params["item"]),
                "error" if params["willRetry"] != true => {
                    report["error"] = turn_error(&params["error"])
                }
                "turn/completed" if params["turn"]["id"] == turn_id => {
                    confirmed = true;
                    report["status"] = params["turn"]["status"].clone();
                    if !params["turn"]["error"].is_null() {
                        report["error"] = turn_error(&params["turn"]["error"]);
                    }
                    if stopping.is_some() || cancellation.as_ref().is_some_and(|c| c.cancelled())
                        || deadline.is_some_and(|d| Instant::now() >= d) {
                        timed_out |= deadline.is_some_and(|d| Instant::now() >= d);
                        report["terminal_status"] = report["status"].clone();
                        report["status"] = json!(if timed_out { "timeout" } else { "interrupted" });
                        report["error"] = json!({"kind":report["status"],"message":"owning request stopped; terminal event confirmed, not credited as success"});
                        break;
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
        if !confirmed {
            self.mark_unconfirmed();
        }
        self.unregister_turn(&thread_id, turn_id);
        self.unsubscribe(&thread_id);
        report["started_at"] = json!(started_at);
        report["elapsed_ms"] = json!(observed_start.elapsed().as_millis() as u64);
        Ok(report)
    }
}

fn report_error(
    req: &RunRequest,
    actual_model: Option<&str>,
    thread: &str,
    turn: &str,
    disabled: Value,
    status: &str,
    kind: &str,
    message: &str,
) -> Value {
    json!({"status":status,"thread_id":thread,"turn_id":turn,"model":actual_model,"requested_model":req.model,"final_message":null,"structured_output":null,"commands":[],"file_changes":[],"refused_requests":[],"usage":null,"disabled":disabled,"started_mcp_servers":[],"error":{"kind":kind,"message":message}})
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
