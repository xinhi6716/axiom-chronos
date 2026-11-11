use std::fs;
use std::path::Path;

fn contains(text: &str, phrase: &str) -> bool {
    text.contains(phrase)
}

fn excludes(text: &str, phrase: &str) -> bool {
    let text = text.to_lowercase();
    if phrase == "agy" {
        !text.split(|c: char| !c.is_alphanumeric()).any(|word| word == phrase)
    } else {
        !text.contains(phrase)
    }
}

#[test]
fn bridge_documentation_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: &[(&str, &[&str])] = &[
        ("docs/bridge.md", &[
            "Unit mode takes exactly `project`, `unit`, `role` and `cwd`, plus `track` and `brief_file` to register a new unit in the same call; brief mode is for Astra.",
            "A registered brief may carry `EFFORT` (`low`, `medium` or `high`) and `TIER` (`luna`, terra units only) as their own lines between GOAL and CONTEXT.",
            "When every unit of a module has passed VERIFY, the unit reply's `accept` carries the module ACCEPT result.",
            "Every wish runs Astra first; no option or marker skips it.",
            "Reviews go to Claude: `chronos:reviewer` reviews one whole unit per call and `chronos:critic` reviews the Astra plan; `chronos:narrator` (Haiku) writes the final report from the run digest and the core's rulings.",
        ]),
        ("README.md", &[
            "Every wish starts with Astra: Astra plans the change, `chronos:critic` reviews the plan, and the core rules on it before any unit is built.",
            "Claude (`chronos:reviewer`, `chronos:critic`) reviews the Codex work; a Haiku narrator writes the final report from the engine digest and the core's rulings.",
        ]),
        ("README.zh-TW.md", &[
            "每次 wish 都先由 Astra 規劃，`chronos:critic` 審查計畫，核心裁決後才開始建置。",
            "審查由 Claude（`chronos:reviewer`、`chronos:critic`）負責審 Codex 的成果；Haiku 敘述員依引擎摘要與核心裁決撰寫最終報告。",
        ]),
    ];
    for (path, phrases) in cases {
        let text = fs::read_to_string(root.join(path)).unwrap();
        for phrase in *phrases {
            assert!(contains(&text, phrase), "{path}: {phrase}");
            assert!(!contains(&text.replace(phrase, ""), phrase), "{path}: {phrase}");
        }
        // Gemini returned in 2.0 only as the optional backup reviewer (agy), so its name may appear.
        for forbidden in ["rewrite", "--astra", "[astra]", "review_code", "second_review"] {
            assert!(excludes(&text, forbidden), "{path}: {forbidden}");
            assert!(!excludes(&format!("{text} {forbidden}"), forbidden), "{path}: {forbidden}");
        }
    }
}

fn has_historical_note(text: &str, title: &str) -> bool {
    let lines: Vec<_> = text.lines().collect();
    lines.first() == Some(&title) && lines.get(2) == Some(&"Gemini 已於第五輪收尾移除。")
}

#[test]
fn historical_docs_note_gemini_removal() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (path, title) in [
        ("docs/round4-adoption.zh-TW.md", "# 第四輪參考資料採納"),
        ("docs/round5-adoption.zh-TW.md", "# 第五輪參考資料採納"),
    ] {
        let text = fs::read_to_string(root.join(path)).unwrap();
        assert!(has_historical_note(&text, title), "{path}");
        let without_note = text.lines().filter(|line| *line != "Gemini 已於第五輪收尾移除。").collect::<Vec<_>>().join("\n");
        assert!(!has_historical_note(&without_note, title), "{path}");
    }
}
