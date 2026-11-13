//! Adds tooltip text to a child button trigger.
use super::button::Button;
use gpui_kit::{App, IntoElement, RenderOnce, SharedString, Window};

/// Associates help text with an existing button without creating a second trigger.
#[derive(IntoElement)]
pub struct Tooltip {
    trigger: Button,
    text: SharedString,
}
impl Tooltip {
    pub fn new(trigger: Button, text: impl Into<SharedString>) -> Self {
        Self {
            trigger,
            text: text.into(),
        }
    }
    pub(super) fn into_trigger(self) -> Button {
        self.trigger.tooltip(self.text)
    }
}
impl RenderOnce for Tooltip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_trigger()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::{button_colors, ButtonVariant};
    use gpui_kit::ElementId;
    #[test]
    fn hover_and_tooltip_do_not_register_two_hover_listeners() {
        use gpui_kit::{div, InteractiveElement, ParentElement, StatefulInteractiveElement};
        for t in crate::desktop::theme::both() {
            let mut parent = div();
            for id in ["first", "second"] {
                let button = Tooltip::new(Button::new(id, "Help", &t), "Details").into_trigger();
                let trigger = div().id(button.id.clone()).on_hover(|_, _, _| {});
                parent = parent.child(super::super::button::hover_container(
                    button.id,
                    trigger,
                    |_, _, _| {},
                ));
            }
            drop(parent);
        }
    }
    #[test]
    fn forwards_text_to_child_button() {
        for t in crate::desktop::theme::both() {
            let trigger = Button::new("trigger", "Help", &t)
                .variant(ButtonVariant::Danger)
                .child("icon");
            let button = Tooltip::new(trigger, "Details").into_trigger();
            assert_eq!(button.id, ElementId::from("trigger"));
            assert_eq!(button.tooltip.as_deref(), Some("Details"));
            assert_eq!(button.children.len(), 1);
            assert_eq!(
                button_colors(button.variant, &button.tokens).background,
                t.fail
            );
        }
    }
}
