//! Focusable frame for dialog contents; presentation and dismissal remain with the owner.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{space, Density, Glass, Step, Tokens};
use gpui_kit::{
    div, AnyElement, App, ElementId, FocusHandle, IntoElement, ParentElement, RenderOnce, Styled,
    Window,
};

#[derive(IntoElement)]
pub struct DialogFrame {
    toolbar: Toolbar,
    level: Glass,
    footer: Option<AnyElement>,
}
impl DialogFrame {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            level: Glass::Hero,
            footer: None,
        }
    }
    pub fn footer(mut self, footer: impl IntoElement) -> Self {
        self.footer = Some(footer.into_any_element());
        self
    }
    pub fn glass(mut self, level: Glass) -> Self {
        self.level = level;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.toolbar = self.toolbar.track_focus(focus);
        self
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.toolbar = self.toolbar.child(child);
        self
    }
}
impl DialogFrame {
    pub(super) fn into_toolbar(self) -> Toolbar {
        let mut toolbar = self.toolbar.glass(self.level);
        if let Some(footer) = self.footer {
            toolbar = toolbar.child(
                div()
                    .flex()
                    .w_full()
                    .justify_end()
                    .gap(space(Density::Comfortable, Step::Sm))
                    .child(footer),
            );
        }
        toolbar
    }
}
impl RenderOnce for DialogFrame {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_toolbar()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::glass;
    #[test]
    fn identity_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            assert_eq!(
                DialogFrame::new("default", &t).into_toolbar().glass,
                Some(Glass::Hero)
            );
            let frame = DialogFrame::new("dialog", &t)
                .glass(Glass::Card)
                .child("content");
            assert_eq!(frame.toolbar.id, ElementId::from("dialog"));
            let toolbar = frame.into_toolbar();
            assert_eq!(toolbar.glass, Some(Glass::Card));
            assert_eq!(glass(toolbar.glass.unwrap(), &t), glass(Glass::Card, &t));
            assert_ne!(glass(toolbar.glass.unwrap(), &t), glass(Glass::Hero, &t));
        }
    }
}

#[cfg(test)]
mod footer_tests {
    use super::*;
    #[test]
    fn footer_is_retained_after_content() {
        for t in crate::desktop::theme::both() {
            let frame = DialogFrame::new("dialog", &t)
                .child("body")
                .footer("actions");
            assert!(frame.footer.is_some());
            assert_eq!(frame.into_toolbar().id, ElementId::from("dialog"));
        }
    }
}
