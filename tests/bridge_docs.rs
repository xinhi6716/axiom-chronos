use std::fs;
use std::path::Path;

fn contains(text: &str, phrase: &str) -> bool {
    text.contains(phrase)
}

fn excludes(text: &str, phrase: &str) -> bool {
    !text.contains(phrase)
}

#[test]
fn bridge_documentation_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: &[(&str, &[&str])] = &[
        ("docs/bridge.md", &[
            "Reviews go to the `chronos:reviewer` agent, one whole unit per call; Gemini only rewrites reports (`gemini_preview` with `stage: \"rewrite\"`).",
            "Unit mode takes exactly `project`, `unit`, `role` and `cwd`; brief mode is for Astra.",
            "When every unit of a module has passed VERIFY, the unit reply's `accept` carries the module ACCEPT result.",
        ]),
        ("README.md", &["Claude (`chronos:reviewer`) reviews the work; Gemini only rewrites reports in plain language."]),
        ("README.zh-TW.md", &["審查由 Claude（`chronos:reviewer`）負責；Gemini 只把報告改寫成白話。"]),
    ];
    for (path, phrases) in cases {
        let text = fs::read_to_string(root.join(path)).unwrap();
        for phrase in *phrases {
            assert!(contains(&text, phrase), "{path}: {phrase}");
            assert!(!contains(&text.replace(phrase, ""), phrase), "{path}: {phrase}");
        }
        for forbidden in ["review_code", "second_review"] {
            assert!(excludes(&text, forbidden), "{path}: {forbidden}");
            assert!(!excludes(&format!("{text} {forbidden}"), forbidden), "{path}: {forbidden}");
        }
    }
}
