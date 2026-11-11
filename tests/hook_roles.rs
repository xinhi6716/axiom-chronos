use axiom::{goal, hook, orch};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
};

const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
const GOAL: &str = "mcp__plugin_chronos_bridge__goal";
const ORCH: &str = "mcp__plugin_chronos_bridge__orch";

fn tool(name: &str, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":name,"tool_input":input})
}
fn sub(mut e: Value) -> Value {
    e["agent_id"] = json!("a");
    e
}
fn denied(home: &Path, e: &Value) -> bool {
    hook::handle(e, Some(home), false)
        .is_some_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny")
}
fn change_goal(home: &Path) {
    goal::open(home, "s", "wish", None).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
}
fn write(path: &Path) -> Value {
    tool("Write", json!({ "file_path": path }))
}

fn reviewer(mut e: Value, id: &str) -> Value {
    e["agent_id"] = json!(id);
    e["agent_type"] = json!("chronos:reviewer");
    e
}
fn reviewer_store(home: &Path) -> std::path::PathBuf {
    let project = home.join("project");
    let g = goal::open(home, "s", "wish", Some(project.to_str().unwrap())).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
    let store = orch::store_for(home, g.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    store
}
fn read(path: &Path) -> Value {
    tool("Read", json!({"file_path":path}))
}

#[test]
fn reviewer_reads_are_bounded_and_bind_only_the_first_pack() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let store = reviewer_store(h);
    let small = h.join("small.txt");
    let large = h.join("large.txt");
    let pack = orch::pack_path(&store, "u");
    let other = orch::pack_path(&store, "v");
    fs::write(&small, vec![b'x'; 50_000]).unwrap();
    fs::write(&large, vec![b'x'; 50_001]).unwrap();
    fs::write(&pack, vec![b'x'; 50_001]).unwrap();
    fs::write(&other, vec![b'x'; 50_001]).unwrap();
    assert!(!denied(h, &reviewer(read(&small), "a")));
    assert!(denied(h, &reviewer(read(&large), "a")));
    let mut limited = read(&large);
    limited["tool_input"]["limit"] = json!(501);
    assert!(!denied(h, &reviewer(limited, "a")));
    fs::create_dir_all(store.join("agents")).unwrap();
    fs::write(store.join("agents/a.reviewer"), "").unwrap();
    assert!(!denied(h, &reviewer(read(&pack), "a")));
    assert_eq!(
        fs::read_to_string(store.join("agents/a.reviewer")).unwrap(),
        "u"
    );
    assert!(denied(h, &reviewer(read(&other), "a")));
    assert!(denied(h, &reviewer(read(&large), "a")));
    let mut other_limited = read(&other);
    other_limited["tool_input"]["limit"] = json!(1);
    assert!(!denied(h, &reviewer(other_limited, "a")));
    assert_eq!(
        fs::read_to_string(store.join("agents/a.reviewer")).unwrap(),
        "u"
    );
    assert!(!denied(h, &sub(read(&large))));
    assert!(!denied(h, &read(&large)));
    let binary = h.join("large.PNG");
    fs::write(&binary, vec![b'x'; 50_001]).unwrap();
    assert!(!denied(h, &reviewer(read(&binary), "a")));
    fs::write(&pack, vec![b'x'; 256 * 1024 + 1]).unwrap();
    assert!(
        denied(h, &reviewer(read(&pack), "a")),
        "256 KB rule still applies"
    );
}

#[test]
fn reviewer_may_write_only_its_bound_verdict() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let store = reviewer_store(h);
    let pack = orch::pack_path(&store, "u");
    fs::write(&pack, "pack").unwrap();
    let verdict = store.join("reports/u.verdict.json");
    assert!(denied(h, &reviewer(write(&verdict), "unbound")));
    assert!(!denied(h, &reviewer(read(&pack), "a")));
    for name in ["Write", "Edit", "NotebookEdit"] {
        let key = if name == "NotebookEdit" {
            "notebook_path"
        } else {
            "file_path"
        };
        let call = |path: &Path| reviewer(tool(name, json!({key:path})), "a");
        assert!(!denied(h, &call(&verdict)), "{name}");
        for path in [
            store.join("reports/v.verdict.json"),
            h.join("orchestrate/other/reports/u.verdict.json"),
            h.join("project/file"),
            store.join("reports/../../other/reports/u.verdict.json"),
        ] {
            assert!(denied(h, &call(&path)), "{name}: {}", path.display());
        }
    }
    assert!(
        !denied(h, &sub(write(&h.join("project/file")))),
        "change-mode non-reviewer retains write access"
    );
    goal::set_status(h, "s", "blocked", Some("note")).unwrap();
    assert!(denied(h, &reviewer(write(&h.join("project/file")), "a")));

    let review = tempfile::tempdir().unwrap();
    let home = review.path();
    let g = goal::open(
        home,
        "s",
        "wish",
        Some(home.join("project").to_str().unwrap()),
    )
    .unwrap();
    goal::set_mode(home, "s", "review").unwrap();
    let store = orch::store_for(home, g.project.as_deref().unwrap()).unwrap();
    fs::create_dir_all(store.join("reports")).unwrap();
    let pack = orch::pack_path(&store, "u");
    fs::write(&pack, "pack").unwrap();
    assert!(!denied(home, &reviewer(read(&pack), "a")));
    assert!(!denied(
        home,
        &reviewer(write(&store.join("reports/u.verdict.json")), "a")
    ));
}

#[test]
fn concurrent_reviewer_pack_reads_bind_once() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path().to_path_buf();
    let store = reviewer_store(&h);
    for unit in ["u", "v"] {
        fs::write(orch::pack_path(&store, unit), "pack").unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = ["u", "v"]
        .into_iter()
        .map(|unit| {
            let (h, store, barrier) = (h.clone(), store.clone(), barrier.clone());
            std::thread::spawn(move || {
                barrier.wait();
                assert!(!denied(
                    &h,
                    &reviewer(read(&orch::pack_path(&store, unit)), "a")
                ));
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let bound = fs::read_to_string(store.join("agents/a.reviewer")).unwrap();
    assert!(["u", "v"].contains(&bound.as_str()));
    for unit in ["u", "v"] {
        assert_eq!(
            !denied(
                &h,
                &reviewer(
                    write(&store.join(format!("reports/{unit}.verdict.json"))),
                    "a"
                )
            ),
            unit == bound
        );
    }
}

#[test]
fn reviewer_verdict_resolves_links_and_parent_components() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let store = reviewer_store(h);
    let pack = orch::pack_path(&store, "u");
    fs::write(&pack, "pack").unwrap();
    assert!(!denied(h, &reviewer(read(&pack), "a")));
    let alias = h.join("alias");
    if !dir_link(&alias, &store.join("reports")) {
        eprintln!("SKIPPED reviewer link checks: links unavailable");
        return;
    }
    assert!(!denied(
        h,
        &reviewer(write(&alias.join("u.verdict.json")), "a")
    ));
    let outside = h.join("elsewhere");
    fs::create_dir_all(outside.join("child")).unwrap();
    let link = store.join("reports/link");
    if !dir_link(&link, &outside.join("child")) {
        return;
    }
    assert!(denied(
        h,
        &reviewer(write(&link.join("../u.verdict.json")), "a")
    ));
    #[cfg(windows)]
    assert!(!denied(
        h,
        &reviewer(write(&store.join("REPORTS/U.VERDICT.JSON")), "a")
    ));
}

#[test]
fn only_the_core_may_waive_findings_even_without_a_goal() {
    let d = tempfile::tempdir().unwrap();
    for args in [json!(["waive", "u"]), json!(["--json", "waive", "u"])] {
        let call = tool(ORCH, json!({"project":"p", "args":args}));
        assert!(denied(d.path(), &sub(call.clone())));
        assert!(hook::handle(&sub(call.clone()), None, false).is_some());
        assert!(!denied(d.path(), &call));
    }
    assert!(!denied(
        d.path(),
        &sub(tool(ORCH, json!({"project":"p","args":["status"]})))
    ));
}

#[test]
fn only_the_core_may_touch_the_goal() {
    let d = tempfile::tempdir().unwrap();
    let close = tool(GOAL, json!({"session":"s","status":"done"}));
    change_goal(d.path());
    assert!(!denied(d.path(), &close));
    assert!(denied(d.path(), &sub(close.clone())));
    for input in [
        json!({"session":"s","classify":"stop"}),
        json!({"session":"s","mode":"change"}),
    ] {
        assert!(denied(d.path(), &sub(tool(GOAL, input))));
    }
    assert!(hook::handle(&sub(close), None, false).is_some());
}

#[test]
fn every_tool_the_hook_gates_reaches_the_hook() {
    let hooks: Value = serde_json::from_str(
        axiom::engine::FILES
            .iter()
            .find(|(path, _)| *path == "hooks/hooks.json")
            .unwrap()
            .1,
    )
    .unwrap();
    let matcher = hooks["hooks"]["PreToolUse"][0]["matcher"].as_str().unwrap();
    let routed: Vec<_> = matcher.split('|').collect();
    for name in [
        "Write",
        "Edit",
        "NotebookEdit",
        "Bash",
        "PowerShell",
        "Read",
        "AskUserQuestion",
        "Agent",
        "Task",
        CODEX,
        GOAL,
    ] {
        assert!(matcher == "*" || routed.contains(&name), "{name} never reaches the hook");
    }
}

#[test]
fn powershell_cannot_replace_read_only_bash() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let shell = tool(
        "PowerShell",
        json!({"command":"Set-Content project.txt changed"}),
    );
    assert!(!denied(h, &shell), "ordinary sessions stay untouched");
    goal::open(h, "s", "wish", None).unwrap();
    assert!(denied(h, &shell));
    assert!(denied(h, &sub(shell.clone())), "undeclared mode");
    goal::set_mode(h, "s", "change").unwrap();
    assert!(denied(h, &shell));
    assert!(!denied(h, &sub(shell)));
}

#[test]
fn goals_units_claims_and_evidence_cannot_be_written_with_file_tools() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    change_goal(h);
    for state in [
        "goals/s.json",
        "goals/other.json",
        "orchestrate/p/units.tsv",
        "orchestrate/p/claims.tsv",
        "orchestrate/p/.orch.lock",
        "orchestrate/p/reports/u.evidence.tsv",
        "orchestrate/p/reports/_full.evidence.tsv",
        "orchestrate/p/reports/_stages.evidence.tsv",
    ] {
        for name in ["Write", "Edit"] {
            let e = tool(name, json!({"file_path": h.join(state)}));
            assert!(denied(h, &e), "core wrote {state}");
            assert!(denied(h, &sub(e)), "agent wrote {state}");
        }
    }
    let mut relative = write(Path::new("../../goals/s.json"));
    relative["cwd"] = json!(h.join("orchestrate/p"));
    assert!(denied(h, &relative));
    assert!(denied(
        h,
        &tool(
            "NotebookEdit",
            json!({"notebook_path": h.join("goals/s.json")})
        )
    ));
    for allowed in [
        "orchestrate/p/decisions.md",
        "orchestrate/p/modules.md",
        "orchestrate/p/briefs/u.md",
        "orchestrate/p/astra/plan.json",
        "orchestrate/p/verdict.json",
        "orchestrate/p/reports/u.md",
    ] {
        assert!(!denied(h, &write(&h.join(allowed))), "core: {allowed}");
        assert!(!denied(h, &sub(write(&h.join(allowed)))), "agent: {allowed}");
    }
}

#[cfg(windows)]
#[test]
fn engine_files_are_protected_under_any_case_on_windows() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    change_goal(h);
    assert!(denied(h, &sub(write(&h.join("GOALS/S.JSON")))));
    let shouted = h.to_string_lossy().to_uppercase();
    assert!(denied(
        h,
        &sub(write(&Path::new(&shouted).join("goals/s.json")))
    ));
}

/// A directory link: a symlink, or on Windows without Developer Mode a junction.
fn dir_link(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_dir(target, link).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success() && link.is_dir());
}

#[test]
fn a_link_cannot_disguise_where_a_write_lands() {
    let d = tempfile::tempdir().unwrap();
    let h = &d.path().join("home");
    change_goal(h);
    let alias = h.join("alias");
    let outside = d.path().join("elsewhere");
    let project = d.path().join("project");
    fs::create_dir_all(&project).unwrap();
    if !dir_link(&alias, &h.join("goals"))
        || !dir_link(&outside, h)
        || !dir_link(&h.join("out"), &project)
    {
        eprintln!("SKIPPED a_link_cannot_disguise_where_a_write_lands: neither a symlink nor a junction can be created here");
        return;
    }
    assert!(alias.join("s.json").is_file());
    for path in [
        alias.join("s.json"),
        alias.join("new.json"),
        outside.join("goals/s.json"),
        outside.join("orchestrate/p/units.tsv"),
    ] {
        assert!(denied(h, &write(&path)), "core wrote {}", path.display());
        assert!(denied(h, &sub(write(&path))), "agent wrote {}", path.display());
    }
    let through = h.join("out/file.txt");
    assert!(denied(h, &write(&through)), "the core reached the project through the data home");
    assert!(!denied(h, &sub(write(&through))), "a change-mode agent may write the project");
    assert!(!denied(h, &write(&outside.join("orchestrate/p/decisions.md"))));
    assert!(!denied(h, &write(&h.join("orchestrate/p/decisions.md"))));
}

#[test]
fn an_unreadable_goal_closes_the_gates() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    goal::open(h, "s", "wish", None).unwrap();
    fs::write(h.join("goals/s.json"), "{not json").unwrap();
    assert!(denied(h, &write(&h.join("../project/file"))));
    assert!(denied(h, &tool("Bash", json!({"command":"rm -rf src"}))));
    let mut no_session = tool("Bash", json!({"command":"rm -rf src"}));
    no_session["session_id"] = json!("not a session");
    assert!(
        !denied(h, &no_session),
        "an invalid id can have no goal to check"
    );
}

#[test]
fn only_chronos_agents_run_during_an_active_goal() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let plain = tool("Agent", json!({"subagent_type":"general-purpose"}));
    let bare = tool("Task", json!({"prompt":"edit the project"}));
    assert!(!denied(h, &plain), "ordinary sessions stay untouched");
    change_goal(h);
    assert!(denied(h, &plain));
    assert!(denied(h, &bare));
    assert!(!denied(
        h,
        &tool("Agent", json!({"subagent_type":"chronos:memory-keeper"}))
    ));
    goal::set_status(h, "s", "blocked", Some("note")).unwrap();
    assert!(!denied(h, &plain), "a closed goal drives no work");
}

#[test]
fn an_undeclared_mode_gives_agents_no_write_access() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    goal::open(h, "s", "wish", None).unwrap();
    assert!(denied(h, &sub(write(&h.join("../project/file")))));
    assert!(denied(
        h,
        &sub(tool("Bash", json!({"command":"rm -rf src"})))
    ));
    assert!(!denied(h, &sub(write(&h.join("orchestrate/p/plan.json")))));
    assert!(!denied(h, &sub(tool("Bash", json!({"command":"git status"})))));
}

#[test]
fn environment_prefixes_that_choose_a_program_are_not_read_only() {
    for command in [
        "GIT_EXTERNAL_DIFF=rm git diff",
        "git_external_diff=rm git diff",
        "GIT_CONFIG_COUNT=1 git log",
        "PATH=. ls",
        "LD_PRELOAD=./x.so cat f",
        "DYLD_INSERT_LIBRARIES=./x cat f",
        "RIPGREP_CONFIG_PATH=./rc rg x",
    ] {
        assert!(!hook::read_only_bash(command), "{command}");
    }
    for command in ["FOO=1 ls", "LANG=C grep x f", "TERM=dumb git log"] {
        assert!(hook::read_only_bash(command), "{command}");
    }
}

#[test]
fn parallel_dispatches_leave_a_dispatcher_on_one_module() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path().to_path_buf();
    change_goal(&h);
    let store = orch::store_for(&h, "demo").unwrap();
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("modules.md"), "map").unwrap();
    fs::write(
        store.join("units.tsv"),
        "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\na\tm1\tpending\t\t\t\t\nb\tm2\tpending\t\t\t\t\nc\tm3\tpending\t\t\t\t\nd\tm4\tpending\t\t\t\t\n",
    )
    .unwrap();
    for round in 0..150 {
        let barrier = Arc::new(Barrier::new(4));
        let threads: Vec<_> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|unit| {
                let (h, barrier) = (h.clone(), barrier.clone());
                std::thread::spawn(move || {
                    let mut e = tool(CODEX, json!({"project":"demo","unit":unit,"role":"terra"}));
                    e["agent_id"] = json!(format!("dispatcher-{round}"));
                    barrier.wait();
                    !denied(&h, &e)
                })
            })
            .collect();
        let allowed = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(allowed, 1, "round {round}: one dispatcher took {allowed} modules");
    }
}
