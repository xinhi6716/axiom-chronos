//! Focusable shortcut keycaps.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{
    font, radius, space, ButtonVariant, Radius, Step, Tokens, TypeSize,
};
use crate::desktop::theme;
use gpui_kit::{
    div, App, ElementId, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
};
#[derive(IntoElement)]
pub struct Kbd {
    id: ElementId,
    keys: Vec<SharedString>,
    tokens: Tokens,
    text_size: TypeSize,
}
impl Kbd {
    fn resolved_style(
        &self,
        d: theme::Density,
    ) -> (
        gpui_kit::Pixels,
        gpui_kit::Pixels,
        gpui_kit::Hsla,
        gpui_kit::Hsla,
        gpui_kit::Hsla,
        gpui_kit::Pixels,
    ) {
        (
            space(d, Step::Xs),
            radius(Radius::Sm),
            self.tokens.border,
            self.tokens.raised,
            self.tokens.text,
            font(d, self.text_size),
        )
    }

    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            keys: Vec::new(),
            tokens: *tokens,
            text_size: TypeSize::Caption,
        }
    }
    pub fn key(mut self, label: impl Into<SharedString>) -> Self {
        self.keys.push(label.into());
        self
    }
    pub fn text_size(mut self, size: TypeSize) -> Self {
        self.text_size = size;
        self
    }
}
impl Kbd {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let (padding, rounding, border, background, text, size) = self.resolved_style(d);
        let keys = self.keys.into_iter().map(|key| {
            div()
                .px(padding)
                .rounded(rounding)
                .border_1()
                .border_color(border)
                .bg(background)
                .text_color(text)
                .text_size(size)
                .child(key)
        });
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Ghost)
            .child(div().flex().gap(padding).children(keys))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_option_uses_caption_token() {
        let t = theme::night();
        let kbd = Kbd::new("caption", &t).text_size(TypeSize::Caption);
        assert_eq!(
            kbd.resolved_style(theme::Density::Dense).5,
            font(theme::Density::Dense, TypeSize::Caption)
        );
        let body = Kbd::new("body", &t).text_size(TypeSize::Body);
        assert_eq!(
            body.resolved_style(theme::Density::Dense).5,
            font(theme::Density::Dense, TypeSize::Body)
        );
    }
    #[test]
    fn resolved_keycap_style_and_labels() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                let kbd = Kbd::new("shortcut", &t).key("Ctrl").key("K");
                assert_eq!(
                    kbd.resolved_style(d),
                    (
                        space(d, Step::Xs),
                        radius(Radius::Sm),
                        t.border,
                        t.raised,
                        t.text,
                        font(d, TypeSize::Caption)
                    )
                );
                assert_eq!(
                    kbd.keys.iter().map(|s| s.as_ref()).collect::<Vec<&str>>(),
                    vec!["Ctrl", "K"]
                );
                assert_eq!(kbd.into_toolbar(d).id, ElementId::from("shortcut"));
            }
        }
    }
}

impl RenderOnce for Kbd {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
