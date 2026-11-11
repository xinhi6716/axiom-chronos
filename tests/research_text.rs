use axiom::engine;

fn missing_phrases<'a>(text: &str, phrases: &'a [&str]) -> Vec<&'a str> {
    phrases
        .iter()
        .copied()
        .filter(|phrase| !text.contains(phrase))
        .collect()
}

#[test]
fn research_instructions_are_embedded_and_checked() {
    let cases: &[(&str, &[&str])] = &[
        ("skills/research/SKILL.md", &[
            "For empirical questions",
            "the claim and alternative explanations in `claims`",
            "the baseline, separation of development and acceptance data, and a threshold fixed before running in `design`",
            "the counterexample that would refute the claim in `falsification`",
            "Literature questions are exempt from experiments",
            "Back each hard claim in the plan with a quote line",
            "data leakage between development and acceptance data",
            "thresholds changed after results",
        ]),
        ("agents/critic.md", &[
            "In the second review",
            "data leakage between development and acceptance data",
            "thresholds changed after results",
        ]),
    ];
    for &(path, phrases) in cases {
        let text = engine::FILES
            .iter()
            .find(|(name, _)| *name == path)
            .unwrap()
            .1;
        assert_eq!(missing_phrases(text, phrases), Vec::<&str>::new(), "{path}");
        for &phrase in phrases {
            let modified = text.replace(phrase, "");
            assert!(
                missing_phrases(&modified, phrases).contains(&phrase),
                "{path}: {phrase}"
            );
        }
    }
}
