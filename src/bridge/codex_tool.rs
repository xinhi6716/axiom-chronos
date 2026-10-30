use super::server::Bridge;
use crate::{
    base::{self, Failure, Res},
    codex::{self, RunRequest},
    goal, orch,
};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};

fn invalid(message: &str) -> Failure {
    Failure::new("invalid_argument", message)
}
fn string<'a>(a: &'a Value, key: &str) -> Option<&'a str> {
    a.get(key).and_then(Value::as_str)
}
pub fn codex_run(b: &Bridge, a: &Value) -> Value {
    match run(b, a) {
        Ok(v) => v,
        Err(e) => json!({"status":"bridge_error","error":e.to_json()}),
    }
}
pub(super) fn destination(root: &Path, path: &str) -> Res<PathBuf> {
    let dest = Path::new(path);
    if !dest.is_absolute() {
        return Err(invalid("save_to must be absolute and inside store"));
    }
    let relative = dest
        .strip_prefix(root)
        .map_err(|_| invalid("save_to must be inside store"))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid("save_to must be inside store"));
    }
    std::fs::create_dir_all(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| Failure::new("io", e.to_string()))?;
    let mut parent = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for c in &components[..components.len() - 1] {
        parent.push(c.as_os_str());
        std::fs::create_dir_all(&parent).map_err(|e| Failure::new("io", e.to_string()))?;
        if !std::fs::canonicalize(&parent)
            .map_err(|e| Failure::new("io", e.to_string()))?
            .starts_with(&canonical_root)
        {
            return Err(invalid("save_to must be inside store"));
        }
    }
    if dest.is_symlink() {
        return Err(invalid("save_to must be inside store"));
    }
    Ok(dest.to_path_buf())
}
struct UnitGuard(Option<(PathBuf, String)>);
impl Drop for UnitGuard {
    fn drop(&mut self) {
        if let Some((store, u)) = &self.0 {
            orch::abort_unit(store, u);
        }
    }
}
fn run(b: &Bridge, a: &Value) -> Res<Value> {
    let role = string(a, "role");
    if a.get("role").is_some() && !matches!(role, Some("terra" | "sol" | "astra")) {
        return Err(invalid("invalid role"));
    }
    let explicit = string(a, "model").filter(|s| !s.is_empty());
    if let (Some(r), Some(m)) = (role, explicit) {
        if !codex::role_matches(r, m) {
            return Err(invalid("model does not match role"));
        }
    }
    if role.is_none() && explicit.is_none() {
        return Err(invalid("model or role is required"));
    }
    let project = string(a, "project");
    let unit = string(a, "unit");
    let save = string(a, "save_to");
    if unit.is_none() && string(a, "brief").is_none() {
        return Err(invalid("brief is required outside unit mode"));
    }
    if a.get("unit").is_some() && (project.is_none() || unit.is_none()) {
        return Err(invalid("unit requires project"));
    }
    if unit.is_some()
        && (!matches!(role, Some("terra" | "sol"))
            || a.get("brief").is_some()
            || save.is_some()
            || string(a, "sandbox").is_some_and(|s| s != "workspace-write"))
    {
        return Err(invalid(
            "unit mode requires terra or sol, no brief, and workspace-write",
        ));
    }
    if save.is_some() && (project.is_none() || role != Some("astra") || unit.is_some()) {
        return Err(invalid("save_to requires project and astra role"));
    }
    if unit.is_none() && string(a, "sandbox").unwrap_or("workspace-write") != "read-only" {
        return Err(Failure::new("unbound_write", "a writing codex_run needs project and unit; register the unit with orch and dispatch it by id"));
    }
    let store = project.map(|p| orch::store_for(b.home()?, p)).transpose()?;
    let dest = match (save, &store) {
        (Some(s), Some(root)) => Some(destination(root, s)?),
        _ => None,
    };
    let cwd = string(a, "cwd").unwrap_or("");
    base::require_directory(cwd)?;
    let plugins = match a.get("keep_plugins") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("keep_plugins must contain strings"))
            })
            .collect::<Res<Vec<_>>>()?,
        _ => return Err(invalid("keep_plugins must be an array")),
    };
    let mut brief = string(a, "brief").unwrap_or("").to_owned();
    let mut attempt = 0;
    let mut guard = UnitGuard(None);
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        let active = goal::find_active(b.home()?, project.unwrap())?;
        if active
            .as_ref()
            .is_some_and(|(_, g)| goal::creative_requested(g))
            && !root.join("modules.md").exists()
        {
            return Err(Failure::new(
                "architect_required",
                "creative goal requires modules.md",
            ));
        }
        let required = active.is_some_and(|(_, g)| goal::astra_requested(&g));
        match orch::begin_unit(root, u, r, Path::new(cwd), required)? {
            orch::Begin::Reused => {
                return Ok(json!({"status":"completed","unit":u,"state":"verified","reused":true}))
            }
            orch::Begin::Run {
                brief: value,
                attempt: n,
            } => {
                brief = value.text;
                attempt = n;
                guard.0 = Some((root.to_path_buf(), u.to_owned()));
            }
        }
    }
    let mut resolved = explicit.unwrap_or("").to_owned();
    let result = (|| -> Res<(String, Value)> {
        let server = b.codex()?;
        let model = match explicit {
            Some(m) => m.to_owned(),
            None => server.resolve_model(role.unwrap())?,
        };
        resolved = model.clone();
        let req = RunRequest {
            brief,
            cwd: cwd.into(),
            model: model.clone(),
            sandbox: string(a, "sandbox").unwrap_or("workspace-write").into(),
            effort: string(a, "effort").map(str::to_owned),
            output_schema: a.get("output_schema").filter(|v| !v.is_null()).cloned(),
            developer_instructions: string(a, "developer_instructions").map(str::to_owned),
            timeout_s: a
                .get("timeout_s")
                .and_then(Value::as_u64)
                .filter(|x| *x > 0)
                .unwrap_or(1800),
            lean: a.get("lean") != Some(&Value::Bool(false)),
            keep_plugins: plugins,
            web_search: a.get("web_search") == Some(&Value::Bool(true)),
        };
        Ok((model, server.run(&req)?))
    })();
    let (model, mut report) = match result {
        Ok(r) => r,
        Err(e) => {
            if dest.is_some() {
                let _ = orch::record_stage(store.as_ref().unwrap(), "astra", &resolved, false);
            }
            return Err(e);
        }
    };
    if let (Some(u), Some(root), Some(r)) = (unit, &store, role) {
        let finished = orch::finish_unit(root, u, Path::new(cwd), r, &model, &report)?;
        let built = finished.state == "built";
        let error = if built {
            Value::Null
        } else if report["status"] != "completed" {
            report["error"].clone()
        } else if !finished.outside_scope.is_empty() {
            json!({"kind":"outside_scope","message":"changes outside unit scope"})
        } else {
            json!({"kind":"verify_failed","message":"unit verification failed"})
        };
        let mut result = json!({"status":if built {"completed"} else {"failed"},"unit":u,"state":finished.state,"model":model,"attempt":attempt,"verify_exit":finished.verify_exit,"outside_scope":finished.outside_scope,"file_changes":report["file_changes"],"usage":report["usage"],"report":root.join("reports").join(format!("{u}.md")),"error":error});
        if !built {
            result["verify_tail"] = json!(finished.verify_tail);
        }
        return Ok(result);
    }
    if let Some(dest) = dest {
        let valid = report["status"] == "completed" && !report["structured_output"].is_null();
        if valid {
            let bytes = serde_json::to_vec_pretty(&report["structured_output"])
                .map_err(|e| Failure::new("io", e.to_string()))?;
            base::atomic_write(&dest, &bytes).map_err(|e| Failure::new("io", e.to_string()))?;
        } else if report["status"] == "completed" {
            report["status"] = json!("failed");
            report["error"] = json!({"kind":"invalid_structured_output","message":"structured output is required"});
        }
        orch::record_stage(store.as_ref().unwrap(), "astra", &model, valid)?;
        let coverage = report["structured_output"].get("coverage").cloned();
        let obj = report.as_object_mut().unwrap();
        obj.remove("structured_output");
        obj.remove("final_message");
        if valid {
            obj.insert("saved_to".into(), json!(dest));
        }
        if let Some(v) = coverage {
            obj.insert("coverage".into(), v);
        }
    }
    Ok(report)
}
