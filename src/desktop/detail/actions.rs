use super::super::{
    i18n::{self, Lang, Table},
    theme::{self, Step},
    ux::seams::{ActionReceipt, UnitActionKind, UnitActions, UnitEvidence},
};
use super::{AppState, Selection, UnitState};
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
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

const TABLE: Table = &[
    ("Evidence", "證據"),
    ("Diff", "差異"),
    ("Review", "審查"),
    ("Cost", "費用"),
    ("Failure", "失敗原因"),
    ("No evidence recorded for this unit", "這個單元沒有留下證據"),
    ("Loading…", "讀取中…"),
    ("Retry", "重試"),
    ("Waive…", "豁免…"),
    ("Confirm waiver", "確認豁免"),
];
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);
pub fn can_waive(reason: &str) -> bool {
    !reason.trim().is_empty()
}
fn marked(id: &'static str, text: String) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .child(text.clone().test_text(text))
        .into_any_element()
}

pub struct UnitActionsPanel {
    state: Entity<AppState>,
    service: Arc<dyn UnitActions>,
    selected: Option<(String, String, UnitState)>,
    evidence: Option<crate::base::Res<UnitEvidence>>,
    reason: Entity<InputState>,
    waive_open: bool,
    in_flight: bool,
    key: String,
    receipt: Option<crate::base::Res<ActionReceipt>>,
}
impl UnitActionsPanel {
    pub fn new(
        state: Entity<AppState>,
        service: Arc<dyn UnitActions>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let reason = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&reason, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        cx.observe(&state, |this, _, cx| this.refresh(cx)).detach();
        let mut panel = Self {
            state,
            service,
            selected: None,
            evidence: None,
            reason,
            waive_open: false,
            in_flight: false,
            key: String::new(),
            receipt: None,
        };
        panel.refresh(cx);
        panel
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let app = self.state.read(cx);
        let selected = app.run().and_then(|run| match app.selection {
            Selection::Unit(i) => run
                .units
                .get(i)
                .map(|u| (run.session.to_string(), u.id.to_string(), u.state)),
            _ => None,
        });
        if selected
            .as_ref()
            .zip(self.selected.as_ref())
            .is_some_and(|(new, old)| new.0 == old.0 && new.1 == old.1)
        {
            self.selected = selected;
            cx.notify();
            return;
        }
        if selected == self.selected {
            return;
        }
        self.selected = selected.clone();
        self.evidence = None;
        self.receipt = None;
        self.in_flight = false;
        self.waive_open = false;
        self.key = format!("unit-action:{}", NEXT_KEY.fetch_add(1, Ordering::Relaxed));
        if let Some((session, unit, _)) = selected {
            let service = self.service.clone();
            let s = session.clone();
            let u = unit.clone();
            let task = cx.background_spawn(async move { service.evidence(&s, &u) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this
                        .selected
                        .as_ref()
                        .is_some_and(|x| x.0 == session && x.1 == unit)
                    {
                        this.evidence = Some(result);
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }
    fn act(&mut self, kind: UnitActionKind, cx: &mut Context<Self>) {
        if self.in_flight || self.receipt.as_ref().is_some_and(|r| r.is_ok()) {
            return;
        }
        if matches!(&kind, UnitActionKind::Waive { reason, .. } if !can_waive(reason)) {
            return;
        }
        let Some((session, unit, _)) = self.selected.clone() else {
            return;
        };
        self.in_flight = true;
        let action = match &kind {
            UnitActionKind::Retry => "retry",
            UnitActionKind::Waive { .. } => "waive",
        };
        let key = format!("{}:{action}", self.key);
        let service = self.service.clone();
        let task = cx.background_spawn(async move { service.act(&session, &unit, &kind, &key) });
        let current_key = self.key.clone();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.key == current_key {
                    this.in_flight = false;
                    this.receipt = Some(result);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
impl Render for UnitActionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let tr = |key| i18n::translate(lang, TABLE, key);
        let d = theme::density(cx);
        let done = self.in_flight || self.receipt.as_ref().is_some_and(|r| r.is_ok());
        let failed = self
            .selected
            .as_ref()
            .is_some_and(|x| x.2 == UnitState::Failed);
        let mut panel = v_flex()
            .id("unit-evidence")
            .test_support()
            .gap(theme::space(d, Step::Sm))
            .child(tr("Evidence").test_text(tr("Evidence")));
        match &self.evidence {
            None => panel = panel.child(marked("unit-evidence-status", tr("Loading…").to_owned())),
            Some(Err(e)) if e.kind == "not_found" => {
                panel = panel.child(marked(
                    "unit-evidence-status",
                    tr("No evidence recorded for this unit").to_owned(),
                ))
            }
            Some(Err(e)) => panel = panel.child(marked("unit-evidence-status", e.message.clone())),
            Some(Ok(e)) => {
                if let Some(v) = &e.diff {
                    panel = panel.child(marked("unit-diff", format!("{}: {}", tr("Diff"), v)));
                }
                if let Some(v) = &e.review {
                    panel = panel.child(marked("unit-review", format!("{}: {}", tr("Review"), v)));
                }
                panel = panel.child(marked(
                    "unit-cost",
                    format!(
                        "{}: {}",
                        tr("Cost"),
                        e.cost_usd
                            .map_or_else(|| "—".to_owned(), |n| format!("${n:.2}"))
                    ),
                ));
                if let Some(v) = &e.failure {
                    panel =
                        panel.child(marked("unit-failure", format!("{}: {}", tr("Failure"), v)));
                }
            }
        }
        panel = panel.when(failed, |panel| {
            panel.child(
                h_flex()
                    .gap(theme::space(d, Step::Sm))
                    .child(
                        Button::new("unit-retry")
                            .small()
                            .tooltip(tr("Retry"))
                            .child(tr("Retry").test_text(tr("Retry")))
                            .disabled(done)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.act(UnitActionKind::Retry, cx)),
                            ),
                    )
                    .child(
                        Button::new("unit-waive")
                            .small()
                            .tooltip(tr("Waive…"))
                            .child(tr("Waive…").test_text(tr("Waive…")))
                            .disabled(done)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reason
                                    .update(cx, |reason, cx| reason.set_value("", window, cx));
                                this.waive_open = true;
                                cx.notify();
                            })),
                    ),
            )
        });
        if self.waive_open {
            panel = panel
                .child(
                    Input::new(&self.reason)
                        .id("unit-waive-reason")
                        .small()
                        .disabled(done),
                )
                .child(
                    Button::new("unit-waive-confirm")
                        .small()
                        .tooltip(tr("Confirm waiver"))
                        .child(tr("Confirm waiver").test_text(tr("Confirm waiver")))
                        .disabled(done || !can_waive(&self.reason.read(cx).value()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            let reason = this.reason.read(cx).value().to_string();
                            this.act(
                                UnitActionKind::Waive {
                                    reason,
                                    finding: None,
                                },
                                cx,
                            );
                        })),
                );
        }
        if let Some(receipt) = &self.receipt {
            panel = panel.child(marked(
                "unit-action-receipt",
                match receipt {
                    Ok(r) => r.message.clone(),
                    Err(e) => e.message.clone(),
                },
            ));
        }
        panel
    }
}
