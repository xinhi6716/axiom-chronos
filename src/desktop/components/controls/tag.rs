//! Focusable, role-coloured tag.
use super::{
    button::{interaction_style, Interaction},
    toolbar::Toolbar,
};
use crate::desktop::components::tokens::{
    font, radius, space, ButtonVariant, Density, Radius, Step, Tokens, TypeSize,
};
use crate::desktop::motion::Driver;
use crate::desktop::theme;
use gpui_kit::{
    div, AnyElement, App, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, Styled, Window,
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
    custom_colors: Option<(Hsla, Hsla, Hsla)>,
    children: Vec<AnyElement>,
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
            custom_colors: None,
            children: Vec::new(),
        }
    }
    /// Colours should be derived from the active design tokens or semantic roles.
    pub fn custom(mut self, fill: Hsla, text: Hsla, border: Hsla) -> Self {
        self.custom_colors = Some((fill, text, border));
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
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
            .child(
                div()
                    .text_color(color)
                    .text_size(size)
                    .child(self.label)
                    .children(self.children),
            )
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let density = theme::density(cx);
        if let Some((fill, text, border)) = self.custom_colors {
            let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
                Interaction::new(cx.focus_handle())
            });
            let focus = state.read(cx).focus.clone().tab_stop(true);
            let mut driver = Driver::new(cx.reduce_motion(), cx);
            let values = state.read(cx);
            let style = interaction_style(
                &values.hover,
                &values.press,
                driver.now(),
                driver.reduce(),
                focus.is_focused(window),
                &self.tokens,
            );
            driver.keep(style.moving);
            driver.finish(window);
            div()
                .id(self.id)
                .track_focus(&focus)
                .flex()
                .items_center()
                .gap(space(Density::Comfortable, Step::Xs))
                .p(space(Density::Comfortable, Step::Xs))
                .rounded(radius(Radius::Sm))
                .border_1()
                .border_color(border)
                .bg(fill)
                .text_color(text)
                .text_size(font(density, TypeSize::Label))
                .shadow(style.ring)
                .child(self.label)
                .children(self.children)
                .into_any_element()
        } else {
            self.into_toolbar(density).into_any_element()
        }
    }
}

#[cfg(test)]
mod custom_tests {
    use super::*;
    #[test]
    fn custom_colours_and_content_are_preserved() {
        for t in theme::both() {
            let tag = Tag::new("tag", "label", &t)
                .custom(t.surface, t.run, t.border)
                .child("extra");
            assert_eq!(tag.custom_colors, Some((t.surface, t.run, t.border)));
            assert_eq!(tag.children.len(), 1);
        }
    }
}
