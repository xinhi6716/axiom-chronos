use super::compute_dir;
use crate::{
    base::{atomic_write, Failure, Res},
    seams::{sha256_hex, ApprovalRequest, Decision, PersonApproval},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UploadManifest {
    pub project: String,
    pub entries: Vec<ManifestEntry>,
    pub digest: String,
}
fn io_error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn escape() -> Failure {
    Failure::new("path_escape", "input escapes project directory")
}
fn collect(
    root: &Path,
    rel: &Path,
    entries: &mut BTreeMap<String, ManifestEntry>,
    ancestors: &mut HashSet<PathBuf>,
) -> Res<()> {
    let full = root.join(rel);
    let resolved = fs::canonicalize(&full).map_err(io_error)?;
    if !resolved.starts_with(root) {
        return Err(escape());
    }
    let metadata = fs::metadata(&resolved).map_err(io_error)?;
    if metadata.is_dir() {
        if !ancestors.insert(resolved.clone()) {
            return Err(escape());
        }
        for item in fs::read_dir(&full).map_err(io_error)? {
            let item = item.map_err(io_error)?;
            collect(root, &rel.join(item.file_name()), entries, ancestors)?;
        }
        ancestors.remove(&resolved);
    } else if metadata.is_file() {
        let path = rel
            .components()
            .map(|c| {
                c.as_os_str()
                    .to_str()
                    .ok_or_else(|| Failure::new("invalid_path", "non-UTF-8 path"))
            })
            .collect::<Res<Vec<_>>>()?
            .join("/");
        let data = fs::read(&resolved).map_err(io_error)?;
        entries.insert(
            path.clone(),
            ManifestEntry {
                path,
                bytes: data.len() as u64,
                sha256: sha256_hex(&data),
            },
        );
    } else {
        return Err(Failure::new(
            "invalid_path",
            "input is not a regular file or directory",
        ));
    }
    Ok(())
}
pub fn build_manifest(project: &str, project_dir: &Path, inputs: &[String]) -> Res<UploadManifest> {
    let root = fs::canonicalize(project_dir).map_err(io_error)?;
    let mut entries = BTreeMap::new();
    for input in inputs {
        let path = Path::new(input);
        // Reject Windows absolute paths and traversal even when examined on Unix.
        if input.starts_with(['/', '\\'])
            || input.as_bytes().get(1) == Some(&b':')
            || input.split(['/', '\\']).any(|p| p == "..")
            || path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(escape());
        }
        collect(&root, path, &mut entries, &mut HashSet::new())?;
    }
    let entries: Vec<_> = entries.into_values().collect();
    let digest = sha256_hex(&serde_json::to_vec(&(project, &entries)).map_err(io_error)?);
    Ok(UploadManifest {
        project: project.to_owned(),
        entries,
        digest,
    })
}
pub fn render(manifest: &UploadManifest) -> String {
    let mut result = String::new();
    let mut total = 0u64;
    for entry in &manifest.entries {
        total = total.saturating_add(entry.bytes);
        let short: String = entry.sha256.chars().take(12).collect();
        result.push_str(&format!(
            "{}  {} bytes  {}\n",
            entry.path, entry.bytes, short
        ));
    }
    result.push_str(&format!(
        "Total: {} files, {} bytes",
        manifest.entries.len(),
        total
    ));
    result
}
static CONSENT_LOCK: Mutex<()> = Mutex::new(());
pub struct ConsentStore {
    path: PathBuf,
}
impl ConsentStore {
    pub fn open(home: &Path) -> Res<Self> {
        let dir = compute_dir(home);
        fs::create_dir_all(&dir).map_err(io_error)?;
        Ok(Self {
            path: dir.join("consent.json"),
        })
    }
    fn read(&self) -> Res<BTreeMap<String, String>> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io_error),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(io_error(e)),
        }
    }
    fn write(&self, data: &BTreeMap<String, String>) -> Res<()> {
        atomic_write(&self.path, &serde_json::to_vec(data).map_err(io_error)?).map_err(io_error)
    }
    pub fn is_granted(&self, manifest: &UploadManifest) -> Res<bool> {
        let _guard = CONSENT_LOCK.lock().unwrap();
        Ok(self.read()?.get(&manifest.project) == Some(&manifest.digest))
    }
    pub fn request(&self, manifest: &UploadManifest, approval: &dyn PersonApproval) -> Res<bool> {
        let request = ApprovalRequest {
            subject: format!("compute:{}", manifest.project),
            action: "upload".into(),
            summary: render(manifest),
            digest: manifest.digest.clone(),
        };
        if approval.decide(&request) != Decision::Approved {
            return Ok(false);
        }
        let _guard = CONSENT_LOCK.lock().unwrap();
        let mut data = self.read()?;
        data.insert(manifest.project.clone(), manifest.digest.clone());
        self.write(&data)?;
        Ok(true)
    }
    pub fn revoke(&self, project: &str) -> Res<()> {
        let _guard = CONSENT_LOCK.lock().unwrap();
        let mut data = self.read()?;
        data.remove(project);
        self.write(&data)
    }
}
