use crate::{
    base::Res,
    desktop::{
        components::{
            panel::{Panel, PanelState},
            tokens::Tokened,
            Button, TextField,
        },
        data::{self, Cursor, Delta, Source},
        i18n::{self, Lang, Table},
        model::AppState,
        motion::Dur,
        theme::{self, Step},
        ux::seams::{Connection, Controller, Frame, SessionStream},
    },
};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        h_flex,
        input::{InputEvent, InputState},
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::sync::{Arc, Mutex};

pub(super) const PANEL_ID: &str = "session.page";
pub(super) const BACK_ID: &str = "action:session.back";
pub(super) const SEND_ID: &str = "action:session.send";
pub(super) const TAKEOVER_ID: &str = "action:session.takeover";
pub(super) const HANDBACK_ID: &str = "action:session.handback";
pub(super) const RECONNECT_ID: &str = "action:session.reconnect";
pub(super) const MAX_FRAMES: usize = 2000;

#[derive(Clone)]
pub(super) struct SessionSnapshot {
    pub(super) connection: Connection,
    pub(super) controller: Controller,
    pub(super) frames: Vec<Frame>,
}
pub(super) struct SessionSource {
    service: Option<Arc<dyn SessionStream>>,
    id: String,
    previous: Option<(Connection, Controller)>,
    status: Arc<Mutex<Option<(Connection, Controller)>>>,
}
impl SessionSource {
    pub(super) fn new(service: Arc<dyn SessionStream>, id: impl Into<String>) -> Self {
        Self {
            service: Some(service),
            id: id.into(),
            previous: None,
            status: Arc::new(Mutex::new(None)),
        }
    }
    fn without_service(id: impl Into<String>) -> Self {
        Self {
            service: None,
            id: id.into(),
            previous: None,
            status: Arc::new(Mutex::new(None)),
        }
    }
}
impl Source<SessionSnapshot> for SessionSource {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<SessionSnapshot>, Cursor)> {
        let Some(service) = &self.service else {
            return Ok((
                Delta::Replace(SessionSnapshot {
                    connection: Connection::Closed,
                    controller: Controller::Worker,
                    frames: Vec::new(),
                }),
                cursor,
            ));
        };
        let connection = service.connection(&self.id);
        let controller = service.controller(&self.id);
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((connection, controller.clone()));
        let frames = if connection == Connection::Live
            || (connection == Connection::Closed
                && service.sessions().iter().any(|s| s.id == self.id))
        {
            service.frames(&self.id, cursor.0)?
        } else {
            Vec::new()
        };
        let next = Cursor(frames.last().map_or(cursor.0, |f| f.seq));
        let changed = self.previous.as_ref() != Some(&(connection, controller.clone()));
        // A fault can discard the first poll without advancing the feed cursor.
        let first = self.previous.is_none() || cursor.0 == 0;
        self.previous = Some((connection, controller.clone()));
        if first {
            let mut snapshot = SessionSnapshot {
                connection,
                controller,
                frames,
            };
            let excess = snapshot.frames.len().saturating_sub(MAX_FRAMES);
            snapshot.frames.drain(..excess);
            Ok((Delta::Replace(snapshot), next))
        } else if !frames.is_empty() || changed {
            Ok((
                Delta::Patch(Box::new(move |snapshot: &mut SessionSnapshot| {
                    snapshot.connection = connection;
                    snapshot.controller = controller;
                    snapshot.frames.extend(frames);
                    let excess = snapshot.frames.len().saturating_sub(MAX_FRAMES);
                    snapshot.frames.drain(..excess);
                })),
                next,
            ))
        } else {
            Ok((Delta::Unchanged, next))
        }
    }
}
fn feed_error(state: &PanelState) -> Option<String> {
    match state {
        PanelState::Failed { reason }
        | PanelState::Offline { reason }
        | PanelState::Corrupt { reason } => Some(reason.clone()),
        _ => None,
    }
}

const TABLE: Table = &[
    ("Back", "返回"),
    ("Connecting", "連線中"),
    ("Live", "即時"),
    ("Connection lost", "連線中斷"),
    ("Closed", "已結束"),
    ("The worker has control", "由 worker 控制"),
    ("Waiting for the service to confirm", "等待服務確認"),
    ("You have control", "由你控制"),
    ("Refused: {}", "遭拒：{}"),
    ("Take over", "接手"),
    ("Hand back", "交還控制"),
    ("Reconnect", "重新連線"),
    ("Send", "送出"),
];
pub fn input_enabled(c: &Controller, mine: Option<&str>) -> bool {
    matches!(c, Controller::You { ticket } if mine == Some(ticket.as_str()))
}
fn marked(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
        .into_any_element()
}
fn action_button(
    id: &'static str,
    label: &'static str,
    tokens: &theme::Tokens,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if enabled {
        Button::new(id, label, tokens)
            .on_click(on_click)
            .into_any_element()
    } else {
        div()
            .id(id)
            .test_support()
            .text_color(tokens.muted)
            .child(label)
            .into_any_element()
    }
}
#[derive(Clone)]
pub(super) struct SessionModel {
    service: Arc<dyn SessionStream>,
    id: String,
    pub(super) connection: Connection,
    pub(super) controller: Controller,
    pub(super) mine: Option<String>,
    pub(super) frames: Vec<Frame>,
    error: Option<String>,
}
impl SessionModel {
    pub(super) fn new(service: Arc<dyn SessionStream>, id: impl Into<String>) -> Self {
        Self {
            service,
            id: id.into(),
            connection: Connection::Connecting,
            controller: Controller::Worker,
            mine: None,
            frames: Vec::new(),
            error: None,
        }
    }
    #[cfg(test)]
    pub(super) fn poll(&mut self) -> bool {
        self.connection = self.service.connection(&self.id);
        self.controller = self.service.controller(&self.id);
        if self.connection == Connection::Live
            || (self.connection == Connection::Closed
                && self
                    .service
                    .sessions()
                    .iter()
                    .any(|session| session.id == self.id))
        {
            match self
                .service
                .frames(&self.id, self.frames.last().map_or(0, |f| f.seq))
            {
                Ok(frames) => {
                    self.frames.extend(frames);
                    let excess = self.frames.len().saturating_sub(MAX_FRAMES);
                    self.frames.drain(..excess);
                }
                Err(e) => self.error = Some(e.message),
            }
        }
        matches!(self.connection, Connection::Connecting | Connection::Live)
    }
    pub(super) fn takeover_enabled(&self) -> bool {
        self.connection == Connection::Live && self.controller == Controller::Worker
    }
    pub(super) fn request_takeover(&mut self) -> bool {
        if !self.takeover_enabled() {
            return false;
        }
        match self.service.request_takeover(&self.id) {
            Ok(ticket) => {
                self.mine = Some(ticket.clone());
                self.controller = Controller::Requested { ticket };
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.message);
                false
            }
        }
    }
    fn input_enabled(&self) -> bool {
        self.connection == Connection::Live && input_enabled(&self.controller, self.mine.as_deref())
    }
    pub(super) fn send(&mut self, text: &str) -> bool {
        if !self.input_enabled() || text.is_empty() {
            return false;
        }
        match self
            .service
            .send_input(&self.id, self.mine.as_deref().unwrap(), text)
        {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.message);
                false
            }
        }
    }
    pub(super) fn hand_back(&mut self) -> bool {
        if !self.input_enabled() {
            return false;
        }
        match self
            .service
            .hand_back(&self.id, self.mine.as_deref().unwrap())
        {
            Ok(()) => {
                self.mine = None;
                self.controller = Controller::Worker;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.message);
                false
            }
        }
    }
    pub(super) fn reconnect(&mut self) -> bool {
        match self.service.reconnect(&self.id) {
            Ok(()) => {
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.message);
                false
            }
        }
    }
    pub(super) fn connection_text(&self, lang: Lang) -> &'static str {
        i18n::translate(
            lang,
            TABLE,
            match self.connection {
                Connection::Connecting => "Connecting",
                Connection::Live => "Live",
                Connection::Lost => "Connection lost",
                Connection::Closed => "Closed",
            },
        )
    }
    pub(super) fn controller_text(&self, lang: Lang) -> String {
        let tr = |key| i18n::translate(lang, TABLE, key);
        match &self.controller {
            Controller::Worker => tr("The worker has control").to_owned(),
            Controller::Requested { .. } => tr("Waiting for the service to confirm").to_owned(),
            Controller::You { .. } => tr("You have control").to_owned(),
            Controller::Refused { reason } => i18n::fill(tr("Refused: {}"), &[reason]),
        }
    }
}
pub struct SessionPage {
    state: Entity<AppState>,
    model: Option<SessionModel>,
    input: Entity<InputState>,
    busy: bool,
    panic_pending: bool,
    feed: data::Feed<SessionSnapshot>,
    status: Arc<Mutex<Option<(Connection, Controller)>>>,
}
impl SessionPage {
    pub fn new(
        state: Entity<AppState>,
        id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let model = state
            .read(cx)
            .ux
            .seams
            .sessions
            .clone()
            .map(|service| SessionModel::new(service, id.to_string()));
        let source = model
            .as_ref()
            .map(|model| SessionSource::new(model.service.clone(), model.id.clone()))
            .unwrap_or_else(|| SessionSource::without_service(id.to_string()));
        let status = source.status.clone();
        let feed = data::Feed::spawn(PANEL_ID, source, Dur::Gentle.get(), cx);
        feed.observe(cx);
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
            cx.notify();
        })
        .detach();
        Self {
            state,
            model,
            input,
            busy: false,
            panic_pending: false,
            feed,
            status,
        }
    }
    fn action(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut SessionModel) + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        let Some(mut model) = self.model.clone() else {
            return;
        };
        self.busy = true;
        if let Some((connection, controller)) = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            model.connection = connection;
            model.controller = controller;
        }
        let prior_controller = model.controller.clone();
        let task = cx.background_spawn(async move {
            action(&mut model);
            (model, prior_controller)
        });
        cx.spawn(async move |this, cx| {
            let (model, prior_controller) = task.await;
            this.update(cx, |this, cx| {
                {
                    let mut status = this.status.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some((_, controller)) = status.as_mut() {
                        if *controller == prior_controller {
                            *controller = model.controller.clone();
                        }
                    }
                }
                this.model = Some(model);
                this.busy = false;
                this.feed.retry();
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
    fn current_model(&self) -> Option<SessionModel> {
        let mut model = self.model.clone()?;
        if let Some(snapshot) = self.feed.snapshot().value {
            model.frames = snapshot.frames.clone();
        }
        if let Some((connection, controller)) = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            model.connection = connection;
            model.controller = controller;
        }
        Some(model)
    }
    fn takeover(&mut self, cx: &mut Context<Self>) {
        self.action(cx, |m| {
            m.request_takeover();
        });
    }
    fn handback(&mut self, cx: &mut Context<Self>) {
        self.action(cx, |m| {
            m.hand_back();
        });
    }
    fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.action(cx, |m| {
            m.reconnect();
        });
    }
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.current_model().is_some_and(|m| m.input_enabled()) {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        if text.is_empty() {
            return;
        }
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.action(cx, move |m| {
            m.send(&text);
        });
    }
}
impl Render for SessionPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let tr = |key| i18n::translate(lang, TABLE, key);
        let d = theme::density(cx);
        let tokens = cx.tokens();
        let model = self.current_model();
        let mut snapshot = self.feed.snapshot();
        let feed_error = feed_error(&snapshot.state);
        let connection = model
            .as_ref()
            .map_or(tr("Closed"), |m| m.connection_text(lang));
        let controller = model.as_ref().map_or_else(
            || tr("The worker has control").to_owned(),
            |m| m.controller_text(lang),
        );
        let enabled = !self.busy && model.as_ref().is_some_and(SessionModel::input_enabled);
        let state = self.state.clone();
        let stream = model
            .as_ref()
            .map(|m| {
                m.frames
                    .iter()
                    .map(|f| f.text.as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let panic_active = data::faults::active(PANEL_ID, cx) == Some(data::faults::Fault::Panic);
        if panic_active && snapshot.state == PanelState::Ready {
            self.panic_pending = true;
        }
        // Keep a caught render panic visible until its retry is clicked.
        if self.panic_pending && !panic_active {
            snapshot.state = PanelState::Failed {
                reason: "panel rendering failed".into(),
            };
        }
        let panel = Panel::new(PANEL_ID, &snapshot.state)
            .retry(cx.listener(|this, _, _, cx| {
                this.panic_pending = false;
                this.feed.retry();
                cx.notify();
            }))
            .content(move |_, _| {
                div()
                    .id(PANEL_ID)
                    .test_support()
                    .size_full()
                    .child(
                        div()
                            .size_full()
                            .overflow_y_scrollbar()
                            .child(marked("session-stream", stream)),
                    )
                    .into_any_element()
            });
        v_flex().size_full().child(
            v_flex()
                .id("session-page")
                .test_support()
                .size_full()
                .gap(theme::space(d, Step::Sm))
                .p(theme::space(d, Step::Lg))
                .child(
                    Button::new(BACK_ID, tr("Back"), &tokens).on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| state.close_page(cx))
                    }),
                )
                .child(marked("session-connection", connection.to_owned()))
                .child(marked("session-controller", controller))
                .child(div().flex_1().min_h_0().child(panel))
                .child(
                    h_flex()
                        .gap(theme::space(d, Step::Sm))
                        .child(action_button(
                            TAKEOVER_ID,
                            tr("Take over"),
                            &tokens,
                            !self.busy
                                && model.as_ref().is_some_and(SessionModel::takeover_enabled),
                            cx.listener(|this, _, _, cx| this.takeover(cx)),
                        ))
                        .child(action_button(
                            HANDBACK_ID,
                            tr("Hand back"),
                            &tokens,
                            enabled,
                            cx.listener(|this, _, _, cx| this.handback(cx)),
                        ))
                        .child(action_button(
                            RECONNECT_ID,
                            tr("Reconnect"),
                            &tokens,
                            !self.busy
                                && model
                                    .as_ref()
                                    .is_some_and(|m| m.connection == Connection::Lost),
                            cx.listener(|this, _, _, cx| this.reconnect(cx)),
                        )),
                )
                .child(if enabled {
                    TextField::new("session-input", &self.input, &tokens).into_any_element()
                } else {
                    div()
                        .id("session-input")
                        .text_color(tokens.muted)
                        .child("")
                        .into_any_element()
                })
                .child(action_button(
                    SEND_ID,
                    tr("Send"),
                    &tokens,
                    enabled,
                    cx.listener(|this, _, window, cx| this.send(window, cx)),
                ))
                .when_some(
                    feed_error.or_else(|| model.as_ref().and_then(|m| m.error.clone())),
                    |this, error| this.child(marked("session-error", error)),
                ),
        )
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;
    use crate::desktop::ux::seams::{FakeSessions, SessionKind, WorkerSession};
    use std::prelude::v1::test;
    struct GapStream;
    impl SessionStream for GapStream {
        fn sessions(&self) -> Vec<WorkerSession> {
            vec![]
        }
        fn frames(&self, _: &str, _: u64) -> crate::base::Res<Vec<Frame>> {
            Err(crate::base::Failure::new("gap", "some frames are missing"))
        }
        fn connection(&self, _: &str) -> Connection {
            Connection::Live
        }
        fn controller(&self, _: &str) -> Controller {
            Controller::Worker
        }
        fn request_takeover(&self, _: &str) -> crate::base::Res<String> {
            unreachable!()
        }
        fn send_input(&self, _: &str, _: &str, _: &str) -> crate::base::Res<()> {
            unreachable!()
        }
        fn hand_back(&self, _: &str, _: &str) -> crate::base::Res<()> {
            unreachable!()
        }
        fn reconnect(&self, _: &str) -> crate::base::Res<()> {
            unreachable!()
        }
    }
    #[test]
    fn frame_gap_reason_is_shown() {
        let mut source = SessionSource::new(Arc::new(GapStream), "s");
        let core = data::feed::FeedCore::new(true, None);
        let loading = core.snapshot();
        assert_eq!(loading.state, PanelState::Loading);
        assert_eq!(
            crate::desktop::components::panel::copy(Lang::En, &loading.state)
                .unwrap()
                .0,
            "Loading"
        );

        let generation = core.begin_poll();
        assert!(core.apply(
            generation,
            Ok((
                Delta::Replace(SessionSnapshot {
                    connection: Connection::Live,
                    controller: Controller::Worker,
                    frames: Vec::new(),
                }),
                Cursor(0)
            )),
            0,
            None
        ));
        let ready = core.snapshot();
        assert_eq!(ready.state, PanelState::Ready);
        assert_eq!(
            crate::desktop::components::panel::copy(Lang::En, &ready.state),
            None
        );

        let generation = core.begin_poll();
        assert!(core.apply(generation, source.poll(core.cursor()), 0, None));
        let failed = core.snapshot();
        assert_eq!(
            failed.state,
            PanelState::Failed {
                reason: "some frames are missing".into()
            }
        );
        assert_eq!(
            crate::desktop::components::panel::copy(Lang::En, &failed.state)
                .unwrap()
                .0,
            "Unable to load"
        );
        assert_eq!(
            feed_error(&failed.state).as_deref(),
            Some("some frames are missing")
        );
    }
    #[test]
    fn absent_service_produces_a_closed_ready_snapshot() {
        let mut source = SessionSource::without_service("missing");
        let core = data::feed::FeedCore::new(true, None);
        let generation = core.begin_poll();
        assert!(core.apply(generation, source.poll(core.cursor()), 0, None));
        let snapshot = core.snapshot();
        assert_eq!(snapshot.state, PanelState::Ready);
        let value = snapshot.value.unwrap();
        assert_eq!(value.connection, Connection::Closed);
        assert_eq!(value.controller, Controller::Worker);
        assert!(value.frames.is_empty());
    }
    #[test]
    fn refusal_disables_takeover_and_input() {
        let fake = Arc::new(FakeSessions::new(vec![WorkerSession {
            id: "s".into(),
            run: "r".into(),
            unit: None,
            title: "s".into(),
            kind: SessionKind::Terminal,
        }]));
        fake.refuse("s", "not supported");
        let mut model = SessionModel::new(fake.clone(), "s");
        assert!(model.poll());
        assert_eq!(model.controller_text(Lang::En), "Refused: not supported");
        assert!(!model.takeover_enabled());
        assert!(!model.input_enabled());
        assert!(!model.request_takeover());
        assert!(!model.send("input"));
        assert!(!model.hand_back());
        assert!(fake.inputs("s").is_empty());
    }
}
