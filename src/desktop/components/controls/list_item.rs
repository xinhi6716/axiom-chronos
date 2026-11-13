//! Selectable token-styled row.
use super::button::{interaction_style, Interaction};
use crate::desktop::components::tokens::{button_colors, radius, ButtonVariant, Radius, Tokens};
use crate::desktop::motion::{self, Driver};
use gpui_kit::component::button::{Button as KitButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, FocusableExt as _};
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement,
    MouseButton, ParentElement, RenderOnce, SharedString, Styled, Window,
};
use std::rc::Rc;
type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
fn item_background(tokens: &Tokens, selected: bool, hover: f32, press: f32) -> gpui_kit::Hsla {
    let colors = button_colors(
        if selected {
            ButtonVariant::Secondary
        } else {
            ButtonVariant::Ghost
        },
        tokens,
    );
    motion::mix(
        motion::mix(colors.background, colors.hover, hover),
        colors.pressed,
        press,
    )
}
fn item_foreground(tokens: &Tokens, selected: bool, disabled: bool) -> gpui_kit::Hsla {
    if disabled {
        tokens.muted
    } else {
        button_colors(
            if selected {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            },
            tokens,
        )
        .foreground
    }
}
fn enabled_handler<T>(disabled: bool, handler: Option<T>) -> Option<T> {
    if disabled {
        None
    } else {
        handler
    }
}
#[derive(IntoElement)]
pub struct ListItem {
    id: ElementId,
    label: SharedString,
    tokens: Tokens,
    selected: bool,
    disabled: bool,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    focus: Option<FocusHandle>,
    on_click: Option<Handler>,
}
impl ListItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tokens: *tokens,
            selected: false,
            disabled: false,
            leading: None,
            trailing: None,
            focus: None,
            on_click: None,
        }
    }
    pub fn leading(mut self, v: impl IntoElement) -> Self {
        self.leading = Some(v.into_any_element());
        self
    }
    pub fn trailing(mut self, v: impl IntoElement) -> Self {
        self.trailing = Some(v.into_any_element());
        self
    }
    pub fn selected(mut self, v: bool) -> Self {
        self.selected = v;
        self
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
    pub fn track_focus(mut self, v: &FocusHandle) -> Self {
        self.focus = Some(v.clone());
        self
    }
    pub fn on_click(mut self, v: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(v));
        self
    }
}
impl gpui_kit::component::Selectable for ListItem {
    fn selected(mut self, value: bool) -> Self {
        self.selected = value;
        self
    }
    fn is_selected(&self) -> bool {
        self.selected
    }
}
impl RenderOnce for ListItem {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| {
            Interaction::new(cx.focus_handle())
        });
        let focus = self
            .focus
            .unwrap_or_else(|| state.read(cx).focus.clone())
            .tab_stop(true);
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
        let background = item_background(&self.tokens, self.selected, style.hover, style.press);
        let foreground = item_foreground(&self.tokens, self.selected, self.disabled);
        let variant = ButtonCustomVariant::new(cx)
            .color(background)
            .foreground(foreground)
            .hover(background)
            .active(background);
        let hover = state.clone();
        let down = state.clone();
        let up = state.clone();
        let out = state;
        let mut button = KitButton::new(self.id)
            .accessibility_label(self.label.clone())
            .rounded(radius(Radius::Md))
            .custom(variant)
            .track_focus(&focus)
            .focus_ring(false)
            .shadow(style.ring)
            .bg(background)
            .text_color(foreground)
            .disabled(self.disabled)
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
            });
        if let Some(v) = self.leading {
            button = button.child(v);
        }
        button = button.child(self.label);
        if let Some(v) = self.trailing {
            button = button.child(v);
        }
        if let Some(cb) = enabled_handler(self.disabled, self.on_click) {
            button = button.on_click(move |e, w, c| cb(e, w, c));
        }
        button
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn item_colours_and_disabled_click() {
        for t in crate::desktop::theme::both() {
            let ghost = button_colors(ButtonVariant::Ghost, &t);
            let selected = button_colors(ButtonVariant::Secondary, &t);
            assert_eq!(item_background(&t, false, 0., 0.), ghost.background);
            assert_eq!(item_background(&t, true, 0., 0.), selected.background);
            assert_eq!(item_background(&t, false, 1., 0.), ghost.hover);
            assert_eq!(item_background(&t, false, 0., 1.), ghost.pressed);
            assert_eq!(item_foreground(&t, true, false), selected.foreground);
            assert_eq!(item_foreground(&t, true, true), t.muted);
            let calls = Rc::new(std::cell::Cell::new(0));
            let captured = calls.clone();
            let disabled = ListItem::new("disabled", "Disabled", &t)
                .disabled(true)
                .on_click(move |_, _, _| captured.set(captured.get() + 1));
            assert!(enabled_handler(disabled.disabled, disabled.on_click).is_none());
            assert_eq!(Rc::strong_count(&calls), 1);
            assert_eq!(calls.get(), 0);
            let enabled = ListItem::new("enabled", "Enabled", &t).on_click(|_, _, _| {});
            assert!(enabled_handler(enabled.disabled, enabled.on_click).is_some());
            let now = std::time::Instant::now();
            let mut hover = motion::SpringValue::new(0., motion::Spring::GENTLE);
            let press = motion::SpringValue::new(0., motion::Spring::SNAPPY);
            hover.set_reduced(1., now, false);
            let style = interaction_style(&hover, &press, now, true, false, &t);
            assert_eq!(
                item_background(&t, false, style.hover, style.press),
                ghost.hover
            );
        }
    }
    #[test]
    fn slots_selection_and_disabled() {
        for t in crate::desktop::theme::both() {
            let row = ListItem::new("row", "Label", &t)
                .leading("icon")
                .trailing("end")
                .selected(true)
                .disabled(true);
            assert_eq!(row.id, ElementId::from("row"));
            assert!(row.selected && row.disabled);
            assert!(row.leading.is_some() && row.trailing.is_some());
        }
    }
}
