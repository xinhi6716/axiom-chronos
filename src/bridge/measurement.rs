//! Measurement runs in a disposable source copy, never in the product worktree.
use crate::base::{self, Failure, Res};
use crate::orch;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::Command};

fn error(e: impl std::fmt::Display) -> Failure { Failure::new("measurement", e.to_string()) }
fn canonical(path: &Path) -> Res<PathBuf> {
    let path = fs::canonicalize(path).map_err(error)?;
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") { return Ok(PathBuf::from(format!(r"\\{rest}"))); }
        if let Some(rest) = text.strip_prefix(r"\\?\") { return Ok(PathBuf::from(rest)); }
    }
    Ok(path)
}
fn git(repo: &Path, args: &[&str], index: Option<&Path>) -> Res<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.current_dir(repo).args(args);
    if let Some(index) = index { cmd.env("GIT_INDEX_FILE", index); }
    let out = cmd.output().map_err(error)?;
    if !out.status.success() { return Err(error(String::from_utf8_lossy(&out.stderr))); }
    Ok(out.stdout)
}

pub(super) struct Measurement {
    pub cwd: PathBuf,
    dir: PathBuf,
    source: PathBuf,
    identity: String,
    files: BTreeMap<String, String>,
    git_state: Value,
}
impl Measurement {
    pub fn prepare(home: &Path, source: &Path, session: Option<&str>) -> Res<Self> {
        let source = canonical(source)?;
        fs::create_dir_all(home).map_err(error)?;
        let home = canonical(home)?;
        if home.starts_with(&source) { return Err(error("measurement data home must be outside the product worktree")); }
        let identity = orch::snapshot(&source, &[".".into()]);
        let tree = identity.strip_prefix("tree:").ok_or_else(|| error("measurement requires a Git source snapshot"))?;
        let listing = git(&source, &["ls-tree", "-r", "-z", tree], None)?;
        let mut names = Vec::new();
        for row in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let row = std::str::from_utf8(row).map_err(error)?;
            let (meta, name) = row.split_once('\t').ok_or_else(|| error("invalid Git inventory"))?;
            if !(meta.starts_with("100644 blob ") || meta.starts_with("100755 blob ")) {
                return Err(error("measurement snapshot rejects symlinks and submodules"));
            }
            if !Path::new(name).components().all(|c| matches!(c, std::path::Component::Normal(_))) {
                return Err(error("invalid source path"));
            }
            names.push(name.to_owned());
        }
        let mut artifacts = home.join("artifacts");
        if let Some(session) = session { base::require_slug("session", session)?; artifacts.push(session); }
        let dir = artifacts.join("measurements").join(base::unique());
        let cwd = dir.join("source");
        fs::create_dir_all(&cwd).map_err(error)?;
        let index = dir.join("checkout.index");
        git(&source, &["read-tree", tree], Some(&index))?;
        let prefix = format!("--prefix={}/", cwd.to_string_lossy().replace('\\', "/"));
        git(&source, &["checkout-index", "--all", &prefix], Some(&index))?;
        fs::remove_file(&index).map_err(error)?;
        let mut files = BTreeMap::new();
        for name in names { files.insert(name.clone(), hash(&cwd.join(name))?); }
        let git_state = json!({
            "head":git(&source,&["rev-parse","--verify","HEAD"],None).ok().map(|v|String::from_utf8_lossy(&v).trim().to_owned()),
            "branch":git(&source,&["symbolic-ref","--quiet","--short","HEAD"],None).ok().map(|v|String::from_utf8_lossy(&v).trim().to_owned()),
            "index_diff":String::from_utf8_lossy(&git(&source,&["diff","--cached","--binary","--no-ext-diff"],None)?),
            "worktree_diff":String::from_utf8_lossy(&git(&source,&["diff","--binary","--no-ext-diff"],None)?),
            "untracked":String::from_utf8_lossy(&git(&source,&["ls-files","--others","--exclude-standard","-z"],None)?)
        });
        let prepared = Self { cwd, dir, source, identity, files, git_state };
        prepared.write(&json!({"status":"pending","source":prepared.source,"source_identity":prepared.identity,
            "workspace":prepared.cwd,"original_files":prepared.files,"git_state":prepared.git_state,"scope":"Git-visible dirty worktree; ignored inputs are not included"}))?;
        Ok(prepared)
    }
    fn write(&self, value: &Value) -> Res<()> {
        base::atomic_write(&self.dir.join("receipt.json"), &serde_json::to_vec_pretty(value).map_err(error)?).map_err(error)
    }
    pub fn finish(&self, report: &mut Value) -> Res<()> {
        let artifacts = inventory(&self.cwd, &self.cwd);
        let unchanged = self.files.iter().all(|(name, expected)| hash(&self.cwd.join(name)).ok().as_ref() == Some(expected));
        let source_after = orch::snapshot(&self.source, &[".".into()]);
        let source_unchanged = !source_after.is_empty() && source_after == self.identity;
        if !unchanged || !source_unchanged || artifacts.is_err() {
            report["status"] = json!("failed");
            report["error"] = json!({"kind":"measurement_source_changed","message":"measurement changed snapshot inputs or the product source changed concurrently"});
        }
        let receipt = self.dir.join("receipt.json");
        let sealed = json!({"status":report["status"],"source":self.source,"source_identity":self.identity,
            "source_after":source_after,"source_unchanged":source_unchanged,"snapshot_inputs_unchanged":unchanged,
            "workspace":self.cwd,"original_files":self.files,"git_state":self.git_state,"artifact_sha256":artifacts.as_ref().ok(),"artifact_error":artifacts.as_ref().err().map(|e|e.to_json()),"report":report,
            "acceptance":"measurement report only; not engineering release or final acceptance"});
        self.write(&sealed)?;
        report["measurement"] = json!({"receipt":receipt,"sha256":hash(&receipt)?,"workspace":self.cwd,
            "source_identity":self.identity,"source_unchanged":source_unchanged,"snapshot_inputs_unchanged":unchanged});
        Ok(())
    }
}
fn inventory(root: &Path, dir: &Path) -> Res<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(dir).map_err(error)? {
        let path = entry.map_err(error)?.path();
        let meta = fs::symlink_metadata(&path).map_err(error)?;
        #[cfg(windows)]
        { use std::os::windows::fs::MetadataExt; if meta.file_attributes() & 0x400 != 0 { return Err(error("linked measurement artifact")); } }
        if meta.file_type().is_symlink() { return Err(error("linked measurement artifact")); }
        if meta.is_dir() { files.extend(inventory(root, &path)?); }
        else { files.insert(path.strip_prefix(root).map_err(error)?.to_string_lossy().replace('\\', "/"), hash(&path)?); }
    }
    Ok(files)
}
fn hash(path: &Path) -> Res<String> {
    let meta = fs::symlink_metadata(path).map_err(error)?;
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; if meta.file_attributes() & 0x400 != 0 { return Err(error("linked measurement input")); } }
    if !meta.is_file() || meta.file_type().is_symlink() { return Err(error("non-file measurement input")); }
    Ok(format!("{:x}", Sha256::digest(fs::read(path).map_err(error)?)))
}
