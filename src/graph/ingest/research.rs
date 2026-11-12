use super::*;
use crate::graph::types::keys;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Component, Path},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceLink {
    pub to: String,
    pub quote: String,
    pub line: Option<u32>,
}

pub fn import_source(
    store: &GraphStore,
    project_dir: &Path,
    artifact: &str,
    links: &[SourceLink],
) -> Res<IngestReport> {
    let scope = store.scope(project_dir, None)?.id;
    let input = Path::new(artifact);
    let (path, key_path) = if input.is_absolute() {
        // The graph database lives at <home>/memory/memory.db.
        let db: String = store.read(|c| {
            c.query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| Failure::new("io", e.to_string()))
        })?;
        let home = Path::new(&db)
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| Failure::new("invalid_argument", "invalid data home"))?;
        let project = project_dir
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Failure::new("invalid_argument", "invalid project directory"))?;
        let canonical_project =
            std::fs::canonicalize(project_dir).map_err(|e| Failure::new("io", e.to_string()))?;
        let mut projects = vec![project.to_string()];
        // Goals and memory records can associate a project slug with a differently named checkout.
        let recorded: Vec<(String, String)> = store.read(|c| {
            let mut q = c
                .prepare("SELECT DISTINCT project, project_dir FROM entries")
                .map_err(|e| Failure::new("io", e.to_string()))?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| Failure::new("io", e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Failure::new("io", e.to_string()))?;
            Ok(rows)
        })?;
        for (slug, dir) in recorded {
            if std::fs::canonicalize(dir).ok().as_ref() == Some(&canonical_project) {
                projects.push(slug);
            }
        }
        for entry in std::fs::read_dir(home.join("goals"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Ok(bytes) = std::fs::read(entry.path()) {
                if let Ok(goal) = serde_json::from_slice::<crate::goal::Goal>(&bytes) {
                    if let (Some(slug), Some(cwd)) = (goal.project, goal.cwd) {
                        if std::fs::canonicalize(cwd).ok().as_ref() == Some(&canonical_project) {
                            projects.push(slug);
                        }
                    }
                }
            }
        }
        let resolved = if input.exists() {
            std::fs::canonicalize(input).map_err(|e| Failure::new("io", e.to_string()))?
        } else {
            let parent = input
                .parent()
                .ok_or_else(|| Failure::new("invalid_argument", "invalid artifact path"))?;
            let name = input
                .file_name()
                .ok_or_else(|| Failure::new("invalid_argument", "invalid artifact path"))?;
            std::fs::canonicalize(parent)
                .map_err(|e| Failure::new("io", e.to_string()))?
                .join(name)
        };
        let rel = projects
            .iter()
            .filter(|slug| crate::base::require_slug("project", slug).is_ok())
            .filter_map(|slug| {
                std::fs::canonicalize(home.join("orchestrate").join(slug).join("research")).ok()
            })
            .find_map(|root| resolved.strip_prefix(root).ok().map(Path::to_path_buf))
            .ok_or_else(|| {
                Failure::new(
                    "invalid_argument",
                    "artifact outside project research directory",
                )
            })?;
        (
            resolved.clone(),
            format!("research/{}", rel.to_string_lossy().replace('\\', "/")),
        )
    } else {
        if input
            .components()
            .any(|x| !matches!(x, Component::Normal(_)))
            || input.as_os_str().is_empty()
        {
            return Err(Failure::new(
                "invalid_argument",
                "artifact must be a relative path inside the project",
            ));
        }
        let root =
            std::fs::canonicalize(project_dir).map_err(|e| Failure::new("io", e.to_string()))?;
        let path = root.join(input);
        if path.exists()
            && !std::fs::canonicalize(&path)
                .map_err(|e| Failure::new("io", e.to_string()))?
                .starts_with(&root)
        {
            return Err(Failure::new("invalid_argument", "artifact outside project"));
        }
        (path, artifact.replace('\\', "/"))
    };
    let key = keys::artifact(&key_path);
    let locator = format!("research:{key}|");
    let mut result = IngestReport::default();
    let mut current = HashSet::new();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes.clone())
                .map_err(|e| Failure::new("invalid_argument", e.to_string()))?;
            let mut b = batch(scope, "research", locator.clone(), &bytes);
            let sha = b.source.sha256.clone();
            b.nodes.push(node(
                key.clone(),
                "artifact",
                &key_path,
                json!({"sha256":sha,"state":"reported/unreviewed"}),
            ));
            for link in links {
                let target = store.node(&NodeRef {
                    scope,
                    key: link.to.clone(),
                })?;
                let reason = if link.quote.is_empty() || !text.contains(&link.quote) {
                    Some("quote is not verbatim in artifact")
                } else if target.is_none() {
                    Some("target key does not exist")
                } else if !target.as_ref().is_some_and(|n| {
                    (n.kind == "file" && link.to.starts_with("file:"))
                        || (n.kind == "symbol" && link.to.starts_with("sym:"))
                }) {
                    Some("target is not a code key")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    result.skipped.push((link.to.clone(), reason.into()));
                    continue;
                }
                b.edges.push(edge(
                    scope,
                    &key,
                    &link.to,
                    "cites",
                    &format!(
                        "{}:{}",
                        link.line.map(|n| n.to_string()).unwrap_or_default(),
                        hash(link.quote.as_bytes())
                    ),
                    json!({"quote":link.quote,"line":link.line,"sha256":sha}),
                ));
            }
            b.replace_source = true;
            current.insert((locator.clone(), sha));
            write(store, &b, &mut result)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Failure::new("io", e.to_string())),
    }
    stale(store, scope, "research", &locator, &current, &mut result)?;
    result.revision = store.revision()?;
    Ok(result)
}
