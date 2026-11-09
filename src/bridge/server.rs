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
    pub wsl: Option<Vec<String>>,
    pub on_windows: bool,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            data_home: base::data_home().ok(),
            codex: base::resolve_command("AXIOM_CODEX_CMD", "codex"),
            wsl: base::resolve_command("AXIOM_WSL_CMD", "wsl.exe").ok(),
            on_windows: cfg!(windows),
        }
    }
}
pub struct Bridge {
    pub config: Config,
    codex: Mutex<Option<Arc<AppServer>>>,
}
impl Bridge {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            codex: Mutex::new(None),
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
    for tool in v.as_array_mut().unwrap() {
        match tool["name"].as_str().unwrap() {
            "codex_run" => {
                tool["inputSchema"]["required"] = json!(["cwd"]);
                for name in ["project", "unit", "save_to", "stage", "track", "brief_file"] {
                    tool["inputSchema"]["properties"][name] = json!({"type":"string"});
                }
            }
            "goal" => {
                tool["description"] = json!(
                    "Manage a session goal: set status, classify an interjection, or set mode."
                );
                tool["inputSchema"]["properties"]["classify"] = json!({"type":"string"});
                tool["inputSchema"]["properties"]["mode"] = json!({"type":"string"});
                tool["inputSchema"]["required"] = json!(["session"]);
            }
            _ => {}
        }
    }
    v
}
const DESCRIPTORS: &str = r###"[{"name": "codex_run", "description": "Run one Codex turn in a fresh ephemeral thread and return a structured report. Write `brief` per `${CLAUDE_PLUGIN_ROOT}/DISPATCH.md` (GOAL, SCOPE, VERIFY): the worker sees nothing else. Use role terra for modular units, role sol for integration and role astra for research plans and scientific rulings. Status is completed, failed, interrupted, timeout or bridge_error; anything but completed carries `error`. Unit mode: pass exactly `project`, `unit`, `role` (terra or sol) and `cwd`, plus `track` and `brief_file` (the brief's path in the store) to register a new unit in the same call; the bridge reads the registered brief, runs its VERIFY (cmd on Windows, sh on Linux) and, when every unit of the module has passed VERIFY, runs the module ACCEPT and returns it as `accept`. Brief mode is for role astra: pass `brief`, `cwd`, `project` and `save_to`.", "inputSchema": {"type": "object", "properties": {"brief": {"type": "string", "description": "Full worker brief; sent as the only user message."}, "cwd": {"type": "string", "description": "Absolute working directory, ideally the unit's own worktree."}, "role": {"type": "string", "enum": ["terra", "sol", "astra"], "description": "Resolved to a model this Codex account lists; the report names it."}, "model": {"type": "string", "description": "Explicit model id; overrides role."}, "sandbox": {"type": "string", "enum": ["read-only", "workspace-write"], "default": "workspace-write"}, "effort": {"type": "string", "description": "Reasoning effort advertised by the model, e.g. low, medium, high."}, "developer_instructions": {"type": "string", "description": "Role contract layered over the user's Codex AGENTS.md."}, "output_schema": {"type": "object", "description": "JSON Schema for the final message; parsed into structured_output."}, "timeout_s": {"type": "integer", "minimum": 10, "default": 1800}, "lean": {"type": "boolean", "default": true, "description": "For this thread only, disable every Codex plugin, configured MCP server, the apps connector, goals, the skills catalog, the personality section and (unless web_search) the web search tool."}, "web_search": {"type": "boolean", "default": false, "description": "Keep the web search tool in lean mode; costs ~2.5k input tokens per call."}, "keep_plugins": {"type": "array", "items": {"type": "string"}, "description": "Plugin ids such as name@marketplace that stay enabled in lean mode."}}, "required": ["brief", "cwd"]}}, {"name": "orch", "description": "Run Pstack's orch bookkeeping CLI (units, inbox, ledger, gates, standing orders, status) on the store for `project`. The store lives in the plugin data directory and survives plugin updates; its path is returned as `store`.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "description": "Lowercase slug naming the program, e.g. todo-stats."}, "args": {"type": "array", "items": {"type": "string"}, "description": "orch arguments, e.g. [\"unit\", \"add\", \"u1\", \"--track\", \"build\"]."}}, "required": ["project", "args"]}}, {"name": "run_on_linux", "description": "Run `python <args>` in the project on Linux through WSL, with ~/.axiom/linux-venv. Windows runs skip or mask POSIX-only tests, so a full suite that passes here can still fail on Linux. Status unavailable means there is no Linux runner, which is not a pass.", "inputSchema": {"type": "object", "properties": {"project_dir": {"type": "string"}, "args": {"type": "array", "items": {"type": "string"}, "description": "Arguments to python, e.g. [\".axiom-experiment/check_baseline.py\"]."}, "timeout_s": {"type": "integer", "minimum": 60, "default": 1800}}, "required": ["project_dir", "args"]}}, {"name": "memory_record", "description": "Store one verified unit as long-run memory: what it built, its interface, and what a later worker reusing it must know. Files are hashed so later searches can flag the entry stale once they change.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string", "description": "Program slug, the same one orch uses."}, "project_dir": {"type": "string", "description": "Absolute project directory; files are relative to it."}, "unit": {"type": "string"}, "title": {"type": "string", "description": "The module, function or feature, as a later brief would name it."}, "summary": {"type": "string", "description": "One or two lines: what it does and its interface."}, "body": {"type": "string", "description": "What a reuser needs: signatures, behaviour decisions, verify command, pitfalls met."}, "symbols": {"type": "array", "items": {"type": "string"}}, "files": {"type": "array", "items": {"type": "string"}}, "wish": {"type": "string", "description": "The wish verbatim, in the user's words; indexed so searches in their language find the entry."}}, "required": ["project", "project_dir", "unit", "title", "summary", "body", "wish"]}}, {"name": "memory_search", "description": "Find earlier units related to `query` (English words and Chinese both match). Returns titles and summaries only, with `stale` true when a recorded file has changed since; fetch a body with memory_get.", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}, "project": {"type": "string", "description": "Limit to one program; omit to search every project."}, "limit": {"type": "integer", "minimum": 1, "maximum": 20, "default": 5}}, "required": ["query"]}}, {"name": "memory_get", "description": "Return one memory entry in full, with its files and whether it is stale.", "inputSchema": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}}, {"name": "memory_digest", "description": "Read a project's short history of wishes and outcomes, one line each; pass `add` to append a line first.", "inputSchema": {"type": "object", "properties": {"project": {"type": "string"}, "add": {"type": "string", "description": "One line: the wish, the outcome, memory ids."}}, "required": ["project"]}}, {"name": "goal", "description": "Close the wish goal of this Claude session: status done once the wish is carried out and verified, blocked with a note when it cannot go on without something only the user has, or waiting with a note while a detached batch runs. The session id is the one in the goal notice.", "inputSchema": {"type": "object", "properties": {"session": {"type": "string"}, "status": {"type": "string", "enum": ["done", "blocked", "waiting"]}, "note": {"type": "string"}}, "required": ["session", "status"]}}, {"name": "bridge_status", "description": "Report the Codex executable and app-server status.", "inputSchema": {"type": "object", "properties": {}}}]"###;
pub fn call(bridge: &Bridge, name: &str, args: &Value) -> Value {
    call_body(|| super::tools::dispatch(bridge, name, args))
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
    let failed = payload
        .get("status")
        .and_then(Value::as_str)
        .map_or(false, |s| s != "completed");
    if failed && payload.get("error").map_or(true, Value::is_null) {
        payload["error"] = json!({"kind":"tool_failed","message":"tool did not complete"});
    }
    json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&payload).unwrap()}],"structuredContent":payload,"isError":failed})
}
pub fn serve(config: Config, input: impl BufRead, output: impl Write + Send + 'static) -> i32 {
    let bridge = Arc::new(Bridge::new(config));
    let out = Arc::new(Mutex::new(output));
    let mut jobs = Vec::new();
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
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let method = msg["method"].as_str().unwrap_or("").to_owned();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        if method == "tools/call" {
            let b = bridge.clone();
            let o = out.clone();
            jobs.push(thread::spawn(move || {
                let progress = params
                    .pointer("/_meta/progressToken")
                    .filter(|v| v.is_string() || v.is_i64() || v.is_u64())
                    .cloned()
                    .map(|token| {
                        let writer = o.clone();
                        base::progress::Progress::new(token, move |msg| write_msg(&writer, msg))
                    });
                let result = base::progress::scope(progress, || {
                    call(
                        &b,
                        params["name"].as_str().unwrap_or(""),
                        params.get("arguments").unwrap_or(&Value::Null),
                    )
                });
                write_msg(&o, json!({"jsonrpc":"2.0","id":id,"result":result}));
            }));
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
    for job in jobs {
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
