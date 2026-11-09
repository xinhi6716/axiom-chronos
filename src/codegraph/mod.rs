use crate::base;
use serde_json::{json, Value};
use std::path::Path;

pub const NAME: &str = "codegraph";
pub const TOOL: &str = "mcp__codegraph__codegraph_explore";

pub fn indexed(project: &Path) -> bool {
    project.join(".codegraph").is_dir()
}
pub fn enabled(project: &Path) -> bool {
    indexed(project) && base::on_path(NAME).is_some()
}
fn server() -> Value {
    json!({"command":NAME,"args":["serve","--mcp"],"env":{"CODEGRAPH_TELEMETRY":"0"}})
}
pub fn claude_config() -> String {
    let mut server = server();
    server["type"] = json!("stdio");
    server["alwaysLoad"] = json!(true);
    json!({"mcpServers":{NAME:server}}).to_string()
}
pub fn codex_server(cwd: &str) -> Value {
    let mut server = server();
    server["cwd"] = json!(cwd);
    server["enabled"] = json!(true);
    server
}
