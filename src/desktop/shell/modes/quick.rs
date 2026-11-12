use super::*;
use crate::desktop::ux::seams::{ModeKind, ModeRef, QuickRequest, QuickStep};
use gpui_kit::component::{
    input::{Input, InputEvent, InputState},
    Selectable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use std::time::Duration;

pub struct QuickTaskPage {
    state: Entity<AppState>,
    input: Entity<InputState>,
    review: bool,
    context: Option<String>,
    id: Option<String>,
    timer: Option<Task<()>>,
}
impl QuickTaskPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.start(cx);
            }
        })
        .detach();
        let context = state.update(cx, |s, _| s.ux.composer_prefill.take());
        Self {
            state,
            input,
            review: false,
            context,
            id: None,
            timer: None,
        }
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.id.is_some() {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let request = QuickRequest {
            cwd: self.state.read(cx).run().and_then(|r| r.cwd.clone()),
            text,
            review: self.review,
            context: self.context.clone(),
        };
        if let Some(id) = service(&self.state, cx).and_then(|m| m.quick_start(&request).ok()) {
            self.id = Some(id);
            self.poll(cx);
            cx.notify();
        }
    }
    fn poll(&mut self, cx: &mut Context<Self>) {
        self.timer = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let keep = this
                .update(cx, |this, cx| {
                    let active = this
                        .id
                        .as_ref()
                        .and_then(|id| service(&this.state, cx).and_then(|m| m.quick(id)))
                        .is_some_and(|t| {
                            !matches!(t.step, QuickStep::Done { .. } | QuickStep::Failed { .. })
                        });
                    cx.notify();
                    active
                })
                .unwrap_or(false);
            if !keep {
                break;
            }
        }));
    }
    fn promote(&mut self, cx: &mut Context<Self>) {
        let Some(id) = &self.id else { return };
        if let Some(context) =
            service(&self.state, cx).and_then(|m| m.handoff(&ModeRef::Quick(id.clone())).ok())
        {
            self.state
                .update(cx, |s, cx| s.open_mode(ModeKind::Wish, Some(context), cx));
        }
    }
}
impl Render for QuickTaskPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let Some(m) = service(&self.state, cx) else {
            return unavailable(self.state.clone(), lang, "quick-page");
        };
        let task = self.id.as_ref().and_then(|id| m.quick(id));
        let mut page = v_flex()
            .id("quick-page")
            .test_support()
            .size_full()
            .p(px(20.))
            .gap(px(10.))
            .child(back(self.state.clone(), lang))
            .child(tr(lang, "Quick task"));
        if let Some(task) = task {
            let (stage, detail) = match task.step {
                QuickStep::Queued => ("Queued", None),
                QuickStep::Building => ("Building", None),
                QuickStep::Verifying => ("Verifying", None),
                QuickStep::Reviewing => ("Reviewing", None),
                QuickStep::Done { summary } => ("Done", Some(summary)),
                QuickStep::Failed { reason } => ("Failed", Some(reason)),
            };
            page = page
                .child(task.request.text)
                .child(marked("quick-step", tr(lang, stage).into()))
                .when(task.request.review, |p| {
                    p.child(marked("quick-review", tr(lang, "Review").into()))
                })
                .children(detail.map(|text| marked("quick-result", text)))
                .child(
                    Button::new("mode-promote")
                        .label(tr(lang, "Make it a wish"))
                        .on_click(cx.listener(|this, _, _, cx| this.promote(cx))),
                );
        } else if self.id.is_none() {
            page = page
                .child(Input::new(&self.input).id("quick-input"))
                .child(
                    Button::new("quick-review-toggle")
                        .label(tr(lang, "Review"))
                        .selected(self.review)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review = !this.review;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("quick-start")
                        .label(tr(lang, "Start"))
                        .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                );
        }
        page.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::ux::seams::{FakeModes, ModeEngine};
    use std::prelude::v1::test;

    #[test]
    fn reviewed_quick_task_reaches_done_and_hands_off_its_request() {
        let modes = FakeModes::new();
        let request = QuickRequest {
            cwd: None,
            text: "Verify change".into(),
            review: true,
            context: Some("Prior chat".into()),
        };
        let id = modes.quick_start(&request).unwrap();
        for expected in [
            QuickStep::Building,
            QuickStep::Verifying,
            QuickStep::Reviewing,
        ] {
            modes.step_quick(&id);
            assert_eq!(modes.quick(&id).unwrap().step, expected);
        }
        modes.step_quick(&id);
        assert!(matches!(
            modes.quick(&id).unwrap().step,
            QuickStep::Done { .. }
        ));
        assert_eq!(modes.handoff(&ModeRef::Quick(id)).unwrap(), "Verify change");
    }
}
