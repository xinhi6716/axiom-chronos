use crate::{
    base::Res,
    desktop::{
        i18n::{self, Lang, Table},
        model::AppState,
        theme::Tokened as _,
        ux::seams::{PaperService, Preview},
    },
    paper::{service::PaperStatus, SpendEstimate},
};
use gpui_kit::{
    base::TestTextExt as _,
    component::{
        button::Button, scroll::ScrollableElement as _, v_flex, Disableable as _, Selectable as _,
    },
    prelude::FluentBuilder as _,
    *,
};
use std::{sync::Arc, time::Duration};

#[cfg(test)]
mod accept;
mod changelog;
mod form;
mod preview;
mod progress;
mod shortlist;

const TABLE: Table = &[
    ("Back", "返回"),
    ("Write paper", "撰寫論文"),
    ("Start", "開始"),
    ("Cancel", "取消"),
    ("Acknowledge estimate", "確認估算"),
    ("Get estimate", "取得估算"),
    ("Service unavailable", "服務無法使用"),
    ("Loading", "載入中"),
    ("Evidence", "證據"),
    ("Shortlist", "候選期刊"),
    ("Journal", "期刊"),
    ("Pass 1", "第 1 輪"),
    ("Pass 2", "第 2 輪"),
    ("Pass 3", "第 3 輪"),
    ("Pass 4", "第 4 輪"),
    ("Figures", "圖表"),
    ("Citations", "引用"),
    ("Numbers", "數值"),
    ("Compile", "編譯"),
    ("Render", "轉譯"),
    ("Disclosure", "揭露"),
    ("Review", "審查"),
    ("Ready", "就緒"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    i18n::translate(lang, TABLE, key)
}
fn step_label(lang: Lang, step: crate::paper::Step) -> &'static str {
    use crate::paper::Step::*;
    tr(
        lang,
        match step {
            Evidence => "Evidence",
            Shortlist => "Shortlist",
            Journal => "Journal",
            Pass1 => "Pass 1",
            Pass2 => "Pass 2",
            Pass3 => "Pass 3",
            Pass4 => "Pass 4",
            Figures => "Figures",
            Citations => "Citations",
            Numbers => "Numbers",
            Compile => "Compile",
            Render => "Render",
            Disclosure => "Disclosure",
            Review => "Review",
            Ready => "Ready",
        },
    )
}
fn marked(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
fn is_chosen(choice: Option<&crate::paper::JournalChoice>, id: &str, hash: &str) -> bool {
    choice.is_some_and(|c| c.journal_id == id && c.shortlist_hash == hash)
}
fn evidence_question(home: &std::path::Path, session: &str) -> Res<Option<String>> {
    let store = crate::desktop::ux::real::session_store(home, session)?;
    Ok(crate::paper::evidence::load(&store)?.map(|bundle| bundle.question))
}
fn can_start(choice: bool, estimate: bool, ack: bool, busy: bool) -> bool {
    choice && estimate && ack && !busy
}

pub struct PaperPage {
    state: Entity<AppState>,
    session: SharedString,
    service: Option<Arc<dyn PaperService>>,
    status: Option<PaperStatus>,
    estimate: Option<SpendEstimate>,
    preview: Option<Preview>,
    preview_pdf: Option<std::path::PathBuf>,
    error: Option<String>,
    status_error: Option<String>,
    ack: bool,
    pending: bool,
    form: form::Form,
    initialized: bool,
    _poll: Task<()>,
}
impl PaperPage {
    pub fn new(
        state: Entity<AppState>,
        session: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = state.read(cx).ux.seams.paper.clone();
        let form = form::Form::new(window, cx);
        let service_poll = service.clone();
        let session_poll = session.to_string();
        let poll = cx.spawn_in(window, async move |this, cx| loop {
            let Some(service) = service_poll.clone() else {
                break;
            };
            let session = session_poll.clone();
            let task = cx.background_spawn(async move {
                let status = service.status(&session);
                let question = if status.as_ref().is_ok_and(|s| s.requirements.is_none()) {
                    evidence_question(&crate::base::axiom_home(), &session)
                        .ok()
                        .flatten()
                } else {
                    None
                };
                (status, question)
            });
            let (status, question) = task.await;
            if this
                .update_in(cx, |this, window, cx| {
                    match status {
                        Ok(s) => {
                            this.status_error = None;
                            if !this.initialized && (s.requirements.is_some() || question.is_some())
                            {
                                this.form.initialize(
                                    s.requirements.as_ref(),
                                    question.as_deref(),
                                    window,
                                    cx,
                                );
                                this.initialized = true;
                            }
                            if this.estimate.as_ref().map(|e| &e.digest)
                                != s.estimate.as_ref().map(|e| &e.digest)
                            {
                                this.ack = false;
                            }
                            this.estimate = s.estimate.clone();
                            if s.pdf != this.preview_pdf {
                                this.preview_pdf = s.pdf.clone();
                                this.preview = None;
                                if s.pdf.is_some() {
                                    this.load_preview(1, cx);
                                }
                            }
                            this.status = Some(s);
                        }
                        Err(e) => this.status_error = Some(e.to_string()),
                    }
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
        });
        Self {
            state,
            session,
            service,
            status: None,
            estimate: None,
            preview: None,
            preview_pdf: None,
            error: None,
            status_error: None,
            ack: false,
            pending: false,
            form,
            initialized: false,
            _poll: poll,
        }
    }
    fn action(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(Arc<dyn PaperService>, String) -> Res<Option<SpendEstimate>> + Send + 'static,
    ) {
        if self.pending {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let session = self.session.to_string();
        let task = cx.background_spawn(async move { f(service, session) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.pending = false;
                match result {
                    Ok(Some(e)) => {
                        this.estimate = Some(e);
                        this.ack = false;
                    }
                    Ok(None) => {}
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn choose(&mut self, id: String, hash: String, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(service) = self.service.clone() else {
            return;
        };
        self.pending = true;
        self.error = None;
        let session = self.session.to_string();
        let task = cx.background_spawn(async move { service.choose_journal(&session, &id, &hash) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.ack = false;
                        this.estimate = None;
                        if let Some(status) = &mut this.status {
                            status.choice = None;
                            status.estimate = None;
                        }
                        this.action(cx, |s, id| s.estimate(&id).map(Some));
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn load_preview(&mut self, page: u32, cx: &mut Context<Self>) {
        let Some(service) = self.service.clone() else {
            return;
        };
        let session = self.session.to_string();
        let task = cx.background_spawn(async move { service.preview(&session, page, 720) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(p) => this.preview = Some(p),
                    Err(e) => this.error = Some(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for PaperPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let t = cx.tokens();
        let chosen = self.status.as_ref().is_some_and(|s| {
            s.shortlist.as_ref().is_some_and(|l| {
                l.ranked
                    .iter()
                    .any(|r| is_chosen(s.choice.as_ref(), &r.journal.id, &l.hash))
            })
        });
        let busy = self.pending || self.status.as_ref().is_some_and(|s| s.running);
        let enabled = can_start(chosen, self.estimate.is_some(), self.ack, busy);
        let state = self.state.clone();
        let mut page = v_flex()
            .id("paper-page")
            .test_support()
            .size_full()
            .p(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Md,
            ))
            .gap(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Sm,
            ))
            .text_color(t.text)
            .child(
                Button::new("paper-back")
                    .child(tr(lang, "Back").test_text(tr(lang, "Back")))
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx))),
            )
            .child(tr(lang, "Write paper").test_text(tr(lang, "Write paper")));
        if self.service.is_none() {
            return page
                .child(tr(lang, "Service unavailable"))
                .into_any_element();
        }
        if let Some(e) = self.error.clone() {
            page = page.child(marked("paper-error", e));
        }
        if let Some(e) = self.status_error.clone() {
            page = page.child(marked("paper-error", e));
        }
        if let Some(s) = &self.status {
            if let Some((kind, reason)) = &s.last_error {
                page = page.child(marked("paper-error", format!("{kind}: {reason}")));
            }
            for (step, ok, detail) in &s.steps {
                if let Some(detail) = detail {
                    page = page.child(marked(
                        "paper-error",
                        format!(
                            "{} · {} · {detail}",
                            step_label(lang, *step),
                            if *ok { "✓" } else { "✗" }
                        ),
                    ));
                }
            }
        } else {
            page = page.child(tr(lang, "Loading"));
        }
        let form = self.form.render(lang, cx);
        let list = shortlist::render(self.status.as_ref(), lang, cx);
        let passes = progress::render(self.status.as_ref(), lang, &t);
        let logs = changelog::render(self.status.as_ref(), lang);
        let preview = preview::render(self.preview.as_ref(), lang, cx);
        let estimate = self.estimate.clone();
        let has_passes = self.status.as_ref().is_some_and(|s| !s.passes.is_empty());
        let mut content =
            v_flex()
                .flex_1()
                .overflow_y_scrollbar()
                .gap(crate::desktop::theme::space(
                    crate::desktop::theme::density(cx),
                    crate::desktop::theme::Step::Sm,
                ));
        if has_passes {
            content = content
                .children(passes)
                .children(logs)
                .children(preview)
                .child(form)
                .children(list);
        } else {
            content = content
                .children(list)
                .child(form)
                .children(passes)
                .children(logs)
                .children(preview);
        }
        page = page.child(content).child(
            gpui_kit::component::h_flex()
                .gap(crate::desktop::theme::space(
                    crate::desktop::theme::density(cx),
                    crate::desktop::theme::Step::Sm,
                ))
                .when_some(estimate, |p, e| {
                    p.child(marked("paper-estimate", e.line)).child(
                        Button::new("paper-ack")
                            .label(tr(lang, "Acknowledge estimate"))
                            .selected(self.ack)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ack = !this.ack;
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("paper-estimate-request")
                        .label(tr(lang, "Get estimate"))
                        .disabled(!chosen || busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.action(cx, |s, id| s.estimate(&id).map(Some))
                        })),
                )
                .child(
                    Button::new("paper-start")
                        .child(tr(lang, "Start").test_text(tr(lang, "Start")))
                        .disabled(!enabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(e) = this.estimate.clone() {
                                if can_start(chosen, true, this.ack, this.pending) {
                                    this.ack = false;
                                    this.action(cx, move |s, id| {
                                        s.start(&id, &e.digest).map(|_| None)
                                    });
                                }
                            }
                        })),
                )
                .when(busy, |p| {
                    p.child(
                        Button::new("paper-cancel")
                            .label(tr(lang, "Cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.action(cx, |s, id| s.cancel(&id).map(|_| None))
                            })),
                    )
                }),
        );
        page.into_any_element()
    }
}
