use super::{authority, tools, BRIDGE};
use crate::{
    base::{self, Failure, Res},
    bridge::{self, Bridge},
    codex::{AppServer, RunRequest},
    engine, goal, hook, orch,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    delivered_core_notices: Mutex<HashSet<String>>,
    producers: AtomicUsize,
    callbacks: Mutex<HashMap<String, usize>>,
    dispatcher_failure: Mutex<Option<String>>,
    pub cancelled: AtomicBool,
    pub budget_exhausted: AtomicBool,
    max_steps: Option<u32>,
    steps: Mutex<HashMap<String, u32>>,
    log: Mutex<File>,
}

fn invalid(s: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", s)
}
struct Producer<'a>(&'a AtomicUsize);
struct TurnBudget { remaining: Duration, last: Instant }
impl TurnBudget {
    fn new(remaining: Duration) -> Self { Self { remaining, last: Instant::now() } }
    fn tick(&mut self, now: Instant, callback: bool) {
        if !callback { self.remaining = self.remaining.saturating_sub(now.saturating_duration_since(self.last)); }
        self.last = now;
    }
}
impl<'a> Producer<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count)
    }
}
impl Drop for Producer<'_> {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::AcqRel); }
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn string<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("")
}
fn interrupted_report(report: &mut Value, reason: &str) {
    report["terminal_status"] = report["status"].clone();
    report["status"] = json!("interrupted");
    report["error"] = json!({"kind":"interrupted","message":reason});
}

#[cfg(test)]
mod interruption_tests {
    use super::*;

    #[test]
    fn late_completion_is_retained_but_not_success() {
        let mut report = json!({"status":"completed", "thread_id":"child", "turn_id":"turn"});
        interrupted_report(&mut report, "deadline reached");
        assert_eq!(report["status"], "interrupted");
        assert_eq!(report["terminal_status"], "completed");
        assert_eq!(report["thread_id"], "child");
        assert_eq!(report["turn_id"], "turn");
    }
    #[test]
    fn outer_1800_second_budget_excludes_owned_callbacks_but_not_model_time() {
        let now = Instant::now();
        let mut budget = TurnBudget { remaining: Duration::from_secs(1800), last: now };
        budget.tick(now + Duration::from_secs(1799), false);
        assert_eq!(budget.remaining, Duration::from_secs(1));
        budget.tick(now + Duration::from_secs(9000), true);
        assert_eq!(budget.remaining, Duration::from_secs(1));
        budget.tick(now + Duration::from_secs(9001), false);
        assert!(budget.remaining.is_zero());
    }
}
use crate::codex::policy::role;
fn capabilities(role: &str) -> Vec<&'static str> {
    let mut names = vec!["Read", "Glob", "Grep"];
    match role {
        "core" => names = vec!["Agent", "workflow", "goal"],
        "dispatcher" => names.extend([
            "workflow",
            "Agent",
            "Write",
            "Bash",
            "codex_run",
            "orch",
            "memory_search",
            "memory_get",
            "memory_digest",
            "gemini_preview",
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

pub(super) fn specs(role: &str) -> Value {
    let bridge = bridge::tools();
    Value::Array(capabilities(role).into_iter().map(|name| {
        let (mut description, mut schema) = match name {
            "workflow" => ("Durable role workflow. Core: status, verdict(pass/reject, revision, short reason), submit. Dispatcher: status and ready after all tests. No model may set a phase.".into(), json!({"type":"object","properties":{"action":{"type":"string","enum":["status","verdict","ready","submit"]},"revision":{"type":"integer"},"decision":{"type":"string","enum":["pass","reject"]},"reason":{"type":"string","maxLength":160}},"required":["action"]})),
            "Agent" => ("Call a distinct Chronos agent. Astra calls use an independent planner context, never the core thread.".into(), json!({"type":"object","properties":{"subagent_type":{"type":"string"},"prompt":{"type":"string"},"description":{"type":"string"},"project":{"type":"string"},"cwd":{"type":"string"},"timeout_s":{"type":"integer","minimum":1},"schema_path":{"type":"string"},"save_to":{"type":"string"},"web_search":{"type":"boolean"},"output_paths":{"type":"array","items":{"type":"string"}}},"required":["subagent_type","prompt"]})),
            "Read" => ("Read UTF-8 text. For images use the native view_image tool.".into(), json!({"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}},"required":["file_path"]})),
            "Write" => ("Write data-home artifacts, or root-authorized exact content output files. Never implement software features through content.".into(), json!({"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"]})),
            "Glob" | "Grep" => (format!("Read-only {name} search"), json!({"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob":{"type":"string"},"-i":{"type":"boolean"},"output_mode":{"type":"string"}},"required":["pattern"]})),
            "Bash" => ("Read-only inspection commands; no writes or expansion.".into(), json!({"type":"object","properties":{"command":{"type":"string"},"timeout":{"type":"integer"}},"required":["command"]})),
            _ => {
                let d = bridge.as_array().unwrap().iter().find(|d| d["name"] == name).unwrap();
                (d["description"].as_str().unwrap_or("").to_owned(), d["inputSchema"].clone())
            }
        };
        if name == "codex_run" {
            description.push_str(" Astra HIGH independent read-only verification or assigned measurement: role astra-verify, sandbox read-only, task_kind verification or measurement. No product edits.");
            schema["properties"]["role"]["enum"] = json!(["terra", "sol", "astra", "astra-verify"]);
            schema["properties"]["task_kind"] = json!({"type":"string", "enum":["verification", "measurement"]});
        }
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
            delivered_core_notices: Mutex::new(HashSet::new()),
            producers: AtomicUsize::new(0),
            callbacks: Mutex::new(HashMap::new()),
            dispatcher_failure: Mutex::new(None),
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
    pub fn enrolled_change_session(&self) -> Res<bool> {
        let root = self.root.lock().unwrap().clone();
        let Some(g) = goal::load(&self.home, &root)? else {
            return Ok(false);
        };
        if g.mode.as_deref() != Some("change") {
            return Ok(false);
        }
        let Some(project) = g.project else {
            return Ok(false);
        };
        let store = orch::store_for(&self.home, &project)?;
        if !store.join("wait-required").exists() && !store.join("core-wait.json").exists() {
            return Ok(false);
        }
        let state = orch::wait::wait_state(&store)?;
        Ok(state.engineering_enrolled || store.join("wait-required").exists())
    }
    pub fn core_notice_delivered(&self, event: &orch::Event) {
        self.delivered_core_notices.lock().unwrap().insert(event.id.clone());
    }
    pub fn idle_blockers(&self) -> Res<Vec<String>> {
        if self.producers.load(Ordering::Acquire) != 0 { return Ok(vec![]); }
        let root = self.root.lock().unwrap().clone();
        let project = goal::load(&self.home, &root)?.and_then(|g| g.project)
            .ok_or_else(|| invalid("enrolled session needs project"))?;
        let store = orch::store_for(&self.home, &project)?;
        std::fs::metadata(store.join("core-wait.json")).map_err(|e| {
            Failure::new("io", format!("enrolled core-wait journal unavailable: {e}"))
        })?;
        let mut blockers = orch::wait::idle_blockers(&store)?;
        if let Some(error) = self.dispatcher_failure.lock().unwrap().as_ref() {
            blockers.push(format!("dispatcher failed; no confirmed producer: {error}"));
        }
        Ok(blockers)
    }
    pub fn has_new_core_notice(&self) -> Res<bool> {
        let pending = self.pending_core_notices()?;
        let delivered = self.delivered_core_notices.lock().unwrap();
        Ok(pending.iter().any(|event| !delivered.contains(&event.id)))
    }
    pub fn pending_core_notices(&self) -> Res<Vec<orch::Event>> {
        let root = self.root.lock().unwrap().clone();
        goal::load(&self.home, &root)?
            .and_then(|g| g.project)
            .map(|p| {
                orch::store_for(&self.home, &p).and_then(|s| {
                    if s.join("core-wait.json").exists() || s.join("wait-required").exists() {
                        orch::wait::core_notices(&s)
                    } else {
                        Ok(vec![])
                    }
                })
            })
            .transpose()
            .map(|v| v.unwrap_or_default())
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
            return Ok(("astra".into(), "medium".into()));
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
            "You are Chronos core, the external user-intent boundary. Relay user intent ONLY to chronos:dispatcher. Do not research, design, plan, inspect implementation, run tests, build, recover operations or assemble. The Rust workflow returns an intent_review boundary: respond only with workflow verdict pass, or reject plus the shortest user-intent conflict reason. After pass, dispatcher calls Astra HIGH architect, Terra workers, Sol assembly and Astra HIGH testing. Only a ready_submit boundary authorizes workflow submit, followed by goal completion. Ordinary engineering failures stay in dispatcher. User communication and major user-owned judgments remain yours. Never directly call another specialist or create another core."
                .to_owned()
        } else {
            std::fs::read_to_string(self.plugin.join("agents").join(format!("{name}.md")))
                .map_err(io)?
        };
        let mut contract = self.expand(&contract, "");
        let root = self.root.lock().unwrap().clone();
        if let Some(g) = if root.is_empty() {
            None
        } else {
            goal::load(&self.home, &root)?
        } {
            contract.push_str(&format!("\nInherited root goal: session={root}, project={}, mode={}. Read-only constrains native project writes, NOT supplied dynamic tool execution. In change mode dispatcher is authorized to register units with chronos_orch and call chronos_codex_run with role terra or sol; the Host performs the writing worker call. Do not infer inability to dispatch from sandbox mode. Use observed tool responses, not assumptions. Only root selects mode.", g.project.as_deref().unwrap_or(""), g.mode.as_deref().unwrap_or("undeclared")));
        }
        Ok(format!("{contract}\nData home: {}\nProject directory: {}\nEngine directory: {}\nChronos is the expressly selected orchestration workflow; use its embedded workflow instead of searching for other default orchestration skills. Follow project AGENTS coding conventions and explicit user requests for additional skills. Only the supplied dynamic capabilities are authorized. Write is confined to data home except root-authorized exact content outputs, which require a declared goal mode. Data-home planning artifacts may be written before mode selection. Dispatch and software mutation require change mode. Do not bypass host authority through native tools. Do not start native subagents or arbitrary nested agents. Core may call only dispatcher; dispatcher owns research Astra MAX, planning architect Astra HIGH, critic, reviewer, memory-keeper and content children. Engineering uses direct chronos_codex_run role terra or sol, never another core. Root can authorize exact output_paths for content directly or pass them to dispatcher for subset delegation; this grants no dispatcher file-write authority. Preserve native view_image for image reading.", self.home.display(), self.cwd.display(), self.plugin.display()))
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
        // RPC callbacks run on separate threads: explicitly preserve the initiating
        // request's cancellation domain through Agent -> Agent/tool -> codex_run.
        let cancellation =
            base::proc::current_cancellation().unwrap_or_else(|| self.bridge.verification.clone());
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
            *host.callbacks.lock().unwrap().entry(caller_id.clone()).or_default() += 1;
            let result = cancellation.scope(|| host.call(&caller, &caller_id, name, args, &outputs));
            *host.callbacks.lock().unwrap().get_mut(&caller_id).unwrap() -= 1;
            let response = match result {
                Ok(v) => json!({"success":base::tool_succeeded(&v),"contentItems":[{"type":"inputText","text":v.to_string()}]}),
                Err(e) => json!({"success":false,"contentItems":[{"type":"inputText","text":json!({"status":"failed","error":e.to_json()}).to_string()}]}),
            };
            if host.notice(json!({"hostEvent":"toolFinished","threadId":caller_id,"toolCallId":call_id,"role":caller,"tool":name,"result":response})).is_err() { host.cancelled.store(true,Ordering::Release); }
            response
        }))
    }
    pub(super) fn call(
        self: &Arc<Self>,
        caller: &str,
        caller_id: &str,
        name: &str,
        args: &Value,
        outputs: &[PathBuf],
    ) -> Res<Value> {
        if base::proc::current_cancellation().is_none() {
            return self
                .bridge
                .verification
                .scope(|| self.call(caller, caller_id, name, args, outputs));
        }
        if self.cancelled.load(Ordering::Acquire)
            || base::proc::current_cancellation().is_some_and(|c| c.cancelled())
        {
            return Err(Failure::new("interrupted", "session cancelled"));
        }
        self.step(caller_id)?;
        let short = name.strip_prefix("chronos_").unwrap_or(name);
        let authority_root = self.root.lock().unwrap().clone();
        let authority_state = authority::read(&self.home, &authority_root)?;
        authority::guard(&authority_state, caller, short, args)?;
        if short == "workflow" { return self.workflow_action(caller, args); }
        if caller == "core" && short == "goal" && args["status"] == "done"
            && authority_state.phase != authority::Phase::Submitted
            && goal::load(&self.home,&authority_root)?.is_some_and(|g| g.mode.as_deref()==Some("change")) {
            return Err(Failure::new("not_done","round must pass testing and workflow submit before goal completion"));
        }

        // Caller identity comes from the host thread, never an actor field in
        // the submitted JSON. Dispatcher cannot self-approve or recover a run.
        if short == "orch" && caller != "core" {
            if let Some(values) = args["args"].as_array() {
                let words: Vec<String> = values.iter().filter_map(Value::as_str).map(str::to_owned).collect();
                if orch::wait_mutation(&words) {
                    // Mirror bridge normalization and never discard malformed values for authorization.
                    let mut normalized = words.clone();
                    if normalized.first().is_some_and(|s| s == "--json") {
                        normalized.remove(0);
                    }
                    normalized.retain(|s| s != "--force");
                    let prepare = caller == "dispatcher"
                        && words.len() == values.len()
                        && normalized.first().is_some_and(|s| s == "wait")
                        && normalized.get(1).is_some_and(|s| s == "prepare");
                    let enrolled = prepare && args["project"].as_str()
                        .and_then(|project| orch::store_for(&self.home, project).ok())
                        .is_some_and(|store| store.join("wait-required").is_file());
                    if !enrolled {
                        return Err(Failure::new(
                            "permission_denied",
                            "only core may mutate checkpoint/release/recovery state; dispatcher may prepare an enrolled task",
                        ));
                    }
                }
            }
        }
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
        if short == "Write" {
            let target = tools::resolved(&tools::path(&cwd, string(args, "file_path")))?;
            if target.starts_with(tools::resolved(&self.home.join("authority"))?) {
                return Err(Failure::new("permission_denied", "authority state is Host-owned"));
            }
            let home = tools::resolved(&self.home.join("orchestrate"))?;
            let name = target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .trim_end_matches([' ', '.'])
                .to_ascii_lowercase();
            if target.starts_with(home)
                && ([
                    "core-wait.json",
                    "wait-required",
                    ".orch.lock",
                    ".orch.mutex",
                    ".worker.claim",
                    "units.tsv",
                ]
                .contains(&name.as_str())
                    || name.ends_with(".evidence.tsv"))
            {
                return Err(Failure::new(
                    "permission_denied",
                    "persisted lifecycle state must be changed through orch, not Write",
                ));
            }
        }
        if matches!(
            short,
            "Agent" | "orch" | "codex_run" | "run_on_linux" | "Write" | "Bash"
        ) && goal::load(&self.home, &root)?.is_some_and(|g| g.interjection.is_some())
        {
            return Err(Failure::new(
                "pending_interjection",
                "classify the pending interjection before proceeding",
            ));
        }
        // Verification commands may mutate the repository indirectly; apply the
        // same root intent and interjection gates before handing them to bridge.
        let verification = short == "run_on_linux"
            || (short == "orch"
                && args["args"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|v| matches!(v.as_str(), Some("accept" | "fulltest")))
                        || orch::wait_mutation(
                            &a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect::<Vec<_>>(),
                        )
                }));
        if verification {
            let _guard = self.goal_lock.lock().unwrap();
            if let Some(g) = goal::load(&self.home, &root)? {
                if g.interjection.is_some() {
                    return Err(Failure::new(
                        "pending_interjection",
                        "classify the pending interjection before verification",
                    ));
                }
                if g.mode.as_deref() != Some("change") {
                    return Err(Failure::new(
                        "goal_mode",
                        "verification commands require change mode",
                    ));
                }
            }
        }
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
            let dispatcher = args["subagent_type"] == "chronos:dispatcher";
            let _producer = dispatcher.then(|| Producer::new(&self.producers));
            let result = self.agent(caller, &cwd, args, outputs);
            if dispatcher {
                *self.dispatcher_failure.lock().unwrap() = match &result {
                    Ok(v) if base::tool_succeeded(v) => None,
                    Ok(v) => Some(format!("status={}; error={}", v["status"], v["error"])),
                    Err(e) => Some(format!("{}: {}", e.kind, e.message)),
                };
            }
            return result;
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
        if short == "codex_run" { args["parent_id"] = json!(caller_id); }
        if short == "goal" {
            args["session"] = json!(root);
        }
        if short == "codex_run"
            && !matches!(
                args["role"].as_str(),
                Some("terra" | "sol" | "astra-verify")
            )
        {
            return Err(Failure::new(
                "permission_denied",
                "dispatcher workers must use terra, sol, or read-only astra-verify; research uses the independent planner",
            ));
        }
        let _producer = (short == "codex_run").then(|| Producer::new(&self.producers));
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
        let mut result = result;
        if short == "codex_run" && result["outcome"] == "awaiting_core" {
            if let (Some(project),Some(unit)) = (args["project"].as_str(),args["unit"].as_str()) {
                let store = orch::store_for(&self.home,project)?;
                let passed=orch::wait::accept_tested_unit(&store,unit)?;
                result["outcome"] = json!(if passed {"verified"} else {"repair_required"});
                if !passed { result["next"]=json!("Read saved verifier findings; repair within the approved contract, then use orch wait prepare for the next attempt. No core technical verdict is required."); }
                result["core_notices"] = json!([]);
            }
        }
        if short == "goal" && args["classify"] == "stop" {
            self.cancelled.store(true, Ordering::Release);
            self.bridge.cancel();
        }
        Ok(result)
    }
    fn workflow_action(&self, caller: &str, args: &Value) -> Res<Value> {
        let root = self.root.lock().unwrap().clone();
        match args["action"].as_str() {
            Some("status") => Ok(json!({"status":"completed","workflow":authority::read(&self.home,&root)?})),
            Some("verdict") => {
                // Validate before releasing the ephemeral research context.
                let mut checked = authority::read(&self.home,&root)?;
                authority::check_research(&checked)?;
                authority::verdict(&mut checked,caller,args)?;
                if args["decision"] == "pass" {
                    if let Some(thread) = checked.research.as_ref().and_then(|r| r["thread_id"].as_str()) {
                        // Ephemeral threads have no rollout to archive. Release
                        // the subscription; never resume this context for planning.
                        let released=self.server.request("thread/unsubscribe",json!({"threadId":thread}))?;
                        if !matches!(released["status"].as_str(),Some("unsubscribed" | "notSubscribed" | "notLoaded")) {
                            return Err(Failure::new("protocol_incomplete","research context release was not acknowledged"));
                        }
                        self.models.lock().unwrap().remove(thread);
                        self.directories.lock().unwrap().remove(thread);
                        self.disabled_plugins.lock().unwrap().remove(thread);
                    }
                }
                let plan_only=goal::load(&self.home,&root)?.is_some_and(|g| matches!(g.mode.as_deref(),Some("answer" | "review")));
                authority::update(&self.home,&root,|s| {
                    let mut result=authority::verdict(s,caller,args)?;
                    if plan_only && args["decision"]=="pass" {
                        s.phase=authority::Phase::Research;
                        result["phase"]=json!(s.phase);
                    }
                    Ok(result)
                })
            },
            Some("ready") => {
                let state=authority::read(&self.home,&root)?;
                if caller != "dispatcher" || state.phase != authority::Phase::ReadySubmit {
                    return Err(Failure::new("permission_denied","independent chronos:tester must pass before ready_submit"));
                }
                Ok(json!({"status":"completed","workflow":state}))
            }
            Some("submit") => {
                let submit=true;
                if caller != "core" { return Err(Failure::new("permission_denied","only core submits a tested round")); }
                let g = goal::load(&self.home,&root)?.ok_or_else(|| invalid("missing goal"))?;
                let store = orch::store_for(&self.home,g.project.as_deref().ok_or_else(|| invalid("missing project"))?)?;
                let checkpoint=authority::read(&self.home,&root)?;
                if checkpoint.phase != authority::Phase::ReadySubmit { return Err(invalid("invalid workflow boundary")); }
                let tested:Value=serde_json::from_str(&checkpoint.reason).map_err(io)?;
                let current=orch::snapshot(&self.cwd,&[".".into()]);
                if current.is_empty() || tested["source"] != current {
                    authority::update(&self.home,&root,|s| {
                        if s.phase==authority::Phase::ReadySubmit { s.phase=authority::Phase::Execution; s.reason="Final tester source is stale; reverify before submission".into(); }
                        Ok(())
                    })?;
                    return Err(Failure::new("not_done","source changed after final testing"));
                }
                if let Err(error)=orch::round_evidence_gate(&store,&self.cwd) {
                    authority::update(&self.home,&root,|s| {
                        if s.phase==authority::Phase::ReadySubmit {
                            s.phase=authority::Phase::Execution;
                            s.reason="Submission evidence changed; dispatcher must reverify".into();
                        }
                        Ok(())
                    })?;
                    return Err(error);
                }
                authority::update(&self.home,&root,|s| {
                    let expected = if submit { authority::Phase::ReadySubmit } else { authority::Phase::Execution };
                    if s.phase != expected { return Err(invalid("invalid workflow boundary")); }
                    s.phase = if submit { authority::Phase::Submitted } else { authority::Phase::ReadySubmit };
                    Ok(json!({"status":"completed","phase":s.phase,"revision":s.revision}))
                })
            }
            _ => Err(invalid("unknown workflow action")),
        }
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
                && !(caller == "dispatcher"
                    && matches!(
                        name,
                        "content" | "reviewer" | "memory-keeper" | "astra" | "architect" | "critic" | "tester"
                    )))
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
            let root = self.root.lock().unwrap().clone();
            let report = self.plan(args)?;
            if report["status"] == "completed" {
                let root = self.root.lock().unwrap().clone();
                authority::update(&self.home, &root, |s| {
                    if s.phase != authority::Phase::ResearchRunning { return Err(invalid("stale research result")); }
                    s.revision += 1;
                    use sha2::{Digest,Sha256};
                    let bytes=std::fs::read(report["saved_to"].as_str().ok_or_else(|| invalid("missing research artifact"))?).map_err(io)?;
                    s.research = Some(json!({"sha256":format!("{:x}",Sha256::digest(bytes)),"artifact":report["saved_to"],"model":report["model"],"summary":report["intent_summary"],"thread_id":report["thread_id"]}));
                    s.phase = authority::Phase::IntentReview;
                    Ok(())
                })?;
            }
            else {
                authority::update(&self.home,&root,|s| { s.phase=authority::Phase::Research;Ok(()) })?;
            }
            return Ok(report);
        }
        let cwd = args["cwd"]
            .as_str()
            .map(|p| tools::path(caller_cwd, p))
            .unwrap_or_else(|| caller_cwd.into());
        base::require_directory(&cwd.to_string_lossy())?;
        if name == "dispatcher" {
            let project = args["project"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    goal::load(&self.home, &self.root.lock().unwrap())
                        .ok()
                        .flatten()
                        .and_then(|g| g.project)
                })
                .ok_or_else(|| invalid("dispatcher needs project"))?;
            // Bootstrap before the first model tool call; a new project's status
            // must not surface an opaque missing-directory error.
            let initialized = bridge::tools::dispatch(
                &self.bridge,
                "orch",
                &json!({"project":project,"args":["init"]}),
            )?;
            if initialized["status"] != "completed" {
                return Err(Failure::new("orch_init", initialized.to_string()));
            }

        }
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
        let final_source = if name == "tester" {
            let root=self.root.lock().unwrap().clone();
            let g=goal::load(&self.home,&root)?.ok_or_else(|| invalid("missing goal"))?;
            let store=orch::store_for(&self.home,g.project.as_deref().ok_or_else(|| invalid("missing project"))?)?;
            orch::round_evidence_gate(&store,&cwd)?;
            let source=orch::snapshot(&cwd,&[".".into()]);
            if source.is_empty() { return Err(Failure::new("not_done","unable to identify final test source")); }
            Some(source)
        } else { None };
        if name == "architect" {
            let root = self.root.lock().unwrap().clone();
            authority::update(&self.home,&root,|s| {
                if s.phase != authority::Phase::Planning { return Err(invalid("planning already running")); }
                authority::check_research(s)?;
                s.phase=authority::Phase::PlanningRunning;Ok(())
            })?;
        }
        let thread = self.start_as(name, None, false, &cwd, reviewer_role)?;
        // Dispatcher receives delegation authority, never direct write authority.
        self.register(&thread, name, outputs)?;
        let timeout=args["timeout_s"].as_u64().filter(|n| *n > 0).unwrap_or(1800);
        let root=self.root.lock().unwrap().clone();
        let result = if name=="dispatcher" && authority::read(&self.home,&root)?.research.is_some() {
            self.dispatch_until_boundary(&thread,string(args,"prompt"),timeout)
        } else {
            self.turn(&thread, name, string(args, "prompt"), None,timeout)
        };
        self.server.unregister_tools(&thread);
        if name == "tester" {
            let report=result.as_ref().map_err(Clone::clone)?;
            let root=self.root.lock().unwrap().clone();
            let artifact=self.home.join("authority").join(&root).join(format!("tester-{}.json",base::unique()));
            base::atomic_write(&artifact,&serde_json::to_vec_pretty(report).map_err(io)?).map_err(io)?;
            let verdict: Value=serde_json::from_str(report["final_message"].as_str().unwrap_or("")).map_err(io)?;
            if report["status"] != "completed" || !verdict["issues"].as_array().is_some_and(|a| a.is_empty())
                || verdict["severity"] != "normal" || verdict["evidence"].as_str().is_none_or(|s| s.trim().is_empty()) {
                return Err(Failure::new("verification_failed",json!({"verdict":verdict,"artifact":artifact,"repair":"Dispatcher owns repair. If source changes are required use implementation; for unchanged-source re-verification/repackaging use DELIVERY verification, never invent source edits. Preserve existing accepted evidence and do not rerun a released unit unless its source or required checks change."}).to_string()));
            }
            if final_source.as_deref() != Some(orch::snapshot(&cwd,&[".".into()]).as_str()) {
                return Err(invalid("source changed during final testing"));
            }
            authority::update(&self.home,&root,|s| {
                if s.phase != authority::Phase::Execution { return Err(invalid("stale tester result")); }
                s.phase=authority::Phase::ReadySubmit;
                s.reason=serde_json::to_string(&json!({"tester":report["model"],"thread":report["thread_id"],"source":final_source,"artifact":artifact,"summary":verdict["evidence"].as_str().unwrap_or("").chars().take(1000).collect::<String>()})).map_err(io)?;
                Ok(())
            })?;
        }
        if name == "architect" {
            let root = self.root.lock().unwrap().clone();
            let validation = if result.as_ref().is_ok_and(|v| v["status"] == "completed") {
                (|| -> Res<()> {
                let g = goal::load(&self.home, &root)?.ok_or_else(|| invalid("missing goal"))?;
                let store = orch::store_for(&self.home, g.project.as_deref().ok_or_else(|| invalid("missing project"))?)?;
                let map = std::fs::read_to_string(store.join("modules.md")).map_err(io)?;
                orch::parse_map(&map).map_err(invalid)?;
                orch::wait::enroll(&store)?;
                authority::update(&self.home, &root, |s| {
                    if s.phase != authority::Phase::PlanningRunning { return Err(invalid("stale planning result")); }
                    s.phase = authority::Phase::Execution;
                    Ok(())
                })?;
                Ok(())
                })()
            } else { Ok(()) };
            // A completed but invalid plan can be repaired by dispatcher.
            // Unknown transport termination keeps the reservation quarantined.
            if validation.is_err() || result.as_ref().is_ok_and(|v| matches!(v["status"].as_str(),Some("failed" | "blocked"))) {
                authority::update(&self.home,&root,|s| {
                    if s.phase==authority::Phase::PlanningRunning {
                        s.phase=authority::Phase::Planning;
                        s.reason="Planning did not produce an accepted module map; dispatcher must repair".into();
                    }
                    Ok(())
                })?;
            }
            validation?;
        }
        if name == "dispatcher" {
            let root = self.root.lock().unwrap().clone();
            let project = args["project"].as_str().map(str::to_owned).or(
                goal::load(&self.home, &root)?.and_then(|g| g.project))
                .ok_or_else(|| invalid("dispatcher needs active goal"))?;
            let store = orch::store_for(&self.home, &project)?;
            let saved = planner_destination(
                &store,
                &store.join(format!("reports/dispatcher-{}.json", base::unique())),
            )?;
            // A transport failure is not evidence that the dispatcher or its
            // descendants stopped. Keep its ownership and uncertainty durable
            // before propagating the failure to the parent.
            let mut report = match &result {
                Ok(report) => report.clone(),
                Err(e) => json!({"status":"unknown","thread_id":thread,
                    "goal":root,"project":project,
                    "error":{"kind":e.kind,"message":e.message},
                    "termination":"unknown"}),
            };
            base::atomic_write(&saved, &serde_json::to_vec_pretty(&report).map_err(io)?)
                .map_err(io)?;
            if let Err(e) = result {
                return Err(Failure::new(
                    &e.kind,
                    format!("{}; dispatcher evidence: {}", e.message, saved.display()),
                ));
            }
            // The dispatcher is still a live producer until its reply is ready.
            // Store contention here is transient, not a dispatcher failure.
            let core_notices = loop {
                match orch::wait::core_notices(&store) {
                    Err(e) if e.kind == "busy" => {
                        // The terminal report is already durable. Do not discard its
                        // pointer, or wait indefinitely for notices, on cancellation.
                        if self.cancelled.load(Ordering::Acquire)
                            || base::proc::current_cancellation().is_some_and(|c| c.cancelled())
                        {
                            break vec![];
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    result => break result?,
                }
            };
            for event in &core_notices {
                // A synchronous dispatcher reply is already delivery to core.
                // Keep the durable outbox pending until a bound verdict, but do
                // not schedule another model turn merely to replay this reply.
                self.core_notice_delivered(event);
                self.notice(json!({"hostEvent":"coreNotice","recipient":"core","event":event}))?;
            }
            if (self.cancelled.load(Ordering::Acquire)
                || base::proc::current_cancellation().is_some_and(|c| c.cancelled()))
                && report["status"] != "interrupted"
            {
                interrupted_report(&mut report, "owning request cancelled; terminal status retained in artifact");
                base::atomic_write(&saved, &serde_json::to_vec_pretty(&report).map_err(io)?)
                    .map_err(io)?;
            }
            let state = if report["status"] == "completed" { orch::wait_state(&store)? } else { orch::WaitState::default() };
            let operations: Vec<_> = state.operations.iter().filter(|op|
                orch::current_checkpoint(&state, &op.task).is_some_and(|c| c.round == op.round)
                    && !state.events.iter().any(|e| e.kind == "recovery" && e.dispatch_id == op.id)
            ).map(|op| {
                let mut reply = op.reply();
                if !reply["error"].is_null() {
                    reply["error"] = json!({"kind":reply["error"]["kind"],"message":"See durable operation outbox for the complete blocking cause."});
                    reply["blocking_cause"] = reply["error"].clone();
                }
                reply
            }).collect();
            let units = if report["status"] == "completed" { orch::read_units(&store)? } else { vec![] };
            let mut verified = !units.is_empty() && units.iter().all(|u| u.state == "verified");
            if verified {
                for unit in &units { verified &= orch::current_verified_unit(&store, &unit.id, &cwd)?; }
            }
            let outcome = if report["status"] != "completed" {
                if report["outcome"] == "timeout" { "timeout" }
                else if report["status"] == "interrupted" { "cancelled" }
                else if report["error"]["kind"] == "turn_unconfirmed" { "unconfirmed" }
                else { "failed" }
            } else if let Some(outcome) = ["unconfirmed", "protocol_incomplete", "failed", "timeout", "cancelled", "blocked"]
                .into_iter().find(|kind| operations.iter().any(|op| op["outcome"] == *kind)) {
                outcome
            } else if verified {
                "verified"
            } else if !core_notices.is_empty() || operations.iter().any(|op| op["outcome"] == "awaiting_core") {
                "awaiting_core"
            } else if !orch::wait::idle_blockers(&store)?.is_empty() {
                "blocked"
            } else {
                if !units.is_empty() && units.iter().all(|u| u.state == "verified") { "blocked" }
                else if !units.is_empty() && units.iter().all(|u| matches!(u.state.as_str(), "built" | "verified")) { "awaiting_core" }
                else { "protocol_incomplete" }
            };
            report["transport_status"] = report["status"].clone();
            report["outcome"] = json!(outcome);
            // Rust journal, not final_message, is the domain authority. Persist
            // identity and deduplicatable delivery before returning to core.
            report["domain_result"] = json!({"outcome":outcome,"operation_id":format!("dispatcher:{thread}"),
                "parent_id":root,"owner":thread,"owner_generation":1,"stage":"dispatcher_reply",
                "delivery_id":format!("dispatcher:{thread}:reply"),"operations":operations});
            if !matches!(outcome, "awaiting_core" | "verified") && report["status"] == "completed" {
                report["status"] = json!(if outcome == "blocked" { "blocked" } else { "failed" });
                report["error"] = json!({"kind":outcome,"message":"Rust domain result does not establish successful engineering completion"});
            }
            base::atomic_write(&saved, &serde_json::to_vec_pretty(&report).map_err(io)?).map_err(io)?;
            let error = if report["error"].is_null() {
                Value::Null
            } else {
                json!({"kind":report["error"]["kind"].as_str().unwrap_or("dispatcher_failed").chars().take(80).collect::<String>(),"message":"Dispatcher failure; see saved_to for complete evidence."})
            };
            // The root gets transport status and artifact pointers, never worker
            // reports (even if a model ignores its bounded-summary instructions).
            let flow = authority::read(&self.home, &root)?;
            if matches!(flow.phase, authority::Phase::IntentReview | authority::Phase::ReadySubmit) {
                return Ok(json!({"status":"completed","outcome":"awaiting_core","workflow":flow,"saved_to":saved,"usage":report["usage"]}));
            }

            return Ok(
                json!({"status":report["status"],"transport_status":report["transport_status"],"outcome":report["outcome"],"result":report["domain_result"],"core_notices":core_notices,"summary":"Dispatcher turn ended; engineering acceptance remains governed by the goal evidence gate.","goal":root,"unit":null,"attempt":null,"recipient":"core","sender":thread,"model":report["model"],"usage":report["usage"],"saved_to":saved,"artifacts":[saved],"error":error,"unknowns":["Installed-version acceptance is not implied by model completion."],"next":"Use goal evidence gate; ordinary repair stays in dispatcher subtree."}),
            );
        }
        result
    }
    /// Hold the caller's one command until a real authority boundary. A model
    /// turn ending is not an engineering stage ending, and never wakes core to
    /// ask whether the dispatcher should continue. No model runs while awaiting
    /// an active child; continuation occurs only after dispatcher turn completion.
    fn dispatch_until_boundary(&self, thread: &str, prompt: &str, timeout: u64) -> Res<Value> {
        let root=self.root.lock().unwrap().clone();
        let g=goal::load(&self.home,&root)?.ok_or_else(||invalid("missing goal"))?;
        let store=orch::store_for(&self.home,g.project.as_deref().ok_or_else(||invalid("missing project"))?)?;
        let progress=|| -> Res<String> {
            use sha2::{Digest,Sha256};
            let mut hash=Sha256::new();
            hash.update(serde_json::to_vec(&authority::read(&self.home,&root)?).map_err(io)?);
            for file in ["units.tsv","core-wait.json"] {
                match std::fs::read(store.join(file)) {
                    Ok(bytes)=>hash.update(bytes),
                    Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
                    Err(e)=>return Err(io(e)),
                }
            }
            Ok(format!("{:x}",hash.finalize()))
        };
        let mut previous=progress()?;
        let mut stalled=0;
        let start=Instant::now();
        let mut input=prompt.to_string();
        for step in 0..16 {
            let remaining=timeout.saturating_sub(start.elapsed().as_secs());
            if remaining==0 { return Err(Failure::new("timeout","dispatcher boundary budget exhausted")); }
            let report=self.turn(thread,"dispatcher",&input,None,remaining)?;
            let artifact=store.join(format!("reports/dispatcher-step-{}-{step}.json",thread));
            base::atomic_write(&artifact,&serde_json::to_vec_pretty(&report).map_err(io)?).map_err(io)?;
            if report["status"]!="completed" { return Ok(report); }
            let flow=authority::read(&self.home,&root)?;
            if matches!(flow.phase,authority::Phase::IntentReview | authority::Phase::ReadySubmit | authority::Phase::Submitted) {
                return Ok(report);
            }
            if matches!(flow.phase,authority::Phase::ResearchRunning | authority::Phase::PlanningRunning) {
                return Err(Failure::new("turn_unconfirmed","reserved specialist did not reach a confirmed boundary"));
            }
            let current=progress()?;
            stalled=if current==previous { stalled+1 } else { 0 };
            if stalled>=2 {
                return Err(Failure::new("dispatcher_stalled",format!("dispatcher ended twice without durable progress; evidence: {}",artifact.display())));
            }
            previous=current;
            input=format!("Rust has retained this command; core is not involved. Current phase: {:?}. Continue the owned workflow through its next required stage. Repair your own brief/registration errors using DISPATCH.md. Do not return merely because planning or a unit finished. Complete full testing and independent chronos:tester before returning ready_submit. Local previous-turn evidence: {}",flow.phase,artifact.display());
        }
        Err(Failure::new("dispatcher_stalled","dispatcher exceeded 16 completed-stage continuations without a core boundary"))
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
        // Validate all local arguments before reserving the running phase: a bad
        // destination/schema has not launched a worker and must remain retryable.
        let root = self.root.lock().unwrap().clone();
        authority::update(&self.home,&root,|s| {
            if s.phase != authority::Phase::Research { return Err(invalid("research already running")); }
            s.phase=authority::Phase::ResearchRunning;Ok(())
        })?;
        // Each Agent call is independent, with its own cwd/search policy and handler.
        let planner = self.start_at("astra", None, args["web_search"] == true, &cwd)?;
        self.register(&planner, "astra", vec![])?;
        let result = self.turn(&planner, "astra", string(args, "prompt"), schema.as_ref(), 1800);
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
            report["intent_summary"] = json!(value["intent_summary"].as_str().unwrap_or(report["final_message"].as_str().unwrap_or("")).chars().take(2000).collect::<String>());
            report["saved_to"] = json!(dest);
            report["coverage"] = value.get("coverage").cloned().unwrap_or(Value::Null);
            if matches!(value["mode"].as_str(), Some("change" | "review" | "answer")) {
                report["recommended_mode"] = value["mode"].clone();
            }
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
    fn turn(&self, thread: &str, name: &str, prompt: &str, schema: Option<&Value>, timeout_s: u64) -> Res<Value> {
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
            let started = match self.server.request("turn/start", params) {
                Ok(v) => v,
                Err(e)
                    if matches!(
                        e.kind.as_str(),
                        "timeout" | "turn_unconfirmed" | "codex_exited"
                    ) =>
                {
                    self.server.mark_unconfirmed();
                    self.server.close();
                    self.server.mark_unconfirmed();
                    return Err(Failure::new(
                        "turn_unconfirmed",
                        "turn/start unconfirmed; connection closed, affecting concurrent calls",
                    ));
                }
                Err(e) => return Err(e),
            };
            let Some(turn) = started["turn"]["id"].as_str() else {
                self.server.mark_unconfirmed();
                self.server.close();
                return Err(Failure::new(
                    "turn_unconfirmed",
                    "turn/start omitted turn id; connection closed, affecting concurrent calls",
                ));
            };
            self.server.register_turn(thread, turn);
            let mut report = json!({"status":"failed","model":model,"thread_id":thread,"turn_id":turn,"final_message":null,"usage":null,"error":null});
            // Dispatcher orchestration has a bounded model budget, but a
            // synchronous engineering callback owns its own persisted deadline.
            // Never cut Terra -> Sol -> Rust -> Astra at an unrelated outer 1800s.
            let mut budget = TurnBudget::new(Duration::from_secs(timeout_s));
            let cancellation = base::proc::current_cancellation();
            let result = loop {
                let callback = name == "dispatcher" && self.callbacks.lock().unwrap()
                    .get(thread).is_some_and(|n| *n > 0);
                budget.tick(Instant::now(), callback);
                if self.budget_exhausted.load(Ordering::Acquire)
                    || self.cancelled.load(Ordering::Acquire)
                    || cancellation.as_ref().is_some_and(|c| c.cancelled())
                    || super::run::interrupted()
                    || budget.remaining.is_zero()
                {
                    let interrupt = self.server.interrupt(thread, turn);
                    let terminal = if interrupt.is_ok() {
                        let until = Instant::now() + Duration::from_secs(2);
                        loop {
                            match events
                                .recv_timeout(until.saturating_duration_since(Instant::now()))
                            {
                                Ok(e)
                                    if e["method"] == "turn/completed"
                                        && e["params"]["turn"]["id"] == turn =>
                                {
                                    self.log(&e)?;
                                    record(&mut report, &e);
                                    break true;
                                }
                                Ok(_) if Instant::now() < until => continue,
                                _ => break false,
                            }
                        }
                    } else {
                        false
                    };
                    if terminal {
                        self.server.unregister_turn(thread, turn);
                        interrupted_report(
                            &mut report,
                            "deadline or cancellation reached; terminal status retained in artifact",
                        );
                        report["outcome"] = json!(if budget.remaining.is_zero() { "timeout" } else { "cancelled" });
                        return Ok(report);
                    }
                    self.server.mark_unconfirmed();
                    return Err(Failure::new(
                        "turn_unconfirmed",
                        "interrupt failed or terminal event unconfirmed; turn may still be running",
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
                    let _ = self.server.interrupt(thread, turn);
                    break Err(Failure::new(
                        "turn_unconfirmed",
                        format!("{e:?}; terminal event unconfirmed"),
                    ));
                }
                record(&mut report, &event);
                if event["method"] == "turn/completed" && event["params"]["turn"]["id"] == turn {
                    budget.tick(Instant::now(), callback);
                    if budget.remaining.is_zero() {
                        interrupted_report(&mut report, "late completion after outer model deadline");
                        report["outcome"] = json!("timeout");
                    }
                    break Ok(report);
                }
            };
            if result.is_ok() {
                self.server.unregister_turn(thread, turn);
            } else {
                self.server.mark_unconfirmed();
            }
            result
        })();
        self.server.unsubscribe(thread);
        result.map(|mut report| {
            // A nested callback can finish as the owner is cancelled and race the
            // parent's interrupt. Confirmed late completion is terminal evidence,
            // not successful completion of the cancelled request.
            if self.cancelled.load(Ordering::Acquire)
                || base::proc::current_cancellation().is_some_and(|c| c.cancelled())
            {
                if report["status"] != "interrupted" {
                    interrupted_report(
                        &mut report,
                        "owning request cancelled; terminal status retained in artifact",
                    );
                }
            }
            report
        })
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
