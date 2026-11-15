//! A non-interactive, token-coloured badge.
use crate::desktop::components::tokens::{
    button_colors, radius, space, ButtonColors, ButtonVariant, Density, Radius, Step, Tokens,
};
use gpui_kit::{
    div, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window,
};

#[derive(IntoElement)]
pub struct Badge {
    id: ElementId,
    label: SharedString,
    tokens: Tokens,
    variant: ButtonVariant,
}
impl Badge {
    fn tab_stop(&self) -> bool {
        false
    }
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tokens: *tokens,
            variant: ButtonVariant::Secondary,
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
}
pub(super) fn badge_colors(variant: ButtonVariant, tokens: &Tokens) -> ButtonColors {
    button_colors(variant, tokens)
}
impl RenderOnce for Badge {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let colors = badge_colors(self.variant, &self.tokens);
        let tab_stop = self.tab_stop();
        div()
            .id(self.id)
            .tab_stop(tab_stop)
            .rounded(radius(Radius::Pill))
            .px(space(Density::Comfortable, Step::Sm))
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(self.label)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badge_is_not_a_tab_stop() {
        let t = crate::desktop::theme::night();
        assert!(!Badge::new("badge", "Badge", &t).tab_stop());
    }
    #[test]
    fn non_interactive_badge_uses_its_identity_and_token_colors() {
        for t in crate::desktop::theme::both() {
            let badge = Badge::new("control", "Label", &t).variant(ButtonVariant::Danger);
            assert_eq!(badge.id, ElementId::from("control"));
            assert_eq!(badge.label.as_ref(), "Label");
            assert_eq!(
                badge_colors(badge.variant, &badge.tokens).background,
                t.fail
            );
        }
    }
}
