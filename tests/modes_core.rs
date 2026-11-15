use axiom::modes::{
    launch::{start_child, LaunchKind, LaunchRequest},
    providers::Member,
    readonly::{ReadOnly, ToolRequest},
    runs::{self, acquire_slot},
    Message, Mode,
};
use serde_json::json;
use std::{fs, path::Path};

fn request(tool: &str, args: serde_json::Value) -> ToolRequest {
    ToolRequest {
        tool: tool.into(),
        args,
    }
}
#[test]
fn modes_serialize_and_parse() {
    let names = ["chat", "quick", "wish", "research", "routine", "plaza"];
    for (mode, name) in Mode::ALL.into_iter().zip(names) {
        assert_eq!(mode.as_str(), name);
        assert_eq!(Mode::parse(name), Some(mode));
        assert_eq!(serde_json::to_value(mode).unwrap(), name);
        assert_eq!(serde_json::from_value::<Mode>(json!(name)).unwrap(), mode);
    }
    assert_eq!(Mode::parse("Chat"), None);
    assert_eq!(Mode::parse("unknown"), None);
    let message: Message = serde_json::from_value(
        json!({"seq":1,"ts":"now","author":"person","kind":"text","text":"hi"}),
    )
    .unwrap();
    assert_eq!(message.thread, None);
    assert_eq!(message.data, json!(null));
}
#[test]
fn runs_and_child() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let session = runs::create_session(home, Some(Path::new("project")), &["src".into()]).unwrap();
    assert_eq!(runs::load_session(home, &session.id).unwrap(), session);
    let parent = runs::start_run(home, &session.id, Mode::Chat, None, None).unwrap();
    let quick = runs::start_run(
        home,
        &session.id,
        Mode::Quick,
        Some(&parent.id),
        Some("ref"),
    )
    .unwrap();
    let wish = runs::start_run(home, &session.id, Mode::Wish, None, None).unwrap();
    let req = LaunchRequest {
        kind: LaunchKind::Wish,
        session: session.id.clone(),
        parent_run: quick.id.clone(),
        goal: "goal".into(),
        scope: vec![],
        verify: None,
        review: false,
        context: None,
        sources: vec![],
    };
    let child = start_child(home, &req).unwrap();
    assert_eq!(child.mode, Mode::Wish);
    assert_eq!(child.session, session.id);
    assert_eq!(child.parent.as_deref(), Some(quick.id.as_str()));
    let mut expected = vec![parent.clone(), quick.clone(), wish, child];
    expected.sort_by(|a, b| (&a.started, &a.id).cmp(&(&b.started, &b.id)));
    assert_eq!(runs::session_runs(home, &session.id).unwrap(), expected);
    assert_eq!(runs::load_run(home, &quick.id).unwrap(), quick);
    let finished = runs::finish_run(home, &parent.id, "done").unwrap();
    assert!(finished.finished.is_some());
    assert_eq!(finished.outcome.as_deref(), Some("done"));
    assert_eq!(
        runs::finish_run(home, &parent.id, "again")
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        runs::start_run(home, &session.id, Mode::Quick, Some("missing"), None)
            .unwrap_err()
            .kind,
        "not_found"
    );
    assert_eq!(
        runs::load_run(home, "../bad").unwrap_err().kind,
        "invalid_argument"
    );
}
#[test]
fn session_scope_and_ids() {
    let home = tempfile::tempdir().unwrap();
    for bad in [
        "..",
        "src/../secret",
        "src\\..\\secret",
        "/absolute",
        "C:\\absolute",
        "C:relative",
        "\\\\server\\share",
    ] {
        assert_eq!(
            runs::create_session(home.path(), None, &[bad.into()])
                .unwrap_err()
                .kind,
            "invalid_argument",
            "{bad}"
        );
    }
    assert_eq!(
        runs::load_session(home.path(), "../bad").unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        runs::load_session(home.path(), "missing").unwrap_err().kind,
        "not_found"
    );
}
#[test]
fn same_slot_pool_for_all_modes() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let _chat = runs::start_run(home.path(), &session.id, Mode::Chat, None, None).unwrap();
    let _wish = runs::start_run(home.path(), &session.id, Mode::Wish, None, None).unwrap();
    let first = acquire_slot(home.path(), 2).unwrap();
    let second = acquire_slot(home.path(), 2).unwrap();
    assert!(acquire_slot(home.path(), 2).is_err());
    drop(first);
    assert!(acquire_slot(home.path(), 2).is_ok());
    drop(second);
}
#[test]
fn record_launch_only_tracks_wish_and_research() {
    let home = tempfile::tempdir().unwrap();
    runs::record_launch(home.path(), "other", "no");
    assert!(!home.path().join("modes").exists());
    runs::record_launch(home.path(), "wish", "wish-ref");
    runs::record_launch(home.path(), "research", "research-ref");
    let mut found = Vec::new();
    for entry in fs::read_dir(home.path().join("modes/sessions")).unwrap() {
        let id = entry.unwrap().file_name().into_string().unwrap();
        let session = runs::load_session(home.path(), &id).unwrap();
        assert!(session.project.is_none());
        assert!(session.scope.is_empty());
        let list = runs::session_runs(home.path(), &id).unwrap();
        assert_eq!(list.len(), 1);
        found.push((list[0].mode, list[0].reference.clone().unwrap()));
    }
    assert_eq!(found.len(), 2);
    assert!(found.contains(&(Mode::Wish, "wish-ref".into())));
    assert!(found.contains(&(Mode::Research, "research-ref".into())));
}
#[test]
fn members() {
    for member in Member::ALL {
        assert_eq!(Member::parse(member.name()), Some(member));
        assert_eq!(Member::parse(&member.name().to_uppercase()), Some(member));
    }
    assert_eq!(Member::parse("unknown"), None);
}
#[test]
fn read_only_tools_confine_and_cap() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("hello.txt"), "hello").unwrap();
    fs::write(project.join("cjk.txt"), "漢".repeat(6000)).unwrap();
    fs::write(home.path().join("secret.txt"), "secret").unwrap();
    let tools = ReadOnly::new(Some(&project), &home.path().join("memory-home"));
    let result = tools
        .call(&request(
            "write_file",
            json!({"path":"created.txt", "content":"bad"}),
        ))
        .unwrap_err();
    assert_eq!(result.kind, "read_only");
    assert!(!project.join("created.txt").exists());
    assert!(!home.path().join("memory-home").exists());
    for path in ["../secret.txt", "C:\\secret", "/absolute", "missing"] {
        assert_eq!(
            tools
                .call(&request("read_file", json!({"path":path})))
                .unwrap_err()
                .kind,
            "invalid_argument"
        );
    }
    assert_eq!(
        tools
            .call(&request("read_file", json!({"path":"hello.txt"})))
            .unwrap(),
        "hello"
    );
    let listing = tools
        .call(&request("list_dir", json!({"path":"."})))
        .unwrap();
    assert!(listing.contains("hello.txt"));
    let result = tools
        .call(&request("read_file", json!({"path":"cjk.txt"})))
        .unwrap();
    assert_eq!(result.len(), 15_999);
    assert_eq!(result, "漢".repeat(5333));
    assert!(tools
        .call(&request("memory_search", json!({"query":"anything"})))
        .unwrap()
        .contains("entries"));
    assert_eq!(
        tools
            .call(&request("memory_get", json!({"id":999})))
            .unwrap_err()
            .kind,
        "not_found"
    );
}
#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(home.path().join("secret"), "secret").unwrap();
    std::os::unix::fs::symlink(home.path().join("secret"), project.join("link")).unwrap();
    let tools = ReadOnly::new(Some(&project), home.path());
    assert_eq!(
        tools
            .call(&request("read_file", json!({"path":"link"})))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}
