use axiom::{goal, hook, orch};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

const ORCH: &str = "mcp__plugin_chronos_bridge__orch";
const CODEX: &str = "mcp__plugin_chronos_bridge__codex_run";
const HEADER: &str = "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\n";
const MAP: &str = "## m1\nPURPOSE a\nPATHS src/a/\nDEPENDS none\nCONTRACT a\nACCEPT src/a/ `test`\n## m2\nPURPOSE b\nPATHS src/b/\nDEPENDS none\nCONTRACT b\nACCEPT src/b/ `test`\n";

fn tool(name: &str, input: Value, agent: Option<&str>) -> Value {
    let mut e = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":name,"tool_input":input});
    if let Some(id) = agent {
        e["agent_id"] = json!(id);
    }
    e
}
fn reason(home: &Path, event: &Value) -> Option<String> {
    let result = hook::handle(event, Some(home), false)?;
    (result["hookSpecificOutput"]["permissionDecision"] == "deny").then(|| {
        result["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .to_owned()
    })
}
fn setup(home: &Path) -> (goal::Goal, PathBuf) {
    let project = home.join("project");
    fs::create_dir_all(&project).unwrap();
    let g = goal::open(home, "s", "wish", Some(project.to_str().unwrap())).unwrap();
    goal::set_mode(home, "s", "change").unwrap();
    let store = orch::store_for_goal(home, &g).unwrap();
    fs::create_dir_all(store.join("briefs")).unwrap();
    fs::write(store.join("modules.md"), MAP).unwrap();
    fs::write(store.join("units.tsv"), HEADER).unwrap();
    (g, store)
}
fn brief(scope: &str) -> String {
    format!("GOAL work\nSCOPE {scope}\nVERIFY test\n")
}
fn write(path: &Path, content: &str, agent: &str) -> Value {
    tool(
        "Write",
        json!({"file_path":path,"content":content}),
        Some(agent),
    )
}
fn assert_foreign(home: &Path, e: &Value) {
    let why = reason(home, e).expect("foreign operation must be denied");
    assert!(why.contains("m1") && why.contains("m2"), "{why}");
}

#[test]
fn dispatcher_cannot_write_foreign_brief() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let (_, store) = setup(h);
    let u1 = store.join("briefs/u1.md");
    assert!(reason(h, &write(&u1, &brief("src/a/x.rs"), "d1")).is_none());
    assert_eq!(fs::read_to_string(store.join("agents/d1")).unwrap(), "m1");
    assert_foreign(h, &write(&u1, &brief("src/b/y.rs"), "d1"));
    assert_foreign(h, &write(&u1, &brief("src/a/x.rs src/b/y.rs"), "d1"));
    assert!(reason(h, &write(&u1, "not a brief", "d1")).is_none());
    assert!(reason(h, &write(&u1, &brief("src/b/y.rs"), "d2")).is_none());
    assert_eq!(fs::read_to_string(store.join("agents/d2")).unwrap(), "m2");
    fs::write(
        store.join("units.tsv"),
        format!("{HEADER}u9\tm2\tpending\t\t\t\t\n"),
    )
    .unwrap();
    let u9 = store.join("briefs/u9.md");
    assert_foreign(h, &write(&u9, &brief("src/a/x.rs"), "d1"));
    assert!(reason(h, &write(&u9, &brief("src/b/y.rs"), "d2")).is_none());
    fs::write(&u1, brief("src/a/x.rs")).unwrap();
    let edit = tool(
        "Edit",
        json!({"file_path":u1,"old_string":"src/a/x.rs","new_string":"src/b/y.rs"}),
        Some("d1"),
    );
    assert_foreign(h, &edit);
    fs::write(&u1, brief("src/a/x.rs src/a/x.rs")).unwrap();
    let edit_all = tool(
        "Edit",
        json!({"file_path":u1,"old_string":"src/a/x.rs","new_string":"src/b/y.rs","replace_all":true}),
        Some("d1"),
    );
    assert_foreign(h, &edit_all);
    let why = reason(h, &write(&u1, &brief("elsewhere/x.rs"), "d1")).unwrap();
    assert!(why.contains("m1") && why.contains("no module"));
    let named = store.join("briefs/other.md");
    fs::write(
        store.join("units.tsv"),
        format!("{HEADER}u9\tm2\tpending\t\t\t\t{}\n", named.display()),
    )
    .unwrap();
    assert_foreign(h, &write(&named, &brief("src/a/x.rs"), "d1"));
    let through_parent = store.join("briefs/../briefs/other.md");
    assert_foreign(h, &write(&through_parent, &brief("src/a/x.rs"), "d1"));
    let alias = h.join("brief-alias");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(store.join("briefs"), &alias).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(store.join("briefs"), &alias).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(store.join("briefs"))
            .output()
            .is_ok_and(|o| o.status.success());
    if linked {
        assert_foreign(
            h,
            &write(&alias.join("other.md"), &brief("src/a/x.rs"), "d1"),
        );
    }
    let outside = store.join("briefs/../outside.md");
    assert!(reason(h, &write(&outside, &brief("src/b/y.rs"), "d1")).is_none());
    let mixed = write(
        &store.join("briefs/unbound.md"),
        &brief("src/a/x.rs src/b/y.rs"),
        "d3",
    );
    assert!(reason(h, &mixed).is_none());
    assert!(!store.join("agents/d3").exists());
}

#[test]
fn dispatcher_cannot_register_foreign_track() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let (g, store) = setup(h);
    assert!(reason(
        h,
        &write(&store.join("briefs/u1.md"), &brief("src/a/x.rs"), "d1")
    )
    .is_none());
    let call = |args: Value| tool(ORCH, json!({"args":args}), Some("d1"));
    assert!(reason(h, &call(json!(["unit", "add", "x", "--track", "m1"]))).is_none());
    assert!(reason(h, &call(json!(["status"]))).is_none());
    for args in [
        json!(["unit", "add", "x", "--track", "m2"]),
        json!(["--json", "unit", "add", "x", "--track", "m2"]),
    ] {
        assert_foreign(h, &call(args));
    }
    fs::write(
        store.join("units.tsv"),
        format!("{HEADER}u9\tm2\tpending\t\t\t\t\n"),
    )
    .unwrap();
    assert_foreign(
        h,
        &tool(
            CODEX,
            json!({"project":g.project,"unit":"u9","role":"terra"}),
            Some("d1"),
        ),
    );
}

#[test]
fn per_wish_store_state_is_protected() {
    let d = tempfile::tempdir().unwrap();
    let h = d.path();
    let (g, store) = setup(h);
    assert!(g.wish_id.is_some());
    for name in ["briefs/x.md", "reports/u.verdict.json"] {
        for agent in [None, Some("unbound")] {
            assert!(reason(
                h,
                &tool("Write", json!({"file_path":store.join(name)}), agent)
            )
            .is_none());
        }
    }
    for name in [
        "units.tsv",
        "claims.tsv",
        ".orch.lock",
        ".orch.lock.x",
        "reports/u.evidence.tsv",
    ] {
        for tool_name in ["Write", "Edit"] {
            assert!(
                reason(
                    h,
                    &tool(
                        tool_name,
                        json!({"file_path":store.join(name)}),
                        Some("unbound")
                    )
                )
                .is_some(),
                "{name}"
            );
        }
    }
    let legacy = h.join("orchestrate").join(g.project.as_ref().unwrap());
    assert!(reason(
        h,
        &tool("Write", json!({"file_path":legacy.join("units.tsv")}), None)
    )
    .is_some());
    fs::create_dir_all(store.join("handoffs")).unwrap();
    fs::write(
        orch::handoff_path(&store, None),
        json!({"module":null,"ts":"now","next":"continue"}).to_string(),
    )
    .unwrap();
    assert!(!orch::handoff_path(&legacy, None).exists());
    let mut other = g.clone();
    other.wish_id = Some("w-9999999999999-ffffff".into());
    fs::write(
        h.join("goals/other.json"),
        serde_json::to_vec(&other).unwrap(),
    )
    .unwrap();
    assert_ne!(
        orch::store_for(h, g.project.as_deref().unwrap()).unwrap(),
        store
    );
    assert!(hook::stop_reason_in(h, "s", &g).contains("Pending handoff"));
    let resumed = hook::handle(
        &json!({"hook_event_name":"SessionStart", "session_id":"s", "source":"compact"}),
        Some(h),
        false,
    )
    .unwrap();
    assert!(resumed["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("continue"));
}
