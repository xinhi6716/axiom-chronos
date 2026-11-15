use crate::{
    base::{Failure, Res},
    memory::Memory,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ToolRequest {
    pub tool: String,
    pub args: Value,
}
pub const READ_TOOLS: [&str; 4] = ["read_file", "list_dir", "memory_search", "memory_get"];
pub struct ReadOnly {
    project: Option<PathBuf>,
    memory_home: PathBuf,
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn io_error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
impl ReadOnly {
    pub fn new(project: Option<&Path>, memory_home: &Path) -> Self {
        Self {
            project: project.map(Path::to_path_buf),
            memory_home: memory_home.to_path_buf(),
        }
    }
    fn path(&self, args: &Value) -> Res<PathBuf> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("path must be a string"))?;
        if !super::runs::relative(path) {
            return Err(invalid("path must be relative and within the project"));
        }
        let project = self
            .project
            .as_ref()
            .ok_or_else(|| invalid("no project"))?
            .canonicalize()
            .map_err(|e| invalid(format!("invalid project path: {e}")))?;
        let target = project
            .join(path)
            .canonicalize()
            .map_err(|e| invalid(format!("invalid path: {e}")))?;
        if !target.starts_with(&project) {
            return Err(invalid("path must be within the project"));
        }
        Ok(target)
    }
    pub fn call(&self, req: &ToolRequest) -> Res<String> {
        if !READ_TOOLS.contains(&req.tool.as_str()) {
            return Err(Failure::new("read_only", "tool is not read-only"));
        }
        let output = match req.tool.as_str() {
            "read_file" => fs::read_to_string(self.path(&req.args)?).map_err(io_error)?,
            "list_dir" => {
                let mut names = fs::read_dir(self.path(&req.args)?)
                    .map_err(io_error)?
                    .map(|entry| {
                        entry
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .map_err(io_error)
                    })
                    .collect::<Res<Vec<_>>>()?;
                names.sort();
                names.join("\n")
            }
            "memory_search" => {
                let query = req
                    .args
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("query must be a string"))?;
                let project = req.args.get("project").and_then(Value::as_str);
                let limit = req.args.get("limit").and_then(Value::as_i64).unwrap_or(5);
                Memory::open(&self.memory_home)?
                    .search(query, project, limit)?
                    .to_string()
            }
            "memory_get" => {
                let id = req
                    .args
                    .get("id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| invalid("id must be an integer"))?;
                Memory::open(&self.memory_home)?.get(id)?.to_string()
            }
            _ => unreachable!(),
        };
        let mut end = output.len().min(16_000);
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        Ok(output[..end].to_owned())
    }
}
