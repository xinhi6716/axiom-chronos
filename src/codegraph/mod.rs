use crate::base;
use serde_json::{json, Value};
use std::path::Path;

pub const SERVER: &str = "codegraph";
pub const TOOL: &str = "mcp__codegraph__codegraph_explore";

pub fn indexed(project: &Path) -> bool {
    project.join(".codegraph").is_dir()
}
pub fn detect(project: &Path) -> Option<String> {
    if indexed(project) {
        base::on_path(SERVER)
    } else {
        None
    }
}
fn server(program: &str) -> Value {
    json!({"command":program,"args":["serve","--mcp"],"env":{"CODEGRAPH_TELEMETRY":"0"}})
}
pub fn claude_config(program: &str) -> String {
    let mut server = server(program);
    server["type"] = json!("stdio");
    server["alwaysLoad"] = json!(true);
    json!({"mcpServers":{SERVER:server}}).to_string()
}
pub fn codex_server(program: &str, cwd: &str) -> Value {
    let mut server = server(program);
    server["cwd"] = json!(cwd);
    server["enabled"] = json!(true);
    server
}
