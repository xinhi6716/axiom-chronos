use crate::{
    base::Res,
    desktop::{
        components::{Button, ButtonVariant, TextField},
        data::{self, Cursor, Delta, Source},
        i18n::{self, Lang, Table},
        model::AppState,
        theme::Tokened as _,
        ux::seams::{
            EstimateRow, PaperService, PaperView, PassModels, PreviewInput, PreviewView, StartInput,
        },
    },
};
use gpui_kit::{
    base::TestTextExt as _,
    component::{scroll::ScrollableElement as _, v_flex},
    prelude::FluentBuilder as _,
    *,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

#[cfg(test)]
mod accept;
mod changelog;
mod form;
mod preview;
mod progress;
mod shortlist;

const STATUS_POLL_SECONDS: u64 = 2;
const PREVIEW_POLL_SECONDS: u64 = 60;

const TABLE: Table = &[
    ("Back", "返回"),
    ("Write paper", "撰寫論文"),
    ("Start", "開始"),
    ("Cancel", "取消"),
    ("Acknowledge estimate", "確認估算"),
    ("Get estimate", "取得估算"),
    (
        "Estimate is stale; request a new estimate",
        "估算已過期，請重新取得估算",
    ),
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
    ("Readiness", "就緒狀態"),
];
fn tr(lang: Lang, key: &'static str) -> &'static str {
    i18n::translate(lang, TABLE, key)
}
fn step_label(lang: Lang, step: &str) -> &str {
    let key = match step {
        "Evidence" => "Evidence",
        "Shortlist" => "Shortlist",
        "Journal" => "Journal",
        "Pass1" => "Pass 1",
        "Pass2" => "Pass 2",
        "Pass3" => "Pass 3",
        "Pass4" => "Pass 4",
        "Figures" => "Figures",
        "Citations" => "Citations",
        "Numbers" => "Numbers",
        "Compile" => "Compile",
        "Render" => "Render",
        "Disclosure" => "Disclosure",
        "Review" => "Review",
        "Ready" => "Ready",
        _ => return step,
    };
    tr(lang, key)
}
fn marked(id: &'static str, text: String) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
}
fn is_chosen(choice: Option<&crate::desktop::ux::seams::JournalRow>, id: &str, hash: &str) -> bool {
    choice.is_some_and(|c| c.id == id && c.hash == hash)
}
fn can_start(choice: bool, estimate: bool, ack: bool, busy: bool) -> bool {
    choice && estimate && ack && !busy
}

fn start_with_estimate(
    choice: bool,
    estimate: Option<&EstimateRow>,
    ack: bool,
    busy: bool,
) -> bool {
    can_start(choice, estimate.is_some_and(|e| !e.stale), ack, busy)
}

#[derive(Clone)]
struct StatusSnapshot {
    status: Res<PaperView>,
    question: Option<String>,
    models: Option<Res<PassModels>>,
    fresh: Option<Res<bool>>,
    epoch: u64,
}

struct StatusSource {
    service: Arc<dyn PaperService>,
    session: String,
    epoch: Arc<AtomicU64>,
    models: Option<PassModels>,
    previous: Option<String>,
}
impl Source<StatusSnapshot> for StatusSource {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<StatusSnapshot>, Cursor)> {
        let epoch = self.epoch.load(Ordering::SeqCst);
        let status = self.service.status(&self.session);
        let fresh = status
            .as_ref()
            .ok()
            .and_then(|s| s.estimate.as_ref())
            .map(|estimate| {
                self.service
                    .estimate_is_fresh(&self.session, &estimate.digest)
            });
        let models = Some(match &self.models {
            Some(models) => Ok(models.clone()),
            None => self.service.models(&self.session),
        });
        if let Some(Ok(models)) = &models {
            self.models = Some(models.clone());
        }
        let question = if status.as_ref().is_ok_and(|s| s.requirements.is_none()) {
            self.service.evidence_question(&self.session).ok().flatten()
        } else {
            None
        };
        let fingerprint = format!("{status:?}|{question:?}|{models:?}|{fresh:?}|{epoch}");
        if cursor.0 != 0 && self.previous.as_ref() == Some(&fingerprint) {
            return Ok((Delta::Unchanged, cursor));
        }
        self.previous = Some(fingerprint);
        Ok((
            Delta::Replace(StatusSnapshot {
                status,
                question,
                models,
                fresh,
                epoch,
            }),
            Cursor(cursor.0.wrapping_add(1)),
        ))
    }
}

struct SectionSource {
    service: Arc<dyn PaperService>,
    session: String,
    previous: Option<PaperView>,
}
impl Source<PaperView> for SectionSource {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<PaperView>, Cursor)> {
        let status = self.service.status(&self.session)?;
        if cursor.0 != 0 && self.previous.as_ref() == Some(&status) {
            return Ok((Delta::Unchanged, cursor));
        }
        self.previous = Some(status.clone());
        Ok((Delta::Replace(status), Cursor(cursor.0.wrapping_add(1))))
    }
}

#[derive(Clone)]
struct PreviewSnapshot {
    generation: u64,
    result: Res<PreviewView>,
}

fn apply_preview(
    current_generation: u64,
    preview: &mut Option<PreviewView>,
    error: &mut Option<String>,
    snapshot: &PreviewSnapshot,
) {
    if snapshot.generation != current_generation {
        return;
    }
    match &snapshot.result {
        Ok(value) => *preview = Some(value.clone()),
        Err(value) => *error = Some(value.to_string()),
    }
}
struct PreviewSource {
    service: Arc<dyn PaperService>,
    session: String,
    page: u32,
    generation: u64,
    loaded: bool,
}
impl Source<PreviewSnapshot> for PreviewSource {
    fn poll(&mut self, cursor: Cursor) -> Res<(Delta<PreviewSnapshot>, Cursor)> {
        if self.loaded && cursor.0 != 0 {
            return Ok((Delta::Unchanged, cursor));
        }
        self.loaded = true;
        let result = self.service.preview(
            &self.session,
            PreviewInput {
                page: self.page,
                width_px: 720,
            },
        );
        Ok((
            Delta::Replace(PreviewSnapshot {
                generation: self.generation,
                result,
            }),
            Cursor(cursor.0.wrapping_add(1)),
        ))
    }
}

pub struct PaperPage {
    state: Entity<AppState>,
    session: SharedString,
    service: Option<Arc<dyn PaperService>>,
    status: Option<PaperView>,
    estimate: Option<EstimateRow>,
    estimate_stale: bool,
    preview: Option<PreviewView>,
    preview_pdf: Option<std::path::PathBuf>,
    preview_generation: u64,
    error: Option<String>,
    status_error: Option<String>,
    ack: bool,
    pending: bool,
    form: form::Form,
    initialized: bool,
    models_loaded: bool,
    ignore_status_estimate: bool,
    estimate_epoch: Arc<AtomicU64>,
    feed: Option<data::Feed<StatusSnapshot>>,
    status_version: u64,
    status_panel_panicked: bool,
    preview_feed: Option<data::Feed<PreviewSnapshot>>,
    preview_version: u64,
    panel_feeds: std::collections::HashMap<&'static str, data::Feed<PaperView>>,
    panel_panics: std::collections::HashSet<&'static str>,
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
        let estimate_epoch = Arc::new(AtomicU64::new(0));
        let feed = service.as_ref().map(|service| {
            let source = StatusSource {
                service: service.clone(),
                session: session.to_string(),
                epoch: estimate_epoch.clone(),
                models: None,
                previous: None,
            };
            let feed = data::Feed::spawn(
                "paper.page",
                source,
                Duration::from_secs(STATUS_POLL_SECONDS),
                cx,
            );
            feed.observe(cx);
            feed
        });
        let panel_feeds = service
            .as_ref()
            .map(|service| {
                [
                    "paper.form.panel",
                    "paper.shortlist.list",
                    "paper.estimate.summary",
                    "paper.progress.passes",
                    "paper.changelog.log",
                    "paper.preview.document",
                    "paper.readiness.status",
                ]
                .into_iter()
                .map(|id| {
                    let source = SectionSource {
                        service: service.clone(),
                        session: session.to_string(),
                        previous: None,
                    };
                    let feed =
                        data::Feed::spawn(id, source, Duration::from_secs(STATUS_POLL_SECONDS), cx);
                    feed.observe(cx);
                    (id, feed)
                })
                .collect()
            })
            .unwrap_or_default();
        Self {
            state,
            session,
            service,
            status: None,
            estimate: None,
            estimate_stale: false,
            preview: None,
            preview_pdf: None,
            preview_generation: 0,
            error: None,
            status_error: None,
            ack: false,
            pending: false,
            form,
            initialized: false,
            models_loaded: false,
            ignore_status_estimate: false,
            estimate_epoch,
            feed,
            status_version: 0,
            status_panel_panicked: false,
            preview_feed: None,
            preview_version: 0,
            panel_feeds,
            panel_panics: Default::default(),
        }
    }
    fn panel(
        &mut self,
        id: &'static str,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::desktop::components::panel::{Panel, PanelState};
        if crate::desktop::data::faults::active(id, cx)
            == Some(crate::desktop::data::faults::Fault::Panic)
        {
            self.panel_panics.insert(id);
        }
        let feed = self.panel_feeds.get(id).cloned();
        let state = if self.panel_panics.contains(id) {
            PanelState::Failed {
                reason: "panel rendering failed".into(),
            }
        } else {
            feed.as_ref()
                .map(|f| f.snapshot().state)
                .unwrap_or(PanelState::Ready)
        };
        Panel::new(id, &state)
            .retry(cx.listener(move |this, _, _, cx| {
                this.panel_panics.remove(id);
                if let Some(feed) = &feed {
                    feed.retry();
                }
                if id == "paper.preview.document" {
                    if let Some(preview) = &this.preview_feed {
                        preview.retry();
                    }
                }
                cx.notify();
            }))
            .content(move |_, _| content)
            .into_any_element()
    }
    fn update_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(feed) = &self.feed else { return };
        let snapshot = feed.snapshot();
        if snapshot.version == self.status_version {
            return;
        }
        self.status_version = snapshot.version;
        let Some(snapshot) = snapshot.value else {
            return;
        };
        match &snapshot.status {
            Ok(s) => {
                self.status_error = None;
                if !self.initialized && (s.requirements.is_some() || snapshot.question.is_some()) {
                    self.form.initialize(
                        s.requirements.as_ref(),
                        snapshot.question.as_deref(),
                        window,
                        cx,
                    );
                    self.initialized = true;
                }
                if !self.ignore_status_estimate
                    && snapshot.epoch == self.estimate_epoch.load(Ordering::SeqCst)
                {
                    if self.estimate.as_ref().map(|e| &e.digest)
                        != s.estimate.as_ref().map(|e| &e.digest)
                    {
                        self.ack = false;
                    }
                    self.estimate = s.estimate.clone().map(Into::into);
                    self.estimate_stale = self.estimate.as_ref().is_some_and(|e| e.stale)
                        || snapshot
                            .fresh
                            .as_ref()
                            .is_some_and(|fresh| !matches!(fresh, Ok(true)));
                    if let Some(estimate) = &mut self.estimate {
                        estimate.stale = self.estimate_stale;
                    }
                    if self.estimate_stale {
                        self.ack = false;
                    }
                }
                if s.pdf != self.preview_pdf {
                    self.preview_generation = self.preview_generation.wrapping_add(1);
                    self.preview_pdf = s.pdf.clone();
                    self.preview = None;
                    if s.pdf.is_some() {
                        self.load_preview(1, cx);
                    }
                }
                self.status = Some(s.clone());
            }
            Err(e) => self.status_error = Some(e.to_string()),
        }
        if let Some(models) = &snapshot.models {
            match models {
                Ok(models) if !self.models_loaded => {
                    self.form.initialize_models(models, window, cx);
                    self.models_loaded = true;
                }
                Err(e) => self.status_error = Some(e.to_string()),
                _ => {}
            }
        }
    }
    fn action(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(Arc<dyn PaperService>, String) -> Res<Option<EstimateRow>> + Send + 'static,
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
                if let Some(feed) = &this.feed {
                    feed.retry();
                }
                match result {
                    Ok(Some(e)) => {
                        this.estimate_epoch.fetch_add(1, Ordering::SeqCst);
                        this.ignore_status_estimate = false;
                        this.estimate_stale = e.stale;
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
                if let Some(feed) = &this.feed {
                    feed.retry();
                }
                match result {
                    Ok(()) => {
                        this.ack = false;
                        this.estimate = None;
                        this.estimate_stale = false;
                        this.estimate_epoch.fetch_add(1, Ordering::SeqCst);
                        this.ignore_status_estimate = true;
                        if let Some(status) = &mut this.status {
                            status.journal = None;
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
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let source = PreviewSource {
            service,
            session: self.session.to_string(),
            page,
            generation: self.preview_generation,
            loaded: false,
        };
        let feed = data::Feed::spawn(
            "paper.preview.document",
            source,
            Duration::from_secs(PREVIEW_POLL_SECONDS),
            cx,
        );
        feed.observe(cx);
        self.preview_version = 0;
        self.preview_feed = Some(feed);
    }
    fn update_preview(&mut self) {
        let Some(feed) = &self.preview_feed else {
            return;
        };
        let snapshot = feed.snapshot();
        if snapshot.version == self.preview_version {
            return;
        }
        self.preview_version = snapshot.version;
        let Some(snapshot) = snapshot.value else {
            return;
        };
        apply_preview(
            self.preview_generation,
            &mut self.preview,
            &mut self.error,
            &snapshot,
        );
    }
}
impl Render for PaperPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_status(window, cx);
        self.update_preview();
        let lang = *cx.global::<Lang>();
        let t = cx.tokens();
        let chosen = self.status.as_ref().is_some_and(|s| {
            s.shortlist
                .iter()
                .any(|row| !row.excluded && is_chosen(s.journal.as_ref(), &row.id, &row.hash))
        });
        let busy = self.pending || self.status.as_ref().is_some_and(|s| s.running);
        let enabled = start_with_estimate(chosen, self.estimate.as_ref(), self.ack, busy);
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
                div().id("action:paper.back").test_support().child(
                    Button::new("paper-back", tr(lang, "Back"), &t)
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.close_page(cx)))
                        .test_text(tr(lang, "Back")),
                ),
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
        let mut status_content = v_flex();
        if let Some(e) = self.status_error.clone() {
            status_content = status_content.child(marked("paper-error", e));
        }
        if let Some(s) = &self.status {
            if let Some((kind, reason)) = &s.last_error {
                status_content =
                    status_content.child(marked("paper-error", format!("{kind}: {reason}")));
            }
            for gate in &s.steps {
                if let Some(detail) = &gate.detail {
                    status_content = status_content.child(marked(
                        "paper-error",
                        format!(
                            "{} · {} · {detail}",
                            step_label(lang, &gate.label),
                            if gate.ok { "✓" } else { "✗" }
                        ),
                    ));
                }
            }
        } else {
            status_content = status_content.child(tr(lang, "Loading"));
        }
        if crate::desktop::data::faults::active("paper.page", cx)
            == Some(crate::desktop::data::faults::Fault::Panic)
        {
            self.status_panel_panicked = true;
        }
        let feed = self.feed.clone();
        let mut panel_state = feed
            .as_ref()
            .map(|f| f.snapshot().state)
            .unwrap_or(crate::desktop::components::panel::PanelState::Ready);
        if self.status_panel_panicked {
            panel_state = crate::desktop::components::panel::PanelState::Failed {
                reason: "panel rendering failed".into(),
            };
        }
        page = page.child(
            crate::desktop::components::panel::Panel::new("paper.page", &panel_state)
                .retry(cx.listener(move |this, _, _, cx| {
                    this.status_panel_panicked = false;
                    if let Some(feed) = &feed {
                        feed.retry();
                    }
                    cx.notify();
                }))
                .content(move |_, _| status_content.into_any_element()),
        );
        let form = v_flex()
            .id("paper.form.panel")
            .test_support()
            .child(self.form.render(lang, self.models_loaded, self.pending, cx));
        let form = self.panel("paper.form.panel", form.into_any_element(), cx);
        let section_status = |id| {
            self.panel_feeds
                .get(id)
                .and_then(|feed| feed.snapshot().value)
        };
        let list_status = section_status("paper.shortlist.list");
        let passes_status = section_status("paper.progress.passes");
        let logs_status = section_status("paper.changelog.log");
        let readiness_status = section_status("paper.readiness.status");
        let list = v_flex()
            .id("paper.shortlist.list")
            .test_support()
            .children(shortlist::render(list_status.as_deref(), lang, cx));
        let list = self.panel("paper.shortlist.list", list.into_any_element(), cx);
        let passes = gpui_kit::component::h_flex()
            .flex_wrap()
            .id("paper.progress.passes")
            .test_support()
            .children(progress::render(passes_status.as_deref(), lang, &t));
        let passes = self.panel("paper.progress.passes", passes.into_any_element(), cx);
        let logs = v_flex()
            .id("paper.changelog.log")
            .test_support()
            .children(changelog::render(logs_status.as_deref(), lang));
        let logs = self.panel("paper.changelog.log", logs.into_any_element(), cx);
        let preview = v_flex()
            .id("paper.preview.document")
            .test_support()
            .children(preview::render(
                self.preview.as_ref(),
                self.preview_pdf.is_some(),
                lang,
                cx,
            ));
        let preview = self.panel("paper.preview.document", preview.into_any_element(), cx);
        let mut readiness = v_flex()
            .id("paper.readiness.status")
            .test_support()
            .child(marked(
                "paper-readiness-label",
                tr(lang, "Readiness").into(),
            ));
        if let Some(status) = &readiness_status {
            for gate in &status.readiness {
                readiness = readiness.child(marked(
                    "paper-readiness-condition",
                    format!(
                        "{} · {} · {}",
                        gate.label,
                        if gate.ok { "✓" } else { "✗" },
                        gate.detail.as_deref().unwrap_or("")
                    ),
                ));
            }
        }
        let readiness = self.panel("paper.readiness.status", readiness.into_any_element(), cx);
        let estimate = self.estimate.clone();
        let mut sections = vec![
            ("paper.progress.passes", passes),
            ("paper.changelog.log", logs),
            ("paper.preview.document", preview),
            ("paper.shortlist.list", list),
            ("paper.readiness.status", readiness),
        ];
        // Make an unavailable section's retry visible without hiding its siblings.
        sections.sort_by_key(|(id, _)| {
            let failed = self.panel_panics.contains(id)
                || self.panel_feeds.get(id).is_some_and(|feed| {
                    !matches!(
                        feed.snapshot().state,
                        crate::desktop::components::panel::PanelState::Ready
                    )
                });
            !failed
        });
        const FORM_COLUMN_SHARE: f32 = 0.6;
        let content = gpui_kit::component::h_flex()
            .items_start()
            .flex_1()
            .min_h_0()
            .gap(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Sm,
            ))
            .child(
                div()
                    .w(gpui_kit::relative(FORM_COLUMN_SHARE))
                    .flex_shrink_0()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(div().size_full().overflow_y_scrollbar().child(form)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .text_size(crate::desktop::theme::font(
                        crate::desktop::theme::density(cx),
                        crate::desktop::theme::TypeSize::Caption,
                    ))
                    .line_height(
                        crate::desktop::theme::font(
                            crate::desktop::theme::density(cx),
                            crate::desktop::theme::TypeSize::Caption,
                        ) * (4. / 3.),
                    )
                    .children(sections.into_iter().map(|(_, element)| element)),
            );
        let estimate_panel = gpui_kit::component::h_flex()
            .flex_wrap()
            .id("paper.estimate.summary")
            .test_support()
            .gap(crate::desktop::theme::space(
                crate::desktop::theme::density(cx),
                crate::desktop::theme::Step::Sm,
            ))
            .when_some(estimate, |p, e| {
                p.child(marked("paper-estimate", e.line))
                    .when(self.estimate_stale, |p| {
                        p.child(marked(
                            "paper-estimate-stale",
                            tr(lang, "Estimate is stale; request a new estimate").into(),
                        ))
                    })
                    .child(
                        div()
                            .id("action:paper.estimate.acknowledge")
                            .test_support()
                            .child(
                                Button::new("paper-ack", tr(lang, "Acknowledge estimate"), &t)
                                    .variant(if self.ack && !self.estimate_stale {
                                        ButtonVariant::Primary
                                    } else {
                                        ButtonVariant::Secondary
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.estimate_stale || this.estimate.is_none() {
                                            return;
                                        }
                                        this.ack = !this.ack;
                                        cx.notify();
                                    })),
                            ),
                    )
            })
            .child(
                div()
                    .id("action:paper.estimate.request")
                    .test_support()
                    .child(
                        Button::new("paper-estimate-request", tr(lang, "Get estimate"), &t)
                            .variant(if chosen && !busy {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Secondary
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !chosen || busy {
                                    return;
                                }
                                this.action(cx, |s, id| s.estimate(&id).map(Some))
                            })),
                    ),
            )
            .child(
                div().id("action:paper.start").test_support().child(
                    Button::new("paper-start", tr(lang, "Start"), &t)
                        .variant(if enabled {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(e) = this.estimate.clone() {
                                if start_with_estimate(chosen, Some(&e), this.ack, busy) {
                                    this.ack = false;
                                    this.action(cx, move |s, id| {
                                        s.start(
                                            &id,
                                            StartInput {
                                                estimate_digest: e.digest,
                                            },
                                        )
                                        .map(|_| None)
                                    });
                                }
                            }
                        }))
                        .test_text(tr(lang, "Start")),
                ),
            )
            .child(
                div().id("action:paper.cancel").test_support().child(
                    Button::new("paper-cancel", tr(lang, "Cancel"), &t)
                        .variant(if busy {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if busy {
                                this.action(cx, |s, id| s.cancel(&id).map(|_| None))
                            }
                        })),
                ),
            );
        let estimate_panel = self.panel(
            "paper.estimate.summary",
            estimate_panel.into_any_element(),
            cx,
        );
        page = page.child(content).child(estimate_panel);
        page.into_any_element()
    }
}
