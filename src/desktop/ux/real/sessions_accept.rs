use super::sessions::ServiceSessions;
use crate::{
    desktop::ux::seams::{Connection, Controller, SessionStream},
    service::{client::Client, login, tray, RunningService, Server, ServerOptions, StartRequest},
    session::{self, Config},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Fixture {
    home: tempfile::TempDir,
    server: Option<RunningService>,
}
impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let storage = crate::storage::dir(home.path());
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(
            storage.join("policy.json"),
            r#"{"reserve_bytes":0,"estimates":{"wish":1}}"#,
        )
        .unwrap();
        let server = Self::start_server(home.path());
        Self { home, server: Some(server) }
    }
    fn start_server(home: &Path) -> RunningService {
        let config_home = home.to_path_buf();
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake_claude.py");
        let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
            if cfg!(windows) {
                "python".into()
            } else {
                "python3".into()
            }
        });
        let calls = || Arc::new(Mutex::new(Vec::new()));
        Server::start(ServerOptions { home: home.to_path_buf(), config: Arc::new(move || Config {
   claude: Ok(vec![python.clone(), "-I".into(), "-c".into(), "import os,runpy,sys,uuid; os.environ['FAKE_CLAUDE_SCENARIO']='tick'; os.environ['FAKE_CLAUDE_SESSION']=str(uuid.uuid4()); os.environ['FAKE_CLAUDE_RECORD']=os.path.join(sys.argv[2],str(os.getpid())+'.json'); sys.argv=[sys.argv[1]]+sys.argv[3:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(), fake.to_string_lossy().into_owned(), config_home.to_string_lossy().into_owned()]),
   home: config_home.clone(), bin: config_home.join("bin"), clock: session::Clock::system() }), tray: Box::new(tray::Recording::new(calls())), desktop: Box::new(tray::Recording::new(calls())), login: Box::new(login::Recording::new(calls())), login_launch:false }).unwrap()
    }
    fn home(&self) -> &Path {
        self.home.path()
    }
    fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            let r = server.quit(true);
            server.wait();
            if !std::thread::panicking() {
                r.unwrap()
            }
        }
    }
}

#[test]
fn real_service_restart_changes_instance_and_recovers_subscription() {
    let mut fixture = Fixture::new();
    let stream = ServiceSessions::new(fixture.home().to_path_buf());
    let client = Client::connect(fixture.home()).unwrap();
    let first_instance = client.snapshot().unwrap().instance;
    let handle = client.start(StartRequest {
        wish: "before restart".into(),
        cwd: fixture.home().to_string_lossy().into_owned(),
        ..Default::default()
    }).unwrap();
    let mut id = String::new();
    wait(|| {
        id = client.snapshot().unwrap().wishes.iter().find(|w| w.handle == handle)
            .and_then(|w| w.session.clone()).unwrap_or_default();
        !id.is_empty()
    });
    wait(|| stream.sessions().iter().any(|s| s.id == id));
    wait(|| stream.connection(&id) == Connection::Live);
    wait(|| !stream.frames(&id, 0).unwrap().is_empty());
    // Hold the consumer state while the server stops, so the test observes a
    // transport loss before it can refresh the server's final end state.
    let state = stream.state.lock().unwrap();
    fixture.stop();
    drop(state);
    wait(|| stream.connection(&id) == Connection::Lost);
    let before = stream.frames(&id, 0).unwrap();
    fixture.server = Some(Fixture::start_server(fixture.home()));
    let client = Client::connect(fixture.home()).unwrap();
    assert_ne!(first_instance, client.snapshot().unwrap().instance);
    stream.reconnect(&id).unwrap();
    wait(|| stream.connection(&id) == Connection::Closed);
    assert_eq!(stream.frames(&id, 0).unwrap(), before);
    let handle = client.start(StartRequest {
        wish: "after restart".into(),
        cwd: fixture.home().to_string_lossy().into_owned(),
        ..Default::default()
    }).unwrap();
    let mut new_id = String::new();
    wait(|| {
        new_id = client.snapshot().unwrap().wishes.iter().find(|w| w.handle == handle)
            .and_then(|w| w.session.clone()).unwrap_or_default();
        !new_id.is_empty()
    });
    wait(|| stream.sessions().iter().any(|s| s.id == new_id));
    wait(|| stream.connection(&new_id) == Connection::Live);
    wait(|| !stream.frames(&new_id, 0).unwrap().is_empty());
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
    }
}
fn wait(mut f: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(15);
    while !f() {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(40));
    }
}
fn seeded_stream() -> (tempfile::TempDir, ServiceSessions) {
    let home = tempfile::tempdir().unwrap();
    let stream = ServiceSessions::new(home.path().to_path_buf());
    let snapshot = serde_json::from_value(serde_json::json!({
        "instance":"one", "cursor":1, "login_start":false,
        "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
            "cwd":".","started_unix":0,"state":"ended","limit_until":null,
            "ended_at":null,"ending":"","disk_line":null,
            "log":[{"time_ms":0,"source":"assistant","text":"one","level":"info"}]}]
    })).unwrap();
    stream.state.lock().unwrap().snapshot(&snapshot);
    (home, stream)
}
#[test]
fn sessions_returns_snapshot_sessions() {
    let (_home, stream) = seeded_stream();
    assert_eq!(stream.sessions().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["s"]);
}
#[test]
fn frames_returns_snapshot_incrementally() {
    let (_home, stream) = seeded_stream();
    let frames = stream.frames("s", 0).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].text, "one");
    assert!(stream.frames("s", frames[0].seq).unwrap().is_empty());
}
#[test]
fn connection_returns_closed_for_ended_session() {
    let (_home, stream) = seeded_stream();
    assert_eq!(stream.connection("s"), Connection::Closed);
}
#[test]
fn real_service_sessions_frames_disconnect_reconnect_and_end() {
    let mut fixture = Fixture::new();
    let stream = ServiceSessions::new(fixture.home().to_path_buf());
    let client = Client::connect(fixture.home()).unwrap();
    let handle = client
        .start(StartRequest {
            wish: "session test".into(),
            cwd: fixture.home().to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let mut id = String::new();
    wait(|| {
        if let Some(s) = client
            .snapshot()
            .unwrap()
            .wishes
            .iter()
            .find(|w| w.handle == handle)
            .and_then(|w| w.session.clone())
        {
            id = s;
            true
        } else {
            false
        }
    });
    wait(|| stream.sessions().iter().any(|s| s.id == id));
    wait(|| stream.connection(&id) == Connection::Live);
    wait(|| !stream.frames(&id, 0).unwrap().is_empty());
    let initial = stream.frames(&id, 0).unwrap();
    let last = initial.last().unwrap().seq;
    wait(|| !stream.frames(&id, last).unwrap().is_empty());
    let frames = stream.frames(&id, 0).unwrap();
    assert!(frames.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    assert!(stream
        .frames(&id, last)
        .unwrap()
        .iter()
        .all(|f| f.seq > last));
    // Force a cursor discontinuity while receiving real IPC events; the
    // adapter must re-snapshot without duplicating already observed ticks.
    let snapshot_count = {
        let mut state = stream.state.lock().unwrap();
        let count = state.snapshot_count;
        state.force_cursor(0);
        count
    };
    wait(|| stream.state.lock().unwrap().snapshot_count > snapshot_count);
    let before_gap = stream.frames(&id, 0).unwrap();
    let gap_cursor = before_gap.last().unwrap().seq;
    wait(|| !stream.frames(&id, gap_cursor).unwrap().is_empty());
    let after_gap = stream.frames(&id, 0).unwrap();
    assert_eq!(&after_gap[..before_gap.len()], &before_gap);
    let texts: std::collections::HashSet<_> = after_gap.iter().map(|f| &f.text).collect();
    assert_eq!(texts.len(), after_gap.len());
    let before_reconnect = after_gap.last().unwrap().seq;
    stream.reconnect(&id).unwrap();
    wait(|| stream.connection(&id) == Connection::Lost);
    wait(|| stream.connection(&id) == Connection::Live);
    wait(|| !stream.frames(&id, before_reconnect).unwrap().is_empty());
    let reconnected = stream.frames(&id, 0).unwrap();
    assert_eq!(&reconnected[..after_gap.len()], &after_gap);
    let texts: std::collections::HashSet<_> = reconnected.iter().map(|f| &f.text).collect();
    assert_eq!(texts.len(), reconnected.len());
    client.stop(&handle).unwrap();
    wait(|| stream.connection(&id) == Connection::Closed);
    let started = Instant::now();
    drop(stream);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "live subscription did not stop"
    );
    fixture.stop();
}
#[test]
fn unsupported_controls_and_lazy_construction() {
    let home = tempfile::tempdir().unwrap();
    let stream = ServiceSessions::new(home.path().to_path_buf());
    assert!(!home.path().join("service").exists());
    assert!(matches!(stream.controller("s"), Controller::Refused { reason } if !reason.is_empty()));
    assert_eq!(
        stream.request_takeover("s").unwrap_err().kind,
        "unsupported"
    );
    assert_eq!(
        stream.send_input("s", "t", "x").unwrap_err().kind,
        "unsupported"
    );
    assert_eq!(stream.hand_back("s", "t").unwrap_err().kind, "unsupported");
}

#[test]
fn sequence_dedup_gap_and_instance_change() {
    use super::sessions::State;
    use crate::service::{Snapshot, WireEvent};
    fn snapshot(instance: &str) -> Snapshot {
        serde_json::from_value(serde_json::json!({"instance":instance,"cursor":0,"login_start":false,"wishes":[{"key":"s","handle":"h","session":"s","text":"wish","cwd":".","started_unix":0,"state":"running","limit_until":null,"ended_at":null,"ending":"","disk_line":null,"log":[]}]})).unwrap()
    }
    fn event(instance: &str, seq: u64, text: &str) -> WireEvent {
        WireEvent {
            seq,
            instance: instance.into(),
            handle: "h".into(),
            session: Some("s".into()),
            kind: "text".into(),
            data: serde_json::json!({"text":text}),
        }
    }
    let mut state = State::default();
    state.snapshot(&snapshot("one"));
    assert!(state.event(&event("one", 1, "a")));
    assert!(state.event(&event("one", 1, "a")));
    assert!(!state.event(&event("one", 3, "gap")));
    assert!(state.event(&event("one", 2, "b")));
    assert_eq!(state.entries["s"].frames.len(), 2);
    state.snapshot(&snapshot("two"));
    assert!(state.event(&event("two", 1, "new")));
    assert_eq!(state.entries["s"].frames.last().unwrap().seq, 3);
    assert!(!state.event(&event("one", 3, "stale")));
}

#[test]
fn drop_joins_after_unavailable_service() {
    let home = tempfile::tempdir().unwrap();
    let stream = ServiceSessions::new(home.path().to_path_buf());
    let _ = stream.sessions();
    let start = Instant::now();
    drop(stream);
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "subscription did not stop"
    );
}

#[test]
fn snapshot_recovers_gap_without_repeating_seen_frames() {
    use super::sessions::State;
    use crate::service::{Snapshot, WireEvent};
    let snapshot = |cursor, texts: &[&str]| -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "instance":"one", "cursor":cursor, "login_start":false,
            "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
                "cwd":".","started_unix":0,"state":"running","limit_until":null,
                "ended_at":null,"ending":"","disk_line":null,
                "log":texts.iter().map(|text| serde_json::json!({"time_ms":0,"source":"assistant","text":text,"level":"info"})).collect::<Vec<_>>() }]
        })).unwrap()
    };
    let event = |seq, text| WireEvent {
        seq,
        instance: "one".into(),
        handle: "h".into(),
        session: Some("s".into()),
        kind: "text".into(),
        data: serde_json::json!({"text":text}),
    };
    let mut state = State::default();
    state.snapshot(&snapshot(1, &["a"]));
    assert!(state.event(&event(2, "b")));
    assert!(state.event(&event(2, "b")));
    assert!(!state.event(&event(4, "d")));
    state.snapshot(&snapshot(4, &["a", "b", "c", "d"]));
    assert!(state.event(&event(4, "d")));
    assert_eq!(
        state.entries["s"]
            .frames
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c", "d"]
    );
    assert!(state.entries["s"]
        .frames
        .windows(2)
        .all(|v| v[0].seq < v[1].seq));
}

#[test]
fn bounded_snapshot_reports_possible_missing_frames() {
    use super::sessions::State;
    use crate::service::Snapshot;
    let snapshot = |cursor, texts: Vec<String>| -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "instance":"one", "cursor":cursor, "login_start":false,
            "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
                "cwd":".","started_unix":0,"state":"running","limit_until":null,
                "ended_at":null,"ending":"","disk_line":null,
                "log":texts.iter().map(|text| serde_json::json!({"time_ms":0,"source":"assistant","text":text,"level":"info"})).collect::<Vec<_>>() }]
        })).unwrap()
    };
    let mut state = State::default();
    state.snapshot(&snapshot(1, vec!["seen".into()]));
    let boundary = state.frames("s", 0).unwrap()[0].seq;
    state.snapshot(&snapshot(3000, (0..2000).map(|n| format!("new {n}")).collect()));
    let error = state.frames("s", boundary).unwrap_err();
    assert_eq!(error.kind, "gap");
    assert!(error.message.contains("missing"));
    let last = state.entries["s"].frames.last().unwrap().seq;
    assert!(state.frames("s", last).unwrap().is_empty());
}

#[test]
fn full_snapshot_complete_overlap_does_not_report_gap() {
    use super::sessions::State;
    use crate::service::Snapshot;
    let snapshot = |cursor, start| -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "instance":"one", "cursor":cursor, "login_start":false,
            "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
                "cwd":".","started_unix":0,"state":"running","limit_until":null,
                "ended_at":null,"ending":"","disk_line":null,
                "log":(start..start + 2000).map(|n| serde_json::json!({"time_ms":0,"source":"assistant","text":format!("line {n}"),"level":"info"})).collect::<Vec<_>>() }]
        })).unwrap()
    };
    let mut state = State::default();
    state.snapshot(&snapshot(2000, 0));
    let last = state.frames("s", 0).unwrap().last().unwrap().seq;
    state.snapshot(&snapshot(2001, 1));
    let new = state.frames("s", last).expect("complete overlap must not report a gap");
    assert_eq!(new.len(), 1);
    assert_eq!(new[0].text, "line 2000");
    assert!(state.frames("s", new[0].seq).unwrap().is_empty());
    state.snapshot(&snapshot(2002, 1));
    assert!(state.frames("s", new[0].seq).unwrap().is_empty());
}

#[test]
fn full_snapshot_overlap_survives_unrelated_cursor_advance() {
    use super::sessions::State;
    use crate::service::Snapshot;
    let snapshot = |cursor, start| -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "instance":"one", "cursor":cursor, "login_start":false,
            "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
                "cwd":".","started_unix":0,"state":"running","limit_until":null,
                "ended_at":null,"ending":"","disk_line":null,
                "log":(start..start + 2000).map(|n| serde_json::json!({"time_ms":n,"source":"assistant","text":format!("line {n}"),"level":"info"})).collect::<Vec<_>>() }]
        })).unwrap()
    };
    let mut state = State::default();
    state.snapshot(&snapshot(2000, 0));
    let last = state.frames("s", 0).unwrap().last().unwrap().seq;
    // The cursor covers all service events, including other sessions.
    state.snapshot(&snapshot(4000, 1));
    let frames = state.frames("s", last).expect("overlapping log is continuous");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].text, "line 2000");
    assert!(state.frames("s", frames[0].seq).unwrap().is_empty());
}

#[test]
fn full_snapshot_repeated_text_cannot_prove_overlap_after_log_capacity_events() {
    use super::sessions::State;
    use crate::service::Snapshot;
    let snapshot = |cursor, last: &str| -> Snapshot {
        let mut texts = vec!["repeated"; 2000];
        texts[1999] = last;
        serde_json::from_value(serde_json::json!({
            "instance":"one", "cursor":cursor, "login_start":false,
            "wishes":[{"key":"s","handle":"h","session":"s","text":"wish",
                "cwd":".","started_unix":0,"state":"running","limit_until":null,
                "ended_at":null,"ending":"","disk_line":null,
                "log":texts.iter().map(|text| serde_json::json!({"time_ms":0,"source":"assistant","text":text,"level":"info"})).collect::<Vec<_>>() }]
        })).unwrap()
    };
    let mut state = State::default();
    state.snapshot(&snapshot(2000, "repeated"));
    let last = state.frames("s", 0).unwrap().last().unwrap().seq;
    state.snapshot(&snapshot(4000, "new"));
    assert_eq!(state.frames("s", last).unwrap_err().kind, "gap");
    let latest = state.entries["s"].frames.last().unwrap().seq;
    assert!(state.frames("s", latest).unwrap().is_empty());
}

#[test]
fn unavailable_service_reports_lost_and_reconnect_is_bounded() {
    let home = tempfile::tempdir().unwrap();
    let stream = ServiceSessions::new(home.path().to_path_buf());
    wait(|| stream.connection("missing") == Connection::Lost);
    assert_eq!(stream.reconnect("missing").unwrap_err().kind, "not_found");
    let start = Instant::now();
    drop(stream);
    assert!(start.elapsed() < Duration::from_secs(5));
}
