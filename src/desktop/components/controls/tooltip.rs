//! Adds tooltip text to a child button trigger.
use super::button::Button;
use crate::desktop::components::tokens::{
    elevation, font, radius, space, Elevation, Radius, Step, Tokens, TypeSize,
};
use crate::desktop::theme::{self, Tokened};
use gpui_kit::{
    div, AnyView, App, AppContext, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, RenderOnce, SharedString, Styled, Window,
};

struct TooltipContent {
    id: ElementId,
    text: SharedString,
    max_width: Option<Pixels>,
    tokens: Tokens,
}
impl TooltipContent {
    fn new(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        max_width: Option<Pixels>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            max_width,
            tokens: *tokens,
        }
    }
}
impl Render for TooltipContent {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = theme::density(cx);
        let mut content = div()
            .id(self.id.clone())
            .bg(self.tokens.raised)
            .text_color(self.tokens.text)
            .border_1()
            .border_color(self.tokens.border)
            .rounded(radius(Radius::Sm))
            .shadow(elevation(Elevation::Overlay, &self.tokens))
            .px(space(d, Step::Sm))
            .py(space(d, Step::Xs))
            .text_size(font(d, TypeSize::Caption))
            .child(self.text.clone());
        if let Some(width) = self.max_width {
            content = content.max_w(width);
        }
        content
    }
}

/// Build token-styled tooltip content inside a GPUI tooltip closure.
pub fn tooltip(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    max_width: Option<Pixels>,
    _: &mut Window,
    cx: &mut App,
) -> AnyView {
    let tokens = cx.tokens();
    let id = id.into();
    let text = text.into();
    cx.new(|_| TooltipContent::new(id, text, max_width, &tokens))
        .into()
}

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
    #[test]
    fn tooltip_content_keeps_id_text_and_width() {
        let t = crate::desktop::theme::night();
        let content = TooltipContent::new(
            "tip",
            "Details",
            Some(crate::desktop::theme::TARGET_MIN),
            &t,
        );
        assert_eq!(content.id, ElementId::from("tip"));
        assert_eq!(content.text.as_ref(), "Details");
        assert_eq!(content.max_width, Some(crate::desktop::theme::TARGET_MIN));
        let _: fn(
            ElementId,
            SharedString,
            Option<gpui_kit::Pixels>,
            &mut Window,
            &mut App,
        ) -> gpui_kit::AnyView = tooltip;
    }
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
