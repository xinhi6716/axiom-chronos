//! Focusable frame for dialog contents; presentation and dismissal remain with the owner.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{Glass, Tokens};
use gpui_kit::{App, ElementId, FocusHandle, IntoElement, RenderOnce, Window};

#[derive(IntoElement)]
pub struct DialogFrame {
    toolbar: Toolbar,
    level: Glass,
}
impl DialogFrame {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            level: Glass::Hero,
        }
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
        self.toolbar.glass(self.level)
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
