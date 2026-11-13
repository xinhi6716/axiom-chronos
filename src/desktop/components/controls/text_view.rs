//! Focusable selectable, wrapped read-only text.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{font, ButtonVariant, Tokens, TypeSize};
use crate::desktop::theme;
use gpui_kit::base::SelectableText;
use gpui_kit::{
    div, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
};
#[derive(IntoElement)]
pub struct TextView {
    id: ElementId,
    text: SharedString,
    size: TypeSize,
    tokens: Tokens,
}
impl TextView {
    fn resolved_style(&self, d: theme::Density) -> (gpui_kit::Pixels, gpui_kit::Hsla) {
        (font(d, self.size), self.tokens.text)
    }
    fn text_id(&self) -> ElementId {
        (self.id.clone(), "text").into()
    }

    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            size: TypeSize::Body,
            tokens: *tokens,
        }
    }
    pub fn size(mut self, size: TypeSize) -> Self {
        self.size = size;
        self
    }
}
impl TextView {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let text_id = self.text_id();
        let (size, color) = self.resolved_style(d);
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Ghost)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_color(color)
                    .text_size(size)
                    .child(SelectableText::new(text_id, self.text)),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_text_style_and_identity() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for size in [
                    TypeSize::Caption,
                    TypeSize::Body,
                    TypeSize::Label,
                    TypeSize::Title,
                    TypeSize::Heading,
                    TypeSize::Display,
                ] {
                    let view = TextView::new("text", "A long text", &t).size(size);
                    assert_eq!(view.resolved_style(d), (font(d, size), t.text));
                    assert_eq!(view.text_id(), (ElementId::from("text"), "text").into());
                    assert_eq!(view.text.as_ref(), "A long text");
                    assert_eq!(view.into_toolbar(d).id, ElementId::from("text"));
                }
            }
        }
    }
}

impl RenderOnce for TextView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
