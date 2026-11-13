//! A token-styled single-line text editor backed by the kit's input state.
use super::button::{interaction_style, Interaction, InteractionStyle};
use crate::desktop::components::tokens::{button_colors, radius, ButtonVariant, Radius, Tokens};
use crate::desktop::motion::{self, Driver};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    div, App, ElementId, Entity, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, StatefulInteractiveElement, Styled, Window,
};

#[derive(IntoElement)]
pub struct TextField {
    style: FieldStyle,
    state: Entity<InputState>,
}
pub(super) struct FieldStyle {
    id: ElementId,
    tokens: Tokens,
    variant: ButtonVariant,
}
pub(super) fn input_id(id: &ElementId) -> ElementId {
    (id.clone(), "input").into()
}
impl FieldStyle {
    pub(super) fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            tokens: *tokens,
            variant: ButtonVariant::Secondary,
        }
    }
    pub(super) fn style(
        &self,
        state: &Interaction,
        now: std::time::Instant,
        reduce: bool,
        focused: bool,
    ) -> InteractionStyle {
        interaction_style(
            &state.hover,
            &state.press,
            now,
            reduce,
            focused,
            &self.tokens,
        )
    }
}
impl TextField {
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>, tokens: &Tokens) -> Self {
        Self {
            style: FieldStyle::new(id, tokens),
            state: state.clone(),
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.style.variant = variant;
        self
    }
}
impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.style.id.clone();
        let tokens = self.style.tokens;
        let variant = self.style.variant;
        let interaction =
            window.use_keyed_state(id.clone(), cx, |_, cx| Interaction::new(cx.focus_handle()));
        let focus = self.state.read(cx).focus_handle(cx);
        let mut driver = Driver::new(cx.reduce_motion(), cx);
        let values = interaction.read(cx);
        let style = self.style.style(
            values,
            driver.now(),
            driver.reduce(),
            focus.is_focused(window),
        );
        driver.keep(style.moving);
        let hover = style.hover;
        let press = style.press;
        driver.finish(window);
        let colors = button_colors(variant, &tokens);
        let background = motion::mix(
            motion::mix(colors.background, colors.hover, hover),
            colors.pressed,
            press,
        );
        let hover_state = interaction.clone();
        let down_state = interaction.clone();
        let up_state = interaction.clone();
        let out_state = interaction;
        div()
            .id(id.clone())
            .rounded(radius(Radius::Md))
            .bg(background)
            .text_color(colors.foreground)
            .border_1()
            .border_color(tokens.border)
            .shadow(style.ring)
            .child(
                Input::new(&self.state)
                    .id(input_id(&id))
                    .appearance(false)
                    .focus_bordered(false),
            )
            .on_hover(move |on, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                hover_state.update(cx, |state, cx| {
                    state.hover(*on, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                down_state.update(cx, |state, cx| {
                    state.press(true, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                up_state.update(cx, |state, cx| {
                    state.press(false, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                out_state.update(cx, |state, cx| {
                    state.press(false, now, reduce);
                    cx.notify();
                });
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let style = FieldStyle::new("field", &t);
            assert_eq!(style.id, ElementId::from("field"));
            assert_eq!(
                button_colors(style.variant, &style.tokens).background,
                t.raised
            );
            assert_eq!(button_colors(ButtonVariant::Danger, &t).background, t.fail);
            assert_ne!(input_id(&style.id), style.id);
            assert_eq!(input_id(&style.id), (style.id.clone(), "input").into());
        }
    }
}
