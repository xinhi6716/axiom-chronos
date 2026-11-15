//! Controlled, keyboard-operable toggle.
use super::button::Button;
use crate::desktop::components::tokens::{ButtonVariant, Radius, Tokens};
use gpui_kit::{App, ElementId, FocusHandle, IntoElement, RenderOnce, SharedString, Window};
use std::rc::Rc;

type Change = Rc<dyn Fn(bool, &mut Window, &mut App)>;
pub(super) fn variant(checked: bool) -> ButtonVariant {
    if checked {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Secondary
    }
}

pub(super) fn next_value(checked: bool) -> bool {
    !checked
}

#[derive(IntoElement)]
pub struct Toggle {
    button: Button,
    checked: bool,
    on_change: Option<Change>,
}
impl Toggle {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            button: Button::new(id, label, tokens).rounded(Radius::Pill),
            checked: false,
            on_change: None,
        }
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.button = self.button.track_focus(focus);
        self
    }
    /// The owner updates `checked` after receiving the requested value.
    pub fn on_change(mut self, callback: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }
}
impl Toggle {
    pub(super) fn into_button(self) -> Button {
        let mut button = self
            .button
            .toggled(self.checked)
            .variant(variant(self.checked));
        if let Some(callback) = self.on_change {
            let next = next_value(self.checked);
            button = button.on_click(move |_, window, cx| callback(next, window, cx));
        }
        button
    }
}

impl RenderOnce for Toggle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_button()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_button_and_next_value() {
        for t in crate::desktop::theme::both() {
            for checked in [false, true] {
                let button = Toggle::new("toggle", "Toggle", &t)
                    .checked(checked)
                    .on_change(|_, _, _| {})
                    .into_button();
                assert_eq!(button.id, ElementId::from("toggle"));
                assert_eq!(
                    button.variant,
                    if checked {
                        ButtonVariant::Primary
                    } else {
                        ButtonVariant::Secondary
                    }
                );
                assert_eq!(button.toggled, Some(checked));
                assert_eq!(button.rounded, Radius::Pill);
                assert_eq!(next_value(checked), !checked);
            }
        }
    }
}
