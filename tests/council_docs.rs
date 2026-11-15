use std::{fs, path::Path};
#[test]
fn release_and_council_documentation_match_the_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(env!("CARGO_PKG_VERSION"), "3.0.0-beta.1");
    for path in [
        "engine/.claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
    ] {
        assert!(fs::read_to_string(root.join(path))
            .unwrap()
            .contains("3.0.0-beta.1"));
    }
    for path in [
        "README.md",
        "README.zh-TW.md",
        "engine/research/COUNCIL.md",
        "docs/releases/2.0.1.md",
    ] {
        let text = fs::read_to_string(root.join(path)).unwrap();
        for key in [
            "AXIOM_FABLE_CAP_USD",
            "AXIOM_ASTRA_TOKEN_CAP",
            "AXIOM_ARGON_TOKEN_CAP",
            "AXIOM_COUNCIL_ROUNDS",
        ] {
            assert!(text.contains(key), "{path}: {key}");
        }
    }
    let release = fs::read_to_string(root.join("docs/releases/2.0.1.md")).unwrap();
    for text in ["繁體中文", "## Install", "## Known limits"] {
        assert!(release.contains(text), "{text}");
    }
    assert!(fs::read_to_string(root.join("CHANGELOG.md"))
        .unwrap()
        .contains("## 2.0.1"));
}

#[test]
fn council_doc_states_robust_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = fs::read_to_string(root.join("engine/research/COUNCIL.md")).unwrap();
    for phrase in [
        "one repair ask in the same turn",
        ".raw.txt",
        ".repair.raw.txt",
        "degraded_members",
        "member_degraded",
        "council-events.jsonl",
        "Anonymisation rewrites only attribution",
    ] {
        assert!(text.contains(phrase), "{phrase}");
    }
    assert!(!text.replace("member_degraded", "").contains("member_degraded"));
}
