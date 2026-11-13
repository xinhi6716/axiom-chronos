//! Focusable, role-coloured tag.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{font, ButtonVariant, Tokens, TypeSize};
use crate::desktop::theme;
use gpui_kit::{
    div, App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TagTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}
impl TagTone {
    pub fn color(self, t: &Tokens) -> Hsla {
        match self {
            Self::Neutral => t.muted,
            Self::Info => t.run,
            Self::Success => t.ok,
            Self::Warning => t.review,
            Self::Danger => t.fail,
        }
    }
}
#[derive(IntoElement)]
pub struct Tag {
    id: ElementId,
    label: SharedString,
    tone: TagTone,
    tokens: Tokens,
}
impl Tag {
    fn resolved_style(&self, d: theme::Density) -> (Hsla, gpui_kit::Pixels) {
        (self.color(), font(d, TypeSize::Label))
    }

    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tone: TagTone::Neutral,
            tokens: *tokens,
        }
    }
    pub fn tone(mut self, tone: TagTone) -> Self {
        self.tone = tone;
        self
    }
    fn color(&self) -> Hsla {
        self.tone.color(&self.tokens)
    }
}
impl Tag {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let (color, size) = self.resolved_style(d);
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Secondary)
            .child(div().text_color(color).text_size(size).child(self.label))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_tag_style() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for (tone, color) in [
                    (TagTone::Neutral, t.muted),
                    (TagTone::Info, t.run),
                    (TagTone::Success, t.ok),
                    (TagTone::Warning, t.review),
                    (TagTone::Danger, t.fail),
                ] {
                    let tag = Tag::new("tag", "Label", &t).tone(tone);
                    assert_eq!(tag.resolved_style(d), (color, font(d, TypeSize::Label)));
                    assert_eq!(tag.into_toolbar(d).id, ElementId::from("tag"));
                }
            }
        }
    }
}

impl RenderOnce for Tag {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
