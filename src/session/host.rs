use super::{tools, BRIDGE};
use crate::{
    base::{self, Failure, Res},
    bridge::{self, Bridge},
    codex::{AppServer, RunRequest},
    engine, goal, hook, orch,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(super) struct Host {
    pub bridge: Arc<Bridge>,
    pub server: Arc<AppServer>,
    pub home: PathBuf,
    pub cwd: PathBuf,
    plugin: PathBuf,
    pub root: Mutex<String>,
    pub goal_lock: Mutex<()>,
    models: Mutex<HashMap<String, String>>,
    directories: Mutex<HashMap<String, PathBuf>>,
    disabled_plugins: Mutex<HashMap<String, Value>>,
    notices: Mutex<Vec<Value>>,
    pub cancelled: AtomicBool,
    pub budget_exhausted: AtomicBool,
    max_steps: Option<u32>,
    steps: Mutex<HashMap<String, u32>>,
    log: Mutex<File>,
}

fn invalid(s: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", s)
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn string<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("")
}
fn role(name: &str) -> Res<(&'static str, &'static str)> {
    match name {
        "core" | "astra" | "architect" => Ok(("astra", "max")),
        "dispatcher" => Ok(("terra", "high")),
        "reviewer" | "critic" => Ok(("sol", "high")),
        "memory-keeper" | "content" => Ok(("luna", "low")),
        _ => Err(invalid(format!("unknown chronos agent: {name}"))),
    }
}
fn capabilities(role: &str) -> Vec<&'static str> {
    let mut names = vec!["Read", "Glob", "Grep"];
    match role {
        "core" => names.extend([
            "Agent",
            "Write",
            "Bash",
            "orch",
            "goal",
            "memory_search",
            "memory_get",
            "memory_digest",
            "gemini_preview",
            "review_code",
        ]),
        "dispatcher" => names.extend([
            "Agent",
            "Write",
            "Bash",
            "codex_run",
            "orch",
            "memory_search",
            "memory_get",
            "review_code",
            "run_on_linux",
        ]),
        "architect" => names.extend(["Write", "Bash"]),
        "critic" => names = vec!["Read"],
        "memory-keeper" => names.extend(["memory_record", "memory_digest"]),
        "content" => names.push("Write"),
        _ => (),
    }
    names
}
fn public_name(name: &str) -> String {
    if ["Agent", "Read", "Glob", "Grep", "Write", "Bash"].contains(&name) {
        name.into()
    } else {
        format!("chronos_{name}")
    }
}
// Only host dynamic tools use aliases. External MCP names and hook contracts stay intact.
fn hook_name(name: &str) -> String {
    if ["Agent", "Read", "Glob", "Grep", "Write", "Bash"].contains(&name) {
        name.into()
    } else {
        format!("{BRIDGE}{name}")
    }
}
fn body(text: &str) -> &str {
    if text.starts_with("---\n") {
        text[4..]
            .split_once("\n---\n")
            .map_or(text, |(_, body)| body)
    } else {
        text
    }
}

fn specs(role: &str) -> Value {
    let bridge = bridge::tools();
    Value::Array(capabilities(role).into_iter().map(|name| {
        let (description, schema) = match name {
            "Agent" => ("Call a distinct Chronos agent. Astra calls use an independent planner context, never the core thread.".into(), json!({"type":"object","properties":{"subagent_type":{"type":"string"},"prompt":{"type":"string"},"description":{"type":"string"},"project":{"type":"string"},"cwd":{"type":"string"},"schema_path":{"type":"string"},"save_to":{"type":"string"},"web_search":{"type":"boolean"},"output_paths":{"type":"array","items":{"type":"string"}}},"required":["subagent_type","prompt"]})),
            "Read" => ("Read UTF-8 text. For images use the native view_image tool.".into(), json!({"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}},"required":["file_path"]})),
            "Write" => ("Write data-home artifacts, or root-authorized exact content output files. Never implement software features through content.".into(), json!({"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"]})),
            "Glob" | "Grep" => (format!("Read-only {name} search"), json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"-i":{"type":"boolean"},"output_mode":{"type":"string"}},"required":["pattern"]})),
            "Bash" => ("Read-only inspection commands; no writes or expansion.".into(), json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"integer"}},"required":["command"]})),
            _ => {
                let d = bridge.as_array().unwrap().iter().find(|d| d["name"] == name).unwrap();
                (d["description"].as_str().unwrap_or("").to_owned(), d["inputSchema"].clone())
            }
        };
        json!({"type":"function","name":public_name(name),"description":description,"inputSchema":schema})
    }).collect())
}

impl Host {
    pub fn new(
        config: &super::Config,
        cwd: &Path,
        log: File,
        max_steps: Option<u32>,
    ) -> Res<Arc<Self>> {
        let bridge = Arc::new(Bridge::new(bridge::Config {
            data_home: Some(config.home.clone()),
            codex: config.codex.clone(),
            agy: Err(Failure::new("not_installed", "Codex-only session")),
            wsl: base::resolve_command("AXIOM_WSL_CMD", "wsl.exe").ok(),
            on_windows: cfg!(windows),
        }));
        let server = bridge.codex()?;
        let plugin = engine::plugin_dir(&config.home).map_err(io)?;
        Ok(Arc::new(Self {
            bridge,
            server,
            home: config.home.clone(),
            cwd: cwd.into(),
            plugin,
            root: Mutex::new(String::new()),
            goal_lock: Mutex::new(()),
            models: Mutex::new(HashMap::new()),
            directories: Mutex::new(HashMap::new()),
            disabled_plugins: Mutex::new(HashMap::new()),
            notices: Mutex::new(Vec::new()),
            cancelled: AtomicBool::new(false),
            budget_exhausted: AtomicBool::new(false),
            max_steps,
            steps: Mutex::new(HashMap::new()),
            log: Mutex::new(log),
        }))
    }
    // Conservative model/tool-step bound per host thread. App-server turn/start
    // alone is insufficient: one turn can contain arbitrarily many tool iterations.
    pub fn step(&self, thread: &str) -> Res<()> {
        let mut steps = self.steps.lock().unwrap();
        let used = steps.entry(thread.into()).or_default();
        if self.max_steps.is_some_and(|limit| *used >= limit) {
            self.budget_exhausted.store(true, Ordering::Release);
            return Err(Failure::new(
                "max_turns",
                "maximum turns reached (model/tool step budget)",
            ));
        }
        *used = used.saturating_add(1);
        Ok(())
    }
    pub fn observe(&self, event: &Value) -> Res<()> {
        if event["method"] == "item/started"
            && matches!(
                event["params"]["item"]["type"].as_str(),
                Some(
                    "commandExecution"
                        | "fileChange"
                        | "mcpToolCall"
                        | "collabAgentToolCall"
                        | "webSearch"
                        | "imageView"
                        | "sleep"
                        | "imageGeneration"
                )
            )
        {
            self.step(string(&event["params"], "threadId"))?;
        }
        // Dynamic callbacks are charged before execution, not again at lifecycle events.
        Ok(())
    }
    pub fn log(&self, event: &Value) -> Res<()> {
        let mut log = self.log.lock().unwrap();
        writeln!(log, "{event}")
            .and_then(|_| log.flush())
            .map_err(io)
    }
    pub fn drain_notices(&self) -> Vec<Value> {
        std::mem::take(&mut *self.notices.lock().unwrap())
    }
    fn notice(&self, event: Value) -> Res<()> {
        self.log(&event)?;
        self.notices.lock().unwrap().push(event);
        Ok(())
    }
    pub fn workflow(&self, skill: &str, arguments: &str) -> Res<String> {
        let text = std::fs::read_to_string(self.plugin.join("skills").join(skill).join("SKILL.md"))
            .map_err(io)?;
        let mut contract = self.expand(&text, arguments);
        if skill == "research" {
            contract.push_str("\nHost session contract: Declare goal mode answer for the plan-only research workflow. Data-home planning artifacts may be written before mode selection. After the final plan and ruling, close the goal done with the goal tool. Do not dispatch engineering or require implementation/full-suite verification. If a required stage fails, report it and close blocked with that error.");
        }
        Ok(contract)
    }
    fn expand(&self, text: &str, arguments: &str) -> String {
        // Expand templates before inserting user text, which must stay verbatim.
        let normalized = text.replace("\r\n", "\n");
        body(&normalized)
            .replace("${CLAUDE_PLUGIN_ROOT}", &self.plugin.to_string_lossy())
            .replace("$CLAUDE_PLUGIN_ROOT", &self.plugin.to_string_lossy())
            .replace(BRIDGE, "chronos_")
            .replace("$ARGUMENTS", arguments)
    }
    fn agent_role(&self, name: &str) -> Res<(String, String)> {
        if name == "core" {
            return Ok(("astra".into(), "max".into()));
        }
        let text = std::fs::read_to_string(self.plugin.join("agents").join(format!("{name}.md")))
            .map_err(io)?;
        let normalized = text.replace("\r\n", "\n");
        let metadata = normalized
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .map(|(metadata, _)| metadata)
            .ok_or_else(|| invalid("agent is missing role metadata"))?;
        let field = |key: &str| {
            metadata
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .map(str::trim)
        };
        let model = field("model:").ok_or_else(|| invalid("agent model is missing"))?;
        let effort = field("effort:").ok_or_else(|| invalid("agent effort is missing"))?;
        let expected = role(name)?;
        if (model, effort) != expected {
            return Err(invalid(format!("invalid role metadata for {name}")));
        }
        Ok((model.into(), effort.into()))
    }
    fn instructions(&self, name: &str) -> Res<String> {
        let contract = if name == "core" {
            "You are Chronos core, the persistent Astra orchestrator. Delegate engineering directly to chronos:dispatcher, not to another Astra wrapper. Use chronos:astra only for independent research planning. Declare change mode before engineering dispatch or project mutation. Normal final text does not complete an active goal; close it with the goal tool once this workflow is complete (engineering requires verification; research ends at its plan and ruling). Classify interjections as amend, answer or stop. All session and agent sandboxes are read-only; only bound codex_run workers may edit software. Native read-only tools, including view_image, remain available. Content agents handle documents/images, never software features."
                .to_owned()
        } else {
            std::fs::read_to_string(self.plugin.join("agents").join(format!("{name}.md")))
                .map_err(io)?
        };
        let contract = self.expand(&contract, "");
        Ok(format!("{contract}\nData home: {}\nProject directory: {}\nEngine directory: {}\nChronos is the expressly selected orchestration workflow; use its embedded workflow instead of searching for other default orchestration skills. Follow project AGENTS coding conventions and explicit user requests for additional skills. Only the supplied dynamic capabilities are authorized. Write is confined to data home except root-authorized exact content outputs, which require a declared goal mode. Data-home planning artifacts may be written before mode selection. Dispatch and software mutation require change mode. Do not bypass host authority through native tools. Do not start native subagents or arbitrary nested agents. Core may call the named Chronos roles; only dispatcher may designate content/reviewer children. Root can authorize exact output_paths for content directly or pass them to dispatcher for subset delegation; this grants no dispatcher file-write authority. Preserve native view_image for image reading.", self.home.display(), self.cwd.display(), self.plugin.display()))
    }
    pub fn start(self: &Arc<Self>, name: &str, resume: Option<&str>, web: bool) -> Res<String> {
        self.start_at(name, resume, web, &self.cwd)
    }
    fn start_at(
        self: &Arc<Self>,
        name: &str,
        resume: Option<&str>,
        web: bool,
        cwd: &Path,
    ) -> Res<String> {
        self.start_as(name, resume, web, cwd, None)
    }
    fn start_as(
        self: &Arc<Self>,
        name: &str,
        resume: Option<&str>,
        web: bool,
        cwd: &Path,
        override_role: Option<&str>,
    ) -> Res<String> {
        if name != "core" && resume.is_some_and(|id| id == *self.root.lock().unwrap()) {
            return Err(invalid(
                "agents and planner must never resume the core thread",
            ));
        }
        let (model_role, _) = self.agent_role(name)?;
        let model_role = override_role.unwrap_or(&model_role);
        let model = self.server.resolve_model(&model_role)?;
        let req = RunRequest {
            brief: String::new(),
            cwd: cwd.to_string_lossy().into_owned(),
            model: model.clone(),
            sandbox: "read-only".into(),
            effort: None,
            output_schema: None,
            developer_instructions: Some(format!(
                "{}\nThis thread's working directory: {}",
                self.instructions(name)?,
                cwd.display()
            )),
            timeout_s: 1800,
            lean: true,
            keep_plugins: vec![],
            web_search: web,
        };
        let (mut params, disabled) = self.server.thread_start_params(&req)?;
        params["ephemeral"] = json!(name != "core");
        params["dynamicTools"] = specs(name);
        // Host authority must not be bypassed through native shell execution.
        params["config"]["features"]["shell_tool"] = json!(false);
        params["config"]["features"]["unified_exec"] = json!(false);
        let method = if let Some(id) = resume {
            params.as_object_mut().unwrap().remove("ephemeral");
            params.as_object_mut().unwrap().remove("dynamicTools");
            params["threadId"] = json!(id);
            "thread/resume"
        } else {
            "thread/start"
        };
        let result = self.server.request(method, params)?;
        let model = result["model"]
            .as_str()
            .ok_or_else(|| {
                Failure::new("codex_rpc", "thread response is missing its actual model")
            })?
            .to_owned();
        if !crate::codex::role_matches(&model_role, &model) {
            return Err(Failure::new(
                "model_mismatch",
                format!("role {model_role} resolved to unexpected model {model}"),
            ));
        }
        if result["sandbox"]["type"] != "readOnly" || result["approvalPolicy"] != "never" {
            return Err(Failure::new(
                "permission_denied",
                "session and agent threads must confirm read-only sandbox and never approvals",
            ));
        }
        let id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| invalid("missing thread id"))?
            .to_owned();
        if resume.is_some_and(|expected| expected != id) {
            return Err(invalid("resume returned a different thread"));
        }
        if name != "core" && id == *self.root.lock().unwrap() {
            return Err(invalid("agent and core thread IDs must be distinct"));
        }
        if resume.is_none() && self.models.lock().unwrap().contains_key(&id) {
            return Err(invalid("a fresh agent must have a distinct thread"));
        }
        self.models
            .lock()
            .unwrap()
            .insert(id.clone(), model.clone());
        self.directories
            .lock()
            .unwrap()
            .insert(id.clone(), cwd.into());
        self.disabled_plugins
            .lock()
            .unwrap()
            .insert(id.clone(), disabled["plugins"].clone());
        self.notice(json!({"hostEvent":"threadStarted","threadId":id,"role":name,"model":model,"resumed":resume.is_some()}))?;
        Ok(id)
    }
    pub fn turn_params(&self, thread: &str, input: Value, effort: &str) -> Value {
        let mut params = json!({"threadId":thread,"input":input,"effort":effort});
        if let Some(disabled) = self
            .disabled_plugins
            .lock()
            .unwrap()
            .get(thread)
            .filter(|v| v.is_array())
        {
            params["disabledPluginIds"] = disabled.clone();
        }
        params
    }
    pub fn register(self: &Arc<Self>, thread: &str, role: &str, outputs: Vec<PathBuf>) -> Res<()> {
        let weak = Arc::downgrade(self);
        let caller = role.to_owned();
        let caller_id = thread.to_owned();
        self.server.register_tools(thread, Arc::new(move |name, args| {
            let Some(host) = weak.upgrade() else { return json!({"success":false,"contentItems":[{"type":"inputText","text":"session closed"}]}); };
            let call_id = base::unique();
            if let Err(e) = host.notice(json!({"hostEvent":"toolStarted","threadId":caller_id,"toolCallId":call_id,"role":caller,"tool":name,"arguments":args})) {
                host.cancelled.store(true,Ordering::Release);
                return json!({"success":false,"contentItems":[{"type":"inputText","text":e.message}]});
            }
            let result = host.call(&caller, &caller_id, name, args, &outputs);
            let response = match result {
                Ok(v) => json!({"success":v["status"].as_str().is_none_or(|s|s=="completed"),"contentItems":[{"type":"inputText","text":v.to_string()}]}),
                Err(e) => json!({"success":false,"contentItems":[{"type":"inputText","text":json!({"status":"failed","error":e.to_json()}).to_string()}]}),
            };
            if host.notice(json!({"hostEvent":"toolFinished","threadId":caller_id,"toolCallId":call_id,"role":caller,"tool":name,"result":response})).is_err() { host.cancelled.store(true,Ordering::Release); }
            response
        }))
    }
    fn call(
        self: &Arc<Self>,
        caller: &str,
        caller_id: &str,
        name: &str,
        args: &Value,
        outputs: &[PathBuf],
    ) -> Res<Value> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Failure::new("interrupted", "session cancelled"));
        }
        self.step(caller_id)?;
        let short = name.strip_prefix("chronos_").unwrap_or(name);
        if !capabilities(caller).contains(&short) {
            return Err(Failure::new(
                "permission_denied",
                "tool is not a capability of this role",
            ));
        }
        let root = self.root.lock().unwrap().clone();
        let cwd = self
            .directories
            .lock()
            .unwrap()
            .get(caller_id)
            .cloned()
            .unwrap_or_else(|| self.cwd.clone());
        let mut event = json!({"hook_event_name":"PreToolUse","session_id":root,"cwd":cwd,"tool_name":hook_name(short),"tool_input":args,"agent_role":caller});
        if caller != "core" {
            event["agent_id"] = json!(caller_id);
        }
        if caller == "content" && short == "Write" {
            let target = tools::resolved(&tools::path(&cwd, string(args, "file_path")))?;
            if outputs.contains(&target) {
                event["authorized_output_path"] = json!(target);
                event["tool_input"]["file_path"] = json!(target);
            }
        }
        {
            // Hooks may persist dispatch/mode state. Do not race a live interjection.
            let _goal = self.goal_lock.lock().unwrap();
            if let Some(denial) = hook::handle(&event, Some(&self.home), true) {
                return Err(Failure::new("permission_denied", denial.to_string()));
            }
        }
        if short == "Agent" {
            return self.agent(caller, &cwd, args, outputs);
        }
        if ["Read", "Glob", "Grep", "Write", "Bash"].contains(&short) {
            return tools::dispatch(
                &self.home,
                &cwd,
                short,
                args,
                if caller == "content" { outputs } else { &[] },
                &self.cancelled,
            );
        }
        let mut args = args.clone();
        if short == "goal" {
            args["session"] = json!(root);
        }
        if short == "codex_run" && !matches!(args["role"].as_str(), Some("terra" | "sol")) {
            return Err(Failure::new(
                "permission_denied",
                "dispatcher workers must use terra or sol; research uses the independent planner",
            ));
        }
        let result = if short == "goal" {
            let _goal = self.goal_lock.lock().unwrap();
            if args.get("status").is_some()
                && args.get("classify").is_none()
                && goal::load(&self.home, &root)?.is_some_and(|g| g.interjection.is_some())
            {
                return Err(Failure::new(
                    "pending_interjection",
                    "classify the user's pending interjection before closing the goal",
                ));
            }
            bridge::tools::dispatch(&self.bridge, short, &args)?
        } else {
            bridge::tools::dispatch(&self.bridge, short, &args)?
        };
        if short == "goal" && args["classify"] == "stop" {
            self.cancelled.store(true, Ordering::Release);
            self.bridge.cancel();
        }
        Ok(result)
    }
    fn agent(
        self: &Arc<Self>,
        caller: &str,
        caller_cwd: &Path,
        args: &Value,
        caller_outputs: &[PathBuf],
    ) -> Res<Value> {
        let name = string(args, "subagent_type")
            .strip_prefix("chronos:")
            .ok_or_else(|| invalid("expected chronos agent"))?;
        role(name)?;
        if name == "core"
            || (caller != "core"
                && !(caller == "dispatcher" && matches!(name, "content" | "reviewer")))
        {
            return Err(Failure::new(
                "permission_denied",
                "nested agent is not designated for this role",
            ));
        }
        if string(args, "prompt").trim().is_empty() {
            return Err(invalid("prompt is required"));
        }
        let mut outputs = Vec::new();
        if let Some(paths) = args.get("output_paths") {
            let paths = paths
                .as_array()
                .ok_or_else(|| invalid("output_paths must be an array"))?;
            if !matches!(name, "content" | "dispatcher") && !paths.is_empty() {
                return Err(invalid(
                    "output_paths is for content or dispatcher delegation only",
                ));
            }
            for p in paths {
                let p = p
                    .as_str()
                    .ok_or_else(|| invalid("output_paths must contain paths"))?;
                let p = tools::resolved(&tools::path(caller_cwd, p))?;
                if caller != "core" && !caller_outputs.contains(&p) {
                    return Err(Failure::new(
                        "permission_denied",
                        "content output requires exact root authorization",
                    ));
                }
                outputs.push(p);
            }
        }
        if name == "astra" {
            return self.plan(args);
        }
        let cwd = args["cwd"]
            .as_str()
            .map(|p| tools::path(caller_cwd, p))
            .unwrap_or_else(|| caller_cwd.into());
        base::require_directory(&cwd.to_string_lossy())?;
        let reviewer_role = if name == "reviewer" {
            let project = args["project"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    goal::load(&self.home, &self.root.lock().unwrap())
                        .ok()
                        .flatten()
                        .and_then(|g| g.project)
                })
                .ok_or_else(|| invalid("reviewer needs project"))?;
            let store = orch::store_for(&self.home, &project)?;
            let units = if store.join("units.tsv").exists() {
                orch::read_units(&store)?
                    .into_iter()
                    .map(|u| u.id)
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            Some(bridge::review_run::role(
                &self.bridge,
                Some(&project),
                &units,
            )?)
        } else {
            None
        };
        let thread = self.start_as(name, None, false, &cwd, reviewer_role)?;
        // Dispatcher receives delegation authority, never direct write authority.
        self.register(&thread, name, outputs)?;
        let result = self.turn(&thread, name, string(args, "prompt"), None);
        self.server.unregister_tools(&thread);
        result
    }
    fn plan(self: &Arc<Self>, args: &Value) -> Res<Value> {
        let cwd = args["cwd"]
            .as_str()
            .map(|p| tools::path(&self.cwd, p))
            .unwrap_or_else(|| self.cwd.clone());
        base::require_directory(&cwd.to_string_lossy())?;
        let schema = if let Some(path) = args["schema_path"].as_str() {
            Some(
                serde_json::from_slice::<Value>(
                    &std::fs::read(tools::path(&cwd, path)).map_err(io)?,
                )
                .map_err(io)?,
            )
        } else {
            None
        };
        let project = args["project"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                goal::load(&self.home, &self.root.lock().unwrap())
                    .ok()
                    .flatten()
                    .and_then(|g| g.project)
            })
            .ok_or_else(|| invalid("planner needs project"))?;
        let store = orch::store_for(&self.home, &project)?;
        let dest = if let Some(path) = args["save_to"].as_str() {
            planner_destination(&store, Path::new(path))?
        } else {
            planner_destination(
                &store,
                &store.join(format!("research/astra-{}.json", base::unique())),
            )?
        };
        // Each Agent call is independent, with its own cwd/search policy and handler.
        let planner = self.start_at("astra", None, args["web_search"] == true, &cwd)?;
        self.register(&planner, "astra", vec![])?;
        let result = self.turn(&planner, "astra", string(args, "prompt"), schema.as_ref());
        self.server.unregister_tools(&planner);
        let mut report = match result {
            Ok(report) => report,
            Err(e) => {
                let model = self
                    .models
                    .lock()
                    .unwrap()
                    .get(&planner)
                    .cloned()
                    .unwrap_or_default();
                orch::record_stage(&store, "astra", &model, false)?;
                return Err(e);
            }
        };
        let parsed = report["final_message"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        let valid = report["status"] == "completed"
            && match (&schema, &parsed) {
                (Some(schema), Some(value)) => base::schema::validate(schema, value).is_ok(),
                (Some(_), None) => false,
                (None, _) => true,
            };
        if valid {
            let value = parsed.unwrap_or_else(|| json!({"text":report["final_message"]}));
            if let Err(e) =
                base::atomic_write(&dest, &serde_json::to_vec_pretty(&value).map_err(io)?)
            {
                orch::record_stage(&store, "astra", string(&report, "model"), false)?;
                return Err(io(e));
            }
            report["saved_to"] = json!(dest);
            report["coverage"] = value.get("coverage").cloned().unwrap_or(Value::Null);
        } else {
            report["status"] = json!("failed");
            if report["error"].is_null() {
                report["error"] = json!({"kind":"invalid_structured_output","message":"planner output did not satisfy schema"});
            }
        }
        orch::record_stage(&store, "astra", string(&report, "model"), valid)?;
        report.as_object_mut().unwrap().remove("final_message");
        Ok(report)
    }
    fn turn(&self, thread: &str, name: &str, prompt: &str, schema: Option<&Value>) -> Res<Value> {
        let events = self.server.subscribe(thread);
        let result = (|| {
            let (_, mut effort) = self.agent_role(name)?;
            let model = self
                .models
                .lock()
                .unwrap()
                .get(thread)
                .cloned()
                .ok_or_else(|| invalid("missing actual thread model"))?;
            if name == "reviewer" && crate::codex::role_matches("astra", &model) {
                effort = "max".into();
            }
            let mut params =
                self.turn_params(thread, json!([{"type":"text","text":prompt}]), &effort);
            if let Some(schema) = schema {
                params["outputSchema"] = schema.clone();
            }
            self.step(thread)?;
            let started = self.server.request("turn/start", params)?;
            let turn = started["turn"]["id"]
                .as_str()
                .ok_or_else(|| invalid("missing turn id"))?;
            self.server.register_turn(thread, turn);
            let mut report = json!({"status":"failed","model":model,"thread_id":thread,"turn_id":turn,"final_message":null,"usage":null,"error":null});
            let deadline = Instant::now() + Duration::from_secs(1800);
            let result = loop {
                if self.budget_exhausted.load(Ordering::Acquire) {
                    self.server.interrupt(thread, turn);
                    break Err(Failure::new(
                        "max_turns",
                        "maximum turns reached (model/tool step budget)",
                    ));
                }
                if self.cancelled.load(Ordering::Acquire)
                    || super::run::interrupted()
                    || Instant::now() >= deadline
                {
                    self.server.interrupt(thread, turn);
                    break Err(Failure::new(
                        "interrupted",
                        "agent turn interrupted or timed out",
                    ));
                }
                let event = match events.recv_timeout(Duration::from_millis(50)) {
                    Ok(e) => e,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => {
                        break Err(Failure::new(
                            "codex_exited",
                            "app-server exited during agent turn",
                        ))
                    }
                };
                if let Err(e) = self.notice_protocol(&event) {
                    self.server.interrupt(thread, turn);
                    break Err(e);
                }
                record(&mut report, &event);
                if event["method"] == "turn/completed" && event["params"]["turn"]["id"] == turn {
                    break Ok(report);
                }
            };
            self.server.unregister_turn(thread, turn);
            result
        })();
        self.server.unsubscribe(thread);
        result
    }
    pub fn shutdown(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.bridge.cancel();
        self.server.close();
    }
    fn notice_protocol(&self, event: &Value) -> Res<()> {
        self.log(event)?;
        self.observe(event)?;
        if event["method"] == "thread/tokenUsage/updated" {
            self.notices.lock().unwrap().push(event.clone());
        }
        Ok(())
    }
}

fn planner_destination(store: &Path, path: &Path) -> Res<PathBuf> {
    std::fs::create_dir_all(store).map_err(io)?;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(invalid(
            "planner save_to must be absolute and inside orch store",
        ));
    }
    let resolved = tools::resolved(path)?;
    if !resolved.starts_with(store.canonicalize().map_err(io)?)
        || resolved == store.canonicalize().map_err(io)?
    {
        return Err(invalid("planner save_to must be inside orch store"));
    }
    if let Some(p) = resolved.parent() {
        std::fs::create_dir_all(p).map_err(io)?;
    }
    Ok(resolved)
}

pub(super) fn record(report: &mut Value, event: &Value) {
    let p = &event["params"];
    match event["method"].as_str().unwrap_or("") {
        "thread/tokenUsage/updated" => report["usage"] = p["tokenUsage"]["total"].clone(),
        "item/completed" if p["item"]["type"] == "agentMessage" => {
            report["final_message"] = p["item"]["text"].clone()
        }
        "error" if p["willRetry"] != true => report["error"] = p["error"].clone(),
        "turn/completed" => {
            report["status"] = p["turn"]["status"].clone();
            if !p["turn"]["error"].is_null() {
                report["error"] = p["turn"]["error"].clone();
            }
        }
        _ => (),
    }
}
