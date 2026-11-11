use std::{fs, path::Path};
#[test]
fn release_and_council_documentation_match_the_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(env!("CARGO_PKG_VERSION"), "2.0.1");
    for path in [
        "engine/.claude-plugin/plugin.json",
        ".claude-plugin/marketplace.json",
    ] {
        assert!(fs::read_to_string(root.join(path))
            .unwrap()
            .contains("2.0.1"));
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
