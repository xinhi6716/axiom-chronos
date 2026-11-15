use super::super::{
    i18n::Lang,
    ux::seams::{
        Connection, Controller, FakeSessions, Frame, SessionKind, SessionStream, WorkerSession,
    },
};
use super::sessions::{
    SessionModel, SessionSnapshot, SessionSource, BACK_ID, HANDBACK_ID, MAX_FRAMES, PANEL_ID,
    RECONNECT_ID, SEND_ID, TAKEOVER_ID,
};
use crate::base::Res;
use crate::desktop::data::{feed::FeedCore, Cursor, Source};
use std::{
    prelude::v1::test,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

fn fake() -> Arc<FakeSessions> {
    Arc::new(FakeSessions::new(vec![WorkerSession {
        id: "terminal".into(),
        run: "run".into(),
        unit: None,
        title: "worker".into(),
        kind: SessionKind::Terminal,
    }]))
}

#[test]
fn takeover_confirmation_handback_and_duplicate_request() {
    let service = fake();
    let mut model = SessionModel::new(service.clone(), "terminal");
    assert!(model.poll());
    assert!(model.request_takeover());
    let ticket = model.mine.clone().unwrap();
    assert!(!model.request_takeover());
    assert_eq!(
        service.controller("terminal"),
        Controller::Requested {
            ticket: ticket.clone()
        }
    );
    assert!(!model.send("before"));
    assert!(service.inputs("terminal").is_empty());
    service.confirm("terminal");
    assert!(model.poll());
    assert!(!model.request_takeover());
    assert_eq!(service.controller("terminal"), Controller::You { ticket });
    assert!(model.send("after"));
    assert_eq!(service.inputs("terminal"), vec!["after"]);
    assert!(model.hand_back());
    assert!(!model.send("handed back"));
    assert_eq!(service.inputs("terminal"), vec!["after"]);
}

#[test]
fn bilingual_statuses_and_refusal() {
    let service = fake();
    let mut model = SessionModel::new(service.clone(), "terminal");
    for (lang, connecting, live, lost, closed, worker, waiting, you, refused) in [
        (
            Lang::En,
            "Connecting",
            "Live",
            "Connection lost",
            "Closed",
            "The worker has control",
            "Waiting for the service to confirm",
            "You have control",
            "Refused: busy",
        ),
        (
            Lang::ZhTw,
            "連線中",
            "即時",
            "連線中斷",
            "已結束",
            "由 worker 控制",
            "等待服務確認",
            "由你控制",
            "遭拒：busy",
        ),
    ] {
        model.connection = Connection::Connecting;
        assert_eq!(model.connection_text(lang), connecting);
        model.connection = Connection::Live;
        assert_eq!(model.connection_text(lang), live);
        model.connection = Connection::Lost;
        assert_eq!(model.connection_text(lang), lost);
        model.connection = Connection::Closed;
        assert_eq!(model.connection_text(lang), closed);
        model.controller = Controller::Worker;
        assert_eq!(model.controller_text(lang), worker);
        model.controller = Controller::Requested { ticket: "t".into() };
        assert_eq!(model.controller_text(lang), waiting);
        model.controller = Controller::You { ticket: "t".into() };
        assert_eq!(model.controller_text(lang), you);
        model.controller = Controller::Refused {
            reason: "busy".into(),
        };
        assert_eq!(model.controller_text(lang), refused);
    }
    assert!(model.poll());
    assert!(model.request_takeover());
    service.refuse("terminal", "busy");
    assert!(model.poll());
    assert_eq!(model.controller_text(Lang::ZhTw), "遭拒：busy");
    assert!(!model.send("no"));
    assert!(service.inputs("terminal").is_empty());
}

struct CountingSessions {
    fake: Arc<FakeSessions>,
    frame_calls: AtomicUsize,
    cursors: Mutex<Vec<u64>>,
}
impl SessionStream for CountingSessions {
    fn sessions(&self) -> Vec<WorkerSession> {
        self.fake.sessions()
    }
    fn frames(&self, id: &str, after: u64) -> Res<Vec<Frame>> {
        self.frame_calls.fetch_add(1, Ordering::SeqCst);
        self.cursors.lock().unwrap().push(after);
        self.fake.frames(id, after)
    }
    fn connection(&self, id: &str) -> Connection {
        self.fake.connection(id)
    }
    fn controller(&self, id: &str) -> Controller {
        self.fake.controller(id)
    }
    fn request_takeover(&self, id: &str) -> Res<String> {
        self.fake.request_takeover(id)
    }
    fn send_input(&self, id: &str, ticket: &str, text: &str) -> Res<()> {
        self.fake.send_input(id, ticket, text)
    }
    fn hand_back(&self, id: &str, ticket: &str) -> Res<()> {
        self.fake.hand_back(id, ticket)
    }
    fn reconnect(&self, id: &str) -> Res<()> {
        self.fake.reconnect(id)
    }
}
#[test]
fn lost_and_closed_stop_frame_polling_and_reconnect_resumes() {
    let fake = fake();
    let service = Arc::new(CountingSessions {
        fake: fake.clone(),
        frame_calls: AtomicUsize::new(0),
        cursors: Mutex::new(Vec::new()),
    });
    let mut model = SessionModel::new(service.clone(), "terminal");
    fake.push("terminal", "first");
    assert!(model.poll());
    assert_eq!(
        model
            .frames
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["first"]
    );
    fake.disconnect("terminal");
    fake.push("terminal", "second");
    assert!(!model.poll());
    assert!(!model.poll());
    assert_eq!(service.frame_calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.frames.len(), 1);
    assert!(model.reconnect());
    assert!(model.poll());
    assert_eq!(service.frame_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        model
            .frames
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    let mut closed = SessionModel::new(service.clone(), "missing");
    assert!(!closed.poll());
    assert!(!closed.poll());
    assert_eq!(closed.connection, Connection::Closed);
    assert_eq!(service.frame_calls.load(Ordering::SeqCst), 2);
}

fn poll_feed(source: &mut SessionSource, core: &FeedCore<SessionSnapshot>) {
    let generation = core.begin_poll();
    let result = source.poll(core.cursor());
    assert!(core.apply(generation, result, 0, None));
}

#[test]
fn feed_uses_incremental_cursor_without_duplicates() {
    let fake = fake();
    let service = Arc::new(CountingSessions {
        fake: fake.clone(),
        frame_calls: AtomicUsize::new(0),
        cursors: Mutex::new(Vec::new()),
    });
    let mut source = SessionSource::new(service.clone(), "terminal");
    let core = FeedCore::new(true, None);
    fake.push("terminal", "one");
    poll_feed(&mut source, &core);
    fake.push("terminal", "two");
    poll_feed(&mut source, &core);
    poll_feed(&mut source, &core);
    assert_eq!(*service.cursors.lock().unwrap(), vec![0, 1, 2]);
    assert_eq!(core.cursor(), Cursor(2));
    assert_eq!(
        core.snapshot()
            .value
            .unwrap()
            .frames
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
}

#[test]
fn feed_bounds_transcript_and_keeps_cursor() {
    let fake = fake();
    let mut source = SessionSource::new(fake.clone(), "terminal");
    let core = FeedCore::new(true, None);
    for n in 0..MAX_FRAMES {
        fake.push("terminal", &n.to_string());
    }
    poll_feed(&mut source, &core);
    for n in MAX_FRAMES..MAX_FRAMES + 5 {
        fake.push("terminal", &n.to_string());
    }
    poll_feed(&mut source, &core);
    let snapshot = core.snapshot().value.unwrap();
    assert_eq!(snapshot.frames.len(), MAX_FRAMES);
    assert_eq!(snapshot.frames.first().unwrap().text, "5");
    assert_eq!(
        snapshot.frames.last().unwrap().text,
        (MAX_FRAMES + 4).to_string()
    );
    assert_eq!(core.cursor(), Cursor((MAX_FRAMES + 5) as u64));
}

#[test]
fn target_ids_are_rendered_from_constants() {
    let target = crate::desktop::targets::find("session").unwrap();
    assert_eq!(target.panels, &[PANEL_ID]);
    assert_eq!(
        target.actions,
        &[BACK_ID, SEND_ID, TAKEOVER_ID, HANDBACK_ID, RECONNECT_ID]
    );
    let source = include_str!("sessions.rs");
    let render = source
        .split("impl Render for SessionPage")
        .nth(1)
        .unwrap()
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert!(render.contains("Panel::new(PANEL_ID, &snapshot.state)"));
    assert!(render.contains(".id(\"session-content\")"));
    assert!(!render.contains(".id(PANEL_ID)"));
    for id in [
        "BACK_ID",
        "SEND_ID",
        "TAKEOVER_ID",
        "HANDBACK_ID",
        "RECONNECT_ID",
    ] {
        assert!(render.contains(id));
    }
    for id in target.panels.iter().chain(target.actions) {
        assert!(!render.contains(id));
    }
}

#[test]
fn initial_fault_recovers_full_transcript_from_unadvanced_cursor() {
    use crate::desktop::components::panel::PanelState;
    use crate::desktop::data::faults::Fault;

    let service = fake();
    service.push("terminal", "first");
    let mut source = SessionSource::new(service, "terminal");
    let core = FeedCore::new(true, None);
    let generation = core.begin_poll();
    assert!(core.apply(
        generation,
        source.poll(core.cursor()),
        0,
        Some(Fault::Corrupt)
    ));
    assert!(matches!(core.snapshot().state, PanelState::Corrupt { .. }));
    assert_eq!(core.cursor(), Cursor(0));
    poll_feed(&mut source, &core);
    assert_eq!(core.snapshot().state, PanelState::Ready);
    assert_eq!(core.snapshot().value.unwrap().frames[0].text, "first");
}
