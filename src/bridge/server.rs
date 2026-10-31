use crate::base::{self, Failure, Res};
use crate::codex::AppServer;
use serde_json::{json, Value};
use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

pub struct Config {
    pub data_home: Option<PathBuf>,
    pub codex: Res<Vec<String>>,
    pub agy: Res<Vec<String>>,
    pub wsl: Option<Vec<String>>,
    pub on_windows: bool,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            data_home: base::data_home().ok(),
            codex: base::resolve_command("AXIOM_CODEX_CMD", "codex"),
            agy: base::resolve_command("AXIOM_AGY_CMD", "agy"),
            wsl: base::resolve_command("AXIOM_WSL_CMD", "wsl.exe").ok(),
            on_windows: cfg!(windows),
        }
    }
}
pub struct Bridge {
    pub config: Config,
    pub(crate) verification: base::proc::Cancellation,
    codex: Mutex<Option<Arc<AppServer>>>,
    pub(crate) attachment: Mutex<Option<Arc<crate::session::Attachment>>>,
}
impl Bridge {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            verification: base::proc::Cancellation::default(),
            codex: Mutex::new(None),
            attachment: Mutex::new(None),
        }
    }
    pub fn codex(&self) -> Res<Arc<AppServer>> {
        let mut lock = self.codex.lock().unwrap_or_else(|e| e.into_inner());
        if lock.is_none() {
            *lock = Some(Arc::new(AppServer::new(self.config.codex.clone()?)));
        }
        Ok(lock.as_ref().unwrap().clone())
    }
    pub fn home(&self) -> Res<&Path> {
        self.config
            .data_home
            .as_deref()
            .ok_or_else(|| Failure::new("config", "data home is not configured"))
    }
    pub fn running(&self) -> bool {
        self.codex
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(false, |c| c.alive())
    }
    pub fn cancel(&self) {
        self.verification.cancel();
        if let Some(attachment) = self
            .attachment
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .cloned()
        {
            attachment.stop();
        }
        if let Some(c) = self
            .codex
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            c.cancel_all();
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(c) = self
            .codex
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            c.close();
        }
    }
}
pub fn tools() -> Value {
    let mut v: Value = serde_json::from_str(DESCRIPTORS).unwrap();
    let a = v.as_array_mut().unwrap();
    a[0]["inputSchema"]["required"] = json!(["cwd"]);
    a[0]["inputSchema"]["properties"]["stop_after"] = json!({"type":"string","enum":["build","assembly","host_test","package","verification"],
        "description":"Enrolled operation only: persist a stage boundary and return typed blocked. Call the same project/unit to resume, never redispatch it. timeout_s is one durable total budget (default 7200 seconds), not renewed by resuming."});
    a[9]["description"] =
        json!("Manage a session goal: set status, classify an interjection, or set mode.");
    a[9]["inputSchema"]["properties"]["classify"] = json!({"type":"string"});
    a[9]["inputSchema"]["properties"]["mode"] = json!({"type":"string"});
    a[9]["inputSchema"]["required"] = json!(["session"]);
    a[1]["description"] = json!("Run a read-only Codex rewrite or independent second review in an empty context. Gemini model names are compatibility aliases; the result reports the resolved Codex model.");
    a[3]["description"] = json!("Review files against their contract with a distinct Codex model role. A major or blocker finding fails the verdict. This is role independence within OpenAI, not cross-provider independence.");
    a[1]["inputSchema"]["properties"]["cwd"]["description"] = json!(
        "Optional directory validated for compatibility; the review runs in an empty directory."
    );
    a[10]["description"] = json!("Report Codex executable and app-server status; agy is disabled.");
    a[2]["description"] = json!("Orchestration bookkeeping. Opt in with wait enroll; every writer then requires wait prepare --task ID --repo ABS --issue ISSUE --proposal PROPOSAL (or wait checkpoint --file JSON). prepare derives the actual contract and next round; it never invents execution evidence. Enrolled codex_run role terra drives actual Terra HIGH, Sol HIGH, Rust build/package and read-only Astra HIGH, plus pre-writer Astra MAX from round six. wait show returns durable evidence. Core uses wait verdict --task ID --verification EVENT_ID --decision approve|reject --assessment JSON (valid_issues, severity, optional decision continue|finish), then wait complete --task ID --verification EVENT_ID. Recovery: wait recover --file EVENT --confirmed not_started|stopped only after independent stopped confirmation; PID death alone is insufficient. Local CLI/bridge is trusted, not cryptographic authentication. No remote transactional exactly-once notification or remote turn-status API is supplied; core response notices use durable replay-safe outbox ids.");
    for (i, names) in [
        (0, vec!["project", "unit", "save_to", "stage"]),
        (1, vec!["project", "stage", "save_to"]),
        (3, vec!["project", "units"]),
    ] {
        for n in names {
            a[i]["inputSchema"]["properties"][n] = if n == "units" {
                json!({"type":"array","items":{"type":"string"}})
            } else {
                json!({"type":"string"})
            };
        }
    }
    a[1]["inputSchema"]["properties"]["stage"]["enum"] = json!(["second_review", "rewrite"]);
    a.push(json!({"name":"session","description":"Attach the external caller as root; begin, resume, call, interject, status or stop. Keep stdio open for multiple calls.","inputSchema":{"type":"object","properties":{"action":{"type":"string"},"session":{"type":"string"},"project":{"type":"string"},"skill":{"type":"string"},"text":{"type":"string"},"tool":{"type":"string"},"arguments":{"type":"object"}},"required":["action"]}}));
    v
}
const DESCRIPTORS: &str = r###"[{"name": "codex_run", "description": "Run one Codex turn in a fresh ephemeral thread and return a structured report. Write `brief` per `${CLAUDE_PLUGIN_ROOT}/DISPATCH.md` (GOAL, SCOPE, VERIFY): the worker sees nothing else. Use role terra for modular units, role sol for integration and role astra for research plans and scientific rulings. Status is completed, failed, interrupted, timeout or bridge_error; anything but completed carries `error`.", "inputSchema": {"type": "object", "properties": {"brief": {"type": "string", "description": "Full worker brief; sent as the only user message."}, "cwd": {"type": "string", "description": "Absolute working directory, ideally the unit's own worktree."}, "role": {"type": "string", "enum": ["terra", "sol", "astra", "astra-verify"], "description": "Resolved to a model this Codex account lists; the report names it."}, "task_kind": {"type": "string", "enum": ["verification", "measurement"]}, "model": {"type": "string", "description": "Explicit model id; overrides role."}, "sandbox": {"type": "string", "enum": ["read-only", "workspace-write"], "default": "workspace-write"}, "effort": {"type": "string", "description": "Reasoning effort advertised by the model, e.g. low, medium, high."}, "developer_instructions": {"type": "string", "description": "Role contract layered over the user's Codex AGENTS.md."}, "output_schema": {"type": "object", "description": "JSON Schema for the final message; parsed into structured_output."}, "timeout_s": {"type": "integer", "minimum": 10, "default": 1800}, "lean": {"type": "boolean", "default": true, "description": "For this thread only, disable every Codex plugin, configured MCP server, the apps connector, goals, the skills catalog, the personality section and (unless web_search) the web search tool."}, "web_search": {"type": "boolean", "default": false, "description": "Keep the web search tool in lean mode; costs ~2.5k input tokens per call."}, "keep_plugins": {"type": "array", "items": {"type": "string"}, "description": "Plugin ids such as name@marketplace that stay enabled in lean mode."}}, "required": ["brief", "cwd"]}}, {"name": "gemini_preview", "description": "Have a Gemini model (through Antigravity, agy) rewrite worker reports into plain language for the user: what was done, how it turned out, what the user must decide. Put the reports in `prompt`. Without `cwd` it runs in an empty directory, so there is nothing for it to change. Status is completed, failed, timeout or bridge_error; an empty reply is failed with kind empty_response.", "inputSchema": {"type": "object", "properties": {"prompt": {"type": "string"}, "cwd": {"type": "string", "description": "Absolute directory agy may read; omit to run in an empty directory."}, "model": {"type": "string", "default": "gemini-4-argon-medium"}, "json_schema": {"type": "object", "description": "Optional JSON Schema; result returned as structured_output."}, "timeout_s": {"type": "integer", "minimum": 10, "default": 600}}, "required": ["prompt"]}}, {"name": "orch", "description": "Run Pstack's orch bookkeeping CLI (units, inbox, ledger, gates, standing orders, status) on the store for `project`. The store lives in the plugin data directory and survives plugin updates; its path is returned as `store`.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "description": "Lowercase slug naming the program, e.g. todo-stats."}, "args": {"type": "array", "items": {"type": "string"}, "description": "orch arguments, e.g. [\"unit\", \"add\", \"u1\", \"--track\", \"build\"]."}}, "required": ["project", "args"]}}, {"name": "review_code", "description": "Have a Gemini model, a different family from the Codex workers, review files against the contract they were built to: contract mismatches, checks that cannot fail, untested promises and platform dependence. Pass every file of the module, tests included. The verdict is fail when any finding is a blocker or major.", "inputSchema": {"type": "object", "properties": {"project_dir": {"type": "string"}, "paths": {"type": "array", "items": {"type": "string"}, "description": "Files relative to project_dir, tests included."}, "contract": {"type": "string", "description": "The module's CONTRACT and the global rules, or the wish when there is no map."}, "model": {"type": "string", "default": "gemini-4-argon-high"}, "timeout_s": {"type": "integer", "minimum": 60, "default": 900}}, "required": ["project_dir", "paths", "contract"]}}, {"name": "run_on_linux", "description": "Run `python <args>` in the project on Linux through WSL, with ~/.axiom/linux-venv. Windows runs skip or mask POSIX-only tests, so a full suite that passes here can still fail on Linux. Status unavailable means there is no Linux runner, which is not a pass.", "inputSchema": {"type": "object", "properties": {"project_dir": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}, "description": "Arguments to python, e.g. [\".axiom-experiment/check_baseline.py\"]."}, "timeout_s": {"type": "integer", "minimum": 60, "default": 1800}}, "required": ["project_dir", "args"]}}, {"name": "memory_record", "description": "Store one verified unit as long-run memory: what it built, its interface, and what a later worker reusing it must know. Files are hashed so later searches can flag the entry stale once they change.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "description": "Program slug, the same one orch uses."}, "project_dir": {"type": "string", "description": "Absolute project directory; files are relative to it."}, "unit": {"type": "string"}, "title": {"type": "string", "description": "The module, function or feature, as a later brief would name it."}, "summary": {"type": "string", "description": "One or two lines: what it does and its interface."}, "body": {"type": "string", "description": "What a reuser needs: signatures, behaviour decisions, verify command, pitfalls met."}, "symbols": {"type": "array", "items": {"type": "string"}}, "files": {"type": "array", "items": {"type": "string"}}, "wish": {"type": "string", "description": "The wish verbatim, in the user's words; indexed so searches in their language find the entry."}}, "required": ["project", "project_dir", "unit", "title", "summary", "body", "wish"]}}, {"name": "memory_search", "description": "Find earlier units related to `query` (English words and Chinese both match). Returns titles and summaries only, with `stale` true when a recorded file has changed since; fetch a body with memory_get.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "project": {"type": "string", "description": "Limit to one program; omit to search every project."}, "limit": {"type": "integer", "minimum": 1, "maximum": 20, "default": 5}}, "required": ["query"]}}, {"name": "memory_get", "description": "Return one memory entry in full, with its files and whether it is stale.", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}}, {"name": "memory_digest", "description": "Read a project's short history of wishes and outcomes, one line each; pass `add` to append a line first.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string"}, "add": {"type": "string", "description": "One line: the wish, the outcome, memory ids."}}, "required": ["project"]}}, {"name": "goal", "description": "Close the wish goal of this Claude session: status done once the wish is carried out and verified, or blocked with a note when it cannot go on without something only the user has. The session id is the one in the goal notice.", "inputSchema": {"type": "object", "properties": {"session": {"type": "string"}, "status": {"type": "string", "enum": ["done", "blocked"]}, "note": {"type": "string"}}, "required": ["session", "status"]}}, {"name": "bridge_status", "description": "Report which Codex, agy and Bun executables the bridge resolved and their versions, whether Java and tla2tools.jar are configured for statechart, and whether the Codex app-server is running.", "inputSchema": {"type": "object", "properties": {}}}]"###;
pub fn call(bridge: &Bridge, name: &str, args: &Value) -> Value {
    call_body(|| {
        if !matches!(name,"session" | "bridge_status") && bridge.attachment.lock().unwrap().is_some() {
            return Err(Failure::new("permission_denied","attached callers must use session tools; raw bridge calls cannot bypass role authority"));
        }
        super::tools::dispatch(bridge, name, args)
    })
}
#[doc(hidden)]
pub fn call_body(body: impl FnOnce() -> Res<Value>) -> Value {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    let mut payload = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => json!({"status":"bridge_error","error":e.to_json()}),
        Err(_) => {
            json!({"status":"bridge_error","error":{"kind":"panic","message":"tool panicked"}})
        }
    };
    let failed = !base::tool_succeeded(&payload);
    if failed && payload.get("error").map_or(true, Value::is_null) {
        payload["error"] = json!({"kind":"tool_failed","message":"tool did not complete"});
    }
    json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&payload).unwrap()}],"structuredContent":payload,"isError":failed})
}
pub fn serve(config: Config, input: impl BufRead, output: impl Write + Send + 'static) -> i32 {
    let bridge = Arc::new(Bridge::new(config));
    let out = Arc::new(Mutex::new(output));
    let mut jobs: Vec<(Value, base::proc::Cancellation, thread::JoinHandle<()>)> = Vec::new();
    const MAX_IN_FLIGHT: usize = 32;
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_msg(
                    &out,
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}}),
                );
                continue;
            }
        };
        let mut i = 0;
        while i < jobs.len() {
            if jobs[i].2.is_finished() {
                let (_, _, job) = jobs.swap_remove(i);
                let _ = job.join();
            } else {
                i += 1;
            }
        }
        let Some(id) = msg.get("id").cloned() else {
            if msg["method"] == "notifications/cancelled" {
                if let Some(id) = msg.pointer("/params/requestId") {
                    for (job_id, cancel, _) in &jobs {
                        if job_id == id {
                            cancel.cancel();
                        }
                    }
                }
            }
            continue;
        };
        let method = msg["method"].as_str().unwrap_or("").to_owned();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        if method == "tools/call" {
            if jobs.len() >= MAX_IN_FLIGHT {
                write_msg(
                    &out,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"server busy"}}),
                );
                continue;
            }
            let b = bridge.clone();
            let o = out.clone();
            let cancel = base::proc::Cancellation::default();
            let scoped = cancel.clone();
            let job_id = id.clone();
            let job = thread::spawn(move || {
                let result = scoped.scope(|| {
                    call(
                        &b,
                        params["name"].as_str().unwrap_or(""),
                        params.get("arguments").unwrap_or(&Value::Null),
                    )
                });
                write_msg(&o, json!({"jsonrpc":"2.0","id":id,"result":result}));
            });
            jobs.push((job_id, cancel, job));
            continue;
        }
        let result = match method.as_str() {
            "initialize" => {
                json!({"protocolVersion":params.get("protocolVersion").cloned().unwrap_or(json!("2025-06-18")),"capabilities":{"tools":{}},"serverInfo":{"name":"chronos-bridge","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools":tools()}),
            _ => {
                write_msg(
                    &out,
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not found: {method}")}}),
                );
                continue;
            }
        };
        write_msg(&out, json!({"jsonrpc":"2.0","id":id,"result":result}));
    }
    for (_, cancel, _) in &jobs {
        cancel.cancel();
    }
    for (_, _, job) in jobs {
        let _ = job.join();
    }
    drop(bridge);
    0
}
fn write_msg<W: Write>(out: &Mutex<W>, msg: Value) {
    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = serde_json::to_writer(&mut *o, &msg);
    let _ = o.write_all(b"\n");
    let _ = o.flush();
}
