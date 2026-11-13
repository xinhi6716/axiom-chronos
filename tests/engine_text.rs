use axiom::engine::FILES;

fn asset(path: &str) -> &'static str {
    FILES.iter().find(|(name, _)| *name == path).unwrap().1
}

fn phrase_count(text: &str, phrase: &str) -> usize {
    text.matches(phrase).count()
}

#[test]
fn contract_sentences_are_embedded() {
    let unit_mode = "Dispatch a registered unit with `project`, `unit`, `role` (`terra` or `sol`), `cwd`, and `cycle: true`, or register and dispatch a new one in the same call by adding `track` and `brief_file`; never add `brief` or `save_to`. Brief mode is for Astra only.";
    let cases = [
        ("DISPATCH.md", "SCOPE lists paths only; notes such as \"(new)\" go in CONTEXT."),
        ("DISPATCH.md", "VERIFY runs under `cmd` on Windows and `sh` on Linux: write commands without `set` or `%VAR%`, and rely on the inherited CARGO_HOME."),
        ("DISPATCH.md", unit_mode),
        ("DISPATCH.md", "SCOPE may span lines or bullets; every path counts."),
        ("DISPATCH.md", "Workers and VERIFY get private scratch (TMP, TEMP, TMPDIR, pytest `--basetemp`) automatically; workers' scratch is a sandbox writable root."),
        ("DISPATCH.md", "Write all module briefs in one call: a single Bash heredoc, or Write for one unit."),
        ("agents/dispatcher.md", r#"`orch` `["module-context", <module>, "--repo", <dir>]`"#),
        ("agents/dispatcher.md", "For any `incomplete` section, Grep its named source file."),
        ("agents/dispatcher.md", "call `memory_search` once per module"),
        ("agents/dispatcher.md", r#"unless the core says "no memory"; then skip it."#),
        ("agents/dispatcher.md", "SCOPE may span lines or bullets. Write and register briefs only for your own module: a foreign brief or `--track` is denied."),
        ("agents/dispatcher.md", r#"A `codex_run` reply may carry `status: "refused"` (`already_running`, `already_verified`) with the `owner`; report it, never retry it."#),
        ("agents/dispatcher.md", "Write all module briefs in one call: a Bash heredoc, or Write for one unit."),
        ("agents/dispatcher.md", "The PostToolUse hook records the review;"),
        ("agents/dispatcher.md", "only when the hook says the review was not recorded, after fixing the cause."),
        ("agents/dispatcher.md", "process every `{unit, pack}` in `review`"),
        ("agents/dispatcher.md", "For `wait`, a later unit runs ACCEPT, or `reason` names what to do."),
        ("agents/dispatcher.md", "For `fix`, run `unit add --fixes <old>` (CONTEXT: findings/contract); engine auto-escalates TIER/EFFORT; dispatch cycle."),
        ("agents/dispatcher.md", "the outcome saying `module_done` is the report: return it without calling `orch report` or `orch status`"),
        ("agents/dispatcher.md", "The `unit add` reply ends with the unit's next step: dispatch it only when that is `ready`; `wait ...` names the units to finish first. Never edit `units.tsv`, `claims.tsv` or evidence files by hand; a superseded unit stays registered."),
        ("agents/dispatcher.md", "Before dispatching a fix, register it separately with `orch` `[\"unit\", \"add\", <new>, \"--track\", <module>, \"--brief\", <path>, \"--fixes\", <old>]`."),
        ("agents/dispatcher.md", "For other takeovers by a later unit of the same module, call `orch` `[\"unit\", \"supersede\", <old>, \"--by\", <new>]`"),
        ("agents/dispatcher.md", "If the reply is `fix_rounds`, report it to the core."),
        ("agents/dispatcher.md", "Call `orch` `[\"review-pack\", <unit>, \"--repo\", <dir>]` only to rebuild a pack."),
        ("agents/dispatcher.md", "the reviewer writes its verdict file, not you."),
        ("agents/dispatcher.md", "Skip step 7 when you are named one module"),
        ("agents/dispatcher.md", "A `stale review` error means files in the unit's SCOPE changed after its VERIFY ran: call `orch` `[\"verify\", <unit>, \"--repo\", <dir>]`, follow the ACCEPT reply rule, then review again."),
        ("DISPATCH.md", "`EFFORT` (`low`, `medium` or `high`) sets the reasoning effort, and `TIER luna` runs a terra unit on the luna model."),
        ("DISPATCH.md", "The bridge reads them from the registered brief, so `codex_run` takes no argument for them, and the worker never sees them."),
        ("agents/dispatcher.md", "put `EFFORT` and `TIER` in its brief as DISPATCH.md describes; never pass them to `codex_run`."),
        ("agents/dispatcher.md", "Before writing any brief, call `memory_search`"),
        ("agents/dispatcher.md", "`memory_get`"),
        ("agents/dispatcher.md", unit_mode),
        ("agents/dispatcher.md", "Register every unit, fix units included, under its module id as track; with a map, only a `sol` unit may use another track."),
        ("agents/dispatcher.md", "A unit's `codex_run` reply carries `accept`. When `accept.ran` is true, ACCEPT already ran for the module; do not call it again. When `accept.reason` is `units_pending` or `claimed`, a later unit of the module will run it. For any other reason, call `orch` `[\"accept\", <module>, \"--repo\", <dir>]` yourself."),
        ("agents/reviewer.md", "You review one whole unit per call: the unit id, every file in it, tests included, and its contract."),
        ("agents/reviewer.md", "{\"unit\": \"<unit id>\", \"tree\": \"<tree id>\", \"verdict\": \"pass\" | \"fail\", \"findings\": ["),
        ("agents/reviewer.md", "Write the unit id you were given into `unit`, and the pack's `Tree reviewed` value into `tree`."),
        ("agents/reviewer.md", "For a pack with `Fixes unit:`, check only the listed findings and the changed lines."),
        ("agents/reviewer.md", "For a pack with `Earlier review at:`, check the earlier findings and the diff shown."),
        ("agents/reviewer.md", "A `blocker` prevents the goal's essential operation or causes severe harm."),
        ("agents/reviewer.md", "A `major` materially violates the contract or leaves important behaviour untested."),
        ("agents/reviewer.md", "A `minor` is a limited issue that does not prevent the essential operation or materially violate the contract."),
        ("agents/reviewer.md", "The optional `needs` field is `\"smoke\"` or `\"manual\"`, only for an `untested` finding that a lib test cannot reach."),
        ("agents/reviewer.md", "Write verdict JSON to `<unit>.verdict.json` in the directory of the review pack (`<store>/reports/`)"),
        ("agents/reviewer.md", "Each finding has `check`, `severity`, `location`, `evidence` and `fix`; quote evidence from the code."),
        ("agents/reviewer.md", "Return exactly one line: `<unit> <tree> <verdict> blocker=<n> major=<n> minor=<n>`."),
        ("agents/dispatcher.md", "`orch` refuses a verdict whose `tree` is not the tree of the unit's last VERIFY"),
        ("agents/critic.md", "Gives the second ruling on an Astra or research plan."),
        ("agents/critic.md", "In the second review"),
        ("skills/research/SKILL.md", "`chronos:critic` reviews it for overreach"),
        ("agents/memory-keeper.md", FIRST),
        ("agents/memory-keeper.md", SECOND),
        ("agents/memory-keeper.md", "`memory_record` refuses a quote its file does not contain"),
    ];
    for (path, phrase) in cases {
        let text = asset(path);
        assert_eq!(phrase_count(text, phrase), 1, "{path}: {phrase}");
    }
}

#[test]
fn made_up_contract_sentence_is_not_embedded() {
    assert_eq!(
        phrase_count(asset("agents/dispatcher.md"), "For `fix`, ask the moon to dispatch a unicorn."),
        0
    );
}

#[test]
fn removed_workarounds_are_gone() {
    fn has_workaround(text: &str) -> bool {
        [".axiom-tmp", "space-separated", "single SCOPE line", "\"needs\": null"]
            .iter()
            .any(|phrase| text.contains(phrase))
    }
    for path in ["DISPATCH.md", "agents/dispatcher.md"] {
        assert!(!has_workaround(asset(path)), "{path}");
    }
    assert!(has_workaround(".axiom-tmp"));
}

const FIRST: &str = "1. Read `<store>/units.tsv` and list the units whose state is `verified`; skip every other unit without reading its files.";
const SECOND: &str = "2. For each listed unit only, read `<store>/reports/<unit>.md` and the current text of the changed files in `<store>/reports/<unit>.json` `file_changes` that still exist.";

fn memory_order(text: &str) -> bool {
    matches!((text.find(FIRST), text.find(SECOND)), (Some(first), Some(second)) if first < second)
}

#[test]
fn memory_reads_verified_units_first() {
    let text = asset("agents/memory-keeper.md");
    assert!(memory_order(text));
    let swapped = text.replace(FIRST, "ORDER_PLACEHOLDER").replace(SECOND, FIRST).replace("ORDER_PLACEHOLDER", SECOND);
    assert!(!memory_order(&swapped));
}

fn no_review_code(text: &str) -> bool {
    !text.contains("review_code")
}

#[test]
fn installed_assets_do_not_use_review_code() {
    for (path, text) in FILES {
        assert!(no_review_code(text), "{path}");
        assert!(!no_review_code(&format!("{text} review_code")), "{path}");
    }
}

#[test]
fn reviewer_and_critic_use_claude() {
    let reviewer = asset("agents/reviewer.md");
    let frontmatter = reviewer.strip_prefix("---\n").or_else(|| reviewer.strip_prefix("---\r\n")).unwrap().split_once("\n---").unwrap().0;
    let tools: Vec<_> = frontmatter.lines().filter_map(|line| line.trim_end_matches('\r').strip_prefix("tools:")).collect();
    assert_eq!(tools, [" Read, Glob, Grep, Write, mcp__codegraph__codegraph_explore"]);
    for line in ["name: reviewer", "model: sonnet", "autoCompactWindow: 200000"] {
        assert!(frontmatter.lines().any(|actual| actual.trim_end_matches('\r') == line));
    }
    assert!(!reviewer.contains("Gemini"));
    let critic_description = asset("agents/critic.md").lines().find(|line| line.starts_with("description:")).unwrap();
    assert!(!critic_description.contains("Gemini"));
    assert!(!critic_description.contains("unavailable"));
}

#[test]
fn dispatcher_drops_per_unit_search_and_ready_poll() {
    let text = asset("agents/dispatcher.md");
    assert!(!text.contains("call `memory_search` for each unit"));
    assert!(!text.contains(r#"Call `orch` `["ready"]` and dispatch"#));
}
