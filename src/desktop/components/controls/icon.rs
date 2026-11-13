//! Focusable icon using a token size and color role.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{space, ButtonVariant, Step, Tokens};
use crate::desktop::theme;
use gpui_kit::component::{Icon as KitIcon, IconName};
use gpui_kit::{App, ElementId, Hsla, IntoElement, RenderOnce, Styled, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconColor {
    Text,
    Muted,
    Info,
    Success,
    Warning,
    Danger,
}
impl IconColor {
    pub fn resolve(self, t: &Tokens) -> Hsla {
        match self {
            Self::Text => t.text,
            Self::Muted => t.muted,
            Self::Info => t.run,
            Self::Success => t.ok,
            Self::Warning => t.review,
            Self::Danger => t.fail,
        }
    }
}
#[derive(IntoElement)]
pub struct Icon {
    id: ElementId,
    name: IconName,
    size: Step,
    color: IconColor,
    tokens: Tokens,
}
impl Icon {
    fn resolved_style(&self, d: theme::Density) -> (gpui_kit::Pixels, Hsla) {
        (space(d, self.size), self.color_value())
    }

    pub fn new(id: impl Into<ElementId>, name: IconName, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            name,
            size: Step::Lg,
            color: IconColor::Text,
            tokens: *tokens,
        }
    }
    pub fn size(mut self, size: Step) -> Self {
        self.size = size;
        self
    }
    pub fn color(mut self, color: IconColor) -> Self {
        self.color = color;
        self
    }
    fn color_value(&self) -> Hsla {
        self.color.resolve(&self.tokens)
    }
}
impl Icon {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let (size, color) = self.resolved_style(d);
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Ghost)
            .child(KitIcon::new(self.name).size(size).text_color(color))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_icon_style() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for size in [Step::Xs, Step::Sm, Step::Md, Step::Lg, Step::Xl, Step::Xxl] {
                    for (role, color) in [
                        (IconColor::Text, t.text),
                        (IconColor::Muted, t.muted),
                        (IconColor::Info, t.run),
                        (IconColor::Success, t.ok),
                        (IconColor::Warning, t.review),
                        (IconColor::Danger, t.fail),
                    ] {
                        let icon = Icon::new("icon", IconName::Plus, &t).size(size).color(role);
                        assert_eq!(icon.resolved_style(d), (space(d, size), color));
                        assert_eq!(icon.into_toolbar(d).id, ElementId::from("icon"));
                    }
                }
            }
        }
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
