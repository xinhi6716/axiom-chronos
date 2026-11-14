use super::*;
use crate::desktop::components::Toggle;
use crate::desktop::{
    data::outbox::{Outbox, Ticket, TicketState},
    model::facade::settings::{Store, UxPrefs},
    ux::seams::{Amendment, Draft, Interjections, Receipt, ReceiptStage},
};
use gpui_kit::base::TestTextExt as _;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

const T: Table = &[
    ("Interjection", "插話"),
    ("Preview", "預覽"),
    ("Send", "送出"),
    ("Running now: {work}", "目前正在進行：{work}"),
    ("Pause the batch first", "先暫停批次"),
    ("Re-plan with Astra + council", "請 Astra 與議會重新規劃"),
    ("Sent", "已送出"),
    ("Received by the core", "核心已收到"),
    ("Applied: {value}", "已處理：{value}"),
    ("Rejected: {value}", "未採用：{value}"),
    ("Pause failed: {value}", "暫停失敗：{value}"),
    ("Send failed: {value}", "送出失敗：{value}"),
    ("Amendment history", "修正紀錄"),
    ("Stage", "階段"),
    ("Units", "單元"),
    ("Batch", "批次"),
    ("Affected", "影響項目"),
];

pub fn rank(s: &ReceiptStage) -> u8 {
    match s {
        ReceiptStage::Sent => 0,
        ReceiptStage::Received => 1,
        _ => 2,
    }
}
pub fn merge(old: Option<&Receipt>, new: &Receipt) -> Receipt {
    match old {
        Some(old) if rank(&old.stage) == 2 || rank(&old.stage) > rank(&new.stage) => old.clone(),
        _ => new.clone(),
    }
}
pub fn stage_text(s: &ReceiptStage, lang: Lang) -> String {
    let (key, value) = match s {
        ReceiptStage::Sent => ("Sent", None),
        ReceiptStage::Received => ("Received by the core", None),
        ReceiptStage::Applied { first_action } => ("Applied: {value}", Some(first_action)),
        ReceiptStage::Rejected { reason } => ("Rejected: {value}", Some(reason)),
    };
    let text = translate(lang, T, key);
    value.map_or_else(|| text.to_owned(), |v| text.replace("{value}", v))
}
pub fn new_client_id(session: &str, now_ms: i64, n: u64) -> String {
    format!("{session}:{now_ms}:{n}")
}
#[derive(Debug, PartialEq)]
pub enum SendError {
    PauseFailed(String),
    Send(String),
}
#[cfg(test)]
pub fn send(seam: &dyn Interjections, d: &Draft) -> Result<Receipt, SendError> {
    if d.pause_first {
        seam.pause_batch(&d.session)
            .map_err(|e| SendError::PauseFailed(e.to_string()))?;
    }
    seam.send(d).map_err(|e| SendError::Send(e.to_string()))
}
pub(super) fn open_draft_id(prefs: &mut UxPrefs, session: &str, make: impl FnOnce() -> String) -> String {
    prefs.draft_client_id(session, make)
}
pub(super) fn edit_draft(prefs: &mut UxPrefs, session: &str, text: &str, make: impl FnOnce() -> String) -> Option<String> {
    edit_draft_with(prefs, session, text, make, |prefs, session, id| prefs.rotate_draft_client_id(session, id))
}
pub(super) fn edit_draft_with(prefs: &mut UxPrefs, session: &str, text: &str, make: impl FnOnce() -> String, rotate: impl FnOnce(&mut UxPrefs, &str, String)) -> Option<String> {
    if prefs.drafts.get(session).map(String::as_str).unwrap_or("") == text { return None; }
    let id = make();
    rotate(prefs, session, id.clone());
    if text.is_empty() { prefs.drafts.remove(session); } else { prefs.drafts.insert(session.to_owned(), text.to_owned()); }
    Some(id)
}
pub(super) fn applied_draft(prefs: &mut UxPrefs, receipt: &Receipt, draft: &Draft) {
    applied_draft_with(prefs, receipt, draft, |prefs, session| prefs.clear_draft(session));
}
pub(super) fn applied_draft_with(prefs: &mut UxPrefs, receipt: &Receipt, draft: &Draft, clear: impl FnOnce(&mut UxPrefs, &str)) {
    if matches!(receipt.stage, ReceiptStage::Applied { .. })
        && receipt.client_id == draft.client_id && receipt.session == draft.session
        && prefs.draft_ids.get(&draft.session) == Some(&draft.client_id)
        && prefs.drafts.get(&draft.session) == Some(&draft.text) {
        clear(prefs, &draft.session);
    }
}
pub(super) fn submit_outbox(outbox: &Outbox<Draft>, draft: &Draft) -> Ticket {
    submit_outbox_with(outbox, draft, |draft| draft.client_id.clone())
}
pub(super) fn submit_outbox_with(outbox: &Outbox<Draft>, draft: &Draft, key: impl FnOnce(&Draft) -> String) -> Ticket {
    outbox.submit(draft.clone(), key(draft))
}
pub(super) fn ticket_draft(prefs: &mut UxPrefs, state: &TicketState, receipt: Option<&Receipt>, draft: &Draft) {
    if matches!(state, TicketState::Applied) {
        if let Some(receipt) = receipt { applied_draft(prefs, receipt, draft); }
    }
}
pub fn history_rows(
    seam: Option<&dyn Interjections>,
    session: &str,
    fallback: &[SharedString],
) -> Vec<Amendment> {
    seam.map_or_else(
        || {
            fallback
                .iter()
                .map(|text| Amendment {
                    client_id: None,
                    text: text.to_string(),
                    at_ms: None,
                    stage: None,
                    affected: vec![],
                    replan: false,
                })
                .collect()
        },
        |s| s.history(session),
    )
}
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub struct InterjectionPanel {
    state: Entity<AppState>,
    session: SharedString,
    seam: Option<Arc<dyn Interjections>>,
    input: Entity<TextAreaState>,
    client_id: String,
    pause: bool,
    replan: bool,
    receipt: Option<Receipt>,
    rows: Vec<Amendment>,
    last_sent: Option<Draft>,
    error: Option<SendError>,
    ticket: Option<Ticket>,
    poll: Option<Task<()>>,
    _subscription: Subscription,
}
impl InterjectionPanel {
    fn new(
        state: Entity<AppState>,
        session: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let seam = state.read(cx).ux.seams.interjections.clone();
        if let Some(service) = seam.clone() {
            state.update(cx, |state, _| {
                if state.ux.seams.interjection_outbox.is_none() {
                    state.ux.seams.interjection_outbox = Some(crate::desktop::data::interjection_outbox_for(service));
                }
            });
        }
        let fallback = state
            .read(cx)
            .ux
            .amendments
            .get(&session)
            .cloned()
            .unwrap_or_default();
        let rows = history_rows(seam.as_deref(), &session, &fallback);
        let saved = Store::current(cx)
            .ux
            .drafts
            .get(session.as_ref())
            .cloned()
            .unwrap_or_default();
        let saved_text = saved.clone();
        let input = TextAreaState::new(window, cx);
        input.update(cx, |input, cx| { input.set_rows(6, cx); input.set_submit_on_enter(false, cx); input.set_value(saved, window, cx); });
        let mut client_id = String::new();
        let now = state.read(cx).ux.now_ms();
        Store::edit(cx, |s| client_id = open_draft_id(&mut s.ux, &session, || new_client_id(&session, now, NEXT_ID.fetch_add(1, Ordering::Relaxed))));
        let receipt = rows.iter().any(|row| row.client_id.as_deref() == Some(client_id.as_str()))
            .then(|| seam.as_ref().and_then(|seam| seam.receipt(&client_id))).flatten();
        let last_sent = receipt.as_ref().map(|_| Draft {
            session: session.to_string(), client_id: client_id.clone(), text: saved_text,
            pause_first: false, replan: false,
        });
        if let (Some(receipt), Some(draft)) = (&receipt, &last_sent) {
            Store::edit(cx, |s| applied_draft(&mut s.ux, receipt, draft));
        }
        let sub = cx.subscribe_in(&input, window, |this, _, event: &TextAreaEvent, _, cx| {
            if matches!(event, TextAreaEvent::Change) {
                let text = this.input.read(cx).value(cx).to_string();
                let session = this.session.to_string();
                let now = this.state.read(cx).ux.now_ms();
                let mut next = None;
                Store::edit(cx, |s| next = edit_draft(&mut s.ux, &session, &text, || new_client_id(&session, now, NEXT_ID.fetch_add(1, Ordering::Relaxed))));
                if let Some(id) = next { this.client_id = id; }
                this.error = None;
                cx.notify();
            }
        });
        Self {
            state,
            session,
            seam,
            input,
            client_id,
            pause: false,
            replan: false,
            receipt,
            rows,
            last_sent,
            error: None,
            ticket: None,
            poll: None,
            _subscription: sub,
        }
    }
    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(seam) = &self.seam else { return };
        if self.ticket.is_some() {
            return;
        }
        let text = self.input.read(cx).value(cx).to_string();
        if text.trim().is_empty() {
            return;
        }
        let mut draft = Draft {
            session: self.session.to_string(),
            client_id: self.client_id.clone(),
            text,
            pause_first: self.pause,
            replan: self.replan,
        };
        if self.last_sent.as_ref().is_some_and(|last| {
            last.client_id == draft.client_id && (last.text != draft.text
                || last.pause_first != draft.pause_first
                || last.replan != draft.replan)
        }) {
            self.client_id = new_client_id(
                &self.session,
                self.state.read(cx).ux.now_ms(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed),
            );
            draft.client_id = self.client_id.clone();
            let session = self.session.to_string();
            let id = self.client_id.clone();
            Store::edit(cx, |s| s.ux.rotate_draft_client_id(&session, id));
            self.last_sent = None;
        }
        let outbox = self.state.read(cx).ux.seams.interjection_outbox.clone();
        let Some(outbox) = outbox else { return };
        // Pause before enqueueing, once per successfully queued draft.
        if self.last_sent.as_ref() != Some(&draft) && draft.pause_first {
            if let Err(e) = seam.pause_batch(&draft.session) {
                self.error = Some(SendError::PauseFailed(e.to_string()));
                cx.notify();
                return;
            }
        }
        if self.receipt.as_ref().is_some_and(|receipt| receipt.client_id == draft.client_id && matches!(receipt.stage, ReceiptStage::Rejected { .. })) {
            self.receipt = None;
        }
        self.ticket = Some(submit_outbox(&outbox, &draft));
        self.last_sent = Some(draft);
        self.error = None;
        self.start_poll(cx);
        cx.notify();
    }
    fn accept_receipt(&mut self, receipt: Receipt) {
        let Some(seam) = &self.seam else { return };
        self.receipt = Some(merge(
            self.receipt
                .as_ref()
                .filter(|r| r.client_id == receipt.client_id),
            &receipt,
        ));
        self.error = None;
        let rows = seam.history(&self.session);
        for mut row in rows {
            if let Some(old) = self
                .rows
                .iter()
                .find(|old| old.client_id.is_some() && old.client_id == row.client_id)
            {
                if let (Some(old_stage), Some(new_stage)) = (&old.stage, &row.stage) {
                    let old_receipt = Receipt {
                        client_id: row.client_id.clone().unwrap(),
                        session: self.session.to_string(),
                        stage: old_stage.clone(),
                        at_ms: old.at_ms.unwrap_or_default(),
                    };
                    let new_receipt = Receipt {
                        stage: new_stage.clone(),
                        ..old_receipt.clone()
                    };
                    row.stage = Some(merge(Some(&old_receipt), &new_receipt).stage);
                } else if row.stage.is_none() {
                    row.stage = old.stage.clone();
                }
            }
            if let Some(receipt) = self
                .receipt
                .as_ref()
                .filter(|r| row.client_id.as_deref() == Some(r.client_id.as_str()))
            {
                let old = row.stage.as_ref().map(|stage| Receipt {
                    client_id: receipt.client_id.clone(),
                    session: self.session.to_string(),
                    stage: stage.clone(),
                    at_ms: row.at_ms.unwrap_or_default(),
                });
                row.stage = Some(merge(old.as_ref(), receipt).stage);
            }
            if let Some(pos) = self
                .rows
                .iter()
                .position(|old| old.client_id.is_some() && old.client_id == row.client_id)
            {
                self.rows[pos] = row;
            } else {
                self.rows.push(row);
            }
        }
    }
    fn needs_poll(&self) -> bool {
        self.ticket.is_some()
            || self.receipt.as_ref().is_some_and(|r| rank(&r.stage) < 2)
            || self
                .rows
                .iter()
                .any(|r| r.client_id.is_some() && r.stage.as_ref().is_none_or(|s| rank(s) < 2))
    }
    fn start_poll(&mut self, cx: &mut Context<Self>) {
        self.poll = None;
        if self.seam.is_none() || !self.needs_poll() {
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(super::super::components::tokens::POLL_INTERVAL)
                .await;
            let Ok(done) = this.update(cx, |this, cx| {
                if let Some(ticket) = this.ticket.clone() {
                    match ticket.state() {
                        TicketState::Pending => {}
                        TicketState::Applied => {
                            if let (Some(seam), Some(draft)) = (&this.seam, this.last_sent.clone())
                            {
                                if let Some(receipt) = seam.receipt(&draft.client_id) {
                                    this.ticket = None;
                                    this.accept_receipt(receipt);
                                    Store::edit(cx, |s| ticket_draft(&mut s.ux, &TicketState::Applied, this.receipt.as_ref(), &draft));
                                }
                            }
                        }
                        TicketState::Failed(reason) | TicketState::Refused(reason) => {
                            let state = ticket.state();
                            if let Some(draft) = &this.last_sent {
                                Store::edit(cx, |s| ticket_draft(&mut s.ux, &state, this.receipt.as_ref(), draft));
                            }
                            let refused = matches!(state, TicketState::Refused(_));
                            this.ticket = None;
                            this.error = Some(SendError::Send(reason.clone()));
                            if refused {
                                if let Some(draft) = &this.last_sent {
                                    this.receipt = Some(Receipt {
                                        client_id: draft.client_id.clone(),
                                        session: draft.session.clone(),
                                        stage: ReceiptStage::Rejected { reason },
                                        at_ms: this.state.read(cx).ux.now_ms(),
                                    });
                                }
                            }
                        }
                    }
                }
                if let Some(seam) = &this.seam {
                    let mut ids = std::collections::HashSet::new();
                    if let Some(old) = &this.receipt {
                        if rank(&old.stage) < 2 {
                            ids.insert(old.client_id.clone());
                        }
                    }
                    for row in &this.rows {
                        if row.stage.as_ref().is_none_or(|s| rank(s) < 2) {
                            if let Some(id) = &row.client_id {
                                ids.insert(id.clone());
                            }
                        }
                    }
                    for id in ids {
                        if let Some(next) = seam.receipt(&id) {
                            if let Some(old) = &this.receipt {
                                if old.client_id == id {
                                    this.receipt = Some(merge(Some(old), &next));
                                }
                            }
                            if matches!(next.stage, ReceiptStage::Applied { .. }) {
                                if let Some(draft) = this.last_sent.as_ref().filter(|draft| draft.client_id == id) {
                                    Store::edit(cx, |s| applied_draft(&mut s.ux, &next, draft));
                                }
                            }
                            for row in this
                                .rows
                                .iter_mut()
                                .filter(|r| r.client_id.as_deref() == Some(&id))
                            {
                                let old = row.stage.as_ref().map(|stage| Receipt {
                                    client_id: id.clone(),
                                    session: this.session.to_string(),
                                    stage: stage.clone(),
                                    at_ms: row.at_ms.unwrap_or_default(),
                                });
                                row.stage = Some(merge(old.as_ref(), &next).stage);
                            }
                        }
                    }
                }
                cx.notify();
                !this.needs_poll()
            }) else {
                break;
            };
            if done {
                break;
            }
        }));
    }
}
impl Render for InterjectionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = theme::density(cx);
        let lang = *cx.global::<Lang>();
        let text = self.input.read(cx).value(cx).to_string();
        let mut history = v_flex()
            .id("amendment-history")
            .gap(theme::space(d, Step::Xs))
            .test_support();
        let title = tr(cx, T, "Amendment history");
        history = history.child(title.test_text(title));
        let fallback = self
            .state
            .read(cx)
            .ux
            .amendments
            .get(&self.session)
            .cloned()
            .unwrap_or_default();
        let live_rows;
        let rows = if self.seam.is_none() {
            live_rows = history_rows(None, &self.session, &fallback);
            &live_rows
        } else {
            &self.rows
        };
        for row in rows {
            let time = row
                .at_ms
                .map(|ms| now_string(ms))
                .unwrap_or_else(|| "—".into());
            let status = row
                .stage
                .as_ref()
                .map(|s| stage_text(s, lang))
                .unwrap_or_else(|| "—".into());
            let affected = if row.affected.is_empty() {
                String::new()
            } else {
                format!(" · {}: {}", tr(cx, T, "Affected"), row.affected.join(", "))
            };
            let line = format!("{time} · {status} · {}{affected}", row.text);
            history = history.child(line.clone().test_text(line));
        }
        let mut body = v_flex()
            .w_full()
            .gap(theme::space(d, Step::Sm))
            .text_size(theme::font(d, TypeSize::Body));
        if let Some(seam) = &self.seam {
            let work = seam.running_work(&self.session);
            let mut parts = Vec::new();
            if let Some(stage) = work.stage {
                parts.push(format!("{}: {stage}", tr(cx, T, "Stage")));
            }
            if !work.units.is_empty() {
                parts.push(format!("{}: {}", tr(cx, T, "Units"), work.units.join(", ")));
            }
            if let Some(batch) = work.batch {
                parts.push(format!("{}: {batch}", tr(cx, T, "Batch")));
            }
            if !parts.is_empty() {
                let warning =
                    tr(cx, T, "Running now: {work}").replace("{work}", &parts.join(" · "));
                body = body.child(
                    div()
                        .id("running-work-warning")
                        .child(warning.clone().test_text(warning))
                        .test_support(),
                );
            }
            let height = theme::row_height(d) * (text.lines().count().clamp(6, 12) as f32);
            let preview = tr(cx, T, "Preview");
            let pause = tr(cx, T, "Pause the batch first");
            let replan = tr(cx, T, "Re-plan with Astra + council");
            let send_label = tr(cx, T, "Send");
            body = body
                .child(
                    div()
                        .id("interject-editor")
                        .h(height).child(TextArea::from_state("interject-input", &self.input, &cx.tokens(), cx))
                        .test_support(),
                )
                .child(
                    v_flex()
                        .id("interject-preview")
                        .child(preview.test_text(preview))
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "interject-draft:{}",
                                    self.client_id
                                )))
                                .child(text.clone().test_text(text.clone()))
                                .test_support(),
                        )
                        .test_support(),
                )
                .child(
                    div()
                        .id("interject-pause-first")
                        .child(
                            Toggle::new("pause-checkbox", pause, &cx.tokens())
                                .checked(self.pause)
                                .on_change({
                                    let view = cx.entity();
                                    move |checked, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.pause = checked;
                                            cx.notify();
                                        });
                                    }
                                })
                                .test_text(pause),
                        )
                        .test_support(),
                )
                .child(
                    div()
                        .id("interject-replan")
                        .child(
                            Toggle::new("replan-checkbox", replan, &cx.tokens())
                                .checked(self.replan)
                                .on_change({
                                    let view = cx.entity();
                                    move |checked, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.replan = checked;
                                            cx.notify();
                                        });
                                    }
                                })
                                .test_text(replan),
                        )
                        .test_support(),
                )
                .child(
                    div()
                        .id("interject-send")
                        .child(
                            Button::new("action:dialog.interjection.send", send_label, &cx.tokens())
                                .accessibility_label(send_label)
                                .disabled(text.trim().is_empty())
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.submit(window, cx)),
                                ).test_text(send_label),
                        )
                        .test_support(),
                );
            if let Some(r) = &self.receipt {
                let label = stage_text(&r.stage, lang);
                body = body.child(
                    div()
                        .id("interject-receipt")
                        .child(label.clone().test_text(label))
                        .test_support(),
                );
            }
            if let Some(error) = &self.error {
                let (key, value) = match error {
                    SendError::PauseFailed(v) => ("Pause failed: {value}", v),
                    SendError::Send(v) => ("Send failed: {value}", v),
                };
                let label = tr(cx, T, key).replace("{value}", value);
                body = body.child(
                    div()
                        .id("interject-error")
                        .child(label.clone().test_text(label))
                        .test_support(),
                );
            }
        }
        div()
            .id("interjection-scroll")
            .max_h(theme::row_height(d) * 18.)
            .overflow_y_scroll()
            .child(body.child(history))
    }
}
fn now_string(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    super::super::model::facade::work::local_clock_at(secs)
}
pub(super) fn open(
    state: Entity<AppState>,
    session: SharedString,
    window: &mut Window,
    cx: &mut App,
) {
    let panel = cx.new(|cx| InterjectionPanel::new(state, session, window, cx));
    panel.update(cx, |panel, cx| panel.start_poll(cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .w(theme::READ_WIDTH)
            .title(tr(cx, T, "Interjection"))
            .on_ok(|_, _, _| false)
            .child(
                DialogFrame::new("dialog.interjection.prompt", &cx.tokens()).child(panel.clone()),
            )
    });
}
pub(super) fn open_button(state: Entity<AppState>, session: SharedString, cx: &mut App) -> Button {
    Button::new("open-interjection", tr(cx, T, "Interjection"), &cx.tokens())
        .on_click(move |_, window, cx| open(state.clone(), session.clone(), window, cx))
}
