#[test]
fn release_and_worker_docs_explain_the_cap() {
    for text in [
        include_str!("../CHANGELOG.md"),
        include_str!("../docs/releases/2.0.1.md"),
        include_str!("../README.md"),
        include_str!("../README.zh-TW.md"),
    ] {
        assert!(text.contains("AXIOM_CODEX_MAX_PARALLEL") && text.contains("48"));
    }
}
