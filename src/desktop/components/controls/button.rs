//! Token-coloured, keyboard-operable button.
use crate::desktop::components::tokens::{button_colors, radius, ButtonVariant, Radius, Tokens};
use crate::desktop::motion::{self, Driver, Settling, Spring, SpringValue};
use gpui_kit::component::button::{Button as KitButton, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::FocusableExt as _;
use gpui_kit::component::{Disableable as _, Selectable as _};
use gpui_kit::{
    div, App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window,
};
use std::rc::Rc;
use std::time::Instant;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) fn hover_container(
    id: ElementId,
    child: impl IntoElement,
    listener: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div().id((id, "hover")).on_hover(listener).child(child)
}

pub(super) struct Interaction {
    pub(super) focus: FocusHandle,
    pub(super) hover: SpringValue,
    pub(super) press: SpringValue,
}

impl Interaction {
    pub(super) fn new(focus: FocusHandle) -> Self {
        Self {
            focus,
            hover: SpringValue::new(0.0, Spring::GENTLE),
            press: SpringValue::new(0.0, Spring::SNAPPY),
        }
    }
    pub(super) fn hover(&mut self, on: bool, now: Instant, reduce: bool) {
        self.hover
            .set_reduced(if on { 1.0 } else { 0.0 }, now, reduce);
        if !on {
            self.press.set_reduced(0.0, now, reduce);
        }
    }
    pub(super) fn press(&mut self, on: bool, now: Instant, reduce: bool) {
        self.press
            .set_reduced(if on { 1.0 } else { 0.0 }, now, reduce);
    }
}

pub(super) fn ring(focused: bool, tokens: &Tokens) -> Vec<gpui_kit::BoxShadow> {
    if focused {
        crate::desktop::theme::focus_ring(tokens)
    } else {
        Vec::new()
    }
}

pub(super) struct InteractionStyle {
    pub(super) ring: Vec<gpui_kit::BoxShadow>,
    pub(super) hover: f32,
    pub(super) press: f32,
    pub(super) moving: bool,
}

pub(super) fn interaction_style(
    hover: &SpringValue,
    press: &SpringValue,
    now: Instant,
    reduce: bool,
    focused: bool,
    tokens: &Tokens,
) -> InteractionStyle {
    InteractionStyle {
        ring: ring(focused, tokens),
        hover: hover.value_reduced(now, reduce),
        press: press.value_reduced(now, reduce),
        moving: !Settling::settled(hover, now, reduce) || !Settling::settled(press, now, reduce),
    }
}

#[cfg(test)]
pub(super) fn test_interaction(tokens: &Tokens) {
    assert!(ring(false, tokens).is_empty());
    assert_eq!(
        ring(true, tokens),
        crate::desktop::theme::focus_ring(tokens)
    );
    assert!(!ring(true, tokens).is_empty());
    let now = Instant::now();
    let mut driver = Driver::at(now, true);
    for spring in [Spring::GENTLE, Spring::SNAPPY] {
        let mut value = SpringValue::new(0., spring);
        value.set_reduced(1., now, true);
        driver.track(&value);
        assert!(value.settled(now));
        assert_eq!(value.value(now), 1.);
    }
    assert!(!driver.wants_frame());
}

#[derive(IntoElement)]
pub struct Button {
    pub(super) id: ElementId,
    label: SharedString,
    icon: Option<gpui_kit::component::IconName>,
    icon_only: bool,
    accessibility_label: Option<SharedString>,
    disabled: bool,
    selected: bool,
    loading: bool,
    gold_leaf: bool,
    pub(super) min_size: Option<gpui_kit::Pixels>,
    pub(super) variant: ButtonVariant,
    pub(super) tokens: Tokens,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
    pub(super) rounded: Radius,
    pub(super) tooltip: Option<SharedString>,
    pub(super) children: Vec<gpui_kit::AnyElement>,
    pub(super) toggled: Option<bool>,
}

impl Button {
    pub(super) fn tab_stop(&self) -> bool {
        !self.disabled
    }
    pub(super) fn accepts_click(&self) -> bool {
        !self.disabled && !self.loading
    }
    pub(super) fn style(
        &self,
        state: &Interaction,
        now: Instant,
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
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            icon_only: false,
            accessibility_label: None,
            disabled: false,
            selected: false,
            loading: false,
            gold_leaf: false,
            min_size: None,
            variant: ButtonVariant::Primary,
            tokens: *tokens,
            focus: None,
            on_click: None,
            rounded: Radius::Md,
            tooltip: None,
            children: Vec::new(),
            toggled: None,
        }
    }
    pub(super) fn icon_only(mut self, icon: gpui_kit::component::IconName) -> Self {
        self.icon = Some(icon);
        self.icon_only = true;
        self
    }
    pub fn leading_icon(mut self, icon: gpui_kit::component::IconName) -> Self {
        self.icon = Some(icon);
        self.icon_only = false;
        self
    }
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    pub fn gold_leaf(mut self) -> Self {
        self.gold_leaf = true;
        self
    }
    pub(super) fn rounded(mut self, rounded: Radius) -> Self {
        self.rounded = rounded;
        self
    }
    pub fn tooltip(mut self, text: impl Into<SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub(super) fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
    pub(super) fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn on_click(
        mut self,
        callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(callback));
        self
    }
}

impl RenderOnce for Button {
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
        let background = motion::mix(
            motion::mix(colors.background, colors.hover, hover),
            colors.pressed,
            press,
        );
        let variant = ButtonCustomVariant::new(cx)
            .color(background)
            .foreground(colors.foreground)
            .hover(background)
            .active(if self.selected {
                colors.pressed
            } else {
                background
            });
        let hover_state = state.clone();
        let down_state = state.clone();
        let up_state = state.clone();
        let out_state = state;
        let accepts_click = self.accepts_click();
        let tab_stop = self.tab_stop();
        let id = self.id;
        let mut button = KitButton::new(id.clone())
            .accessibility_label(
                self.accessibility_label
                    .unwrap_or_else(|| self.label.clone()),
            )
            .rounded(radius(self.rounded))
            .custom(variant)
            .track_focus(&focus)
            .tab_stop(tab_stop)
            .focus_ring(false)
            .shadow(style.ring)
            .disabled(self.disabled)
            .selected(self.selected)
            .loading(self.loading)
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
            });
        if self.gold_leaf {
            button = crate::desktop::theme::gold_leaf(button, &self.tokens);
        }
        if let Some(size) = self.min_size {
            button = button.min_w(size).min_h(size);
        }
        let has_icon = self.icon.is_some();
        if let Some(icon) = self.icon {
            button = button.icon(icon);
        }
        if !self.icon_only {
            button = button.label(self.label);
        }
        if self.loading && !has_icon {
            button = button.child("…");
        }
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(tooltip);
        }
        if let Some(toggled) = self.toggled {
            button = button.toggled(toggled);
        }
        for child in self.children {
            button = button.child(child);
        }
        if let Some(callback) = self.on_click.filter(|_| accepts_click) {
            button = button.on_click(move |event, window, cx| callback(event, window, cx));
        }
        hover_container(id, button, move |on, _, cx| {
            let now = motion::now(cx);
            let reduce = cx.reduce_motion();
            hover_state.update(cx, |state, cx| {
                state.hover(*on, now, reduce);
                cx.notify();
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_states_and_label_are_preserved() {
        let t = crate::desktop::theme::night();
        let b = Button::new("extended", "Visible", &t)
            .leading_icon(gpui_kit::component::IconName::Plus)
            .accessibility_label("Accessible")
            .tooltip("Tip")
            .selected(true)
            .loading(true)
            .disabled(true)
            .gold_leaf();
        assert_eq!(b.accessibility_label.as_deref(), Some("Accessible"));
        assert!(b.icon.is_some() && b.selected && b.loading && b.disabled && b.gold_leaf);
        assert!(!b.tab_stop());
        assert!(!b.accepts_click());
        let enabled = Button::new("enabled", "Enabled", &t);
        assert!(enabled.tab_stop() && enabled.accepts_click());
        let disabled = Button::new("disabled", "Disabled", &t).disabled(true);
        assert!(!disabled.tab_stop() && !disabled.accepts_click());
        let loading = Button::new("loading", "Loading", &t).loading(true);
        assert!(loading.tab_stop() && !loading.accepts_click());
    }
    #[test]
    fn identity_and_variant_are_preserved() {
        let t = crate::desktop::theme::night();
        let button = Button::new("save", "Save", &t).variant(ButtonVariant::Danger);
        assert_eq!(button.id, ElementId::from("save"));
        assert_eq!(button.variant, ButtonVariant::Danger);
        assert_eq!(
            button_colors(button.variant, &button.tokens).background,
            t.fail
        );
    }
    #[test]
    fn forwards_radius_tooltip_children_and_toggle() {
        let t = crate::desktop::theme::night();
        let button = Button::new("forward", "Label", &t)
            .rounded(Radius::Pill)
            .tooltip("help")
            .child("child")
            .toggled(true);
        assert_eq!(button.rounded, Radius::Pill);
        assert_eq!(button.tooltip.as_deref(), Some("help"));
        assert_eq!(button.children.len(), 1);
        assert_eq!(button.toggled, Some(true));
    }
    #[test]
    fn render_interaction_values_respect_motion_and_focus() {
        let t = crate::desktop::theme::night();
        let now = Instant::now();
        let mut hover = SpringValue::new(0., Spring::GENTLE);
        let mut press = SpringValue::new(0., Spring::SNAPPY);
        hover.set_reduced(1., now, false);
        press.set_reduced(1., now, false);
        let normal = interaction_style(&hover, &press, now, false, false, &t);
        assert!(normal.moving);
        assert!(normal.ring.is_empty());
        let reduced = interaction_style(&hover, &press, now, true, true, &t);
        assert!(!reduced.moving);
        assert_eq!((reduced.hover, reduced.press), (1., 1.));
        assert_eq!(reduced.ring, crate::desktop::theme::focus_ring(&t));
        assert!(!reduced.ring.is_empty());
    }
    #[test]
    fn variants_use_theme_roles() {
        for t in crate::desktop::theme::both() {
            assert_eq!(button_colors(ButtonVariant::Primary, &t).background, t.gold);
            assert_eq!(
                button_colors(ButtonVariant::Secondary, &t).background,
                t.raised
            );
            assert_eq!(button_colors(ButtonVariant::Ghost, &t).foreground, t.text);
            assert_eq!(button_colors(ButtonVariant::Danger, &t).background, t.fail);
        }
    }
    #[test]
    fn focused_ring_comes_from_theme() {
        let t = crate::desktop::theme::night();
        assert!(ring(false, &t).is_empty());
        assert_eq!(ring(true, &t), crate::desktop::theme::focus_ring(&t));
        assert!(!ring(true, &t).is_empty());
    }
    #[test]
    fn springs_snap_under_reduced_motion() {
        let now = Instant::now();
        for spring in [Spring::GENTLE, Spring::SNAPPY] {
            let mut value = SpringValue::new(0.0, spring);
            value.set_reduced(1.0, now, false);
            assert!(!value.settled(now));
            assert_eq!(value.value_reduced(now, true), 1.0);
            value.set_reduced(0.0, now, true);
            assert!(value.settled(now));
            assert_eq!(value.value(now), 0.0);
        }
    }
}
