use crate::base::{atomic_write, Failure, Res};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path},
};

const NAME: &str = "axiom-laya.json";
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub format: u32,
    pub model_version: String,
    pub layout: String,
    pub source: String,
    pub revision: String,
    pub files: BTreeMap<String, String>,
}
fn io_failure(path: &Path, e: impl std::fmt::Display) -> Failure {
    Failure::new("laya_artifact_io", format!("{}: {e}", path.display()))
}
pub fn sha256_file(path: &Path) -> Res<String> {
    let file = File::open(path).map_err(|e| io_failure(path, e))?;
    let mut reader = BufReader::new(file);
    let mut hash = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| io_failure(path, e))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn read_manifest(dir: &Path) -> Res<Manifest> {
    let path = dir.join(NAME);
    let bytes = fs::read(&path).map_err(|e| io_failure(&path, e))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|e| Failure::new("laya_manifest_invalid", format!("{}: {e}", path.display())))?;
    if manifest.format != 1 {
        return Err(Failure::new(
            "laya_manifest_invalid",
            format!("unsupported manifest format {}", manifest.format),
        ));
    }
    Ok(manifest)
}
pub fn verify_dir(dir: &Path) -> Res<Manifest> {
    let manifest = read_manifest(dir)?;
    for (name, expected) in &manifest.files {
        let path = Path::new(name);
        if name.is_empty()
            || name.contains('\\')
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Failure::new(
                "laya_manifest_invalid",
                format!("invalid artifact path {name}"),
            ));
        }
        let full = dir.join(path);
        if !full.is_file() {
            return Err(Failure::new(
                "laya_artifact_missing",
                format!("missing artifact {name}"),
            ));
        }
        if sha256_file(&full)? != *expected {
            return Err(Failure::new(
                "laya_sha256_mismatch",
                format!("sha256 mismatch: {name}"),
            ));
        }
    }
    Ok(manifest)
}
fn collect(dir: &Path, here: &Path, files: &mut BTreeMap<String, String>) -> Res<()> {
    for entry in fs::read_dir(here).map_err(|e| io_failure(here, e))? {
        let entry = entry.map_err(|e| io_failure(here, e))?;
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| io_failure(&path, e))?;
        if ty.is_dir() {
            collect(dir, &path, files)?;
        } else if ty.is_file() && path != dir.join(NAME) {
            let rel = path.strip_prefix(dir).map_err(|e| io_failure(&path, e))?;
            let name = rel
                .components()
                .map(|c| {
                    c.as_os_str().to_str().ok_or_else(|| {
                        Failure::new(
                            "laya_artifact_io",
                            format!("non-UTF-8 artifact path: {}", path.display()),
                        )
                    })
                })
                .collect::<Res<Vec<_>>>()?
                .join("/");
            files.insert(name, sha256_file(&path)?);
        }
    }
    Ok(())
}
pub fn write_manifest(
    dir: &Path,
    model_version: &str,
    layout: &str,
    source: &str,
    revision: &str,
) -> Res<Manifest> {
    let mut files = BTreeMap::new();
    collect(dir, dir, &mut files)?;
    let manifest = Manifest {
        format: 1,
        model_version: model_version.into(),
        layout: layout.into(),
        source: source.into(),
        revision: revision.into(),
        files,
    };
    let value = serde_json::to_value(&manifest)
        .map_err(|e| Failure::new("laya_manifest_invalid", e.to_string()))?;
    let mut bytes = serde_json::to_vec_pretty(&value)
        .map_err(|e| Failure::new("laya_manifest_invalid", e.to_string()))?;
    bytes.push(b'\n');
    let path = dir.join(NAME);
    atomic_write(&path, &bytes).map_err(|e| io_failure(&path, e))?;
    Ok(manifest)
}
