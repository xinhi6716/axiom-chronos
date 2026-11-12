use super::*;
use serde_json::{json, Value};
use std::fs;

const WISH: &str = "w-1234567890123-abcdef";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn setup() -> (tests::Rig, AppState) {
    let mut rig = tests::rig();
    // Keep unrelated quota, ledger and memory workers from launching processes in these tests.
    rig.engine.quota_busy = true;
    rig.engine.ledger_busy = true;
    rig.engine.memory_busy = true;
    let mut state = tests::state_with(vec![]);
    state.ux = Default::default();
    (rig, state)
}
fn goal(home: &Path, session: &str) {
    write(&home.join("goals").join(format!("{session}.json")), &json!({
        "wish":"Build it", "wish_id":WISH, "project":"proj", "status":"active", "amendments":["Keep it small"]
    }).to_string());
}
fn log(home: &Path, session: &str, time: &str, complete: bool, extra: &[Value]) {
    let mut values = vec![json!({"type":"system","subtype":"init","session_id":session})];
    values.extend_from_slice(extra);
    let mut model = json!({"inputTokens":1,"outputTokens":3,"cacheReadInputTokens":2,"cacheCreationInputTokens":0,"costUSD":0.5});
    if !complete {
        model.as_object_mut().unwrap().remove("outputTokens");
    }
    values.push(
        json!({"type":"result","total_cost_usd":0.5,"modelUsage":{"claude-sonnet-5-5":model}}),
    );
    write(
        &home
            .join("logs")
            .join(format!("20261109-{time}-wish.jsonl")),
        &values.iter().map(|v| format!("{v}\n")).collect::<String>(),
    );
}
fn poll(rig: &mut tests::Rig, state: &mut AppState) {
    rig.engine.tick();
    apply(state, rig.updates());
}

#[test]
fn disk_poll_populates_activity_notices_council_and_amendments_without_replay() {
    let (mut rig, mut state) = setup();
    let home = rig._dir.path();
    goal(home, "s1");
    log(
        home,
        "s1",
        "100000",
        true,
        &[
            json!({"type":"chronos","subtype":"max_turns","turn_limit":12}),
            json!({"type":"council","subtype":"member_degraded","member":"fable","phase":"A","round":1,"status":"quota","reason":"limit"}),
            json!({"error":{"kind":"outside_scope"},"unit":"u1","outside_scope":["other.rs"]}),
        ],
    );
    let store = home.join("orchestrate/proj").join(WISH);
    write(
        &store.join("events.jsonl"),
        &format!(
            "{}\n",
            json!({"seq":1,"ts":"2026-01-01T00:00:00Z","ts_ms":1234,"kind":"activity_start","wish":WISH,"module":"m","unit":null,"data":{"activity":"a","actor":"core","phase":"brief"}})
        ),
    );
    write(&store.join("astra/council.json"), &json!({"key":"k","question":"q","members":[{"name":"astra","model":"m","family":"f","status":"ok","tokens":0,"usd":0.1}],"rounds":[{"phase":"A","n":2,"positions":[]}],"claims":[],"cost":{"usd":0.1,"cap_usd":5.0,"cut":[]},"degraded":false}).to_string());
    poll(&mut rig, &mut state);
    assert_eq!(state.ux.activity["s1"].last_activity_ms, Some(1234));
    assert_eq!(state.ux.activity["s1"].open.len(), 1);
    assert_eq!(state.ux.notices["s1"].len(), 3);
    assert_eq!(state.ux.council["s1"].round, 2);
    assert_eq!(state.ux.amendments["s1"][0].as_ref(), "Keep it small");
    assert_eq!(state.ux.wish_ids["s1"].as_ref(), WISH);
    poll(&mut rig, &mut state);
    assert_eq!(state.ux.activity["s1"].last_activity_ms, Some(1234));
    assert_eq!(state.ux.activity["s1"].last_seq, 1);
    assert_eq!(state.ux.notices["s1"].len(), 3);
    assert_eq!(state.ux.usage_sessions.len(), 1);
}

#[test]
fn resumed_sessions_share_wish_and_missing_figures_are_incomplete() {
    let (mut rig, mut state) = setup();
    let home = rig._dir.path();
    goal(home, "s1");
    goal(home, "s2");
    log(home, "s1", "100000", true, &[]);
    log(home, "s2", "110000", false, &[]);
    poll(&mut rig, &mut state);
    let sessions = &state.ux.usage_sessions;
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|s| s.wish.as_ref() == WISH));
    assert!(
        sessions
            .iter()
            .find(|s| s.session.as_ref() == "s1")
            .unwrap()
            .complete
    );
    assert!(
        !sessions
            .iter()
            .find(|s| s.session.as_ref() == "s2")
            .unwrap()
            .complete
    );
    let grouped = crate::desktop::usage::wish_scope::aggregate(sessions, Some(WISH));
    assert_eq!(grouped.sessions, 2);
    assert!(grouped.incomplete);
    poll(&mut rig, &mut state);
    assert_eq!(state.ux.usage_sessions.len(), 2);

    let before: Vec<_> = state
        .ux
        .usage_sessions
        .iter()
        .map(|s| s.session.clone())
        .collect();
    let mut snapshot = rig.engine.snapshot.clone().unwrap();
    snapshot.wishes.reverse();
    let rows = rig
        .engine
        .ux_feed
        .poll(&snapshot, &rig.engine.hosted, &rig.engine.ledgers);
    apply(&mut state, vec![Update::Ux(rows)]);
    assert_eq!(
        state
            .ux
            .usage_sessions
            .iter()
            .map(|s| s.session.clone())
            .collect::<Vec<_>>(),
        before
    );
    snapshot.wishes.retain(|w| w.session != before[0].as_ref());
    let rows = rig
        .engine
        .ux_feed
        .poll(&snapshot, &rig.engine.hosted, &rig.engine.ledgers);
    apply(&mut state, vec![Update::Ux(rows)]);
    assert_eq!(state.ux.usage_sessions.len(), 1);
    assert_eq!(state.ux.usage_sessions[0].session, before[1]);
    snapshot.wishes.clear();
    let rows = rig
        .engine
        .ux_feed
        .poll(&snapshot, &rig.engine.hosted, &rig.engine.ledgers);
    apply(&mut state, vec![Update::Ux(rows)]);
    assert!(state.ux.usage_sessions.is_empty());
}

#[test]
fn renamed_session_preserves_ux_data() {
    let (mut rig, mut state) = setup();
    let home = rig._dir.path().to_path_buf();
    goal(&home, "s1");
    log(
        &home,
        "s1",
        "100000",
        true,
        &[json!({"type":"chronos","subtype":"max_turns"})],
    );
    let store = home.join("orchestrate/proj").join(WISH);
    write(
        &store.join("events.jsonl"),
        &format!(
            "{}\n",
            json!({"seq":1,"ts":"","ts_ms":1234,"kind":"activity_start","wish":WISH,"module":"m","unit":null,"data":{"activity":"a","actor":"core","phase":"brief"}})
        ),
    );
    write(
        &store.join("astra/council-events.jsonl"),
        &format!(
            "{}\n",
            json!({"type":"council","subtype":"turn_start","member":"fable","phase":"A","round":1})
        ),
    );
    poll(&mut rig, &mut state);
    rig.engine.renamed.push(("s1".into(), "s2".into()));
    rig.engine.publish();
    assert_eq!(rig.engine.ux_feed.last_seq("s1"), None);
    assert_eq!(rig.engine.ux_feed.last_seq("s2"), Some(1));
    apply(&mut state, rig.updates());
    assert_eq!(state.ux.activity["s2"].last_seen_ms, Some(1234));
    assert_eq!(state.ux.notices["s2"].len(), 1);
    assert_eq!(state.ux.council["s2"].round, 1);
    assert_eq!(state.ux.amendments["s2"][0].as_ref(), "Keep it small");
    assert_eq!(state.ux.wish_ids["s2"].as_ref(), WISH);
    assert_eq!(state.ux.usage_sessions[0].session.as_ref(), "s2");
    assert!(!state.ux.activity.contains_key("s1"));
    fs::remove_file(home.join("goals/s1.json")).unwrap();
    goal(&home, "s2");
    log(
        &home,
        "s2",
        "100000",
        true,
        &[json!({"type":"chronos","subtype":"max_turns"})],
    );
    poll(&mut rig, &mut state);
    assert_eq!(rig.engine.ux_feed.last_seq("s2"), Some(1));
    assert_eq!(state.ux.activity["s2"].last_activity_ms, Some(1234));
    assert_eq!(state.ux.activity["s2"].last_seq, 1);
}

#[test]
fn oversized_events_are_skipped_until_the_file_fits() {
    let (mut rig, mut state) = setup();
    let home = rig._dir.path();
    goal(home, "s1");
    log(
        home,
        "s1",
        "100000",
        true,
        &[json!({"type":"chronos","subtype":"max_turns"})],
    );
    let path = home
        .join("orchestrate/proj")
        .join(WISH)
        .join("events.jsonl");
    let event = format!(
        "{}\n",
        json!({"seq":1,"ts":"","ts_ms":1234,"kind":"activity_start","wish":WISH,"module":"m","unit":null,"data":{"activity":"a","actor":"core","phase":"brief"}})
    );
    write(&path, &(event.clone() + &"x".repeat(4 << 20)));
    poll(&mut rig, &mut state);
    assert_eq!(rig.engine.ux_feed.last_seq("s1"), Some(0));
    assert!(!state.ux.activity.contains_key("s1"));
    assert_eq!(state.ux.notices["s1"].len(), 1);
    write(&path, &event);
    poll(&mut rig, &mut state);
    assert_eq!(state.ux.activity["s1"].last_seq, 1);
    assert_eq!(state.ux.activity["s1"].last_activity_ms, Some(1234));
}

#[test]
fn oversized_log_is_read_over_polls_and_yields_a_later_notice() {
    use std::io::Write;
    let (mut rig, mut state) = setup();
    let home = rig._dir.path().to_path_buf();
    goal(&home, "s1");
    log(&home, "s1", "100000", true, &[]);
    let path = home.join("logs/20261109-100000-wish.jsonl");
    poll(&mut rig, &mut state);
    let before = rig.engine.ux_feed.log_offset(&path).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&vec![b'x'; (4 << 20) + 10]).unwrap();
    file.write_all(b"\n{\"type\":\"chronos\",\"subtype\":\"max_turns\"}\n")
        .unwrap();
    poll(&mut rig, &mut state);
    assert_eq!(
        rig.engine.ux_feed.log_offset(&path),
        Some(before + (4 << 20))
    );
    assert!(state.ux.notices["s1"].is_empty());
    poll(&mut rig, &mut state);
    assert_eq!(state.ux.notices["s1"].len(), 1);
    assert_eq!(state.ux.notices["s1"][0].key, "max_turns");
}

#[test]
fn hosted_notices_include_sessions_with_and_without_a_wish() {
    let (mut rig, mut state) = setup();
    goal(rig._dir.path(), "s1");
    for session in ["s1", "early"] {
        let mut hosted = Hosted::new(false, "wish".into(), rig._dir.path().into());
        hosted.say(
            "host",
            json!({"type":"chronos","subtype":"max_turns","turn_limit":12}).to_string(),
            LogLevel::Info,
        );
        rig.engine.hosted.insert(session.into(), hosted);
    }
    poll(&mut rig, &mut state);
    for session in ["s1", "early"] {
        assert_eq!(state.ux.notices[session].len(), 1);
        assert_eq!(state.ux.notices[session][0].key, "max_turns");
    }
    assert!(state
        .ux
        .usage_sessions
        .iter()
        .all(|s| s.session.as_ref() != "early"));
}
