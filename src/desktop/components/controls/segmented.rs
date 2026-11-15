//! A controlled segmented selection.
use super::tabs::Tabs;
use crate::desktop::components::tokens::{Radius, Tokens};
use gpui_kit::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};

#[derive(IntoElement)]
pub struct Segmented {
    pub(super) tabs: Tabs,
}
impl Segmented {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            tabs: Tabs::new(id, tokens).rounded(Radius::Pill),
        }
    }
    pub fn item(mut self, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        self.tabs = self.tabs.item(id, label);
        self
    }
    pub fn selected(mut self, index: usize) -> Self {
        self.tabs = self.tabs.selected(index);
        self
    }
    pub fn on_select(mut self, callback: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.tabs = self.tabs.on_select(callback);
        self
    }
}
impl RenderOnce for Segmented {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.tabs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::ButtonVariant;
    #[test]
    fn forwarded_tabs_produce_selected_buttons_and_indices() {
        for t in crate::desktop::theme::both() {
            for selected in [0, 1, 99, usize::MAX] {
                let segment = Segmented::new("segments", &t)
                    .item("one", "One")
                    .item("two", "Two")
                    .selected(selected)
                    .on_select(|_, _, _| {});
                assert_eq!(segment.tabs.id, ElementId::from("segments"));
                assert_eq!(segment.tabs.rounded, Radius::Pill);
                for (index, id) in ["one", "two"].into_iter().enumerate() {
                    let button = segment.tabs.item_button(index).unwrap();
                    assert_eq!(button.id, ElementId::from(id));
                    assert_eq!(button.rounded, Radius::Pill);
                    assert_eq!(
                        button.variant,
                        if selected == index {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Ghost
                        }
                    );
                    assert_eq!(button.toggled, Some(index == selected));
                    assert_eq!(super::super::tabs::selection(index, 2), Some(index));
                }
                assert!(segment.tabs.item_button(2).is_none());
            }
        }
    }
}
