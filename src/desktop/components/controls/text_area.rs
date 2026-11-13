//! Token-styled multi-line editor backed by the kit's IME-capable editing engine.
use super::button::{ring, Interaction};
use super::text_field::{input_id, FieldStyle};
use crate::desktop::components::tokens::{button_colors, radius, ButtonVariant, Radius, Tokens};
use crate::desktop::motion::{self, Driver};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    div, App, ElementId, Entity, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window,
};
use std::rc::Rc;

type Submit = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
fn clamp_rows(rows: usize, min: usize, max: usize) -> usize {
    let min = min.max(1);
    rows.clamp(min, max.max(min))
}
fn should_submit(enter: bool, primary: bool) -> bool {
    enter && primary
}
struct AreaStyle {
    id: ElementId,
    tokens: Tokens,
    field: FieldStyle,
}
fn area_style(id: impl Into<ElementId>, tokens: &Tokens) -> AreaStyle {
    let id = id.into();
    AreaStyle {
        field: FieldStyle::new(id.clone(), tokens),
        id,
        tokens: *tokens,
    }
}
impl AreaStyle {
    fn id(&self) -> ElementId {
        self.id.clone()
    }
    fn tab_stop(&self) -> bool {
        true
    }
    fn focus_ring(&self, focused: bool) -> Vec<gpui_kit::BoxShadow> {
        ring(focused, &self.tokens)
    }
}
#[derive(IntoElement)]
pub struct TextArea {
    style: AreaStyle,
    state: Entity<TextareaState>,
    min_rows: usize,
    max_rows: usize,
    placeholder: Option<SharedString>,
    on_submit: Option<Submit>,
}
impl TextArea {
    pub fn new(id: impl Into<ElementId>, state: &Entity<TextareaState>, tokens: &Tokens) -> Self {
        Self {
            style: area_style(id, tokens),
            state: state.clone(),
            min_rows: 1,
            max_rows: 1,
            placeholder: None,
            on_submit: None,
        }
    }
    pub fn rows(mut self, min: usize, max: usize) -> Self {
        self.min_rows = clamp_rows(min, 1, usize::MAX);
        self.max_rows = max.max(self.min_rows);
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = Some(text.into());
        self
    }
    pub fn on_submit(
        mut self,
        callback: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_submit = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for TextArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.style.id();
        let config = window.use_keyed_state((id.clone(), "config"), cx, |_, _| {
            None::<(gpui_kit::EntityId, usize, usize, Option<SharedString>)>
        });
        let desired = (
            self.state.entity_id(),
            self.min_rows,
            self.max_rows,
            self.placeholder.clone(),
        );
        if config.read(cx).as_ref() != Some(&desired) {
            self.state.update(cx, |state, cx| {
                state.set_auto_grow(desired.1, desired.2, cx);
                state.set_placeholder(desired.3.clone().unwrap_or_default(), window, cx);
            });
            config.update(cx, |value, _| *value = Some(desired));
        }
        let interaction =
            window.use_keyed_state(id.clone(), cx, |_, cx| Interaction::new(cx.focus_handle()));
        let focus = self
            .state
            .read(cx)
            .focus_handle(cx)
            .tab_stop(self.style.tab_stop());
        let mut driver = Driver::new(cx.reduce_motion(), cx);
        let style = self.style.field.style(
            interaction.read(cx),
            driver.now(),
            driver.reduce(),
            focus.is_focused(window),
        );
        driver.keep(style.moving);
        driver.finish(window);
        let colors = button_colors(ButtonVariant::Secondary, &self.style.tokens);
        let background = motion::mix(
            motion::mix(colors.background, colors.hover, style.hover),
            colors.pressed,
            style.press,
        );
        let hover = interaction.clone();
        let down = interaction.clone();
        let up = interaction.clone();
        let out = interaction;
        let input = self.state.clone();
        let submit = self.on_submit;
        div()
            .id(id.clone())
            .rounded(radius(Radius::Md))
            .bg(background)
            .text_color(colors.foreground)
            .border_1()
            .border_color(self.style.tokens.border)
            .shadow(style.ring)
            .child(
                Textarea::new(&self.state)
                    .appearance(false)
                    .bordered(false)
                    .accessibility_id(format!("{:?}", input_id(&id))),
            )
            .capture_key_down(move |event, window, cx| {
                let m = event.keystroke.modifiers;
                let primary = if cfg!(target_os = "macos") {
                    m.platform
                } else {
                    m.control
                };
                if should_submit(event.keystroke.key == "enter", primary) {
                    if let Some(callback) = &submit {
                        callback(input.read(cx).value(), window, cx);
                        cx.stop_propagation();
                    }
                }
            })
            .on_hover(move |on, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                hover.update(cx, |s, cx| {
                    s.hover(*on, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                down.update(cx, |s, cx| {
                    s.press(true, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                up.update(cx, |s, cx| {
                    s.press(false, now, reduce);
                    cx.notify();
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                let now = motion::now(cx);
                let reduce = cx.reduce_motion();
                out.update(cx, |s, cx| {
                    s.press(false, now, reduce);
                    cx.notify();
                });
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rows_submit_and_field_style() {
        for t in crate::desktop::theme::both() {
            assert_eq!(clamp_rows(0, 0, 0), 1);
            assert_eq!(clamp_rows(12, 2, 5), 5);
            assert_eq!(clamp_rows(1, 2, 5), 2);
            assert_eq!(clamp_rows(usize::MAX, 5, 2), 5);
            assert!(should_submit(true, true));
            assert!(!should_submit(false, true));
            assert!(!should_submit(true, false));
            let style = area_style("area", &t);
            assert_eq!(style.id(), gpui_kit::ElementId::from("area"));
            assert!(style.tab_stop());
            assert_eq!(
                style.focus_ring(true),
                crate::desktop::theme::focus_ring(&t)
            );
            assert!(style.focus_ring(false).is_empty());
        }
    }
}
