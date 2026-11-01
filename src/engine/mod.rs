use std::path::{Path, PathBuf};

// Engine is the source of truth; the checked-in Codex package is its distributable copy.
pub const FILES: &[(&str, &str)] = &[
    ("DISPATCH.md", include_str!("../../engine/DISPATCH.md")),
    (
        "agents/dispatcher.md",
        include_str!("../../engine/agents/dispatcher.md"),
    ),
    (
        "agents/astra.md",
        include_str!("../../engine/agents/astra.md"),
    ),
    (
        "agents/architect.md",
        include_str!("../../engine/agents/architect.md"),
    ),
    (
        "agents/engineer.md",
        include_str!("../../engine/agents/engineer.md"),
    ),
    (
        "agents/assembler.md",
        include_str!("../../engine/agents/assembler.md"),
    ),
    (
        "agents/reviewer.md",
        include_str!("../../engine/agents/reviewer.md"),
    ),
    (
        "agents/memory-keeper.md",
        include_str!("../../engine/agents/memory-keeper.md"),
    ),
    (
        "agents/artifact.md",
        include_str!("../../engine/agents/artifact.md"),
    ),
    (
        ".codex-plugin/plugin.json",
        include_str!("../../engine/.codex-plugin/plugin.json"),
    ),
    (
        "skills/wish/SKILL.md",
        include_str!("../../engine/skills/wish/SKILL.md"),
    ),
    (
        "skills/research/SKILL.md",
        include_str!("../../engine/skills/research/SKILL.md"),
    ),
    (
        "assets/chronos.svg",
        include_str!("../../codex-plugin/assets/chronos.svg"),
    ),
    (
        "assets/chronos-dark.svg",
        include_str!("../../codex-plugin/assets/chronos-dark.svg"),
    ),
];

pub fn plugin_dir(home: &Path) -> std::io::Result<PathBuf> {
    let dir = home.join("engine").join(env!("CARGO_PKG_VERSION"));
    for &(path, contents) in FILES {
        let target = dir.join(path);
        if std::fs::read(&target).ok().as_deref() != Some(contents.as_bytes()) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, contents)?;
        }
    }
    Ok(dir)
}
