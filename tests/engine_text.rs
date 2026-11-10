use axiom::engine::FILES;

fn asset(path: &str) -> &'static str {
    FILES.iter().find(|(name, _)| *name == path).unwrap().1
}

fn contains(text: &str, phrase: &str) -> bool {
    text.contains(phrase)
}

#[test]
fn contract_sentences_are_embedded() {
    let unit_mode = "Dispatch a registered unit with exactly `project`, `unit`, `role` (`terra` or `sol`) and `cwd`, or register and dispatch a new one in the same call by adding `track` and `brief_file`; never add `brief` or `save_to`. Brief mode is for Astra only.";
    let cases = [
        ("DISPATCH.md", "SCOPE lists paths only; notes such as \"(new)\" go in CONTEXT."),
        ("DISPATCH.md", "VERIFY runs under `cmd` on Windows and `sh` on Linux: write commands without `set` or `%VAR%`, and rely on the inherited CARGO_HOME."),
        ("DISPATCH.md", unit_mode),
        ("DISPATCH.md", "Tests that write temp files put them under `.axiom-tmp/<unit>/` in the project root (pytest: `--basetemp=.axiom-tmp/<unit>` in VERIFY); the bridge leaves that directory out of the SCOPE check and removes it after the unit."),
        ("agents/dispatcher.md", "The `unit add` reply ends with the unit's next step: dispatch it only when that is `ready`; `wait ...` names the units to finish first. Never edit `units.tsv`, `claims.tsv` or evidence files by hand; a superseded unit stays registered."),
        ("agents/dispatcher.md", "When a fix or later unit of the same module takes over from a unit for good, call `orch` `[\"unit\", \"supersede\", <old>, \"--by\", <new>]`: the old unit keeps its history and stops blocking `done` and dependent modules once `<new>` is verified."),
        ("agents/dispatcher.md", "A `stale review` error means files in the unit's SCOPE changed after its VERIFY ran: call `orch` `[\"verify\", <unit>, \"--repo\", <dir>]`, follow the ACCEPT reply rule, then review again."),
        ("DISPATCH.md", "`EFFORT` (`low`, `medium` or `high`) sets the reasoning effort, and `TIER luna` runs a terra unit on the luna model."),
        ("DISPATCH.md", "The bridge reads them from the registered brief, so `codex_run` takes no argument for them, and the worker never sees them."),
        ("agents/dispatcher.md", "put `EFFORT` and `TIER` in its brief as DISPATCH.md describes; never pass them to `codex_run`."),
        ("agents/dispatcher.md", "Before writing any brief, call `memory_search`"),
        ("agents/dispatcher.md", "`memory_get`"),
        ("agents/dispatcher.md", unit_mode),
        ("agents/dispatcher.md", "A unit that is ready to run is registered by its own `codex_run` call: add `track` (the module id, or \"build\") and `brief_file` (the brief path) to the dispatch, in the same message as the Write."),
        ("agents/dispatcher.md", "Register every unit, fix units included, under its module id as track; with a map, only a `sol` unit may use another track."),
        ("agents/dispatcher.md", "A unit's `codex_run` reply carries `accept`. When `accept.ran` is true, ACCEPT already ran for the module; do not call it again. When `accept.reason` is `units_pending` or `claimed`, a later unit of the module will run it. For any other reason, call `orch` `[\"accept\", <module>, \"--repo\", <dir>]` yourself."),
        ("agents/dispatcher.md", "Give `chronos:reviewer` one whole unit per call: the unit id, every file of the unit including tests, and the module CONTRACT."),
        ("agents/dispatcher.md", "The `orch` `[\"review\", ...]` reply lists the ready and waiting units; use it instead of a separate `[\"ready\"]` call."),
        ("agents/reviewer.md", "You review one whole unit per call: the unit id, every file in it, tests included, and its contract."),
        ("agents/reviewer.md", "{\"unit\": \"<unit id>\", \"verdict\": \"pass\" | \"fail\", \"findings\": ["),
        ("agents/reviewer.md", "Write the unit id you were given into `unit`."),
        ("agents/critic.md", "Gives the second ruling on an Astra or research plan."),
        ("agents/critic.md", "In the second review"),
        ("skills/research/SKILL.md", "`chronos:critic` reviews it for overreach"),
        ("agents/memory-keeper.md", FIRST),
        ("agents/memory-keeper.md", SECOND),
        ("agents/memory-keeper.md", "`memory_record` refuses a quote its file does not contain"),
    ];
    for (path, phrase) in cases {
        let text = asset(path);
        assert!(contains(text, phrase), "{path}: {phrase}");
        assert!(!contains(&text.replace(phrase, ""), phrase), "{path}: {phrase}");
    }
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
    assert_eq!(tools, [" Read, Glob, Grep, mcp__codegraph__codegraph_explore"]);
    assert!(!reviewer.contains("Gemini"));
    let critic_description = asset("agents/critic.md").lines().find(|line| line.starts_with("description:")).unwrap();
    assert!(!critic_description.contains("Gemini"));
    assert!(!critic_description.contains("unavailable"));
}
