//! Focusable alert with an optional action.
use super::{tag::TagTone, toolbar::Toolbar};
use crate::desktop::components::tokens::{font, space, ButtonVariant, Step, Tokens, TypeSize};
use crate::desktop::theme;
use gpui_kit::{
    div, AnyElement, App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window,
};
#[derive(IntoElement)]
pub struct Alert {
    id: ElementId,
    title: SharedString,
    body: SharedString,
    tone: TagTone,
    action: Option<AnyElement>,
    tokens: Tokens,
}
impl Alert {
    fn resolved_style(
        &self,
        d: theme::Density,
    ) -> (
        gpui_kit::Pixels,
        Hsla,
        gpui_kit::Pixels,
        Hsla,
        gpui_kit::Pixels,
    ) {
        (
            space(d, Step::Xs),
            self.color(),
            font(d, TypeSize::Title),
            self.tokens.text,
            font(d, TypeSize::Body),
        )
    }

    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            tone: TagTone::Info,
            action: None,
            tokens: *tokens,
        }
    }
    pub fn tone(mut self, tone: TagTone) -> Self {
        self.tone = tone;
        self
    }
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
    fn color(&self) -> Hsla {
        self.tone.color(&self.tokens)
    }
}
impl Alert {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let (gap, color, title_size, body_color, body_size) = self.resolved_style(d);
        let content = div()
            .flex()
            .flex_col()
            .gap(gap)
            .child(
                div()
                    .text_color(color)
                    .text_size(title_size)
                    .child(self.title),
            )
            .child(
                div()
                    .text_color(body_color)
                    .text_size(body_size)
                    .child(self.body),
            )
            .children(self.action);
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Secondary)
            .child(content)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_alert_style_and_action() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for (tone, color) in [
                    (TagTone::Neutral, t.muted),
                    (TagTone::Info, t.run),
                    (TagTone::Success, t.ok),
                    (TagTone::Warning, t.review),
                    (TagTone::Danger, t.fail),
                ] {
                    let alert = Alert::new("alert", "Title", "Body", &t)
                        .tone(tone)
                        .action("Action");
                    assert_eq!(
                        alert.resolved_style(d),
                        (
                            space(d, Step::Xs),
                            color,
                            font(d, TypeSize::Title),
                            t.text,
                            font(d, TypeSize::Body)
                        )
                    );
                    assert_eq!(alert.title.as_ref(), "Title");
                    assert_eq!(alert.body.as_ref(), "Body");
                    assert!(alert.action.is_some());
                    assert_eq!(alert.into_toolbar(d).id, ElementId::from("alert"));
                }
            }
        }
    }
}

impl RenderOnce for Alert {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
