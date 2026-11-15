use super::*;
use crate::desktop::ux::seams::{ModeKind, ModeRef};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;

pub struct ChatPage {
    state: Entity<AppState>,
    input: Entity<InputState>,
    thread: String,
}
impl ChatPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(window, cx);
            }
        })
        .detach();
        Self {
            state,
            input,
            thread: unique("chat"),
        }
    }
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        if service(&self.state, cx).is_some_and(|m| m.chat_send(&self.thread, &text).is_ok()) {
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
            cx.notify();
        }
    }
    fn promote(&mut self, cx: &mut Context<Self>) {
        if let Some(context) = service(&self.state, cx)
            .and_then(|m| m.handoff(&ModeRef::Chat(self.thread.clone())).ok())
        {
            self.state.update(cx, |s, cx| {
                s.ux.composer_prefill = Some(context.clone());
                s.open_mode(ModeKind::QuickTask, Some(context), cx);
            });
        }
    }
}
impl Render for ChatPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = *cx.global::<Lang>();
        let Some(modes) = service(&self.state, cx) else {
            return unavailable(self.state.clone(), lang, "chat-page");
        };
        let messages = modes.chat(&self.thread).messages;
        v_flex()
            .id("chat-page")
            .test_support()
            .size_full()
            .p(px(20.))
            .gap(px(10.))
            .child(back(self.state.clone(), lang))
            .child(tr(lang, "Chat"))
            .child(
                v_flex()
                    .flex_1()
                    .overflow_y_scrollbar()
                    .children(messages.into_iter().map(|m| {
                        div().id(("chat-message-slot", m.id as usize)).child(marked(
                            "chat-message",
                            format!(
                                "{} {}: {}",
                                glyph(m.from),
                                display_name(m.from, lang),
                                m.text
                            ),
                        ))
                    })),
            )
            .child(Input::new(&self.input).id("chat-input"))
            .child(
                Button::new("chat-send")
                    .label(tr(lang, "Send"))
                    .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
            )
            .child(
                Button::new("mode-promote")
                    .label(tr(lang, "Make it a quick task"))
                    .on_click(cx.listener(|this, _, _, cx| this.promote(cx))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::ux::seams::{FakeModes, ModeEngine};
    use std::prelude::v1::test;

    #[test]
    fn chat_handoff_preserves_both_sides_for_the_quick_request() {
        let modes = FakeModes::new();
        modes.chat_send("thread", "Investigate this").unwrap();
        let context = modes.handoff(&ModeRef::Chat("thread".into())).unwrap();
        let request = crate::desktop::ux::seams::QuickRequest {
            cwd: None,
            text: "Fix it".into(),
            review: false,
            context: Some(context.clone()),
        };
        let id = modes.quick_start(&request).unwrap();
        assert_eq!(
            modes.quick(&id).unwrap().request.context.as_deref(),
            Some(context.as_str())
        );
        assert!(context.contains("Investigate this"));
        assert!(context.contains("identify the next step"));
    }
}
