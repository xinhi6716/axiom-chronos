use super::permissions::Permission;
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANIFEST: &str = "manifest.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub entry: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default = "default_parallel")]
    pub max_parallel: u32,
    pub panel: Panel,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub title: String,
    pub description: String,
}

fn default_parallel() -> u32 {
    1
}
pub fn modules_root(home: &Path) -> PathBuf {
    home.join("workshop")
}
pub fn data_root(home: &Path, name: &str) -> PathBuf {
    home.join("workshop-data").join(name)
}

pub fn load(dir: &Path) -> Res<Manifest> {
    let invalid = |message: String| Failure::new("invalid_manifest", message);
    let bytes = std::fs::read(dir.join(MANIFEST)).map_err(|e| invalid(e.to_string()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))?;
    base::require_slug("module name", &manifest.name).map_err(|e| invalid(e.message))?;
    if dir.file_name().and_then(|s| s.to_str()) != Some(manifest.name.as_str()) {
        return Err(invalid("module name must match its directory".into()));
    }
    if manifest.version.trim().is_empty()
        || manifest.entry.is_empty()
        || manifest.entry.iter().any(|s| s.trim().is_empty())
    {
        return Err(invalid("version and entry must be non-empty".into()));
    }
    if !(1..=8).contains(&manifest.max_parallel) {
        return Err(invalid("max_parallel must be between 1 and 8".into()));
    }
    for permission in &manifest.permissions {
        Permission::parse(permission).map_err(|e| invalid(e.message))?;
    }
    Ok(manifest)
}

pub fn discover(home: &Path) -> Res<Vec<(PathBuf, Manifest)>> {
    Ok(discover_report(home)?.0)
}

pub fn discover_report(home: &Path) -> Res<(Vec<(PathBuf, Manifest)>, Vec<(PathBuf, Failure)>)> {
    let entries = match std::fs::read_dir(modules_root(home)) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(Failure::new("io", e.to_string())),
    };
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| Failure::new("io", e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| Failure::new("io", e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let path = entry.path();
        match load(&path) {
            Ok(m) => valid.push((path, m)),
            Err(e) => invalid.push((path, e)),
        }
    }
    valid.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    invalid.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((valid, invalid))
}
