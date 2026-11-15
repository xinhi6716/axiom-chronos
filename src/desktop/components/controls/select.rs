//! A token-styled select field backed by PopupMenu.
use super::button::{ring, Button};
use super::keys::{Confirm, MoveDown, MoveUp};
use super::popup_menu::{self, PopupMenu};
use crate::desktop::components::tokens::{ButtonVariant, Tokens};
use gpui_kit::{
    div, App, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Styled, Window,
};
use std::rc::Rc;

type OnSelect = Rc<dyn Fn(usize, &mut Window, &mut App)>;
#[derive(Clone)]
struct OptionItem {
    id: ElementId,
    label: SharedString,
}
fn move_index(current: Option<usize>, count: usize, forward: bool) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(match (current.filter(|&i| i < count), forward) {
        (Some(i), true) => (i + 1) % count,
        (Some(i), false) => (i + count - 1) % count,
        (None, true) => 0,
        (None, false) => count - 1,
    })
}
struct SelectState {
    focus: FocusHandle,
    menu_focus: FocusHandle,
    selected: Option<usize>,
    highlight: Option<usize>,
    specified: usize,
    open: bool,
    generation: usize,
}
#[derive(IntoElement)]
pub struct Select {
    pub(super) id: ElementId,
    tokens: Tokens,
    options: Vec<OptionItem>,
    selected: usize,
    on_select: Option<OnSelect>,
}
impl Select {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            tokens: *tokens,
            options: Vec::new(),
            selected: 0,
            on_select: None,
        }
    }
    pub fn option(mut self, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        self.options.push(OptionItem {
            id: id.into(),
            label: label.into(),
        });
        self
    }
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }
    pub fn on_select(mut self, callback: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(callback));
        self
    }
    fn selected_index(&self) -> Option<usize> {
        (!self.options.is_empty()).then(|| self.selected.min(self.options.len() - 1))
    }
    fn tab_stop(&self) -> bool {
        true
    }
    fn focus_ring(&self, focused: bool) -> Vec<gpui_kit::BoxShadow> {
        ring(focused, &self.tokens)
    }
}
impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        popup_menu::bind(cx);
        let initial = self.selected_index();
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| SelectState {
            focus: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            selected: initial,
            highlight: initial,
            specified: self.selected,
            open: false,
            generation: 0,
        });
        if state.read(cx).specified != self.selected {
            state.update(cx, |s, _| {
                s.specified = self.selected;
                s.selected = initial;
            });
        }
        let selected = state
            .read(cx)
            .selected
            .or(initial)
            .map(|i| i.min(self.options.len().saturating_sub(1)))
            .filter(|_| !self.options.is_empty());
        let focus = state.read(cx).focus.clone().tab_stop(self.tab_stop());
        let open = state.read(cx).open;
        let label = selected
            .and_then(|i| self.options.get(i))
            .map(|o| o.label.clone())
            .unwrap_or_default();
        let show = |state: &gpui_kit::Entity<SelectState>,
                    window: &mut Window,
                    cx: &mut App,
                    highlight: Option<usize>| {
            let menu_focus = state.update(cx, |s, cx| {
                s.highlight = highlight;
                s.open = true;
                s.generation = s.generation.wrapping_add(1);
                cx.notify();
                s.menu_focus.clone()
            });
            menu_focus.focus(window, cx);
        };
        let click = state.clone();
        let trigger = Button::new((self.id.clone(), "field"), label, &self.tokens)
            .variant(ButtonVariant::Secondary)
            .track_focus(&focus)
            .on_click(move |_, window, cx| {
                show(&click, window, cx, selected);
            });
        let enter = state.clone();
        let up = state.clone();
        let down = state.clone();
        let count = self.options.len();
        let mut root = div()
            .id(self.id.clone())
            .relative()
            .key_context("AxiomPopupMenu")
            .shadow(self.focus_ring(focus.is_focused(window)))
            .child(trigger)
            .on_action(move |_: &Confirm, window, cx| {
                if !enter.read(cx).open {
                    show(&enter, window, cx, selected);
                    cx.stop_propagation();
                }
            })
            .on_action(move |_: &MoveUp, window, cx| {
                if !up.read(cx).open {
                    let highlight = move_index(selected, count, false);
                    show(&up, window, cx, highlight);
                    cx.stop_propagation();
                }
            })
            .on_action(move |_: &MoveDown, window, cx| {
                if !down.read(cx).open {
                    let highlight = move_index(selected, count, true);
                    show(&down, window, cx, highlight);
                    cx.stop_propagation();
                }
            });
        if open {
            let menu_focus = state.read(cx).menu_focus.clone();
            let generation = state.read(cx).generation;
            let close = state.clone();
            let mut menu = PopupMenu::new(
                (self.id.clone(), format!("menu-{generation}")),
                &self.tokens,
            )
            .initially_selected(state.read(cx).highlight)
            .track_focus(&menu_focus)
            .on_dismiss(move |window, cx| {
                let focus = close.update(cx, |s, cx| {
                    s.open = false;
                    cx.notify();
                    s.focus.clone()
                });
                focus.focus(window, cx);
            });
            for (index, option) in self.options.into_iter().enumerate() {
                let state = state.clone();
                let callback = self.on_select.clone();
                menu = menu.item(option.id, option.label, true, move |window, cx| {
                    state.update(cx, |s, cx| {
                        s.selected = Some(index);
                        cx.notify();
                    });
                    if let Some(callback) = &callback {
                        callback(index, window, cx);
                    }
                });
            }
            root = root.child(div().absolute().top_full().left_0().child(menu));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_and_builder() {
        for t in crate::desktop::theme::both() {
            let select = Select::new("select", &t)
                .option("a", "A")
                .option("b", "B")
                .selected(99);
            assert_eq!(select.id, gpui_kit::ElementId::from("select"));
            assert!(select.tab_stop());
            assert_eq!(select.selected_index(), Some(1));
            assert_eq!(move_index(Some(1), 2, true), Some(0));
            assert_eq!(move_index(Some(0), 2, false), Some(1));
            assert_eq!(move_index(None, 0, true), None);
            assert_eq!(
                select.focus_ring(true),
                crate::desktop::theme::focus_ring(&t)
            );
            assert!(select.focus_ring(false).is_empty());
            assert_eq!(
                Select::new("empty", &t)
                    .selected(usize::MAX)
                    .selected_index(),
                None
            );
            assert_eq!(move_index(None, 2, false), Some(1));
        }
    }
}
