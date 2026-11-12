use axiom::modes::{
    api::ModesApi,
    context::{bounded, truncate_utf8, DEFAULT_CONTEXT_BYTES},
    runs,
    store::Transcript,
    Message, Mode,
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

fn append(store: &Transcript, text: &str, thread: Option<&str>) -> Message {
    store
        .append("user", "text", text, thread, json!({"value": text}))
        .unwrap()
}

#[test]
fn reopen_and_damaged_tail() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let store = Transcript::open(home.path(), &session.id).unwrap();
    assert_eq!(store.last_seq().unwrap(), 0);
    let first = append(&store, "first", None);
    drop(store);
    let path = home
        .path()
        .join("modes/sessions")
        .join(&session.id)
        .join("transcript.jsonl");
    let before = fs::read(&path).unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"{\"seq\":2,\"text\":\"broken").unwrap();
    drop(file);
    let store = Transcript::open(home.path(), &session.id).unwrap();
    assert_eq!(store.read(0, 10).unwrap(), vec![first.clone()]);
    assert_eq!(store.last_seq().unwrap(), 1);
    let second = append(&store, "second", Some("topic"));
    assert_eq!(second.seq, 2);
    assert_eq!(store.read(0, 10).unwrap(), vec![first, second.clone()]);
    assert_eq!(store.get(2).unwrap(), Some(second));
    assert_eq!(store.get(3).unwrap(), None);
    assert!(fs::read(&path).unwrap().starts_with(&before));
}

#[test]
fn invalid_and_missing_sessions() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        Transcript::open(home.path(), "../x").unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        Transcript::open(home.path(), "missing").unwrap_err().kind,
        "not_found"
    );
}

#[test]
fn concurrent_appends_are_contiguous_across_handles() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let path = home.path().to_owned();
        let id = session.id.clone();
        let barrier = barrier.clone();
        workers.push(thread::spawn(move || {
            let store = Transcript::open(&path, &id).unwrap();
            barrier.wait();
            (0..12)
                .map(|_| append(&store, "parallel", None).seq)
                .collect::<Vec<_>>()
        }));
    }
    let mut seqs: Vec<_> = workers
        .into_iter()
        .flat_map(|w| w.join().unwrap())
        .collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=96).collect::<Vec<_>>());
    let store = Transcript::open(home.path(), &session.id).unwrap();
    assert_eq!(
        store
            .read(0, 100)
            .unwrap()
            .iter()
            .map(|m| m.seq)
            .collect::<Vec<_>>(),
        seqs
    );
}

#[test]
fn pages_snapshots_and_threads() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let store = Transcript::open(home.path(), &session.id).unwrap();
    let first = append(&store, "a", Some("one"));
    let second = append(&store, "b", Some("two"));
    let third = append(&store, "c", Some("one"));
    assert_eq!(
        store.read(0, 2).unwrap(),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(store.read(2, 2).unwrap(), vec![third.clone()]);
    assert!(store.read(0, 0).unwrap().is_empty());
    assert_eq!(
        store.thread("one").unwrap(),
        vec![first.clone(), third.clone()]
    );
    let snapshot = store.snapshot(Some("one")).unwrap();
    let all = store.snapshot(None).unwrap();
    append(&store, "later", Some("one"));
    assert_eq!(snapshot.session, session.id);
    assert_eq!(snapshot.thread.as_deref(), Some("one"));
    assert_eq!(snapshot.upto_seq, 3);
    assert_eq!(snapshot.messages, vec![first, third]);
    assert_eq!(all.upto_seq, 3);
    assert_eq!(all.messages.len(), 3);
    assert_eq!(store.last_seq().unwrap(), 4);
}

#[test]
fn utf8_context_caps_and_recency() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let store = Transcript::open(home.path(), &session.id).unwrap();
    append(&store, "old", None);
    append(&store, "漢字漢字", None);
    let messages = store.read(0, 10).unwrap();
    assert_eq!(DEFAULT_CONTEXT_BYTES, 24_000);
    assert_eq!(truncate_utf8("漢字", 0), "");
    assert_eq!(truncate_utf8("漢字", 2), "");
    assert_eq!(truncate_utf8("漢字", 4), "漢");
    assert_eq!(truncate_utf8("漢字", 100), "漢字");
    for cap in 0..=40 {
        let result = bounded(&messages, cap);
        assert!(result.len() <= cap, "cap {cap}");
        assert!(std::str::from_utf8(result.as_bytes()).is_ok());
    }
    assert_eq!(bounded(&messages, 0), "");
    assert_eq!(bounded(&messages, 8), "user: ");
    assert_eq!(bounded(&messages, 9), "user: 漢");
    assert_eq!(
        bounded(&messages, "user: 漢字漢字\n".len()),
        "user: 漢字漢字\n"
    );
    assert_eq!(bounded(&messages, 100), "user: old\nuser: 漢字漢字\n");
}

#[test]
fn facade_round_trip() {
    let home = tempfile::tempdir().unwrap();
    let api = ModesApi::new(home.path());
    let scope = vec!["src".to_owned()];
    let session = api
        .open_session(Some(Path::new("project")), &scope)
        .unwrap();
    assert_eq!(api.session(&session.id).unwrap(), session);
    let store = Transcript::open(home.path(), &session.id).unwrap();
    let first = append(&store, "hello", None);
    let second = append(&store, "world", None);
    assert_eq!(api.transcript(&session.id, 1, 10).unwrap(), vec![second]);
    assert_eq!(
        api.context(&session.id, 100).unwrap(),
        "user: hello\nuser: world\n"
    );
    assert_eq!(
        api.context(&session.id, "user: world\n".len()).unwrap(),
        "user: world\n"
    );
    assert_eq!(store.get(1).unwrap(), Some(first));
    assert!(api.runs(&session.id).unwrap().is_empty());
    let run = runs::start_run(home.path(), &session.id, Mode::Chat, None, None).unwrap();
    assert_eq!(api.runs(&session.id).unwrap(), vec![run]);
}

#[test]
fn process_append_worker() {
    let Some(home) = std::env::var_os("AXIOM_TRANSCRIPT_TEST_HOME") else {
        return;
    };
    let id = std::env::var("AXIOM_TRANSCRIPT_TEST_SESSION").unwrap();
    let store = Transcript::open(Path::new(&home), &id).unwrap();
    for _ in 0..16 {
        append(&store, "process", None);
    }
}

#[test]
fn concurrent_processes_and_existing_lock_file() {
    let home = tempfile::tempdir().unwrap();
    let session = runs::create_session(home.path(), None, &[]).unwrap();
    let lock = home
        .path()
        .join("modes/sessions")
        .join(&session.id)
        .join("transcript.lock");
    fs::write(lock, b"left over").unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children: Vec<_> = (0..3)
        .map(|_| {
            std::process::Command::new(&executable)
                .args(["--exact", "process_append_worker"])
                .env("AXIOM_TRANSCRIPT_TEST_HOME", home.path())
                .env("AXIOM_TRANSCRIPT_TEST_SESSION", &session.id)
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let store = Transcript::open(home.path(), &session.id).unwrap();
    assert_eq!(
        store
            .read(0, 100)
            .unwrap()
            .iter()
            .map(|m| m.seq)
            .collect::<Vec<_>>(),
        (1..=48).collect::<Vec<_>>()
    );
}
