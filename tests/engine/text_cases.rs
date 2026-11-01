use std::fs;
#[test]
fn skills_explain_boundary_and_replay() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/skills");
    let wish = fs::read_to_string(root.join("wish/SKILL.md")).unwrap();
    let research = fs::read_to_string(root.join("research/SKILL.md")).unwrap();
    for phrase in [
        "dispatch",
        "request id",
        "needs_intent_review",
        "Unknown writes",
        "ready_to_commit",
    ] {
        assert!(wish.contains(phrase), "{phrase}");
    }
    for phrase in [
        "needs_intent_review",
        "do not automatically decide",
        "request id",
    ] {
        assert!(research.contains(phrase), "{phrase}");
    }
}
