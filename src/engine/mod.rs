use crate::base;
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
