use crate::base::{self, Failure, Res};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

pub const BRIDGE: &str = "mcp__plugin_chronos_bridge__";
pub const TOOLS: [&str; 16] = [
    "Agent",
    "Read",
    "Glob",
    "Grep",
    "Write",
    "Bash",
    "mcp__plugin_chronos_bridge__codex_run",
    "mcp__plugin_chronos_bridge__gemini_preview",
    "mcp__plugin_chronos_bridge__review_code",
    "mcp__plugin_chronos_bridge__run_on_linux",
    "mcp__plugin_chronos_bridge__orch",
    "mcp__plugin_chronos_bridge__memory_record",
    "mcp__plugin_chronos_bridge__memory_search",
    "mcp__plugin_chronos_bridge__memory_get",
    "mcp__plugin_chronos_bridge__memory_digest",
    "mcp__plugin_chronos_bridge__goal",
];

pub struct Config {
    pub codex: Res<Vec<String>>,
    pub home: PathBuf,
    pub bin: PathBuf,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            codex: base::resolve_command("AXIOM_CODEX_CMD", "codex"),
            home: base::axiom_home(),
            bin: std::env::current_exe().unwrap_or_default(),
        }
    }
}
pub fn command(config: &Config, _plugin_dir: &Path, _max_turns: Option<u32>) -> Res<Vec<String>> {
    let mut cmd = config.codex.clone().map_err(|e| {
        if e.kind == "not_installed" {
            Failure::new(
                "not_installed",
                "codex is not on PATH; install the Codex CLI and sign in",
            )
        } else {
            e
        }
    })?;
    cmd.extend(["app-server".into(), "--listen".into(), "stdio://".into()]);
    Ok(cmd)
}
pub fn user_message(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":text}]}})
    )
}
fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn short(s: &str) -> String {
    s.chars().take(100).collect()
}
fn line(out: &mut dyn Write, depth: usize, text: &str) {
    let _ = writeln!(out, "{}{text}", "  ".repeat(depth));
    let _ = out.flush();
}
fn comma(n: u64) -> String {
    let t = n.to_string();
    let mut x = String::new();
    for (i, c) in t.chars().enumerate() {
        if i > 0 && (t.len() - i) % 3 == 0 {
            x.push(',');
        }
        x.push(c);
    }
    x
}
#[derive(Default)]
pub struct Renderer {
    result: Option<Value>,
    agents: BTreeMap<String, String>,
    calls: BTreeMap<String, (String, usize)>,
    usage: BTreeMap<String, (u64, u64, u64)>,
    thread_models: BTreeMap<String, String>,
    resumed: bool,
}
impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn event(&mut self, event: &Value, out: &mut dyn Write) {
        match event["hostEvent"].as_str() {
            Some("threadStarted") => {
                self.resumed |= event["resumed"] == true;
                self.thread_models
                    .insert(s(&event["threadId"]).into(), s(&event["model"]).into());
                return;
            }
            Some("toolStarted") => {
                let depth = usize::from(event["role"] != "core");
                let name = s(&event["tool"]);
                let args = &event["arguments"];
                if name == "Agent" {
                    line(
                        out,
                        depth,
                        &format!(
                            "> {}: {}",
                            s(&args["subagent_type"]),
                            short(s(&args["prompt"]))
                        ),
                    );
                } else {
                    line(
                        out,
                        depth,
                        &format!("- {}", name.strip_prefix(BRIDGE).unwrap_or(name)),
                    );
                }
                return;
            }
            Some("toolFinished") => {
                if event["tool"] == "Agent" {
                    line(
                        out,
                        usize::from(event["role"] != "core"),
                        "< agent finished",
                    );
                } else {
                    let name = s(&event["tool"]);
                    let name = name
                        .strip_prefix("chronos_")
                        .or_else(|| name.strip_prefix(BRIDGE))
                        .unwrap_or(name);
                    if matches!(name, "codex_run" | "gemini_preview" | "review_code") {
                        if let Some(items) = event["result"]["contentItems"].as_array() {
                            for item in items.iter().filter(|i| i["type"] == "inputText") {
                                if let Ok(report) = serde_json::from_str::<Value>(s(&item["text"]))
                                {
                                    let key = format!(
                                        "bridge:{}:{}",
                                        s(&event["threadId"]),
                                        event
                                            .get("toolCallId")
                                            .or_else(|| event.get("callId"))
                                            .unwrap_or(event)
                                    );
                                    self.bridge_usage(&report, key);
                                }
                            }
                        }
                    }
                }
                return;
            }
            _ => (),
        }
        if let Some(method) = event["method"].as_str() {
            let p = &event["params"];
            match method {
                "item/tool/call" => {
                    let name = s(&p["tool"]);
                    let args = &p["arguments"];
                    if name == "Agent" {
                        line(
                            out,
                            0,
                            &format!(
                                "> {}: {}",
                                s(&args["subagent_type"]),
                                short(s(&args["prompt"]))
                            ),
                        );
                    } else {
                        line(
                            out,
                            0,
                            &format!("- {}", name.strip_prefix(BRIDGE).unwrap_or(name)),
                        );
                    }
                }
                "item/completed" if p["item"]["type"] == "agentMessage" => {
                    line(out, 0, s(&p["item"]["text"]))
                }
                "thread/tokenUsage/updated" => {
                    let u = &p["tokenUsage"]["total"];
                    let key = p["threadId"].as_str().unwrap_or("core").to_owned();
                    self.usage.insert(
                        key,
                        (
                            u["inputTokens"].as_u64().unwrap_or(0),
                            u["cachedInputTokens"].as_u64().unwrap_or(0),
                            u["outputTokens"].as_u64().unwrap_or(0),
                        ),
                    );
                }
                _ => (),
            }
            return;
        }
        match s(&event["type"]) {
            "system" if s(&event["subtype"]) == "init" => {
                let bridge = event["mcp_servers"]
                    .as_array()
                    .and_then(|a| a.iter().find(|v| s(&v["name"]) == "plugin:chronos:bridge"));
                let status = bridge.map(|v| s(&v["status"])).unwrap_or("missing");
                if status != "connected" {
                    line(out, 0, &format!("! the plugin:chronos:bridge MCP server is {status}; run `axiom doctor`"));
                }
            }
            "assistant" => {
                let depth = usize::from(!event["parent_tool_use_id"].is_null());
                if let Some(blocks) = event["message"]["content"].as_array() {
                    for b in blocks {
                        if s(&b["type"]) != "tool_use" {
                            continue;
                        }
                        let name = s(&b["name"]);
                        let args = &b["input"];
                        let id = s(&b["id"]).to_string();
                        let text = if name == "Agent" {
                            let agent = args["subagent_type"].as_str().unwrap_or("agent");
                            self.agents.insert(id, agent.into());
                            Some(format!("> {agent}: {}", s(&args["description"])))
                        } else if let Some(tool) = name.strip_prefix(BRIDGE) {
                            if tool == "codex_run" {
                                let role = args["role"]
                                    .as_str()
                                    .filter(|x| !x.is_empty())
                                    .or_else(|| args["model"].as_str())
                                    .unwrap_or("None");
                                self.calls.insert(id, (role.into(), depth));
                                let brief = s(&args["brief"]);
                                let label = brief
                                    .lines()
                                    .find_map(|l| {
                                        l.strip_prefix("GOAL")
                                            .map(str::trim)
                                            .filter(|s| !s.is_empty())
                                    })
                                    .or_else(|| {
                                        brief
                                            .lines()
                                            .map(str::trim)
                                            .find(|l| !l.is_empty() && *l != "GOAL")
                                    })
                                    .unwrap_or("(no brief)");
                                Some(format!("- codex {role}: {}", short(label)))
                            } else if tool == "orch" {
                                let args = args["args"]
                                    .as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(Value::as_str)
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    })
                                    .unwrap_or_default();
                                Some(format!("- orch {}", short(&args)))
                            } else {
                                Some(format!("- {tool}"))
                            }
                        } else if name == "Bash" {
                            Some(format!("- $ {}", short(s(&args["command"]))))
                        } else {
                            None
                        };
                        if let Some(text) = text {
                            line(out, depth, &text);
                        }
                    }
                }
            }
            "user" => {
                if let Some(blocks) = event["message"]["content"].as_array() {
                    for b in blocks {
                        if s(&b["type"]) != "tool_result" {
                            continue;
                        }
                        let id = s(&b["tool_use_id"]);
                        if let Some(agent) = self.agents.remove(id) {
                            line(out, 0, &format!("< {agent} finished"));
                        }
                        if let Some((role, depth)) = self.calls.remove(id) {
                            self.codex_result(b, &role, depth, out);
                        }
                    }
                }
            }
            "result" => {
                self.result = Some(event.clone());
                let _ = write!(out, "\n{}\n", s(&event["result"]));
            }
            _ => {}
        }
    }
    fn bridge_usage(&mut self, report: &Value, key: String) {
        if report["reused"] == true {
            return;
        }
        let Some(u) = report["usage"].as_object() else {
            return;
        };
        let Some(model) = report["model"]
            .as_str()
            .filter(|m| !m.is_empty())
            .or_else(|| report["reviewer"].as_str().filter(|m| !m.is_empty()))
        else {
            return;
        };
        // A report's usage is the actual thread total. Replace, don't add, if the
        // same result/total arrives twice or native events already cover its thread.
        let key = report["thread_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .unwrap_or(&key)
            .to_owned();
        self.thread_models.insert(key.clone(), model.into());
        self.usage.insert(
            key,
            (
                u.get("inputTokens").and_then(Value::as_u64).unwrap_or(0),
                u.get("cachedInputTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                u.get("outputTokens").and_then(Value::as_u64).unwrap_or(0),
            ),
        );
    }
    fn codex_result(&mut self, block: &Value, role: &str, depth: usize, out: &mut dyn Write) {
        let content = &block["content"];
        let text = if let Some(s) = content.as_str() {
            s.to_string()
        } else if let Some(a) = content.as_array() {
            a.iter()
                .filter(|x| s(&x["type"]) == "text")
                .map(|x| s(&x["text"]))
                .collect::<String>()
        } else {
            return;
        };
        let Ok(report) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        let cov = report["structured_output"]
            .get("coverage")
            .filter(|v| v.is_object())
            .or_else(|| report.get("coverage"));
        if let Some(c) = cov {
            if let (Some(read), Some(not)) = (c["read"].as_array(), c["not_read"].as_array()) {
                let fmt = |v: &Vec<Value>| {
                    let t = v
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; ");
                    if t.is_empty() {
                        "(none)".to_string()
                    } else {
                        t
                    }
                };
                line(out, depth + 1, &format!("astra read: {}", fmt(read)));
                line(out, depth + 1, &format!("astra did not read: {}", fmt(not)));
            }
        }
        if let Some(u) = report["usage"].as_object() {
            let model = report["model"]
                .as_str()
                .filter(|x| !x.is_empty())
                .unwrap_or(role);
            let v = self.usage.entry(model.into()).or_default();
            v.0 += u.get("inputTokens").and_then(Value::as_u64).unwrap_or(0);
            v.1 += u
                .get("cachedInputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            v.2 += u.get("outputTokens").and_then(Value::as_u64).unwrap_or(0);
        }
    }
    pub fn summary(&self, out: &mut dyn Write) {
        if !self.usage.is_empty() {
            let mut models = BTreeMap::<String, (u64, u64, u64)>::new();
            for (thread, (i, c, o)) in &self.usage {
                let model = self.thread_models.get(thread).unwrap_or(thread);
                let total = models.entry(model.clone()).or_default();
                total.0 += i;
                total.1 += c;
                total.2 += o;
            }
            let totals = models
                .iter()
                .map(|(m, (i, c, o))| {
                    format!(
                        "{m} {} in ({} cached) / {} out",
                        comma(*i),
                        comma(*c),
                        comma(*o)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "codex tokens: {totals}");
            if self.resumed {
                let _ = writeln!(
                    out,
                    "usage includes cumulative totals for resumed threads, not new-run cost"
                );
            }
        }
    }
}
