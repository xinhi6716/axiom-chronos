use axiom::{
    app::Poller,
    bridge::{call, Bridge, Config},
    goal, orch,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn invoke(bridge: &Bridge, project: &str, args: &[&str]) -> Value {
    let result = call(bridge, "orch", &json!({"project": project, "args": args}));
    assert_eq!(result["isError"], false, "{result}");
    result["structuredContent"].clone()
}

fn list(bridge: &Bridge, project: &str) -> Value {
    let result = invoke(bridge, project, &["--json", "unit", "list"]);
    serde_json::from_str(result["stdout"].as_str().unwrap()).unwrap()
}

#[test]
fn consecutive_wishes_isolate_store_and_resume() {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let cwd = scratch.path().to_str().unwrap();
    let mut config = Config::from_env();
    config.data_home = Some(home.path().into());
    let bridge = Bridge::new(config);
    let first = goal::open(home.path(), "first", "first wish", Some(cwd)).unwrap();
    let project = first.project.as_deref().unwrap();
    let first_store = orch::store_for_goal(home.path(), &first).unwrap();
    invoke(&bridge, project, &["init"]);
    invoke(&bridge, project, &["unit", "add", "u1", "--track", "core"]);
    let digest = call(
        &bridge,
        "memory_digest",
        &json!({"project": project, "add":"first wish outcome"}),
    );
    assert_eq!(digest["isError"], false, "{digest}");
    fs::create_dir_all(first_store.join("reports")).unwrap();
    fs::write(
        orch::verdict_path(&first_store, "u1"),
        r#"{"unit":"u1","tree":"t","verdict":"pass","findings":[]}"#,
    )
    .unwrap();
    fs::write(
        first_store.join("reports/u1.evidence.tsv"),
        "kind\tcommand\texit_code\tcommit\tts\nverify\ttest\t0\tt\tnow\n",
    )
    .unwrap();
    fs::create_dir_all(first_store.join("handoffs")).unwrap();
    fs::write(orch::handoff_path(&first_store, None), "first handoff").unwrap();
    fs::write(
        first_store.join("claims.tsv"),
        "unit\tattempt\tscope\tstarted\nu1\ta\tsrc\tnow\n",
    )
    .unwrap();
    let second = goal::open(home.path(), "second", "second wish", Some(cwd)).unwrap();
    assert_ne!(first.wish_id, second.wish_id);
    let second_store = orch::store_for_goal(home.path(), &second).unwrap();
    assert_ne!(first_store, second_store);
    assert_eq!(orch::store_for(home.path(), project).unwrap(), second_store);
    invoke(&bridge, project, &["init"]);
    assert!(list(&bridge, project).as_array().unwrap().is_empty());
    assert!(orch::read_verdict(&first_store, "u1").unwrap().is_some());
    assert!(orch::read_verdict(&second_store, "u1").unwrap().is_none());
    assert!(orch::evidence(&second_store, "u1").unwrap().is_empty());
    assert!(!orch::handoff_path(&second_store, None).exists());
    assert!(!second_store.join("claims.tsv").exists());
    goal::interrupt(home.path(), "second", "paused").unwrap();
    let resumed = goal::reopen(home.path(), "second").unwrap();
    assert_eq!(resumed.wish_id, second.wish_id);
    invoke(&bridge, project, &["unit", "add", "u2", "--track", "core"]);
    let units = list(&bridge, project);
    assert_eq!(units.as_array().unwrap().len(), 1);
    assert_eq!(units[0]["id"], "u2");
    let digest = call(&bridge, "memory_digest", &json!({"project": project}));
    assert!(digest["structuredContent"]["digest"]
        .as_str()
        .unwrap()
        .contains("first wish outcome"));

    let snapshot = Poller::new(home.path()).poll();
    for (session, id, path) in [("first", "u1", first_store), ("second", "u2", second_store)] {
        let view = snapshot
            .wishes
            .iter()
            .find(|w| w.session == session)
            .unwrap();
        assert_eq!(view.store.as_deref(), Some(path.as_path()));
        assert_eq!(view.units.len(), 1);
        assert_eq!(view.units[0].id, id);
    }
}

#[test]
fn legacy_goal_without_wish_id_uses_project_store() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("goals")).unwrap();
    fs::write(
        home.path().join("goals/legacy.json"),
        json!({"wish":"old", "status":"active", "project":"demo"}).to_string(),
    )
    .unwrap();
    let goal = goal::load(home.path(), "legacy").unwrap().unwrap();
    let expected = home.path().join("orchestrate/demo");
    assert_eq!(orch::store_for(home.path(), "demo").unwrap(), expected);
    assert_eq!(orch::store_for_goal(home.path(), &goal).unwrap(), expected);
    assert_eq!(orch::wish_of_store(&expected), "demo");
    assert_eq!(
        orch::wish_store(home.path(), "demo", Some("w-123-bad"))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        orch::wish_store(home.path(), "../bad", None)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
}

#[test]
fn current_wish_prefers_newest_active_then_newest_other_status() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("goals");
    fs::create_dir_all(&dir).unwrap();
    for (file, status, id) in [
        ("a.json", "active", "w-1700000000000-000001"),
        ("b.json", "active", "w-1700000000000-000002"),
        ("c.json", "blocked", "w-1700000000000-000003"),
    ] {
        fs::write(
            dir.join(file),
            json!({"wish":"x","project":"demo","status":status,"wish_id":id}).to_string(),
        )
        .unwrap();
    }
    fs::write(dir.join("broken.json"), "not json").unwrap();
    fs::write(
        dir.join("ignored.txt"),
        json!({"wish":"x","project":"demo","status":"active","wish_id":"w-9999999999999-ffffff"})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        orch::current_wish(home.path(), "demo").unwrap().as_deref(),
        Some("w-1700000000000-000002")
    );
    fs::remove_file(dir.join("a.json")).unwrap();
    fs::remove_file(dir.join("b.json")).unwrap();
    assert_eq!(
        orch::current_wish(home.path(), "demo").unwrap().as_deref(),
        Some("w-1700000000000-000003")
    );
    assert_eq!(
        orch::wish_of_store(&orch::store_for(home.path(), "demo").unwrap()),
        "w-1700000000000-000003"
    );
}

#[test]
fn claims_upgrade_legacy_and_preserve_new_fields() {
    let store = tempfile::tempdir().unwrap();
    let file = store.path().join("claims.tsv");
    fs::write(
        &file,
        "unit\tattempt\tscope\tstarted\na\t1\tsrc\tnow\nb\t2\tdoc\tlater\n",
    )
    .unwrap();
    orch::release_claim(store.path(), "a", "1").unwrap();
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\nb\t2\tdoc\tlater\t\t\t\t\n"
    );
    fs::write(&file, "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\na\t1\tsrc\tnow\tm\twish\t123\tp q\nb\t2\tdoc\tlater\tm2\twish2\t456\tr s\n").unwrap();
    orch::release_claim(store.path(), "a", "1").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "unit\tattempt\tscope\tstarted\tmodule\twish\tpid\tpeers\nb\t2\tdoc\tlater\tm2\twish2\t456\tr s\n");
}

#[test]
fn closing_older_goal_checks_its_own_store() {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let cwd = scratch.path().to_str().unwrap();
    let first = goal::open(home.path(), "first", "first", Some(cwd)).unwrap();
    let store = orch::store_for_goal(home.path(), &first).unwrap();
    for args in [vec!["init"], vec!["unit", "add", "u1", "--track", "core"]] {
        let result = orch::run(
            &store,
            &args.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            false,
            false,
        );
        assert_eq!(result.exit_code, 0, "{}", result.stderr);
    }
    goal::open(home.path(), "second", "second", Some(cwd)).unwrap();
    assert!(goal::set_status(home.path(), "first", "done", None).is_err());
    assert_eq!(
        goal::load(home.path(), "first").unwrap().unwrap().status,
        "active"
    );
}

#[test]
fn cut_off_blocks_active_goal_with_turn_limit() {
    let home = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let original = goal::open(home.path(), "session", "wish", scratch.path().to_str()).unwrap();
    goal::interject(home.path(), "session", "please").unwrap();
    let blocked = goal::cut_off(home.path(), "session", 12).unwrap().unwrap();
    assert_eq!(blocked.status, "blocked");
    assert_eq!(blocked.reason.as_deref(), Some("max_turns"));
    assert_eq!(blocked.turn_limit, Some(12));
    assert_eq!(
        blocked.note.as_deref(),
        Some("cut off at 12 turns. Resume with: axiom wish --resume session")
    );
    assert!(blocked.interjection.is_none());
    let before = fs::read(home.path().join("goals/session.json")).unwrap();
    assert!(goal::cut_off(home.path(), "session", 99).unwrap().is_none());
    assert_eq!(
        fs::read(home.path().join("goals/session.json")).unwrap(),
        before
    );
    assert!(goal::cut_off(home.path(), "missing", 1).unwrap().is_none());
    let resumed = goal::reopen(home.path(), "session").unwrap();
    assert_eq!(resumed.wish_id, original.wish_id);
    assert_eq!(resumed.turn_limit, None);
    assert_eq!(resumed.reason, None);
    goal::open(home.path(), "done", "wish", None).unwrap();
    goal::set_status(home.path(), "done", "done", None).unwrap();
    let done_path = Path::new(home.path()).join("goals/done.json");
    let before = fs::read(&done_path).unwrap();
    assert!(goal::cut_off(home.path(), "done", 12).unwrap().is_none());
    assert_eq!(fs::read(done_path).unwrap(), before);
}
