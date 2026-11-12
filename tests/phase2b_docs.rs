use axiom::engine::FILES;
use std::{fs, path::Path};

fn asset(path: &str) -> &'static str {
    FILES.iter().find(|(name, _)| *name == path).expect(path).1
}

fn has_all(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().all(|phrase| text.contains(phrase))
}

#[test]
fn embedded_phase2b_contract() {
    let cases: &[(&str, &[&str])] = &[
        (
            "agents/dispatcher.md",
            &[
                "every review item's `reviewer`",
                "A `findings_rejected` reply is not a fix",
                "do not dispatch one",
            ],
        ),
        (
            "skills/wish/SKILL.md",
            &[
                "`approval`",
                "`request`",
                "`status`",
                "`execute`",
                "`axiom approval approve|deny`",
                "`adjudication`",
                "`person_requests`",
                "`interjection`",
                "`applied`",
                "`declined`",
                "`replan`",
                "`pause_batch`",
                "`policy set`",
            ],
        ),
        (
            "agents/reviewer.md",
            &[
                "`<unit>.verdict.json`",
                "\"unit\": \"<unit id>\"",
                "\"verdict\": \"pass\" | \"fail\"",
                "`<unit> <tree> <verdict> blocker=<n> major=<n> minor=<n>`",
            ],
        ),
        (
            "agents/memory-keeper.md",
            &[
                "correction: approval <id> denied",
                "`approval_denied`",
                "`<store>/events.jsonl`",
                "data contains the id, action and reason",
                "Record each denial and its reason as a correction",
                "do not use an approval id as a unit",
            ],
        ),
        (
            "agents/pm-qa.md",
            &[
                "acceptance semantics",
                "state-machine",
                "lifecycle",
                "tests that can pass falsely",
                "`<unit>.pm-qa.verdict.json`",
                "`<store>/reports/`",
                "only your own verdict file",
                "\"unit\": \"<unit id>\"",
                "\"verdict\": \"pass\" | \"fail\"",
                "`<unit> <tree> <verdict> blocker=<n> major=<n> minor=<n>`",
            ],
        ),
    ];
    for (path, phrases) in cases {
        let text = asset(path);
        for phrase in *phrases {
            assert!(has_all(text, &[*phrase]), "{path}: missing {phrase}");
        }
    }
    for role in [
        "adjudicator",
        "verifier",
        "astra-reviewer",
        "co-core",
        "arbiter",
    ] {
        let path = format!("roles/{role}.md");
        let text = asset(&path);
        assert!(
            !text.trim().is_empty() && has_all(text, &[role, "only the supplied evidence pack"]),
            "{path}"
        );
    }
    assert!(!asset("agents/pm-qa.md").contains("`<unit>.verdict.json`"));
}

#[test]
fn bridge_phase2b_contract() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/bridge.md");
    let text = fs::read_to_string(path).unwrap();
    for phrase in [
        "`approval`",
        "`request`",
        "`status`",
        "`execute`",
        "`axiom approval approve|deny`",
        "`interjection`",
        "`applied`",
        "`declined`",
        "`replan`",
        "`pause_batch`",
        "`policy set`",
        "`budget_exceeded`",
        "`findings_rejected`",
        "`axiom stats`",
        "person-only",
        "CLI approval immediately executes",
        "`plan_revision`",
        "`approval_denied`",
        "no fix unit is created",
    ] {
        assert!(
            has_all(&text, &[phrase]),
            "docs/bridge.md: missing {phrase}"
        );
    }
}

#[test]
fn missing_phrase_is_rejected() {
    assert!(has_all("approval_denied event", &["approval_denied"]));
    assert!(!has_all("approval event", &["approval_denied"]));
}
