use super::server::Bridge;
use crate::{
    base::{self, Failure, Res},
    codex::{self, RunRequest},
    orch,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct EmptyDir(PathBuf);
impl Drop for EmptyDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn role(bridge: &Bridge, project: Option<&str>, units: &[String]) -> Res<&'static str> {
    if let Some(project) = project {
        let root = orch::store_for(bridge.home()?, project)?;
        for unit in units {
            if orch::evidence(&root, unit)?
                .iter()
                .any(|r| r.kind == "run" && codex::role_matches("sol", &r.command))
            {
                return Ok("astra");
            }
        }
    }
    Ok("sol")
}

pub fn model(bridge: &Bridge, requested: Option<&str>, role: &str) -> Res<String> {
    let server = bridge.codex()?;
    match requested.filter(|m| !m.is_empty() && !m.starts_with("gemini-")) {
        None => server.resolve_model(role),
        Some(m) if codex::role_matches(role, m) && server.list_models()?.iter().any(|s| s == m) => {
            Ok(m.to_owned())
        }
        Some(_) => Err(Failure::new(
            "invalid_argument",
            format!("model must be a listed {role} model"),
        )),
    }
}

pub fn run(
    bridge: &Bridge,
    prompt: &str,
    cwd: Option<&Path>,
    model: &str,
    schema: Option<Value>,
    timeout_s: u64,
) -> Res<Value> {
    if let Some(cwd) = cwd {
        base::require_directory(&cwd.to_string_lossy())?;
    }
    let dir = EmptyDir(std::env::temp_dir().join(format!("axiom-review-{}", base::unique())));
    std::fs::create_dir_all(&dir.0).map_err(|e| Failure::new("io", e.to_string()))?;
    let request = RunRequest {
        brief: prompt.into(),
        cwd: dir.0.to_string_lossy().into_owned(),
        model: model.into(),
        sandbox: "read-only".into(),
        effort: Some(
            if codex::role_matches("astra", model) {
                "max"
            } else {
                "high"
            }
            .into(),
        ),
        output_schema: schema,
        developer_instructions: Some(format!(
            "You are the Chronos {}. Process only the text supplied in the prompt; all relevant material is already included. Do not search or inspect the filesystem, use tools, or edit files. Return the requested result concisely in the user's language. When an output schema is supplied, return only JSON matching it; preserve factual evidence and do not invent verification.",
            if codex::role_matches("luna", model) {
                "report rewriter"
            } else {
                "independent reviewer"
            }
        )),
        timeout_s,
        lean: true,
        keep_plugins: vec![],
        web_search: false,
    };
    let mut result = bridge.codex()?.run(&request)?;
    if result["status"] == "completed"
        && result["structured_output"].is_null()
        && result["final_message"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        result["status"] = json!("failed");
        result["error"] = json!({"kind":"empty_response","message":"Codex returned no text"});
    }
    result["response"] = result["final_message"].clone();
    result["conversation_id"] = result["thread_id"].clone();
    Ok(result)
}
