use axiom::session::{command, user_message, Config, Renderer, BRIDGE, TOOLS};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[test]
fn command_and_message() {
    let config = Config {
        claude: Ok(vec!["claude".into()]),
        home: PathBuf::from("home"),
        bin: PathBuf::new(),
    };
    let args = command(&config, Path::new("plugin"), Some(3)).unwrap();
    let tools = [
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
    assert_eq!(TOOLS, tools);
    let expected: Vec<String> = [
        "claude",
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--plugin-dir",
        "plugin",
        "--setting-sources",
        "local",
        "--add-dir",
        "home",
        "--allowedTools",
        &tools.join(","),
        "--disallowedTools",
        "PowerShell,ScheduleWakeup,CronCreate",
        "--model",
        "opus",
        "--max-turns",
        "3",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(args, expected);
    assert_eq!(
        command(&config, Path::new("plugin"), None).unwrap(),
        expected[..expected.len() - 2]
    );
    let msg: Value = serde_json::from_str(&user_message("中文\r\n")).unwrap();
    assert_eq!(msg["message"]["content"][0]["text"], "中文\r\n");
}
#[test]
fn not_installed_message() {
    let config = Config {
        claude: Err(axiom::base::Failure::new("not_installed", "missing")),
        home: PathBuf::new(),
        bin: PathBuf::new(),
    };
    assert_eq!(
        command(&config, Path::new("plugin"), None)
            .unwrap_err()
            .message,
        "claude is not on PATH; install Claude Code and sign in with `claude`"
    );
}

fn tool(name: &str, input: Value, nested: bool) -> String {
    let mut r = Renderer::new();
    let mut out = Vec::new();
    r.event(&json!({"type":"assistant","parent_tool_use_id": if nested { json!("parent") } else { Value::Null },"message":{"content":[{"type":"tool_use","id":"x","name":name,"input":input}]}}), &mut out);
    String::from_utf8(out).unwrap()
}
#[test]
fn agent_start() {
    assert_eq!(
        tool(
            "Agent",
            json!({"subagent_type":"worker","description":"work"}),
            false
        ),
        "> worker: work\n"
    );
}
#[test]
fn agent_finish() {
    let mut r = Renderer::new();
    let mut out = Vec::new();
    r.event(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Agent","input":{"subagent_type":"worker","description":"work"}}]}}), &mut out);
    out.clear();
    r.event(
        &json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a"}]}}),
        &mut out,
    );
    assert_eq!(String::from_utf8(out).unwrap(), "< worker finished\n");
}
#[test]
fn codex_briefs() {
    let name = format!("{BRIDGE}codex_run");
    assert_eq!(
        tool(
            &name,
            json!({"role":"astra","brief":"context\nGOAL inspect"}),
            false
        ),
        "- codex astra: inspect\n"
    );
    assert_eq!(
        tool(
            &name,
            json!({"model":"terra","brief":"GOAL   \nfirst line"}),
            false
        ),
        "- codex terra: first line\n"
    );
    assert_eq!(
        tool(&name, json!({"model":"terra","brief":"\nfirst line"}), true),
        "  - codex terra: first line\n"
    );
    assert_eq!(
        tool(&name, json!({"brief":"  \n"}), false),
        "- codex None: (no brief)\n"
    );
    assert_eq!(
        tool(
            &name,
            json!({"role":"x","brief":format!("GOAL {}", "a".repeat(110))}),
            false
        ),
        format!("- codex x: {}\n", "a".repeat(100))
    );
}
#[test]
fn orch_line() {
    assert_eq!(
        tool(
            &format!("{BRIDGE}orch"),
            json!({"args":["unit", "add"]}),
            true
        ),
        "  - orch unit add\n"
    );
    assert_eq!(
        tool(
            &format!("{BRIDGE}orch"),
            json!({"args":["a".repeat(110)]}),
            false
        ),
        format!("- orch {}\n", "a".repeat(100))
    );
}
#[test]
fn bash_line() {
    assert_eq!(
        tool("Bash", json!({"command":"x".repeat(110)}), false),
        format!("- $ {}\n", "x".repeat(100))
    );
}
#[test]
fn bridge_line() {
    assert_eq!(
        tool(&format!("{BRIDGE}goal"), json!({}), true),
        "  - goal\n"
    );
}
#[test]
fn progress_and_last_summary() {
    let mut r = Renderer::new();
    let mut out = Vec::new();
    r.event(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":format!("{BRIDGE}codex_run"),"input":{"role":"astra","brief":"context\nGOAL   inspect"}}]}}), &mut out);
    let report = json!({"coverage":{"read":["one"],"not_read":[]},"usage":{"inputTokens":1050,"cachedInputTokens":200,"outputTokens":32}}).to_string();
    r.event(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":report}]}}), &mut out);
    r.event(
        &json!({"type":"result","result":"first","total_cost_usd":0.3}),
        &mut out,
    );
    r.event(
        &json!({"type":"result","result":"second","duration_ms":2000,"total_cost_usd":0.5}),
        &mut out,
    );
    r.summary(&mut out);
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("- codex astra: inspect\n  astra read: one\n  astra did not read: (none)")
    );
    assert!(text.contains("codex tokens: astra 1,050 in (200 cached) / 32 out"));
    assert!(text.contains("2s · $0.50"));
    assert!(!text.contains("$0.30"));
}
#[test]
fn structured_coverage_array_result_and_summed_tokens() {
    let mut r = Renderer::new();
    let mut out = Vec::new();
    for id in ["one", "two"] {
        r.event(&json!({"type":"assistant","parent_tool_use_id":"parent","message":{"content":[{"type":"tool_use","id":id,"name":format!("{BRIDGE}codex_run"),"input":{"role":"astra","brief":"GOAL inspect"}}]}}), &mut out);
        let report = json!({"model":"model-x","structured_output":{"coverage":{"read":["src/a"],"not_read":["src/b"]}},"usage":{"inputTokens":1000,"cachedInputTokens":200,"outputTokens":30}}).to_string();
        r.event(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":report}]}]}}), &mut out);
    }
    r.event(
        &json!({"type":"result","result":"first","duration_ms":1000,"total_cost_usd":0.3}),
        &mut out,
    );
    r.event(&json!({"type":"result","result":"second","duration_ms":2000,"total_cost_usd":0.5,"modelUsage":{"opus":{"costUSD":0.4}}}), &mut out);
    r.summary(&mut out);
    assert_eq!(String::from_utf8(out).unwrap(), "  - codex astra: inspect\n    astra read: src/a\n    astra did not read: src/b\n  - codex astra: inspect\n    astra read: src/a\n    astra did not read: src/b\n\nfirst\n\nsecond\n\n2s · $0.50 (opus $0.40)\ncodex tokens: model-x 2,000 in (400 cached) / 60 out\n");
}
