//! A controlled set of individually focusable tabs.
use super::{button::Button, toolbar::Toolbar};
use crate::desktop::components::tokens::{ButtonVariant, Radius, Tokens};
use gpui_kit::{App, ElementId, IntoElement, RenderOnce, SharedString, Window};
use std::rc::Rc;

type Select = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub(super) fn variant(selected: bool) -> ButtonVariant {
    if selected {
        ButtonVariant::Primary
    } else {
        ButtonVariant::Ghost
    }
}
pub(super) fn selection(index: usize, item_count: usize) -> Option<usize> {
    (index < item_count).then_some(index)
}
struct Item {
    id: ElementId,
    label: SharedString,
}
#[derive(IntoElement)]
pub struct Tabs {
    pub(super) id: ElementId,
    tokens: Tokens,
    items: Vec<Item>,
    selected: usize,
    on_select: Option<Select>,
    pub(super) rounded: Radius,
}
impl Tabs {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            tokens: *tokens,
            items: Vec::new(),
            selected: 0,
            on_select: None,
            rounded: Radius::Md,
        }
    }
    /// Each item has its own stable identity and keyboard focus.
    pub fn item(mut self, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        self.items.push(Item {
            id: id.into(),
            label: label.into(),
        });
        self
    }
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }
    pub fn selected_index(self, index: usize) -> Self {
        self.selected(index)
    }
    pub fn on_click(self, callback: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select(move |index, window, cx| callback(&index, window, cx))
    }
    pub fn on_select(mut self, callback: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(callback));
        self
    }
    pub(super) fn item_button(&self, index: usize) -> Option<Button> {
        let selected_index = selection(index, self.items.len())?;
        let item = self.items.get(selected_index)?;
        let selected = index == self.selected;
        let mut button = Button::new(item.id.clone(), item.label.clone(), &self.tokens)
            .rounded(self.rounded)
            .toggled(selected)
            .variant(variant(selected));
        if let Some(callback) = self.on_select.clone() {
            button = button.on_click(move |_, window, cx| callback(selected_index, window, cx));
        }
        Some(button)
    }
    pub(super) fn rounded(mut self, radius: Radius) -> Self {
        self.rounded = radius;
        self
    }
}
impl RenderOnce for Tabs {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut toolbar = Toolbar::new(self.id.clone(), &self.tokens);
        for index in 0..self.items.len() {
            toolbar = toolbar.child(self.item_button(index).expect("item index is in range"));
        }
        toolbar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_buttons_keep_identity_selection_and_index() {
        for t in crate::desktop::theme::both() {
            for selected in [0, 1, 99, usize::MAX] {
                let tabs = Tabs::new("tabs", &t)
                    .item("first", "First")
                    .item("second", "Second")
                    .selected(selected)
                    .on_select(|_, _, _| {});
                assert_eq!(tabs.id, ElementId::from("tabs"));
                for (index, id) in ["first", "second"].into_iter().enumerate() {
                    let button = tabs.item_button(index).unwrap();
                    assert_eq!(button.id, ElementId::from(id));
                    assert_eq!(
                        button.variant,
                        if index == selected {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Ghost
                        }
                    );
                    assert_eq!(button.toggled, Some(index == selected));
                    assert_eq!(selection(index, tabs.items.len()), Some(index));
                }
                assert!(tabs.item_button(2).is_none());
                assert_eq!(selection(usize::MAX, tabs.items.len()), None);
            }
        }
    }
}

#[cfg(test)]
mod wishes_api_tests {
    use super::*;
    #[test]
    fn selected_index_and_click_wire_items() {
        for t in crate::desktop::theme::both() {
            let tabs = Tabs::new("tabs", &t)
                .item("a", "A")
                .item("b", "B")
                .selected_index(1)
                .on_click(|_, _, _| {});
            assert!(tabs.on_select.is_some());
            assert_eq!(tabs.item_button(1).unwrap().toggled, Some(true));
            assert_eq!(tabs.item_button(0).unwrap().toggled, Some(false));
        }
    }
}
