use std::{fs, path::Path};

fn read(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap().replace("\r\n", "\n")
}
fn contains_all(text: &str, parts: &[&str]) -> bool { parts.iter().all(|p| text.contains(p)) }
fn clean(text: &str) -> bool {
    const FORBIDDEN: &[&str] = &["sol/high by default", "sol is refused", "TIER luna", "legacy role", "legacy `luna`", "legacy luna", "legacy missing effort", "full test must fail"];
    assert!(!FORBIDDEN.is_empty());
    let lower = text.to_lowercase();
    !FORBIDDEN.iter().any(|p| lower.contains(&p.to_lowercase()))
}
fn check(text: &str, parts: &[&str]) {
    assert!(contains_all(text, parts), "missing: {:?}", parts.iter().filter(|p| !text.contains(**p)).collect::<Vec<_>>());
    assert!(!contains_all("negative control", parts));
}
fn under_budget(n: usize) -> bool { n <= 20_758 }

#[test]
fn forbidden_worker_paths_are_removed() {
    fn visit(path: &Path) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { visit(&path); }
            else if let Ok(text) = fs::read_to_string(&path) { assert!(clean(&text), "{}", path.display()); }
        }
    }
    visit(&Path::new(env!("CARGO_MANIFEST_DIR")).join("engine"));
    for path in ["src/bridge/server.rs", "docs/bridge.md"] { assert!(clean(&read(path)), "{path}"); }
    assert!(!clean("Use sol/high by default"));
}

#[test]
fn dispatch_and_wish_contract() {
    let dispatcher = read("engine/agents/dispatcher.md");
    check(&dispatcher, &["dispatcher and core never choose worker model or effort", "terra is the floor", "disk_space", "resumable", "VERIFY not run; checkpoint kept", "no fix unit", "re-dispatch the same unit", "VERIFY only",
        "When `accept.ran` is true, ACCEPT already ran for the module; do not call it again.",
        "Review each unit against CONTRACT and global rules, or wish and decisions without a map.",
        "If the reply is `fix_rounds`, report it to the core.",
        "Call `orch` `[\"fulltest\", \"--repo\", <dir>]` with the map's FULL TEST, or `[\"fulltest\", \"--repo\", <dir>, \"--command\", <cmd>]` with the project full test command without a map; use `run_on_linux` for Python commands.",
        "A CI run that fails after pushing is a failed full test, even when the local one passed.",
        "Run no `fulltest` between that call and the fix: a pass clears the failure.",
        "If more than one unit changed files, dispatch the `role: \"sol\"` unit above with the failing part of the CI log in CONTEXT, never `terra` fix units.",
        "`done` stays refused until a later `fulltest` passes, so run it after the fix is reviewed, push again and treat a new CI failure likewise."]);
    let dispatch = read("engine/DISPATCH.md");
    check(&dispatch, &["Both TIER and EFFORT are required together.", "RISK <class>", "trivial, standard, state_machine, concurrency, migration, parser, security, privacy, cross_module, prior_failure, novel", "no ROUTING line and no RISK", "routing_missing"]);
    let wish = read("engine/skills/wish/SKILL.md");
    check(&wish, &["dispatcher and core never choose worker model or effort", "terra is the floor", "after each start and each completion notice", "schedule", "next", "--cursor", "Never poll, sleep or watch files",
        "After a `fix_rounds` report, waive findings with `orch` `[\"waive\", <unit>, <n>, \"--reason\", <text>]` or close the goal as `blocked`.",
        "If units were dispatched, call bridge `orch` with `[\"report\", \"--repo\", <project directory>]`: its digest of unit states, acceptance, full test, sol, changed files, findings, decisions and Astra's coverage is the report.",
        "Start the `chronos:memory-keeper` agent in the foreground with store path, slug, project directory and wish.",
        "Close in step 6's narrator message (a `waiting` close has none): `done` when requirements are met, otherwise `blocked`; report unit states when dispatched."]);
}

#[test]
fn dispatcher_background_matches_hook() {
    let wish = read("engine/skills/wish/SKILL.md");
    let requirements = [
        "For each claimed module in `start`, launch a fresh dispatcher naming its module with `run_in_background: true`",
        "On `done`, start a fresh dispatcher for full test in the foreground.",
        "Without a map, run the dispatcher in the foreground.",
    ];
    check(&wish, &requirements);
    assert!(!contains_all(&wish.replace(requirements[0], ""), &requirements));
    let hook = read("src/hook/mod.rs");
    check(&hook, &["store.join(\"modules.md\").is_file()", "Target::Module(m)", "claimed_modules(&store)", "claims.contains(&m)", "Run {} in the foreground; waiting needs no polling.", "subagent.unwrap_or(\"this agent\")"]);
    let docs = read("docs/bridge.md");
    check(&docs, &["The hook allows `chronos:dispatcher` with `run_in_background: true` only for an engine-claimed module: the goal store has `modules.md` and the target module is claimed. Otherwise it must run in the foreground."]);
}

#[test]
fn planners_give_rubric_and_precedence() {
    for path in ["engine/agents/architect.md", "engine/agents/astra.md"] {
        let text = read(path);
        check(&text, &["trivial terra/medium", "standard sol/high", "state_machine, concurrency, migration, parser, security, privacy, cross_module, prior_failure", "sol/xhigh", "novel astra/xhigh", "fix record, map unit, map default, brief TIER+EFFORT, brief RISK rubric", "routing_mismatch"]);
    }
}

#[test]
fn bridge_documents_routing_and_escalation() {
    let src = read("src/bridge/server.rs");
    let description = src.split("\"name\": \"codex_run\", \"description\": \"").nth(1).unwrap().split("\", \"inputSchema\"").next().unwrap();
    check(description, &["role is a label", "TIER is terra, sol or astra", "both together", "same-attempt snapshot, fix record, map unit line, map default line, brief TIER+EFFORT, brief RISK rubric, explicit model", "terra is the floor", "luna", "model_floor", "routing_missing"]);
    let docs = read("docs/bridge.md");
    check(&docs, &["same-attempt snapshot, fix record, map unit line, map default line, brief TIER+EFFORT, brief RISK rubric, explicit model", "routing_missing", "model_floor", "terra/any and sol/low or medium → sol/high", "sol/high → sol/xhigh → astra/xhigh → astra/max", "sol/max → astra/max", "astra/high → astra/xhigh", "Astra/max is saturated"]);
}

#[test]
fn always_loaded_budget_and_negative_control() {
    let total: usize = ["engine/agents/dispatcher.md", "engine/DISPATCH.md", "engine/skills/wish/SKILL.md"].iter().map(|p| read(p).len()).sum();
    assert!(under_budget(total), "{total}");
    assert!(under_budget(20_758));
    assert!(!under_budget(20_759));
}
