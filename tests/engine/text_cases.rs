use std::{fs, path::Path};

fn missing_phrase<'a>(text: &str, phrase: &'a str) -> Option<&'a str> {
    (!text.contains(phrase)).then_some(phrase)
}

#[test]
fn required_phrases_are_present() {
    let cases = [
        ("agents/dispatcher.md", "Call `codex_run` yourself"),
        ("agents/dispatcher.md", "Never forward Codex work through another agent"),
        ("agents/dispatcher.md", "Never wait with `sleep`; a `codex_run` call returns when its turn ends"),
        ("agents/dispatcher.md", "`cargo fetch --locked`"),
        ("agents/dispatcher.md", "`cargo test --locked --offline`"),
        ("skills/wish/SKILL.md", "Hand work back to `chronos:dispatcher` only when `orch` `[\"ready\"]` lists a ready unit or a unit whose next step is `accept` or `review`"),
        ("skills/wish/SKILL.md", "or the report shows `handoff pending`"),
        ("skills/wish/SKILL.md", "`[creative]`"),
        ("skills/wish/SKILL.md", "only when the ruling chose a candidate"),
        ("skills/wish/SKILL.md", "`chronos:memory-keeper` agent in the foreground"),
        ("skills/wish/SKILL.md", "at most three candidates"),
        ("skills/wish/SKILL.md", "the existing capability, the missing link, the code location, the user-visible value, and a falsifiable acceptance check"),
        ("skills/wish/SKILL.md", "evidence strength, then direct user value, then reuse of what exists"),
        ("skills/wish/SKILL.md", "exactly one smallest increment"),
        ("skills/wish/SKILL.md", "each as a heading `### <id> <title>`"),
        ("agents/architect.md", "SERVES     plan item ids"),
        ("agents/architect.md", "OUT OF SCOPE <id> <reason>"),
        ("skills/wish/SKILL.md", "In creative mode `[no-architect]` is ignored"),
        ("skills/wish/SKILL.md", "declare the mode after the ruling"),
        ("skills/wish/SKILL.md", "no follow-up exploration"),
        ("skills/wish/SKILL.md", "`area: feature`"),
        ("agents/dispatcher.md", "including fix and sol units"),
        ("agents/dispatcher.md", "`orch` `[\"map\"]`"),
        ("agents/dispatcher.md", "`orch` `[\"ready\"]`"),
        ("agents/dispatcher.md", "`orch` `[\"accept\", <module>"),
        ("agents/dispatcher.md", "`orch` `[\"fulltest\", \"--repo\""),
        ("agents/dispatcher.md", "A CI run that fails after pushing is a failed full test"),
        ("agents/dispatcher.md", "`orch` `[\"fulltest\", \"--repo\", <dir>, \"--external\""),
        ("agents/dispatcher.md", "If more than one unit changed files, dispatch the `role: \"sol\"` unit above"),
        ("agents/dispatcher.md", "never `terra` fix units"),
        ("agents/dispatcher.md", "`done` stays refused until a later `fulltest` passes"),
        ("agents/dispatcher.md", "`orch` `[\"review\", <unit>"),
        ("agents/dispatcher.md", "parallel `codex_run` calls with `project` the slug, `unit` the id, `role: \"terra\"`, and `cwd` the project directory"),
        ("agents/dispatcher.md", "Give the files and contract to `chronos:reviewer`"),
        ("skills/wish/SKILL.md", "Declare the task mode (`review`, `answer` or `change`) with the bridge `goal` tool."),
        ("skills/wish/SKILL.md", "Give the `chronos:critic` agent the review schema, the plan path"),
        ("skills/wish/SKILL.md", "An old `[astra]` marker at the start of the wish is removed as plain text and changes nothing."),
        ("skills/wish/SKILL.md", "Every wish runs step 2a after step 2; there is no marker or option that skips it."),
        ("skills/wish/SKILL.md", "Write the report yourself, in the wish's language and in plain words: what was done, how it turned out, and which choices were decided for the user."),
        ("skills/wish/SKILL.md", "output path `<store>/astra/plan.json`"),
        ("skills/wish/SKILL.md", "`## Coverage`"),
        ("skills/wish/SKILL.md", "End the report with one `Provenance:` line"),
        ("skills/wish/SKILL.md", "`review-failed`, or `unreviewed`"),
        ("skills/research/SKILL.md", "output path `<store>/research/plan.json`"),
        ("skills/research/SKILL.md", "Give the `chronos:critic` agent the review schema, the plan path"),
        ("agents/astra.md", "`save_to` the given path"),
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
        ("skills/wish/SKILL.md", "If it then starts with `[astra]`, drop that marker and do step 2a after step 2.", "wish_flow.rs::every_wish_runs_astra"),
        ("skills/wish/SKILL.md", "If `gemini_preview` fails, write that rewrite yourself from the same material.", "wish_flow.rs::report_is_written_in_plain_language"),
        ("skills/wish/SKILL.md", "If any unit is not `verified` or the full test fails", "orch/lifecycle_cases.rs::ready_lists_waiting_units_with_next_step"),
        ("agents/dispatcher.md", "general-purpose", "hook.rs::nested_agents_only_reviewer"),
        ("agents/dispatcher.md", "time.sleep", "bridge/codex_cases.rs::unbound_writing_run_is_refused_before_codex_starts"),
        ("DISPATCH.md", "For deletions, SCOPE lists each path to delete.", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("DISPATCH.md", "The worker's report is a claim.", "orch/lifecycle_cases.rs::lifecycle_reuse_scope_and_limit"),
        ("agents/architect.md", "Order the modules so each depends only on earlier ones and on existing code.", "orch/parse_cases.rs::map_rejections_name_module"),
        ("agents/astra.md", "Write its `structured_output` to the given path.", "bridge/codex_cases.rs::astra_save_records_only_astra"),
        ("agents/astra.md", "or the model is not an Astra model", "bridge/codex_cases.rs::sol_and_astra_restrictions"),
        ("agents/reviewer.md", "You change nothing.", "engine/text_cases.rs::reviewer_tools_are_read_only"),
        ("agents/reviewer.md", "and the verdict is `fail` when any finding is a blocker or major", "orch/lifecycle_cases.rs::extra_commands_preserve_output_and_failures"),
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
    assert_eq!(tools, [" Read, Glob, Grep, mcp__codegraph__codegraph_explore"]);
}
