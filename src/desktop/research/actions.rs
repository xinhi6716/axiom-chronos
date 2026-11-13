use super::Workbench;
use crate::{
    base::Res,
    desktop::{
        i18n::{translate, Lang, Table},
        ux::seams::{Draft, Interjections, ReceiptStage},
    },
    research_workspace::{
        annotations::{
            self, AnnotationStore, Correction, CorrectionDraft, ReceiptRef, Reverify, Verdict,
        },
        index::Index,
        manifest::Located,
    },
    seams::Clock,
};
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{
        h_flex,
        input::{Input, InputState},
        v_flex,
    },
    *,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
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

pub fn resolve_session(l: &Located, goals_home: Option<&Path>) -> Option<String> {
    l.manifest
        .session
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| {
            let project = l.store.as_ref()?.file_name()?.to_str()?;
            crate::goal::find_active(goals_home?, project)
                .ok()
                .flatten()
                .map(|(id, _)| id)
        })
}
pub(super) fn who() -> String {
    ["USERNAME", "USER"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "you".into())
}
pub(super) struct State {
    pub(super) store: AnnotationStore,
    pub(super) session: Option<String>,
    pub(super) bridge: Option<Arc<dyn Interjections>>,
}
impl State {
    pub(super) fn spot(
        &mut self,
        ix: &Index,
        target: &str,
        node: Option<&str>,
        verdict: Verdict,
        clock: &dyn Clock,
    ) -> Res<()> {
        self.store
            .spot_check(ix, target, node, verdict, &who(), clock)?;
        Ok(())
    }
    fn send(&mut self, c: &Correction, text: String) -> Res<()> {
        let (Some(session), Some(bridge)) = (&self.session, &self.bridge) else {
            return Ok(());
        };
        let (client_id, _) = annotations::reverify_request(c, session);
        let d = Draft {
            session: session.clone(),
            client_id: client_id.clone(),
            text: text.clone(),
            pause_first: false,
            replan: false,
        };
        if bridge.send(&d).is_ok() {
            self.store.set_receipt(
                &c.id,
                ReceiptRef {
                    client_id,
                    session: session.clone(),
                    stage: "sent".into(),
                    detail: None,
                },
            )?;
        } else {
            self.store.keep_draft(&c.id, &text)?;
        }
        Ok(())
    }
    pub(super) fn correct(&mut self, ix: &Index, d: CorrectionDraft, clock: &dyn Clock) -> Res<()> {
        let c = self.store.correct(ix, d, clock)?;
        self.store
            .set_reverify(&c.id, annotations::reverify(ix, &c))?;
        if let Some(session) = &self.session {
            let (_, text) = annotations::reverify_request(&c, session);
            self.send(&c, text)?;
        }
        Ok(())
    }
    pub(super) fn retry(&mut self, id: &str) -> Res<()> {
        if let Some(c) = self
            .store
            .corrections(None)
            .into_iter()
            .find(|c| c.id == id && c.receipt.is_none())
        {
            if let Some(text) = c.draft.clone() {
                self.send(&c, text)?;
            }
        }
        Ok(())
    }
    pub(super) fn poll(&mut self) -> Res<()> {
        let Some(bridge) = self.bridge.clone() else {
            return Ok(());
        };
        for c in self.store.corrections(None) {
            let Some(mut r) = c.receipt else {
                continue;
            };
            if r.stage != "sent" && r.stage != "received" {
                continue;
            }
            if let Some(found) = bridge.receipt(&r.client_id) {
                let (stage, detail) = match found.stage {
                    ReceiptStage::Sent => ("sent", None),
                    ReceiptStage::Received => ("received", None),
                    ReceiptStage::Applied { first_action } => ("applied", Some(first_action)),
                    ReceiptStage::Rejected { reason } => ("rejected", Some(reason)),
                };
                if r.stage != stage || r.detail != detail {
                    r.stage = stage.into();
                    r.detail = detail;
                    self.store.set_receipt(&c.id, r)?;
                }
            }
        }
        Ok(())
    }
}
type Shared = Arc<Mutex<State>>;

pub struct Actions {
    wb: Entity<Workbench>,
    key: Option<(std::path::PathBuf, String)>,
    generation: u64,
    state: Option<Shared>,
    corrections: Vec<Correction>,
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
            state: None,
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
            .zip(wb.located.as_ref().map(|l| l.manifest.research_id.clone()));
        if key == self.key {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.key = key.clone();
        self.state = None;
        self.corrections.clear();
        self.session = false;
        self.error = false;
        self.editing = None;
        self.rows.clear();
        self.polling = false;
        let (Some((home, id)), Some(located)) = (key, wb.located.clone()) else {
            cx.notify();
            return;
        };
        let goals = wb.seams.goals_home.clone();
        let bridge = wb.interjections.clone();
        let task = cx.background_spawn(async move {
            let store = AnnotationStore::open(&home, &id)?;
            let session = bridge
                .as_ref()
                .and_then(|_| resolve_session(&located, goals.as_deref()));
            let corrections = store.corrections(None);
            Ok::<_, crate::base::Failure>((
                Arc::new(Mutex::new(State {
                    store,
                    session: session.clone(),
                    bridge,
                })),
                corrections,
                session.is_some(),
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((state, corrections, session)) => {
                        this.state = Some(state);
                        this.corrections = corrections;
                        this.session = session;
                        this.schedule(cx);
                    }
                    Err(_) => this.error = true,
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn run(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut State) -> Res<()> + Send + 'static,
    ) {
        let Some(state) = self.state.clone() else {
            return;
        };
        let generation = self.generation;
        let task = cx.background_spawn(async move {
            let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
            let result = f(&mut state);
            (result, state.store.corrections(None))
        });
        cx.spawn(async move |this, cx| {
            let (result, corrections) = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.error = result.is_err();
                this.corrections = corrections;
                this.schedule(cx);
                cx.notify();
            });
        })
        .detach();
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
            cx.background_executor().timer(Duration::from_secs(1)).await;
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
        v: Verdict,
        cx: &mut Context<Self>,
    ) {
        let wb = self.wb.read(cx);
        let Some(ix) = wb.index.clone() else {
            return;
        };
        let clock = wb.clock.clone();
        let target = target.to_owned();
        let node = node.map(str::to_owned);
        self.run(cx, move |s| {
            s.spot(&ix, &target, node.as_deref(), v, clock.as_ref())
        });
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
        let wb = self.wb.read(cx);
        let Some(ix) = wb.index.clone() else {
            return;
        };
        let clock = wb.clock.clone();
        let draft = CorrectionDraft {
            target,
            field,
            after,
            why,
            who: who(),
        };
        self.run(cx, move |s| s.correct(&ix, draft, clock.as_ref()));
    }
    pub fn retry_send(&mut self, correction_id: &str, cx: &mut Context<Self>) {
        let id = correction_id.to_owned();
        self.run(cx, move |s| s.retry(&id));
    }
    pub fn poll_receipts(&mut self, cx: &mut Context<Self>) {
        self.run(cx, |s| s.poll());
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
    let mut row = h_flex();
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
                        Verdict::Confirm
                    } else {
                        Verdict::Dispute {
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
    let mut form = v_flex().id("research-correct-form").test_support();
    if editing {
        form = form
            .child(tr(lang, "After"))
            .child(Input::new(&after).id("research-correct-after"))
            .child(tr(lang, "Why"))
            .child(Input::new(&why).id("research-correct-why"));
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
    let mut list = v_flex().id("research-receipts").test_support();
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
