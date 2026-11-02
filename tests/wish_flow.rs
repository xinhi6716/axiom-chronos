const SENTENCE: &str = "If creative mode chose no candidate, skip only the architect and the dispatcher, then run step 5 (memory keeper and Gemini rewrite), step 6 (report) and step 7 (close) with the exploration and ruling as the report.";

#[test]
fn creative_no_candidate_runs_remaining_steps() {
    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;

    let has = |value: &str| value.contains(SENTENCE);
    assert!(has(text));
    for word in ["memory keeper", "rewrite", "report", "close"] {
        assert!(SENTENCE.contains(word));
        let replacement = SENTENCE.replace(word, "");
        assert_ne!(replacement, SENTENCE);
        let modified = text.replace(SENTENCE, &replacement);
        assert!(!has(&modified));
    }

    let has_forbidden = |value: &str| value.contains("skip to the close");
    assert!(!has_forbidden(text));
    assert!(has_forbidden(&format!("{text}\nskip to the close")));

    let before_step_five = |value: &str| {
        value
            .split('\n')
            .take_while(|line| !line.starts_with("5. "))
            .any(|line| line.contains(SENTENCE))
    };
    assert!(before_step_five(text));
    assert!(text.lines().any(|line| line.starts_with("5. ")));
    let step_five = text.lines().find(|line| line.starts_with("5. ")).unwrap();
    let moved = text
        .replace(SENTENCE, "")
        .replacen(step_five, &format!("{step_five}\n{SENTENCE}"), 1);
    assert!(!before_step_five(&moved));
    assert!(text.lines().any(|line| line.starts_with("7. ")));
}
