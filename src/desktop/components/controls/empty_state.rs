//! A focusable empty-state message with optional caller-supplied content.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{ButtonVariant, Tokens};
use gpui_kit::{App, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, Window};

#[derive(IntoElement)]
pub struct EmptyState {
    toolbar: Toolbar,
    variant: ButtonVariant,
}
impl EmptyState {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens).child(message.into()),
            variant: ButtonVariant::Ghost,
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.toolbar = self.toolbar.track_focus(focus);
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.toolbar = self.toolbar.child(child);
        self
    }
}
impl EmptyState {
    pub(super) fn into_toolbar(self) -> Toolbar {
        self.toolbar.variant(self.variant)
    }
}
impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_toolbar()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::button_colors;
    #[test]
    fn identity_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let empty = EmptyState::new("empty", "No items", &t).variant(ButtonVariant::Danger);
            assert_eq!(empty.toolbar.id, ElementId::from("empty"));
            assert_eq!(button_colors(empty.variant, &t).background, t.fail);
        }
    }
}
