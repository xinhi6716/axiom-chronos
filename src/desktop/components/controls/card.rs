//! Focusable glass card.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{Glass, Tokens};
use gpui_kit::{App, ElementId, FocusHandle, IntoElement, RenderOnce, Window};

#[derive(IntoElement)]
pub struct Card {
    toolbar: Toolbar,
    level: Glass,
}
impl Card {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            level: Glass::Card,
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
impl Card {
    pub(super) fn into_toolbar(self) -> Toolbar {
        self.toolbar.glass(self.level)
    }
}
impl RenderOnce for Card {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_toolbar()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::glass;
    #[test]
    fn identity_and_glass_selection() {
        for t in crate::desktop::theme::both() {
            let card = Card::new("card", &t).glass(Glass::Hero).child("content");
            assert_eq!(card.toolbar.id, ElementId::from("card"));
            assert_eq!(card.level, Glass::Hero);
            assert_eq!(glass(card.level, &t).border, t.border.opacity(0.90));
            assert_ne!(glass(Glass::Card, &t).fill, glass(card.level, &t).fill);
            let toolbar = card.into_toolbar();
            assert_eq!(toolbar.id, ElementId::from("card"));
        }
    }
}
