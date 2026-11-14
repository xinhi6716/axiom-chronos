use super::interjections::{EngineInterjections, InterjectTransport, ServiceTransport};
use crate::{
    base::{Failure, Res},
    desktop::ux::seams::{Draft, Interjections, ReceiptStage},
    engine::interjections::{self, Links},
    service::{client::Client, login, tray, RunningService, Server, ServerOptions, StartRequest},
    session::{self, Config},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

// Stop and join even when an assertion panics, so the fake hosts cannot outlive a test.
struct ServiceFixture {
    home: tempfile::TempDir,
    server: Option<RunningService>,
}
impl ServiceFixture {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let storage = crate::storage::dir(home.path());
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(
            storage.join("policy.json"),
            r#"{"reserve_bytes":0,"estimates":{"wish":1}}"#,
        )
        .unwrap();
        let config_home = home.path().to_path_buf();
        let fake = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fakes/fake_claude.py");
        let python = std::env::var("AXIOM_TEST_PYTHON").unwrap_or_else(|_| {
            let names = if cfg!(windows) {
                ["python", "python3"]
            } else {
                ["python3", "python"]
            };
            names
                .into_iter()
                .find(|name| {
                    crate::base::proc::cmd(name)
                        .arg("--version")
                        .output()
                        .is_ok_and(|out| out.status.success())
                })
                .expect("Python not found")
                .into()
        });
        let calls = || Arc::new(Mutex::new(Vec::new()));
        let server = Server::start(ServerOptions {
            home: home.path().to_path_buf(),
            config: Arc::new(move || Config {
                claude: Ok(vec![
                    python.clone(), "-I".into(), "-c".into(),
                    "import os,runpy,sys,uuid; os.environ['FAKE_CLAUDE_SCENARIO']='tick'; os.environ['FAKE_CLAUDE_SESSION']=str(uuid.uuid4()); os.environ['FAKE_CLAUDE_RECORD']=os.path.join(sys.argv[2],str(os.getpid())+'.json'); sys.argv=[sys.argv[1]]+sys.argv[3:]; runpy.run_path(sys.argv[0],run_name='__main__')".into(),
                    fake.to_string_lossy().into_owned(), config_home.to_string_lossy().into_owned(),
                ]),
                home: config_home.clone(), bin: config_home.join("bin"), clock: session::Clock::system(),
            }),
            tray: Box::new(tray::Recording::new(calls())),
            desktop: Box::new(tray::Recording::new(calls())),
            login: Box::new(login::Recording::new(calls())),
            login_launch: false,
        }).unwrap();
        Self {
            home,
            server: Some(server),
        }
    }
    fn home(&self) -> &Path {
        self.home.path()
    }
    fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            let result = server.quit(true);
            server.wait();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
impl Drop for ServiceFixture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn hosted_session(client: &Client, home: &Path, wish: &str) -> (String, String) {
    let handle = client
        .start(StartRequest {
            wish: wish.into(),
            cwd: home.to_string_lossy().into_owned(),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let snapshot = client.snapshot().unwrap();
        if let Some(session) = snapshot
            .wishes
            .iter()
            .find(|w| w.handle == handle && !w.log.is_empty())
            .and_then(|w| w.session.clone())
        {
            return (handle, session);
        }
        assert!(
            Instant::now() < deadline,
            "wish {wish} did not report a session and log; home: {}; snapshot: {}",
            home.display(),
            serde_json::to_string(&snapshot).unwrap()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

const SERVICE_TEXT: &str = "你好，世界 🌍\nsecond line: café";

#[test]
fn right_session_receives_text_byte_for_byte() {
    let fixture = ServiceFixture::new();
    let client = Client::connect(fixture.home()).unwrap();
    let (a, session_a) = hosted_session(&client, fixture.home(), "wish A");
    let (b, session_b) = hosted_session(&client, fixture.home(), "wish B");
    assert_ne!(session_a, session_b);
    ServiceTransport::new(fixture.home().into())
        .interject(&session_a, SERVICE_TEXT)
        .unwrap();
    let expected = format!("heard: {SERVICE_TEXT}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let snapshot = client.snapshot().unwrap();
        let wish = snapshot.wishes.iter().find(|w| w.handle == a).unwrap();
        if wish.log.iter().any(|entry| entry.text == expected) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "wish A did not receive the exact text"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let deadline = Instant::now() + Duration::from_millis(700);
    loop {
        let snapshot = client.snapshot().unwrap();
        let wish = snapshot.wishes.iter().find(|w| w.handle == b).unwrap();
        assert!(
            !wish
                .log
                .iter()
                .any(|entry| entry.text.contains(SERVICE_TEXT)),
            "wish B received A's text"
        );
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn unknown_session_is_rejected() {
    let fixture = ServiceFixture::new();
    let client = Client::connect(fixture.home()).unwrap();
    hosted_session(&client, fixture.home(), "wish A");
    hosted_session(&client, fixture.home(), "wish B");
    let error = ServiceTransport::new(fixture.home().into())
        .interject("unknown-session", SERVICE_TEXT)
        .unwrap_err();
    assert_eq!(error.kind, "not_found");
    let deadline = Instant::now() + Duration::from_millis(700);
    loop {
        let snapshot = client.snapshot().unwrap();
        assert!(snapshot
            .wishes
            .iter()
            .all(|w| w.log.iter().all(|entry| !entry.text.contains(SERVICE_TEXT))));
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn service_disconnect_is_an_error() {
    let mut fixture = ServiceFixture::new();
    let client = Client::connect(fixture.home()).unwrap();
    let (_, session) = hosted_session(&client, fixture.home(), "wish A");
    fixture.stop();
    assert!(ServiceTransport::new(fixture.home().into())
        .interject(&session, SERVICE_TEXT)
        .is_err());
}

struct Transport {
    home: PathBuf,
    sent: Mutex<Vec<(String, String)>>,
    fail_pause: bool,
    fail_interject: AtomicBool,
    trim_line_endings: AtomicBool,
}
impl InterjectTransport for Transport {
    fn interject(&self, session: &str, text: &str) -> Res<()> {
        self.sent
            .lock()
            .unwrap()
            .push((session.into(), text.into()));
        if self.fail_interject.swap(false, Ordering::SeqCst) {
            return Err(Failure::new("send_failed", "send failed"));
        }
        let delivered = if self.trim_line_endings.load(Ordering::SeqCst) {
            text.trim_end_matches(['\r', '\n'])
        } else {
            text
        };
        interjections::submit(&self.home, session, delivered)?
            .ok_or_else(|| Failure::new("not_found", "submission refused"))?;
        Ok(())
    }
    fn pause(&self, _: &str) -> Res<()> {
        if self.fail_pause {
            Err(Failure::new("pause_failed", "pause failed"))
        } else {
            Ok(())
        }
    }
}
fn setup(
    home: &std::path::Path,
    sessions: &[&str],
    fail_pause: bool,
) -> (Arc<Transport>, EngineInterjections) {
    for session in sessions {
        crate::goal::open(home, session, "wish", Some(home.to_str().unwrap())).unwrap();
    }
    let transport = Arc::new(Transport {
        home: home.into(),
        sent: Mutex::new(vec![]),
        fail_pause,
        fail_interject: AtomicBool::new(false),
        trim_line_endings: AtomicBool::new(false),
    });
    (
        transport.clone(),
        EngineInterjections::new(home.into(), transport),
    )
}
fn draft(session: &str, id: &str, text: &str) -> Draft {
    Draft {
        session: session.into(),
        client_id: id.into(),
        text: text.into(),
        pause_first: false,
        replan: true,
    }
}
#[test]
fn outbox_preserves_direct_receipt_and_rejected_receipt() {
    use crate::desktop::data::{interjection_outbox_for, outbox::TicketState};
    struct Recording {
        inner: Arc<EngineInterjections>,
        result: Mutex<Option<crate::desktop::ux::seams::Receipt>>,
    }
    impl Interjections for Recording {
        fn send(&self, d: &Draft) -> Res<crate::desktop::ux::seams::Receipt> {
            let receipt = self.inner.send(d)?;
            *self.result.lock().unwrap() = Some(receipt.clone());
            Ok(receipt)
        }
        fn receipt(&self, id: &str) -> Option<crate::desktop::ux::seams::Receipt> {
            self.inner.receipt(id)
        }
        fn history(&self, session: &str) -> Vec<crate::desktop::ux::seams::Amendment> {
            self.inner.history(session)
        }
        fn running_work(&self, session: &str) -> crate::desktop::ux::seams::RunningWork {
            self.inner.running_work(session)
        }
        fn pause_batch(&self, session: &str) -> Res<()> {
            self.inner.pause_batch(session)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let (transport, adapter) = setup(dir.path(), &["one"], false);
    let adapter = Arc::new(adapter);
    let d = draft("one", "client", "decline it");
    adapter.send(&d).unwrap();
    let direct = adapter.send(&d).unwrap();
    let recording = Arc::new(Recording {
        inner: adapter.clone(),
        result: Mutex::new(None),
    });
    let outbox = interjection_outbox_for(recording.clone());
    let ticket = outbox.submit(d.clone(), &d.client_id);
    let deadline = Instant::now() + Duration::from_secs(10);
    while ticket.state() == TicketState::Pending {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(ticket.state(), TicketState::Applied);
    assert_eq!(*recording.result.lock().unwrap(), Some(direct.clone()));
    assert_eq!(adapter.send(&d).unwrap(), direct);
    assert_eq!(transport.sent.lock().unwrap().len(), 1);
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::decline(dir.path(), "one", "ij-1", "no thanks").unwrap();
    let rejected = adapter.send(&d).unwrap();
    assert!(matches!(rejected.stage, ReceiptStage::Rejected { .. }));
    let second = interjection_outbox_for(recording.clone()).submit(d.clone(), &d.client_id);
    let deadline = Instant::now() + Duration::from_secs(10);
    while second.state() == TicketState::Pending {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(second.state(), TicketState::Applied);
    assert_eq!(*recording.result.lock().unwrap(), Some(rejected.clone()));
    assert_eq!(adapter.send(&d).unwrap(), rejected);
}
#[test]
fn sent_received_applied_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let (_, adapter) = setup(dir.path(), &["one"], false);
    let d = draft("one", "client", "change it");
    assert_eq!(adapter.send(&d).unwrap().stage, ReceiptStage::Sent);
    assert_eq!(
        adapter.receipt("client").unwrap().stage,
        ReceiptStage::Received
    );
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::apply(
        dir.path(),
        "one",
        "ij-1",
        Links {
            plan_revision: None,
            units: vec!["u1".into()],
            scope: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        adapter.receipt("client").unwrap().stage,
        ReceiptStage::Applied {
            first_action: "u1".into()
        }
    );
    let history = adapter.history("one");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].affected, vec!["u1"]);
    assert!(history[0].replan);
}
#[test]
fn declined_and_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let (transport, adapter) = setup(dir.path(), &["one"], false);
    let d = draft("one", "client", "decline it");
    adapter.send(&d).unwrap();
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::decline(dir.path(), "one", "ij-1", "no thanks").unwrap();
    let reconnected = EngineInterjections::new(dir.path().into(), transport.clone());
    assert_eq!(
        reconnected.send(&d).unwrap().stage,
        ReceiptStage::Rejected {
            reason: "no thanks".into()
        }
    );
    assert_eq!(transport.sent.lock().unwrap().len(), 1);
    assert_eq!(
        crate::goal::load(dir.path(), "one")
            .unwrap()
            .unwrap()
            .interjection_counter,
        1
    );
}
#[test]
fn identical_text_is_isolated_and_unclaimed_history_is_included() {
    let dir = tempfile::tempdir().unwrap();
    let (_, adapter) = setup(dir.path(), &["one", "two"], false);
    adapter.send(&draft("one", "c1", "same")).unwrap();
    adapter.send(&draft("two", "c2", "same")).unwrap();
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::decline(dir.path(), "one", "ij-1", "one only").unwrap();
    assert_eq!(
        adapter.receipt("c1").unwrap().stage,
        ReceiptStage::Rejected {
            reason: "one only".into()
        }
    );
    assert_eq!(adapter.receipt("c2").unwrap().stage, ReceiptStage::Received);
    interjections::submit(dir.path(), "two", "outside").unwrap();
    let history = adapter.history("two");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].client_id, None);
    assert_eq!(history[1].text, "outside");
    assert_eq!(adapter.history("one").len(), 1);
}
#[test]
fn pause_failure_does_not_send_or_persist() {
    let dir = tempfile::tempdir().unwrap();
    let (transport, adapter) = setup(dir.path(), &["one"], true);
    let mut d = draft("one", "client", "text");
    d.pause_first = true;
    assert_eq!(adapter.send(&d).unwrap_err().kind, "pause_failed");
    assert!(transport.sent.lock().unwrap().is_empty());
    assert_eq!(adapter.receipt("client"), None);
    assert!(!dir.path().join("desktop").exists());
}

#[test]
fn failed_send_is_not_persisted_and_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let (transport, adapter) = setup(dir.path(), &["one"], false);
    transport.fail_interject.store(true, Ordering::SeqCst);
    let d = draft("one", "client", "text");
    assert_eq!(adapter.send(&d).unwrap_err().kind, "send_failed");
    assert_eq!(adapter.receipt("client"), None);
    assert!(adapter.history("one").is_empty());
    assert!(!dir.path().join("desktop/interjections/one.json").exists());
    assert_eq!(adapter.send(&d).unwrap().stage, ReceiptStage::Sent);
    assert_eq!(transport.sent.lock().unwrap().len(), 2);
    assert_eq!(
        adapter.receipt("client").unwrap().stage,
        ReceiptStage::Received
    );
    assert_eq!(
        crate::goal::load(dir.path(), "one")
            .unwrap()
            .unwrap()
            .interjection_counter,
        1
    );
}

#[test]
fn earlier_external_identical_text_is_not_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let (_, adapter) = setup(dir.path(), &["one"], false);
    interjections::submit(dir.path(), "one", "same").unwrap();
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::decline(dir.path(), "one", "ij-1", "external").unwrap();
    adapter.send(&draft("one", "client", "same")).unwrap();
    assert_eq!(
        adapter.receipt("client").unwrap().stage,
        ReceiptStage::Received
    );
    let history = adapter.history("one");
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].client_id, None);
    assert_eq!(
        history[1].stage,
        Some(ReceiptStage::Rejected {
            reason: "external".into()
        })
    );
}

#[test]
fn receipt_matches_hosted_transport_line_ending_trimming() {
    let dir = tempfile::tempdir().unwrap();
    let (transport, adapter) = setup(dir.path(), &["one"], false);
    transport.trim_line_endings.store(true, Ordering::SeqCst);
    adapter.send(&draft("one", "client", "text\r\n")).unwrap();
    assert_eq!(
        adapter.receipt("client").unwrap().stage,
        ReceiptStage::Received
    );
}

#[test]
fn running_work_and_unclaimed_events_without_prior_send() {
    let dir = tempfile::tempdir().unwrap();
    let (_, adapter) = setup(dir.path(), &["one"], false);
    interjections::submit(dir.path(), "one", "external").unwrap();
    let history = adapter.history("one");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].client_id, None);
    assert_eq!(history[0].text, "external");
    let store = super::session_store(dir.path(), "one").unwrap();
    std::fs::write(store.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tt\tactive\t\t\t\t\nu2\tt\tverified\t\t\t\t\nu3\tt\tsuperseded:u1\t\t\t\t\n").unwrap();
    let batch = crate::goal::Batch {
        progress_file: dir.path().join("progress").to_string_lossy().into_owned(),
        marker_file: dir.path().join("marker").to_string_lossy().into_owned(),
        pid: 1,
    };
    crate::goal::set_status_with_batch(
        dir.path(),
        "one",
        "waiting",
        Some("waiting for batch"),
        Some(batch),
    )
    .unwrap();
    let work = adapter.running_work("one");
    assert_eq!(work.stage.as_deref(), Some("waiting"));
    assert_eq!(work.units, vec!["u1"]);
    assert_eq!(work.batch.as_deref(), dir.path().join("progress").to_str());
}

#[test]
fn same_digest_claimed_once_per_entry() {
    let dir = tempfile::tempdir().unwrap();
    let (_, adapter) = setup(dir.path(), &["one"], false);
    adapter.send(&draft("one", "c1", "same")).unwrap();
    adapter.send(&draft("one", "c2", "same")).unwrap();
    assert_eq!(adapter.receipt("c2").unwrap().stage, ReceiptStage::Received);
    interjections::classify(dir.path(), "one", "amend").unwrap();
    interjections::apply(
        dir.path(),
        "one",
        "ij-2",
        Links {
            plan_revision: None,
            units: vec![],
            scope: vec![],
        },
    )
    .unwrap();
    assert_eq!(
        adapter.receipt("c2").unwrap().stage,
        ReceiptStage::Applied {
            first_action: "replan".into()
        }
    );
    assert_eq!(adapter.receipt("c1").unwrap().stage, ReceiptStage::Received);
}
