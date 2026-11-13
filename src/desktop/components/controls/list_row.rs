//! A token-coloured, keyboard-operable list row.
use super::button::Button;
use crate::desktop::components::tokens::{ButtonVariant, Radius, Tokens};
use gpui_kit::{
    App, ClickEvent, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, Window,
};

#[derive(IntoElement)]
pub struct ListRow {
    button: Button,
}

impl ListRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            button: Button::new(id, label, tokens)
                .variant(ButtonVariant::Ghost)
                .rounded(Radius::Md),
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.button = self.button.variant(variant);
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.button = self.button.track_focus(focus);
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.button = self.button.on_click(callback);
        self
    }
    /// Append a cell to the row.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.button = self.button.child(child);
        self
    }
}
impl ListRow {
    pub(super) fn into_button(self) -> Button {
        self.button
    }
}
impl RenderOnce for ListRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_button()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::button_colors;
    #[test]
    fn render_forwards_button_configuration() {
        for t in crate::desktop::theme::both() {
            let control = ListRow::new("control", "Label", &t)
                .variant(ButtonVariant::Danger)
                .child("detail");
            let button = control.into_button();
            assert_eq!(button.id, ElementId::from("control"));
            assert_eq!(button.rounded, Radius::Md);
            assert_eq!(button.children.len(), 1);
            assert_eq!(
                button_colors(button.variant, &button.tokens).background,
                t.fail
            );
        }
    }
}
