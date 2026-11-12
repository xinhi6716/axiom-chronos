use std::fs;
use std::path::Path;

fn excludes(text: &str, phrase: &str) -> bool {
    let text = text.to_lowercase();
    if phrase == "--astra" {
        !text.match_indices(phrase).any(|(at, _)| {
            text[at + phrase.len()..]
                .chars()
                .next()
                .map_or(true, |c| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        })
    } else if phrase == "agy" {
        !text
            .split(|c: char| !c.is_alphanumeric())
            .any(|word| word == phrase)
    } else {
        !text.replace("rewrites only attribution", "").contains(phrase)
    }
}

#[test]
fn bridge_documentation_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let cases: &[(&str, &[&str])] = &[
        ("docs/bridge.md", &[
            "Each wish has its own store, `<data home>/orchestrate/<project>/<wish id>`, resolved by `orch::wish_store`;",
            "Workers and VERIFY commands get private scratch (`TMP`, `TEMP`, `TMPDIR`, pytest `--basetemp`) under `<store>/scratch/`",
            "`scope-unattributed`",
            r#"returns `status: "refused"` with `error.kind` `already_running` or `already_verified` and the `owner`"#,
            "A dispatcher is bound to one module by its first `codex_run`",
            "stops with exit code 3 and a `chronos` `max_turns` event",
            r#"`"needs": null` in a verdict reads as omitted"#,
            "repairs a malformed Argon review once",
            "Unit mode takes `project`, `unit`, `role` and `cwd`, optionally `cycle: true`, plus `track` and `brief_file` to register a new unit in the same call; brief mode is for Astra.",
            "A registered brief may carry `EFFORT` (`low`, `medium` or `high`) and `TIER` (`luna`, terra units only) as their own lines between GOAL and CONTEXT.",
            "With `cycle: true`, the reply is an outcome with `next`, `reason`, `verdict`, `review` entries `{unit, pack}`, `paths` and `module_done`, alongside `accept`, `verify_output` and `report`.",
            r#"`orch` `["module-context", <module>, "--repo", <dir>]` returns bounded module context (16,384 bytes), with `missing` and `incomplete` sections naming sources to consult."#,
            r#"`orch` `["map", "--repo", <dir>]` lints the map and reports all problems together, including `NEW_PATHS` and `SHARED_OWNER`."#,
            "A PostToolUse review hook records the reviewer verdict and says `Chronos recorded the review.`; if it says the review was not recorded, fix the cause and call `orch review` yourself.",
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
            assert_eq!(text.matches(phrase).count(), 1, "{path}: {phrase}");
        }
        // Gemini returned in 2.0 only as the optional backup reviewer (agy), so its name may appear.
        for forbidden in [
            "rewrite",
            "--astra",
            "[astra]",
            "review_code",
            "second_review",
        ] {
            assert!(excludes(&text, forbidden), "{path}: {forbidden}");
            assert!(
                !excludes(&format!("{text} {forbidden}"), forbidden),
                "{path}: {forbidden}"
            );
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
        let without_note = text
            .lines()
            .filter(|line| *line != "Gemini 已於第五輪收尾移除。")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!has_historical_note(&without_note, title), "{path}");
    }
}

#[test]
fn astra_budget_flag_is_distinct_from_the_removed_astra_switch() {
    assert!(excludes("`--astra-cap 600000`", "--astra"));
    for sample in ["`--astra`", "--astra value", "--astra=value"] {
        assert!(!excludes(sample, "--astra"));
    }
}
