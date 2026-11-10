const SENTENCE: &str = "If creative mode chose no candidate, skip only the architect and the dispatcher, then run step 5 (memory keeper), step 6 (report) and step 7 (close) with the exploration and ruling as the report.";

#[test]
fn creative_no_candidate_runs_remaining_steps() {
    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;

    let has = |value: &str| value.contains(SENTENCE);
    assert!(has(text));
    for word in ["memory keeper", "report", "close"] {
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
    let moved =
        text.replace(SENTENCE, "")
            .replacen(step_five, &format!("{step_five}\n{SENTENCE}"), 1);
    assert!(!before_step_five(&moved));
    assert!(text.lines().any(|line| line.starts_with("7. ")));
}

#[test]
fn every_wish_runs_astra() {
    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;
    for phrase in [
        "An old `[astra]` marker at the start of the wish is removed as plain text and changes nothing.",
        "Every wish runs step 2a after step 2; there is no marker or option that skips it.",
    ] {
        let has = |value: &str| value.contains(phrase);
        assert!(has(text));
        assert!(!has(&text.replace(phrase, "")));
    }
    for phrase in [
        "If it then starts with `[astra]`, drop that marker and do step 2a after step 2.",
        "[no-astra]",
    ] {
        let has = |value: &str| value.contains(phrase);
        assert!(!has(text));
        assert!(has(&format!("{text}\n{phrase}")));
    }
    let step_one = text.lines().find(|line| line.starts_with("1. ")).unwrap();
    assert!(step_one.contains("`[no-architect]`"));
    assert!(step_one.contains("skip step 3"));
    assert!(step_one.contains("In creative mode `[no-architect]` is ignored"));
}

#[test]
fn report_is_written_in_plain_language() {
    let no_gemini = |files: &[(&str, &str)]| {
        files
            .iter()
            .all(|(_, text)| !text.to_lowercase().contains("gemini"))
    };
    assert!(no_gemini(axiom::engine::FILES));
    let mut modified = axiom::engine::FILES.to_vec();
    let appended = format!("{}Gemini", modified[0].1);
    modified[0].1 = &appended;
    assert!(!no_gemini(&modified));

    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;
    assert!(!text.to_lowercase().contains("rewrite"));
    assert!(!text.contains("stage:"));
    for phrase in [
        "Give `chronos:narrator` the `orch` `[\"report\"]` digest",
        "Do not reprint its text.",
        "The rulings stay yours; step 7 closes the goal.",
    ] {
        let has = |value: &str| value.contains(phrase);
        assert!(has(text), "missing {phrase}");
        assert_eq!(text.matches(phrase).count(), 1);
        assert!(!has(&text.replace(phrase, "")));
    }
    let steps: Vec<_> = text
        .lines()
        .filter_map(|line| {
            ["5. ", "6. ", "7. "]
                .iter()
                .position(|prefix| line.starts_with(prefix))
        })
        .collect();
    assert_eq!(steps, [0, 1, 2]);
}

#[test]
fn a_long_batch_is_piloted_then_waited_on_without_a_model() {
    let file = |name: &str| {
        axiom::engine::FILES
            .iter()
            .find(|(path, _)| *path == name)
            .unwrap()
            .1
    };
    let skill = file("skills/wish/SKILL.md");
    let step = skill.lines().find(|line| line.starts_with("7. ")).unwrap();
    assert!(step.contains("skills/wish/BATCH.md"));
    assert!(!skill.contains("`$!` in Git Bash is not a Windows pid"));
    let batch = file("skills/wish/BATCH.md");
    for phrase in [
        "one pilot unit before any fan-out",
        "Only after a clean pilot",
        "`progress_file`",
        "`marker_file`",
        "`pid`",
        "`$!` in Git Bash is not a Windows pid",
        "No model runs while the goal waits",
        "`axiom wait <session>`",
        "A batch that died without its marker is a failure",
        "Never watch a batch.",
    ] {
        let has = |value: &str| value.contains(phrase);
        assert!(has(batch), "missing {phrase}");
        assert!(!has(&batch.replace(phrase, "")));
    }
    let pilot = batch.find("one pilot unit").unwrap();
    assert!(pilot < batch.find("launch the full batch").unwrap());
    assert!(
        batch.find("launch the full batch").unwrap()
            < batch.find("Close the goal as `waiting`").unwrap()
    );

    let dispatcher = file("agents/dispatcher.md");
    let phrase = "report its progress file, completion marker and pid";
    assert!(dispatcher.contains(phrase));
    assert!(!dispatcher.replace(phrase, "").contains(phrase));
}

#[test]
fn memory_keeper_denial_names_no_rewrite() {
    let home = tempfile::tempdir().unwrap();
    axiom::goal::open(home.path(), "s", "wish", None).unwrap();
    let event = serde_json::json!({
        "hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Agent",
        "tool_input": {"subagent_type": "chronos:memory-keeper", "run_in_background": true}
    });
    let result = axiom::hook::handle(&event, Some(home.path()), false).unwrap();
    let output = &result["hookSpecificOutput"];
    assert_eq!(output["permissionDecision"], "deny");
    let reason = output["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("foreground"));
    assert!(!reason.to_lowercase().contains("rewrite"));
}

#[test]
fn the_core_reads_a_digest_not_the_reports_directory() {
    let text = axiom::engine::FILES
        .iter()
        .find(|(path, _)| *path == "skills/wish/SKILL.md")
        .unwrap()
        .1;
    let step = text.lines().find(|l| l.starts_with("5. ")).unwrap();
    assert!(step.contains(r#"`["report", "--repo", <project directory>]`"#));
    assert!(step.contains("Never read `<store>/reports/` file by file"));
    assert!(!text.contains("Read every file in `<store>/reports/`"));
    let report = text.lines().find(|l| l.starts_with("6. ")).unwrap();
    assert!(report.contains("chronos:narrator"));
    let hand_off = text.lines().find(|l| l.starts_with("4. ")).unwrap();
    assert!(hand_off.contains("Never poll, sleep or watch files while an agent runs"));
}

#[test]
fn dispatcher_reports_the_digest_and_reviews_through_the_pack() {
    let asset = |name: &str| {
        axiom::engine::FILES
            .iter()
            .find(|(path, _)| *path == name)
            .unwrap()
            .1
    };
    let dispatcher = asset("agents/dispatcher.md");
    assert!(dispatcher.contains(r#"`orch` `["review-pack", <unit>, "--repo", <dir>]`"#));
    let report = dispatcher.split("## Report").nth(1).unwrap();
    assert!(report.contains(r#"`orch` `["report", "--repo", <dir>, "--module", <module>]` unedited"#));
    assert!(report.contains("at most ten lines"));
    assert!(!report.contains("each decided choice"));
    let reviewer = asset("agents/reviewer.md");
    assert!(reviewer.contains("Read the review pack you were given."));
    assert!(reviewer.contains("\"tree\": \"<tree id>\""));
    assert!(reviewer.contains("the pack's `Tree reviewed` value into `tree`"));
    assert!(dispatcher.contains("save its verdict unedited"));
    assert!(reviewer.contains("tests included"));
    assert!(!reviewer.contains("Read every file you were given"));
    for check in ["contract_mismatch", "vacuous_check", "untested", "platform"] {
        assert!(reviewer.contains(check), "{check}");
    }
}
