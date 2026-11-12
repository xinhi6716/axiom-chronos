use super::super::{
    i18n::{self, Lang, Table},
    theme::{self, Step},
    ux::seams::{Connection, Controller, Frame, SessionStream},
};
use super::AppState;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        button::Button,
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex, Disableable as _, Sizable as _,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{sync::Arc, time::Duration};
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
    pub(super) fn poll(&mut self) -> bool {
        self.connection = self.service.connection(&self.id);
        self.controller = self.service.controller(&self.id);
        if self.connection == Connection::Live {
            match self
                .service
                .frames(&self.id, self.frames.last().map_or(0, |f| f.seq))
            {
                Ok(frames) => self.frames.extend(frames),
                Err(e) => self.error = Some(e.message),
            }
            true
        } else {
            false
        }
    }
    pub(super) fn request_takeover(&mut self) -> bool {
        if self.connection != Connection::Live
            || matches!(
                self.controller,
                Controller::Requested { .. } | Controller::You { .. }
            )
        {
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
    timer: Option<Task<()>>,
    generation: u64,
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
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
            cx.notify();
        })
        .detach();
        let mut page = Self {
            state,
            model,
            input,
            busy: false,
            timer: None,
            generation: 0,
        };
        page.refresh(cx);
        page
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(mut model) = self.model.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let running = model.poll();
            (model, running)
        });
        cx.spawn(async move |this, cx| {
            let (model, running) = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.model = Some(model);
                this.timer = None;
                if running {
                    this.schedule(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn schedule(&mut self, cx: &mut Context<Self>) {
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            this.update(cx, |this, cx| {
                if !this.busy
                    && this
                        .model
                        .as_ref()
                        .is_some_and(|m| m.connection == Connection::Live)
                {
                    this.refresh(cx);
                }
            })
            .ok();
        }));
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
        self.generation += 1;
        self.timer = None;
        let task = cx.background_spawn(async move {
            action(&mut model);
            model
        });
        cx.spawn(async move |this, cx| {
            let model = task.await;
            this.update(cx, |this, cx| {
                this.model = Some(model);
                this.busy = false;
                this.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
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
        if self.busy || !self.model.as_ref().is_some_and(SessionModel::input_enabled) {
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
        let connection = self
            .model
            .as_ref()
            .map_or(tr("Closed"), |m| m.connection_text(lang));
        let controller = self.model.as_ref().map_or_else(
            || tr("The worker has control").to_owned(),
            |m| m.controller_text(lang),
        );
        let enabled = !self.busy && self.model.as_ref().is_some_and(SessionModel::input_enabled);
        let state = self.state.clone();
        let stream = self
            .model
            .as_ref()
            .map(|m| {
                m.frames
                    .iter()
                    .map(|f| f.text.as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        v_flex()
            .id("session-page")
            .test_support()
            .size_full()
            .gap(theme::space(d, Step::Sm))
            .p(theme::space(d, Step::Lg))
            .child(
                Button::new("page-back")
                    .small()
                    .tooltip(tr("Back"))
                    .child(tr("Back").test_text(tr("Back")))
                    .on_click(move |_, _, cx| state.update(cx, |state, cx| state.close_page(cx))),
            )
            .child(marked("session-connection", connection.to_owned()))
            .child(marked("session-controller", controller))
            .child(
                div()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(marked("session-stream", stream)),
            )
            .child(
                h_flex()
                    .gap(theme::space(d, Step::Sm))
                    .child(
                        Button::new("session-takeover")
                            .small()
                            .tooltip(tr("Take over"))
                            .child(tr("Take over").test_text(tr("Take over")))
                            .disabled(
                                self.busy
                                    || self.model.as_ref().is_none_or(|m| {
                                        m.connection != Connection::Live
                                            || matches!(
                                                m.controller,
                                                Controller::Requested { .. }
                                                    | Controller::You { .. }
                                            )
                                    }),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.takeover(cx))),
                    )
                    .child(
                        Button::new("session-handback")
                            .small()
                            .tooltip(tr("Hand back"))
                            .child(tr("Hand back").test_text(tr("Hand back")))
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.handback(cx))),
                    )
                    .child(
                        Button::new("session-reconnect")
                            .small()
                            .tooltip(tr("Reconnect"))
                            .child(tr("Reconnect").test_text(tr("Reconnect")))
                            .disabled(
                                self.busy
                                    || self
                                        .model
                                        .as_ref()
                                        .is_none_or(|m| m.connection != Connection::Lost),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.reconnect(cx))),
                    ),
            )
            .child(
                Input::new(&self.input)
                    .id("session-input")
                    .small()
                    .disabled(!enabled),
            )
            .child(
                Button::new("session-send")
                    .small()
                    .tooltip(tr("Send"))
                    .child(tr("Send").test_text(tr("Send")))
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            )
            .when_some(
                self.model.as_ref().and_then(|m| m.error.clone()),
                |this, error| this.child(marked("session-error", error)),
            )
    }
}
