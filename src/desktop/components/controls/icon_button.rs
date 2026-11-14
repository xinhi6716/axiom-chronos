//! An icon-only button with an explicit accessible name.
use super::button::Button;
use crate::desktop::components::tokens::{ButtonVariant, Tokens};
use gpui_kit::component::IconName;
use gpui_kit::{
    App, ClickEvent, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, Window,
};

#[derive(IntoElement)]
pub struct IconButton {
    button: Button,
}

impl IconButton {
    pub fn new(
        id: impl Into<ElementId>,
        icon: IconName,
        label: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            button: Button::new(id, label, tokens).icon_only(icon),
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.button = self.button.variant(variant);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.button = self.button.loading(loading);
        self
    }
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.button = self.button.tooltip(text);
        self
    }
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.button = self.button.accessibility_label(label);
        self
    }
    pub(super) fn min_size(mut self, size: gpui_kit::Pixels) -> Self {
        self.button.min_size = Some(size);
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
}
impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.button
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_options_reach_button() {
        let t = crate::desktop::theme::night();
        let b = IconButton::new("i", IconName::Copy, "Copy", &t)
            .accessibility_label("Copy value")
            .tooltip("Copy")
            .selected(true)
            .loading(true)
            .disabled(true);
        assert!(!b.button.tab_stop());
        assert!(!b.button.accepts_click());
        assert_eq!(b.button.tooltip.as_deref(), Some("Copy"));
    }
    use crate::desktop::components::tokens::button_colors;
    use crate::desktop::motion::{Spring, SpringValue};
    #[test]
    fn uses_button_tokens_focus_and_reduced_springs() {
        let t = crate::desktop::theme::night();
        let control =
            IconButton::new("icon", IconName::Plus, "Add", &t).variant(ButtonVariant::Danger);
        assert_eq!(control.button.id, ElementId::from("icon"));
        assert_eq!(control.button.variant, ButtonVariant::Danger);
        assert_eq!(
            button_colors(control.button.variant, &control.button.tokens).background,
            t.fail
        );
        let now = std::time::Instant::now();
        let mut hover = SpringValue::new(0., Spring::GENTLE);
        let mut press = SpringValue::new(0., Spring::SNAPPY);
        hover.set_reduced(1., now, false);
        press.set_reduced(1., now, false);
        let style = super::super::button::interaction_style(
            &hover,
            &press,
            now,
            true,
            true,
            &control.button.tokens,
        );
        assert_eq!((style.hover, style.press), (1., 1.));
        assert!(!style.moving);
        assert_eq!(style.ring, crate::desktop::theme::focus_ring(&t));
        assert!(!style.ring.is_empty());
    }
}
