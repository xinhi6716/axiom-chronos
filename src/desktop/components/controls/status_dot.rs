//! Focusable, token-coloured status indicator.
use super::button::Button;
use crate::desktop::components::tokens::{
    status_color, ButtonVariant, Status, Tokens, STATUS_DOT_SIZE,
};
use gpui_kit::{
    div, App, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, Styled, Window,
};

#[derive(IntoElement)]
pub struct StatusDot {
    button: Button,
    status: Status,
    tokens: Tokens,
}
impl StatusDot {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        status: Status,
        tokens: &Tokens,
    ) -> Self {
        Self {
            button: Button::new(id, label, tokens).variant(ButtonVariant::Ghost),
            status,
            tokens: *tokens,
        }
    }
    pub fn status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.button = self.button.track_focus(focus);
        self
    }
}
impl StatusDot {
    pub(super) fn into_button(self) -> Button {
        self.button.child(
            div()
                .size(STATUS_DOT_SIZE)
                .rounded_full()
                .bg(status_color(self.status, &self.tokens)),
        )
    }
}
impl RenderOnce for StatusDot {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_button()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let dot = StatusDot::new("status", "Status", Status::Running, &t).status(Status::Error);
            assert_eq!(dot.button.id, ElementId::from("status"));
            assert_eq!(status_color(dot.status, &dot.tokens), t.fail);
            assert_eq!(
                [
                    Status::Idle,
                    Status::Running,
                    Status::Success,
                    Status::Warning,
                    Status::Error
                ]
                .map(|s| status_color(s, &t)),
                [t.idle, t.run, t.ok, t.review, t.fail]
            );
        }
    }
}
