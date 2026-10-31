use std::path::{Path, PathBuf};

pub const FILES: &[(&str, &str)] = &[
    ("agents/tester.md", include_str!("../../engine/agents/tester.md")),
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
        "agents/content.md",
        include_str!("../../engine/agents/content.md"),
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
