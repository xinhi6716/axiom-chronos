//! A focusable toolbar containing independently operable controls.
use super::button::{interaction_style, Interaction, InteractionStyle};
use crate::desktop::components::tokens::{
    button_colors, glass, radius, space, ButtonVariant, Density, Glass, Radius, Step, Tokens,
};
use crate::desktop::motion::{self, Driver};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, AnyElement, App, ElementId, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, StatefulInteractiveElement, Styled, Window,
};

#[derive(IntoElement)]
pub struct Toolbar {
    pub(super) id: ElementId,
    tokens: Tokens,
    variant: ButtonVariant,
    focus: Option<FocusHandle>,
    children: Vec<AnyElement>,
    pub(super) glass: Option<Glass>,
}
impl Toolbar {
    pub(super) fn tab_stop(&self) -> bool {
        true
    }
    pub(super) fn style(
        &self,
        state: &Interaction,
        now: std::time::Instant,
        reduce: bool,
        focused: bool,
    ) -> InteractionStyle {
        toolbar_style(
            &state.hover,
            &state.press,
            now,
            reduce,
            focused,
            &self.tokens,
        )
    }
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            tokens: *tokens,
            variant: ButtonVariant::Secondary,
            focus: None,
            children: Vec::new(),
            glass: None,
        }
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub(super) fn glass(mut self, level: Glass) -> Self {
        self.glass = Some(level);
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
}
pub(super) fn toolbar_style(
    hover: &motion::SpringValue,
    press: &motion::SpringValue,
    now: std::time::Instant,
    reduce: bool,
    focused: bool,
    tokens: &Tokens,
) -> InteractionStyle {
    interaction_style(hover, press, now, reduce, focused, tokens)
}
impl RenderOnce for Toolbar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            Interaction::new(cx.focus_handle())
        });
        let focus = self
            .focus
            .clone()
            .unwrap_or_else(|| state.read(cx).focus.clone())
            .tab_stop(self.tab_stop());
        let mut driver = Driver::new(cx.reduce_motion(), cx);
        let interaction = state.read(cx);
        let style = self.style(
            interaction,
            driver.now(),
            driver.reduce(),
            focus.is_focused(window),
        );
        driver.keep(style.moving);
        let hover = style.hover;
        let press = style.press;
        driver.finish(window);
        let colors = button_colors(self.variant, &self.tokens);
        let glass = self.glass.map(|level| glass(level, &self.tokens));
        let background = motion::mix(
            motion::mix(
                glass.map_or(colors.background, |style| style.fill),
                colors.hover,
                hover,
            ),
            colors.pressed,
            press,
        );
        let hover_state = state.clone();
        let down_state = state.clone();
        let up_state = state.clone();
        let out_state = state;
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex()
            .items_center()
            .when(glass.is_some(), |this| this.flex_col().items_stretch())
            .gap(space(Density::Comfortable, Step::Sm))
            .p(space(Density::Comfortable, Step::Xs))
            .rounded(radius(Radius::Md))
            .when_some(glass, |this, style| {
                this.border_1().border_color(style.border)
            })
            .bg(background)
            .text_color(colors.foreground)
            .shadow(style.ring)
            .children(self.children)
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
    fn identity_colors_and_children() {
        for t in crate::desktop::theme::both() {
            let control = Toolbar::new("toolbar", &t)
                .variant(ButtonVariant::Danger)
                .child("Item");
            assert_eq!(control.id, ElementId::from("toolbar"));
            assert_eq!(control.children.len(), 1);
            assert_eq!(
                button_colors(control.variant, &control.tokens).background,
                t.fail
            );
        }
    }
}
