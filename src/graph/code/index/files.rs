use crate::graph::code::language_of;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: String,
    pub abs: PathBuf,
}
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn classify(bytes: &[u8]) -> Option<&'static str> {
    if bytes.iter().take(8192).any(|b| *b == 0) {
        return Some("binary");
    }
    // Generated markers are only meaningful in the file header.
    let header = String::from_utf8_lossy(&bytes[..bytes.len().min(8192)]);
    if header
        .lines()
        .take(10)
        .any(|line| line.contains("@generated") || line.contains("DO NOT EDIT"))
    {
        return Some("generated");
    }
    None
}
fn excluded(path: &str) -> bool {
    path.split('/').any(|p| {
        matches!(
            p,
            ".git"
                | ".chronos"
                | "target"
                | "build"
                | "dist"
                | "venv"
                | ".venv"
                | "node_modules"
                | "__pycache__"
        )
    }) || path.ends_with(".min.js")
}
fn walk(
    root: &Path,
    dir: &Path,
    seen: &mut HashSet<PathBuf>,
    paths: &mut Vec<String>,
    skipped: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let canon = fs::canonicalize(dir).map_err(|e| e.to_string())?;
    if !seen.insert(canon) {
        return Ok(());
    }
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if excluded(&rel) {
            continue;
        }
        let target = match fs::canonicalize(&path) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((rel, e.to_string()));
                continue;
            }
        };
        if !target.starts_with(root) {
            skipped.push((rel, "symlink escapes root".into()));
            continue;
        }
        if target.is_dir() {
            walk(root, &path, seen, paths, skipped)?;
        } else if target.is_file() {
            paths.push(rel);
        }
    }
    Ok(())
}
pub fn discover(root: &Path) -> Result<(Vec<Candidate>, Vec<(String, String)>), String> {
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let git = crate::base::proc::cmd("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .current_dir(&root)
        .output();
    let mut skipped = Vec::new();
    let mut paths = match git {
        Ok(o) if o.status.success() => String::from_utf8(o.stdout)
            .map_err(|e| e.to_string())?
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('\\', "/"))
            .collect(),
        _ => {
            let mut p = Vec::new();
            walk(&root, &root, &mut HashSet::new(), &mut p, &mut skipped)?;
            p
        }
    };
    paths.sort();
    paths.dedup();
    let mut candidates = Vec::new();
    for path in paths {
        if excluded(&path) {
            continue;
        }
        let abs = root.join(&path);
        let target = match fs::canonicalize(&abs) {
            Ok(p) => p,
            Err(e) => {
                skipped.push((path, e.to_string()));
                continue;
            }
        };
        if !target.starts_with(&root) {
            skipped.push((path, "symlink escapes root".into()));
            continue;
        }
        if !target.is_file() || language_of(&path).is_none() {
            continue;
        }
        match fs::read(&abs) {
            Ok(bytes) => {
                if let Some(reason) = classify(&bytes) {
                    skipped.push((path, reason.into()));
                } else {
                    candidates.push(Candidate { path, abs });
                }
            }
            Err(e) => skipped.push((path, e.to_string())),
        }
    }
    skipped.sort();
    Ok((candidates, skipped))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_and_hash() {
        assert_eq!(classify(b"hello\0world"), Some("binary"));
        assert_eq!(classify(b"// @generated\n"), Some("generated"));
        assert_eq!(classify(b"fn main() {}"), None);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn discovery_walk() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("main.rs"), "fn main() {}").unwrap();
        fs::write(d.path().join("generated.py"), "# DO NOT EDIT\n").unwrap();
        fs::create_dir(d.path().join("target")).unwrap();
        fs::write(d.path().join("target/no.rs"), "").unwrap();
        let (files, skipped) = discover(d.path()).unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["main.rs"]
        );
        assert!(skipped.contains(&("generated.py".into(), "generated".into())));
    }
}
