use axiom::engine::FILES;

fn asset(path: &str) -> &'static str {
    FILES.iter().find(|(name, _)| *name == path).unwrap().1
}

#[test]
fn agents_set_auto_compact_window_after_model() {
    for path in ["agents/dispatcher.md", "agents/reviewer.md"] {
        let mut lines = asset(path).lines();
        assert_eq!(lines.next(), Some("---"), "{path}");
        let remaining: Vec<_> = lines.collect();
        let end = remaining.iter().position(|line| *line == "---")
            .unwrap_or_else(|| panic!("{path}: missing frontmatter end"));
        let frontmatter = &remaining[..end];
        assert!(
            frontmatter.windows(2).any(|pair| pair == ["model: sonnet", "autoCompactWindow: 200000"]),
            "{path}: expected autoCompactWindow directly after model: sonnet"
        );
    }
}

#[test]
fn wish_skill_has_no_auto_compact_window() {
    assert!(!asset("skills/wish/SKILL.md").contains("autoCompactWindow"));
}
