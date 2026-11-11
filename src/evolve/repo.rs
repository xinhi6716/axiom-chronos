use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RepoInfo {
    pub root: PathBuf,
    pub base_commit: String,
    pub branch: String,
    pub original_branch: String,
}

fn io(error: impl std::fmt::Display) -> Failure {
    Failure::new("io", error.to_string())
}

fn git(dir: &Path, args: &[&str]) -> Res<std::process::Output> {
    base::proc::cmd("git")
        .current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .map_err(io)
}

fn git_text(dir: &Path, args: &[&str]) -> Res<String> {
    let output = git(dir, args)?;
    if !output.status.success() {
        return Err(Failure::new(
            "git",
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim_end().to_owned())
        .map_err(io)
}

fn is_axiom_package(contents: &str) -> bool {
    let mut package = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            package = line.split('#').next().unwrap_or("").trim() == "[package]";
        } else if package {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() != "name" {
                    continue;
                }
                let value = value.trim();
                let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') else {
                    return false;
                };
                let rest = &value[1..];
                let Some(end) = rest.find(quote) else {
                    return false;
                };
                let trailing = rest[end + 1..].trim();
                return &rest[..end] == "axiom-chronos"
                    && (trailing.is_empty() || trailing.starts_with('#'));
            }
        }
    }
    false
}

pub fn check(dir: &Path, date: &str) -> Res<RepoInfo> {
    let root = fs::canonicalize(dir)
        .map_err(|_| Failure::new("not_git_repo", "repository directory does not exist"))?;
    let output = git(&root, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        return Err(Failure::new("not_git_repo", "not a git worktree"));
    }
    let top = String::from_utf8(output.stdout).map_err(io)?;
    let top = fs::canonicalize(top.trim_end()).map_err(io)?;
    if root != top {
        return Err(Failure::new(
            "not_repo_root",
            "directory is not the worktree root",
        ));
    }
    let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap_or_default();
    if !is_axiom_package(&manifest) {
        return Err(Failure::new(
            "wrong_package",
            "not the axiom-chronos package",
        ));
    }
    if !git_text(&root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return Err(Failure::new("dirty_tree", "worktree has changes"));
    }
    let branch = format!("evolve/{date}");
    let reference = format!("refs/heads/{branch}");
    let exists = git(&root, &["show-ref", "--verify", "--quiet", &reference])?;
    if exists.status.success() {
        return Err(Failure::new(
            "branch_exists",
            "evolve branch already exists",
        ));
    }
    if exists.status.code() != Some(1) {
        return Err(Failure::new(
            "git",
            String::from_utf8_lossy(&exists.stderr).trim(),
        ));
    }
    Ok(RepoInfo {
        base_commit: git_text(&root, &["rev-parse", "--verify", "HEAD"])?,
        original_branch: git_text(&root, &["symbolic-ref", "--short", "HEAD"])?,
        root,
        branch,
    })
}

pub fn create_branch(info: &RepoInfo) -> Res<()> {
    let output = git(&info.root, &["switch", "-c", &info.branch])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Failure::new(
            "git",
            String::from_utf8_lossy(&output.stderr).trim(),
        ))
    }
}

#[derive(Serialize, Deserialize)]
struct Settings {
    repo: PathBuf,
}

pub fn remembered(home: &Path) -> Res<Option<PathBuf>> {
    let contents = match fs::read(home.join("evolve/settings.json")) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    let settings: Settings = serde_json::from_slice(&contents).map_err(io)?;
    if !settings.repo.is_absolute() {
        return Err(Failure::new(
            "invalid_argument",
            "remembered repository must be absolute",
        ));
    }
    Ok(Some(settings.repo))
}

pub fn remember(home: &Path, dir: &Path) -> Res<()> {
    let repo = fs::canonicalize(dir).map_err(io)?;
    let path = home.join("evolve/settings.json");
    fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    let bytes = serde_json::to_vec_pretty(&Settings { repo }).map_err(io)?;
    base::atomic_write(&path, &bytes).map_err(io)
}

pub fn resolve(home: &Path, repo: Option<&Path>) -> Res<PathBuf> {
    if let Some(repo) = repo {
        return Ok(repo.to_path_buf());
    }
    remembered(home)?
        .ok_or_else(|| Failure::new("no_repo", "no repository specified or remembered"))
}
