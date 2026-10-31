use std::{fs, path::Path};

fn missing_phrase<'a>(text: &str, phrase: &'a str) -> Option<&'a str> {
    (!text.contains(phrase)).then_some(phrase)
}

#[test]
fn core_routes_technical_context_without_ruling_or_report_ingestion() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("engine");
    let wish = fs::read_to_string(root.join("skills/wish/SKILL.md")).unwrap();
    let research = fs::read_to_string(root.join("skills/research/SKILL.md")).unwrap();
    for text in [&wish, &research] {
        assert!(!text.contains("Rule on each finding yourself"));
        assert!(!text.contains("Read every file"));
        assert!(text.contains("do not start another core model"));
    }
    assert!(wish.contains("Declare change mode after a qualifying ruling"));
    assert!(wish.contains("no technical design, rulings, implementation, assembly"));
    assert!(research.contains("core must not do it"));
    assert!(wish.contains("Read-only is not an inability to dispatch"));
}

#[test]
fn required_phrases_are_present() {
    let cases = [
        ("agents/dispatcher.md", "Call `codex_run` yourself"),
        ("agents/dispatcher.md", "Never forward engineering Codex work through another agent"),
        ("agents/dispatcher.md", "Never wait with `sleep`; a `codex_run` call returns when its turn ends"),
        ("agents/dispatcher.md", "`cargo test --locked --offline`"),
        ("skills/wish/SKILL.md", "resume the same dispatcher ownership"),
        ("skills/wish/SKILL.md", "`[creative]`"),
        ("skills/wish/SKILL.md", "engineering change mode is selected only after the independent ruling"),
        ("skills/wish/SKILL.md", "at most three candidates"),
        ("skills/wish/SKILL.md", "the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check"),
        ("skills/wish/SKILL.md", "evidence strength, then direct user value, then reuse of what exists"),
        ("skills/wish/SKILL.md", "exactly one smallest increment"),
        ("skills/wish/SKILL.md", "In creative mode `[no-architect]` is ignored"),
        ("skills/wish/SKILL.md", "Declare change mode after a qualifying ruling"),
        ("skills/wish/SKILL.md", "no follow-up exploration"),
        ("skills/wish/SKILL.md", "`area: feature`"),
        ("agents/dispatcher.md", "including fix and sol units"),
        ("agents/dispatcher.md", "`orch` `[\"map\"]`"),
        ("agents/dispatcher.md", "`orch` `[\"ready\"]`"),
        ("agents/dispatcher.md", "`orch` `[\"accept\", <module>"),
        ("agents/dispatcher.md", "`orch` `[\"fulltest\", \"--repo\""),
        ("agents/dispatcher.md", "`orch` `[\"review\", <unit>"),
        ("agents/dispatcher.md", "parallel `codex_run` calls with `project` the slug, `unit` the id, `role: \"terra\"`, and `cwd` the project directory"),
        ("agents/dispatcher.md", "`review_code` with `project`, `units`, `project_dir`, `paths`, and `contract`"),
        ("agents/dispatcher.md", "give the same files and contract to `chronos:reviewer`"),
        ("skills/wish/SKILL.md", "Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool."),
        ("skills/wish/SKILL.md", "`stage: \"second_review\"`"),
        ("skills/wish/SKILL.md", "`stage: \"rewrite\"`"),
        ("skills/wish/SKILL.md", "output path `<store>/astra/plan.json`"),
        ("skills/wish/SKILL.md", "`## Coverage`"),
        ("skills/wish/SKILL.md", "`review-failed`, or `unreviewed`"),
        ("skills/research/SKILL.md", "output path `<store>/research/plan.json`"),
        ("skills/research/SKILL.md", "second-review stage"),
        ("agents/astra.md", "do not call `codex_run`"),
        ("agents/content.md", "exact files authorized by `output_paths`"),
        ("agents/dispatcher.md", "<actual model returned by the agent>"),
        ("skills/research/SKILL.md", "`schema_path:"),
        ("skills/wish/SKILL.md", "`subagent_type: \"chronos:astra\"`"),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("engine");
    for (asset, phrase) in cases {
        let text = fs::read_to_string(root.join(asset)).unwrap();
        assert_eq!(
            missing_phrase(&text, phrase),
            None,
            "{asset}: missing {phrase}"
        );
        let modified = text.replace(phrase, "");
        assert_eq!(missing_phrase(&modified, phrase), Some(phrase));
    }
    let dispatch = fs::read_to_string(root.join("DISPATCH.md")).unwrap();
    assert!(!dispatch.contains("After the worker returns"));
}

#[test]
fn deleted_rules_have_enforcing_tests() {
    let cases = [
        ("skills/wish/SKILL.md", "If any unit is not `verified` or the full test fails", "orch/lifecycle_cases.rs::ready_lists_waiting_units_with_next_step"),
        ("agents/dispatcher.md", "general-purpose", "hook.rs::nested_agents_only_reviewer"),
        ("agents/dispatcher.md", "time.sleep", "bridge/codex_cases.rs::unbound_writing_run_is_refused_before_codex_starts"),
        ("DISPATCH.md", "For deletions, SCOPE lists each path to delete.", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("DISPATCH.md", "The worker's report is a claim.", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/architect.md", "Order the modules so each depends only on earlier ones and on existing code.", "orch/parse_cases.rs::map_rejections_name_module"),
        ("agents/reviewer.md", "You change nothing.", "engine/text_cases.rs::reviewer_tools_are_read_only"),
        ("agents/reviewer.md", "and the verdict is `fail` when any finding is a blocker or major", "agy/review_cases.rs::review_verdict_and_failures"),
        ("agents/memory-keeper.md", "Only units in state `verified` count; that state means the dispatcher confirmed VERIFY passed and its review passed.", "memory.rs::record_search_stale_digest_and_validation"),
        ("agents/memory-keeper.md", "never read another unit's brief in its place", "memory.rs::record_search_stale_digest_and_validation"),
        ("agents/memory-keeper.md", "at most 12 lines", "memory.rs::record_search_stale_digest_and_validation"),
        ("agents/dispatcher.md", "Write the report's `final_message`, unedited", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/dispatcher.md", "VERIFY passes when the report's `commands`", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/dispatcher.md", "Set the state with `orch`", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/dispatcher.md", "If that fails too, leave it failed and continue.", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/dispatcher.md", "A unit whose review did not finish", "orch/evidence_cases.rs::state_table"),
        ("agents/dispatcher.md", "You never dismiss a blocker or major finding yourself", "orch/evidence_cases.rs::state_table"),
        ("agents/dispatcher.md", "Resolve every remaining `built` unit", "goal.rs::done_requires_verified_units_and_current_evidence"),
        ("skills/wish/SKILL.md", "You never call Codex or edit the project yourself", "hook.rs::core_codex_always_denied"),
        ("skills/wish/SKILL.md", "Unless the run is headless", "hook.rs::question_headless"),
        ("skills/wish/SKILL.md", "at most four questions", "hook.rs::question_limit"),
        ("skills/wish/SKILL.md", "Continue only when every unit is `verified` and the full test passes.", "goal.rs::done_requires_verified_units_and_current_evidence"),
    ];
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (asset, sentence, test) in cases {
        let text = fs::read_to_string(root.join("engine").join(asset)).unwrap();
        assert!(
            !text.contains(sentence),
            "{asset} still contains {sentence}"
        );
        let (file, function) = test.split_once("::").unwrap();
        let source = fs::read_to_string(root.join("tests").join(file)).unwrap();
        let declaration = format!("fn {function}(");
        assert!(source.contains(&declaration), "missing {test}");
    }
}

#[test]
fn reviewer_tools_are_read_only() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("engine/agents/reviewer.md"))
            .unwrap();
    let frontmatter = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .unwrap();
    let frontmatter = frontmatter.split_once("\n---").unwrap().0;
    let tools: Vec<_> = frontmatter
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("tools:"))
        .collect();
    assert_eq!(tools, [" Read, Glob, Grep"]);
}
