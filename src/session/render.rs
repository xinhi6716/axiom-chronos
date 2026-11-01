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
    pub claude: Res<Vec<String>>,
    pub home: PathBuf,
    pub bin: PathBuf,
}
impl Config {
    pub fn from_env() -> Self {
        Self {
            claude: base::resolve_command("AXIOM_CLAUDE_CMD", "claude"),
            home: base::axiom_home(),
            bin: std::env::current_exe().unwrap_or_default(),
        }
    }
}
pub fn command(config: &Config, plugin_dir: &Path, max_turns: Option<u32>) -> Res<Vec<String>> {
    let mut cmd = config.claude.clone().map_err(|e| {
        if e.kind == "not_installed" {
            Failure::new(
                "not_installed",
                "claude is not on PATH; install Claude Code and sign in with `claude`",
            )
        } else {
            e
        }
    })?;
    cmd.extend([
        "-p".into(),
        "--input-format".into(),
        "stream-json".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--plugin-dir".into(),
        plugin_dir.to_string_lossy().into_owned(),
        "--setting-sources".into(),
        "local".into(),
        "--add-dir".into(),
        config.home.to_string_lossy().into_owned(),
        "--allowedTools".into(),
        TOOLS.join(","),
        "--disallowedTools".into(),
        "PowerShell,ScheduleWakeup,CronCreate".into(),
        "--model".into(),
        "opus".into(),
    ]);
    if let Some(n) = max_turns {
        cmd.extend(["--max-turns".into(), n.to_string()]);
    }
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
}
impl Renderer {
    pub(super) fn is_error(&self) -> bool {
        self.result
            .as_ref()
            .and_then(|v| v["is_error"].as_bool())
            .unwrap_or(false)
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn event(&mut self, event: &Value, out: &mut dyn Write) {
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
                        if let Some((role, depth)) = self.calls.get(id).cloned() {
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
        if let Some(result) = &self.result {
            let duration = result["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0;
            let cost = result["total_cost_usd"].as_f64().unwrap_or(0.0);
            let models = result["modelUsage"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| format!("{k} ${:.2}", v["costUSD"].as_f64().unwrap_or(0.0)))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "\n{duration:.0}s · ${cost:.2}{}",
                if models.is_empty() {
                    String::new()
                } else {
                    format!(" ({models})")
                }
            );
        }
        if !self.usage.is_empty() {
            let totals = self
                .usage
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
        }
    }
}
