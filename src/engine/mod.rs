use crate::base;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const FILES: &[(&str, &str)] = &[
    (
        ".claude-plugin/plugin.json",
        include_str!("../../engine/.claude-plugin/plugin.json"),
    ),
    ("DISPATCH.md", include_str!("../../engine/DISPATCH.md")),
    (
        "agents/architect.md",
        include_str!("../../engine/agents/architect.md"),
    ),
    (
        "agents/astra.md",
        include_str!("../../engine/agents/astra.md"),
    ),
    (
        "agents/critic.md",
        include_str!("../../engine/agents/critic.md"),
    ),
    (
        "agents/dispatcher.md",
        include_str!("../../engine/agents/dispatcher.md"),
    ),
    (
        "agents/memory-keeper.md",
        include_str!("../../engine/agents/memory-keeper.md"),
    ),
    (
        "agents/narrator.md",
        include_str!("../../engine/agents/narrator.md"),
    ),
    (
        "agents/reviewer.md",
        include_str!("../../engine/agents/reviewer.md"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../engine/hooks/hooks.json"),
    ),
    (
        "research/audit.schema.json",
        include_str!("../../engine/research/audit.schema.json"),
    ),
    (
        "research/council.schema.json",
        include_str!("../../engine/research/council.schema.json"),
    ),
    (
        "research/plan.schema.json",
        include_str!("../../engine/research/plan.schema.json"),
    ),
    (
        "research/review.schema.json",
        include_str!("../../engine/research/review.schema.json"),
    ),
    (
        "skills/research/SKILL.md",
        include_str!("../../engine/skills/research/SKILL.md"),
    ),
    (
        "skills/wish/SKILL.md",
        include_str!("../../engine/skills/wish/SKILL.md"),
    ),
];

fn short_hash(digest: &[u8]) -> String {
    digest[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Lengths are hashed too, so bytes cannot move between a path and its contents unnoticed.
#[doc(hidden)]
pub fn content_id(files: &[(&str, &str)]) -> String {
    let mut hash = Sha256::new();
    for (path, contents) in files {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update((contents.len() as u64).to_le_bytes());
        hash.update(contents);
    }
    short_hash(&hash.finalize())
}

pub fn build_id() -> String {
    content_id(FILES)
}

pub fn binary_id(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| short_hash(&Sha256::digest(bytes)))
}

/// Each build gets its own directory and none is ever removed, so a build cannot change or delete
/// the rules another running build is reading.
#[doc(hidden)]
pub fn install(home: &Path, version: &str, files: &[(&str, &str)]) -> std::io::Result<PathBuf> {
    let dir = home
        .join("engine")
        .join(format!("{version}-{}", content_id(files)));
    for &(path, contents) in files {
        let target = dir.join(path);
        let in_place = || std::fs::read(&target).ok().as_deref() == Some(contents.as_bytes());
        if !in_place() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // A concurrent launch of this build can hold the file open once it has written it.
            if let Err(e) = base::atomic_write(&target, contents.as_bytes()) {
                if !in_place() {
                    return Err(e);
                }
            }
        }
    }
    Ok(dir)
}

pub fn plugin_dir(home: &Path) -> std::io::Result<PathBuf> {
    install(home, env!("CARGO_PKG_VERSION"), FILES)
}

pub fn plugin_version(root: &Path) -> Result<String, String> {
    let manifest = root.join(".claude-plugin/plugin.json");
    let bytes =
        std::fs::read(&manifest).map_err(|e| format!("cannot read {}: {e}", manifest.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} is not valid JSON: {e}", manifest.display()))?;
    value["version"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} declares no version", manifest.display()))
}

fn normalized(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// Embedded files the plugin at `root` lacks or holds with other contents. Line endings do not
/// count: a Windows checkout of the same commit differs from the build only in those.
pub fn differing_files(root: &Path) -> Vec<&'static str> {
    FILES
        .iter()
        .filter(|(path, contents)| {
            std::fs::read(root.join(path)).map_or(true, |bytes| {
                normalized(&bytes) != normalized(contents.as_bytes())
            })
        })
        .map(|(path, _)| *path)
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct PluginCheck {
    pub version: String,
    pub differing: Vec<&'static str>,
}

/// Err is the explanation to stop on: the plugin at `root` cannot be shown to be this binary's.
pub fn check_plugin(root: &Path) -> Result<PluginCheck, String> {
    let binary = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe().map_or_else(|_| "axiom".into(), |p| p.display().to_string());
    let version = plugin_version(root).map_err(|why| {
        format!(
            "the version of the plugin at {} is unavailable ({why}) and the axiom binary at {exe} is version {binary}. Reinstall the plugin, for example `claude plugin update chronos@axiom-chronos`, then restart Claude Code and run `axiom doctor`.",
            root.display()
        )
    })?;
    if version != binary {
        return Err(format!(
            "the plugin at {} is version {version} but the axiom binary at {exe} is version {binary}, and the plugin's rules and the binary's gates must come from the same release. Update the plugin with `claude plugin update chronos@axiom-chronos`, or install axiom {version} and put it on PATH or in AXIOM_BIN, then restart Claude Code and run `axiom doctor`.",
            root.display()
        ));
    }
    Ok(PluginCheck {
        version,
        differing: differing_files(root),
    })
}
