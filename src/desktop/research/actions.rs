use super::Workbench;
use crate::desktop::{
    components::TextField,
    i18n::{translate, Lang, Table},
    theme::Tokened as _,
    ux::seams::research::{ActionRequest, Correction, CorrectionStatus, Reverify, SpotVerdict},
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::input::InputState,
    *,
};

const T: Table = &[
    ("Confirm", "確認"),
    ("Dispute", "提出異議"),
    ("After", "修正後"),
    ("Why", "原因"),
    ("Submit correction", "提交修正"),
    ("Retry", "重試"),
    ("Not sent; your request is kept", "尚未送出；已保留您的請求"),
    (
        "There is no active session for this research.",
        "此研究沒有進行中的工作階段。",
    ),
    ("Who", "執行者"),
    ("Version", "版本"),
    ("Re-verification", "重新驗證"),
    ("Pending", "待處理"),
    ("Passed", "通過"),
    ("Failed", "失敗"),
    ("Receipt", "回執"),
    ("Not requested", "未請求"),
    ("Sent", "已送出"),
    ("Received", "已收到"),
    ("Applied", "已套用"),
    ("Rejected", "已拒絕"),
    ("Invalidated", "受影響項目"),
    ("Action failed", "操作失敗"),
];
pub(super) fn tr(lang: Lang, key: &'static str) -> &'static str {
    translate(lang, T, key)
}

use super::adapter::actions::who;

pub struct Actions {
    wb: Entity<Workbench>,
    key: Option<(std::path::PathBuf, String)>,
    generation: u64,
    state: bool,
    corrections: Vec<CorrectionStatus>,
    session: bool,
    rows: Vec<(String, Option<String>)>,
    editing: Option<(String, String)>,
    after: Option<Entity<InputState>>,
    why: Option<Entity<InputState>>,
    error: bool,
    polling: bool,
}
impl Actions {
    pub fn new(wb: Entity<Workbench>, cx: &mut Context<Self>) -> Self {
        cx.observe(&wb, |this, _, cx| this.refresh(cx)).detach();
        let mut this = Self {
            wb,
            key: None,
            generation: 0,
            state: false,
            corrections: vec![],
            session: false,
            rows: vec![],
            editing: None,
            after: None,
            why: None,
            error: false,
            polling: false,
        };
        this.refresh(cx);
        this
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let wb = self.wb.read(cx);
        let key = wb
            .seams
            .research_home
            .clone()
            .zip(wb.located.as_ref().map(|l| l.id.clone()));
        if key == self.key {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.key = key.clone();
        self.state = false;
        self.corrections.clear();
        self.session = false;
        self.error = false;
        self.editing = None;
        self.rows.clear();
        self.polling = false;
        if key.is_none() {
            cx.notify();
            return;
        }
        self.request(cx, ActionRequest::Open);
    }
    fn request(&mut self, cx: &mut Context<Self>, request: ActionRequest) {
        let wb = self.wb.read(cx);
        let port = wb.port.clone();
        let now = wb.clock.now_ms();
        let generation = self.generation;
        let (reply, response) = std::sync::mpsc::channel();
        port.action(request, now, reply);
        let task = cx.background_spawn(async move { response.recv() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(snapshot)) => {
                        this.state = true;
                        this.error = false;
                        this.session = snapshot.session;
                        this.corrections = snapshot.corrections;
                        this.schedule(cx);
                    }
                    _ => this.error = true,
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn run(&mut self, cx: &mut Context<Self>, request: ActionRequest) {
        if self.state {
            self.request(cx, request);
        }
    }
    fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.polling
            || !self.corrections.iter().any(|c| {
                c.receipt
                    .as_ref()
                    .is_some_and(|r| r.stage == "sent" || r.stage == "received")
            })
        {
            return;
        }
        self.polling = true;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(super::adapter::receipt_poll_interval())
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.polling = false;
                    this.poll_receipts(cx);
                }
            });
        })
        .detach();
    }
    pub fn spot_check(
        &mut self,
        target: &str,
        node: Option<&str>,
        v: SpotVerdict,
        cx: &mut Context<Self>,
    ) {
        self.run(
            cx,
            ActionRequest::Spot {
                target: target.to_owned(),
                node: node.map(str::to_owned),
                verdict: v,
            },
        );
    }
    pub fn begin_correction(&mut self, target: &str, field: &str, cx: &mut Context<Self>) {
        self.editing = Some((target.into(), field.into()));
        self.after = None;
        self.why = None;
        cx.notify();
    }
    pub fn submit_correction(&mut self, after: String, why: String, cx: &mut Context<Self>) {
        let Some((target, field)) = self.editing.clone() else {
            return;
        };
        let draft = Correction {
            target,
            field,
            after,
            why,
            who: who(),
        };
        self.run(cx, ActionRequest::Correct(draft));
    }
    pub fn retry_send(&mut self, correction_id: &str, cx: &mut Context<Self>) {
        self.run(cx, ActionRequest::Retry(correction_id.to_owned()));
    }
    pub fn poll_receipts(&mut self, cx: &mut Context<Self>) {
        self.run(cx, ActionRequest::Poll);
    }
    fn row(&mut self, target: &str, node: Option<&str>) -> usize {
        row_index(&mut self.rows, target, node)
    }
}
pub(super) fn row_index(
    rows: &mut Vec<(String, Option<String>)>,
    target: &str,
    node: Option<&str>,
) -> usize {
    let key = (target.to_owned(), node.map(str::to_owned));
    if let Some(i) = rows.iter().position(|r| *r == key) {
        i
    } else {
        rows.push(key);
        rows.len() - 1
    }
}

fn marked(id: impl Into<ElementId>, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
pub fn spot_check_buttons(
    actions: &Entity<Actions>,
    target: &str,
    node: Option<&str>,
    cx: &mut App,
) -> AnyElement {
    let i = actions.update(cx, |a, _| a.row(target, node));
    let lang = *cx.global::<Lang>();
    let mut row = div().flex();
    for (kind, label) in [("confirm", "Confirm"), ("dispute", "Dispute")] {
        let a = actions.clone();
        let target = target.to_owned();
        let node = node.map(str::to_owned);
        let text = tr(lang, label).to_owned();
        row = row.child(
            div()
                .id(SharedString::from(format!("research-spot-{kind}-{i}")))
                .test_support()
                .cursor_pointer()
                .child(text.clone().test_text(text))
                .on_click(move |_, _, cx| {
                    let v = if kind == "confirm" {
                        SpotVerdict::Confirm
                    } else {
                        SpotVerdict::Dispute {
                            reason: "Disputed at spot-check".into(),
                        }
                    };
                    a.update(cx, |a, cx| a.spot_check(&target, node.as_deref(), v, cx));
                }),
        );
    }
    row.into_any_element()
}
pub fn correction_form(actions: &Entity<Actions>, window: &mut Window, cx: &mut App) -> AnyElement {
    let lang = *cx.global::<Lang>();
    let colors = cx.tokens();
    let (after, why, editing, session) = actions.update(cx, |a, cx| {
        let after = a
            .after
            .get_or_insert_with(|| cx.new(|cx| InputState::new(window, cx)))
            .clone();
        let why = a
            .why
            .get_or_insert_with(|| cx.new(|cx| InputState::new(window, cx)))
            .clone();
        (after, why, a.editing.is_some(), a.session)
    });
    let mut form = div()
        .flex()
        .flex_col()
        .id("research-correct-form")
        .test_support();
    if editing {
        form = form
            .child(tr(lang, "After"))
            .child({
                let input = after.clone();
                div()
                    .id("research-correct-after")
                    .test_support()
                    .on_click(move |_, window, cx| {
                        input.read(cx).focus_handle(cx).focus(window, cx)
                    })
                    .child(TextField::new(
                        "research-correct-after-field",
                        &after,
                        &colors,
                    ))
            })
            .child(tr(lang, "Why"))
            .child({
                let input = why.clone();
                div()
                    .id("research-correct-why")
                    .test_support()
                    .on_click(move |_, window, cx| {
                        input.read(cx).focus_handle(cx).focus(window, cx)
                    })
                    .child(TextField::new("research-correct-why-field", &why, &colors))
            });
        let a = actions.clone();
        let text = tr(lang, "Submit correction").to_owned();
        form = form.child(
            div()
                .id("research-correct-submit")
                .test_support()
                .cursor_pointer()
                .child(text.clone().test_text(text))
                .on_click(move |_, _, cx| {
                    let after = after.read(cx).value().to_string();
                    let why = why.read(cx).value().to_string();
                    a.update(cx, |a, cx| a.submit_correction(after, why, cx));
                }),
        );
        if !session {
            let text = tr(lang, "There is no active session for this research.").to_owned();
            form = form.child(text.clone().test_text(text));
        }
    }
    form.into_any_element()
}
pub fn receipts(actions: &Entity<Actions>, cx: &App) -> AnyElement {
    let lang = *cx.global::<Lang>();
    let state = actions.read(cx);
    let mut list = div()
        .flex()
        .flex_col()
        .id("research-receipts")
        .test_support();
    if state.error {
        let text = tr(lang, "Action failed").to_owned();
        list = list.child(text.clone().test_text(text));
    }
    for (i, c) in state.corrections.iter().enumerate() {
        let verify = match &c.reverify {
            Reverify::Pending => tr(lang, "Pending").to_owned(),
            Reverify::Passed => tr(lang, "Passed").to_owned(),
            Reverify::Failed { reason } => format!("{}: {reason}", tr(lang, "Failed")),
        };
        let receipt = c
            .receipt
            .as_ref()
            .map(|r| {
                let stage = match r.stage.as_str() {
                    "sent" => tr(lang, "Sent"),
                    "received" => tr(lang, "Received"),
                    "applied" => tr(lang, "Applied"),
                    "rejected" => tr(lang, "Rejected"),
                    _ => r.stage.as_str(),
                };
                match &r.detail {
                    Some(d) => format!("{stage}: {d}"),
                    None => stage.to_owned(),
                }
            })
            .unwrap_or_else(|| tr(lang, "Not requested").into());
        let text = format!(
            "{}: {} | {}: {} | {}: {} | {}: {} | {}: {} | {}: {}",
            tr(lang, "Who"),
            c.who,
            tr(lang, "Why"),
            c.why,
            tr(lang, "Version"),
            c.research_version,
            tr(lang, "Re-verification"),
            verify,
            tr(lang, "Receipt"),
            receipt,
            tr(lang, "Invalidated"),
            c.invalidates.len()
        );
        list = list.child(marked(
            SharedString::from(format!("research-receipt-{i}")),
            text,
        ));
        if c.draft.is_some() && c.receipt.is_none() {
            let text = tr(lang, "Not sent; your request is kept").to_owned();
            list = list.child(text.clone().test_text(text));
            let a = actions.clone();
            let id = c.id.clone();
            let text = tr(lang, "Retry").to_owned();
            list = list.child(
                div()
                    .id(SharedString::from(format!("research-retry-{i}")))
                    .test_support()
                    .cursor_pointer()
                    .child(text.clone().test_text(text))
                    .on_click(move |_, _, cx| a.update(cx, |a, cx| a.retry_send(&id, cx))),
            );
        }
    }
    list.into_any_element()
}
