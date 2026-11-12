use axiom::{
    base::{Failure, Res},
    modes::{
        launch::{LaunchKind, LaunchRequest, Launched, Launcher},
        plaza::{
            self,
            access::Budgets,
            pins::{self, ConclusionSink, Pin, PinKind},
        },
        runs,
        store::Transcript,
        Source,
    },
};
use serde_json::{json, Value};
use std::{fs, sync::Mutex};

#[derive(Default)]
struct Sink {
    accepted: Mutex<Vec<Pin>>,
    fail: bool,
}
impl ConclusionSink for Sink {
    fn accept(&self, pin: &Pin) -> Res<()> {
        self.accepted.lock().unwrap().push(pin.clone());
        if self.fail {
            Err(Failure::new("sink_error", "rejected"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn pin_persists_sources_deduplicates_and_records_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let session = runs::create_session(tmp.path(), None, &[]).unwrap();
    let transcript = Transcript::open(tmp.path(), &session.id).unwrap();
    let own = Source {
        session: session.id.clone(),
        seq: 1,
        author: "opus".into(),
    };
    let other = Source {
        session: "other".into(),
        seq: 7,
        author: "person".into(),
    };
    let message = transcript
        .append(
            "opus",
            "text",
            "choose this",
            Some("t1"),
            json!({"sources": [own, other, other, {"session": "invalid"}, "bad"]}),
        )
        .unwrap();
    let sink = Sink::default();
    let first = pins::pin(
        tmp.path(),
        &session.id,
        message.seq,
        PinKind::Decision,
        "person",
        &sink,
    )
    .unwrap();
    assert_eq!(first.sources, vec![own, other]);
    assert_eq!(first.text, message.text);
    assert_eq!(first.author, message.author);
    assert_eq!(first.thread, message.thread);
    assert_eq!(first.pinned_by, "person");
    assert_eq!(first.id.len(), 32);
    assert!(first.id.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(sink.accepted.lock().unwrap().as_slice(), &[first.clone()]);
    assert_eq!(
        pins::pins(tmp.path(), &session.id).unwrap(),
        vec![first.clone()]
    );
    let reopened = Transcript::open(tmp.path(), &session.id).unwrap();
    let messages = reopened.read(0, 100).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].kind, "pin");
    assert_eq!(messages[1].author, "person");
    assert_eq!(messages[1].thread.as_deref(), Some("t1"));
    assert_eq!(
        messages[1].data,
        json!({"pin": first.id, "seq": message.seq, "kind": "decision"})
    );
    let duplicate = pins::pin(
        tmp.path(),
        &session.id,
        message.seq,
        PinKind::Decision,
        "someone",
        &sink,
    )
    .unwrap();
    assert_eq!(duplicate, first);
    let path = tmp
        .path()
        .join("modes/sessions")
        .join(&session.id)
        .join("pins.jsonl");
    assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
    assert_eq!(sink.accepted.lock().unwrap().len(), 1);
    let memory = pins::pin(
        tmp.path(),
        &session.id,
        message.seq,
        PinKind::Memory,
        "person",
        &sink,
    )
    .unwrap();
    assert_ne!(memory.id, first.id);
    assert_eq!(memory.kind, PinKind::Memory);
    assert_eq!(
        pins::pins(tmp.path(), &session.id).unwrap(),
        vec![first, memory]
    );
    assert_eq!(sink.accepted.lock().unwrap().len(), 2);
}

#[test]
fn missing_and_failing_sink_store_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let session = runs::create_session(tmp.path(), None, &[]).unwrap();
    let sink = Sink::default();
    assert_eq!(
        pins::pin(
            tmp.path(),
            &session.id,
            42,
            PinKind::Decision,
            "person",
            &sink
        )
        .unwrap_err()
        .kind,
        "not_found"
    );
    assert!(pins::pins(tmp.path(), &session.id).unwrap().is_empty());
    assert!(sink.accepted.lock().unwrap().is_empty());
    let transcript = Transcript::open(tmp.path(), &session.id).unwrap();
    let message = transcript
        .append("person", "text", "hello", None, Value::Null)
        .unwrap();
    let failing = Sink {
        fail: true,
        ..Sink::default()
    };
    assert_eq!(
        pins::pin(
            tmp.path(),
            &session.id,
            message.seq,
            PinKind::Memory,
            "person",
            &failing
        )
        .unwrap_err()
        .kind,
        "sink_error"
    );
    assert!(pins::pins(tmp.path(), &session.id).unwrap().is_empty());
    assert_eq!(transcript.read(0, 100).unwrap(), vec![message]);
    assert!(!tmp
        .path()
        .join("modes/sessions")
        .join(&session.id)
        .join("pins.jsonl")
        .exists());
    assert!(pins::pins(tmp.path(), "../bad").is_err());
}

#[test]
fn concurrent_duplicate_pins_call_sink_once() {
    let tmp = tempfile::tempdir().unwrap();
    let session = runs::create_session(tmp.path(), None, &[]).unwrap();
    let transcript = Transcript::open(tmp.path(), &session.id).unwrap();
    let message = transcript
        .append("person", "text", "hello", None, Value::Null)
        .unwrap();
    let sink = Sink::default();
    std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    pins::pin(
                        tmp.path(),
                        &session.id,
                        message.seq,
                        PinKind::Decision,
                        "person",
                        &sink,
                    )
                    .unwrap()
                })
            })
            .collect();
        let pins: Vec<_> = tasks.into_iter().map(|task| task.join().unwrap()).collect();
        assert!(pins.iter().all(|pin| pin == &pins[0]));
    });
    assert_eq!(sink.accepted.lock().unwrap().len(), 1);
    assert_eq!(pins::pins(tmp.path(), &session.id).unwrap().len(), 1);
    assert_eq!(transcript.read(0, 100).unwrap().len(), 2);
}

struct FakeLauncher {
    request: Mutex<Option<LaunchRequest>>,
    child: Launched,
}
impl Launcher for FakeLauncher {
    fn launch(&self, request: &LaunchRequest) -> Res<Launched> {
        *self.request.lock().unwrap() = Some(request.clone());
        Ok(self.child.clone())
    }
}

#[test]
fn promotion_uses_only_thread_snapshot_and_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let room = plaza::open(
        tmp.path(),
        Some(tmp.path()),
        &[],
        Budgets(Default::default()),
    )
    .unwrap();
    let transcript = Transcript::open(tmp.path(), &room.session.id).unwrap();
    let first = transcript
        .append("person", "text", "one", Some("t1"), Value::Null)
        .unwrap();
    transcript
        .append("person", "text", "other thread", Some("t2"), Value::Null)
        .unwrap();
    let second = transcript
        .append("opus", "text", "two", Some("t1"), Value::Null)
        .unwrap();
    let launcher = FakeLauncher {
        request: Mutex::new(None),
        child: Launched {
            run: room.run.clone(),
            detail: Value::Null,
        },
    };
    let result = pins::promote_thread(
        tmp.path(),
        &room,
        "t1",
        LaunchKind::Wish,
        "goal",
        None,
        &launcher,
    )
    .unwrap();
    let request = launcher.request.lock().unwrap().clone().unwrap();
    let context = request.context.unwrap();
    assert_eq!(context.thread.as_deref(), Some("t1"));
    assert_eq!(context.messages, vec![first.clone(), second.clone()]);
    let sources = vec![
        Source {
            session: room.session.id.clone(),
            seq: first.seq,
            author: first.author,
        },
        Source {
            session: room.session.id.clone(),
            seq: second.seq,
            author: second.author,
        },
    ];
    assert_eq!(request.sources, sources);
    assert_eq!(result.sources, sources);
    assert_eq!(request.parent_run, room.run.id);
    assert_eq!(
        pins::promote_thread(
            tmp.path(),
            &room,
            "../bad",
            LaunchKind::Wish,
            "goal",
            None,
            &launcher
        )
        .unwrap_err()
        .kind,
        "invalid_argument"
    );
}
