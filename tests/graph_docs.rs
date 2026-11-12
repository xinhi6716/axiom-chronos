use std::{fs, path::Path};

fn contains_all(text: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| text.contains(needle))
}

#[test]
fn graph_documents_and_agent_contracts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: &[(&str, &[&str])] = &[
        (
            "docs/bridge.md",
            &[
                "graph_explore",
                "memory_record",
                "axiom dream",
                "axiom graph",
                "CONSTRAINTS.md",
                "aliases.csv",
                "launch",
            ],
        ),
        ("README.md", &["syn", "tree-sitter"]),
        ("README.zh-TW.md", &["syn", "tree-sitter"]),
        (
            "docs/graph-ab.md",
            &[
                "not independent",
                "tree-sitter",
                "unknown",
                "10%",
                "Result: measured; frozen acceptance not met",
                "<!-- AB_RESULT -->",
                "-24.08%",
                "665991",
                "3654546",
                "src/inventory.rs",
            ],
        ),
        ("engine/agents/dispatcher.md", &["graph_explore"]),
        ("engine/agents/architect.md", &["graph_explore"]),
        ("engine/agents/reviewer.md", &["graph_explore"]),
        ("engine/agents/memory-keeper.md", &["memory_record"]),
        ("engine/skills/wish/SKILL.md", &["Launch context"]),
    ];
    for (path, needles) in cases {
        let text = fs::read_to_string(root.join(path))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(
            contains_all(&text, needles),
            "{path} is missing a required phrase"
        );
    }
}

#[test]
fn contains_all_rejects_a_missing_needle() {
    assert!(contains_all("alpha beta", &["alpha", "beta"]));
    assert!(!contains_all("alpha beta", &["alpha", "missing"]));
}
