//! Token-styled menu and context-menu trigger.
use super::button::{ring, Button};
use super::keys::{self, Cancel, Confirm, MoveDown, MoveUp};
use crate::desktop::components::tokens::{
    radius, space, ButtonVariant, Density, Radius, Step, Tokens,
};
use gpui_kit::{
    div, AnyElement, App, ElementId, FocusHandle, Global, InteractiveElement, IntoElement,
    MouseButton, ParentElement, RenderOnce, SharedString, Styled, Window,
};
use std::rc::Rc;

type Handler = Rc<dyn Fn(&mut Window, &mut App)>;
#[derive(Clone)]
enum Entry {
    Item {
        id: ElementId,
        label: SharedString,
        enabled: bool,
        handler: Handler,
    },
    Separator,
}
#[derive(Clone, IntoElement)]
pub struct PopupMenu {
    pub(super) id: ElementId,
    tokens: Tokens,
    entries: Vec<Entry>,
    focus: Option<FocusHandle>,
    dismiss: Option<Handler>,
    initial: Option<usize>,
}
impl PopupMenu {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            tokens: *tokens,
            entries: Vec::new(),
            focus: None,
            dismiss: None,
            initial: None,
        }
    }
    pub fn item(
        mut self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        enabled: bool,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.entries.push(Entry::Item {
            id: id.into(),
            label: label.into(),
            enabled,
            handler: Rc::new(handler),
        });
        self
    }
    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }
    pub fn on_dismiss(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.dismiss = Some(Rc::new(callback));
        self
    }
    pub(super) fn initially_selected(mut self, index: Option<usize>) -> Self {
        self.initial = index;
        self
    }
    fn tab_stop(&self) -> bool {
        true
    }
    fn focus_ring(&self, focused: bool) -> Vec<gpui_kit::BoxShadow> {
        ring(focused, &self.tokens)
    }
    pub(super) fn move_selection(&self, current: Option<usize>, forward: bool) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        let start = current.filter(|&n| n < len);
        for step in 0..len {
            let index = match (start, forward) {
                (Some(i), true) => (i + 1 + step) % len,
                (Some(i), false) => (i + len - 1 - step) % len,
                (None, true) => step,
                (None, false) => len - 1 - step,
            };
            if matches!(self.entries[index], Entry::Item { enabled: true, .. }) {
                return Some(index);
            }
        }
        None
    }
}
struct Bindings;
impl Global for Bindings {}
pub(super) fn bind(cx: &mut App) {
    if !cx.has_global::<Bindings>() {
        keys::bind_list("AxiomPopupMenu", cx);
        cx.set_global(Bindings);
    }
}
struct MenuState {
    focus: FocusHandle,
    selected: Option<usize>,
}
impl RenderOnce for PopupMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        bind(cx);
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| MenuState {
            focus: cx.focus_handle(),
            selected: self.initial,
        });
        let focus = self
            .focus
            .clone()
            .unwrap_or_else(|| state.read(cx).focus.clone())
            .tab_stop(self.tab_stop());
        let selection = state
            .read(cx)
            .selected
            .filter(|&i| matches!(self.entries.get(i), Some(Entry::Item { enabled: true, .. })));
        let up = state.clone();
        let down = state.clone();
        let confirm = state.clone();
        let up_menu = self.clone();
        let down_menu = self.clone();
        let confirm_entries = self.entries.clone();
        let confirm_dismiss = self.dismiss.clone();
        let cancel_dismiss = self.dismiss.clone();
        let mut panel = div()
            .id(self.id.clone())
            .key_context("AxiomPopupMenu")
            .track_focus(&focus)
            .flex()
            .flex_col()
            .p(space(Density::default(), Step::Xs))
            .bg(self.tokens.raised)
            .text_color(self.tokens.text)
            .rounded(radius(Radius::Md))
            .border_1()
            .border_color(self.tokens.border)
            .shadow(self.focus_ring(focus.is_focused(window)))
            .on_action(move |_: &MoveUp, _, cx| {
                up.update(cx, |s, cx| {
                    s.selected = up_menu.move_selection(s.selected, false);
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .on_action(move |_: &MoveDown, _, cx| {
                down.update(cx, |s, cx| {
                    s.selected = down_menu.move_selection(s.selected, true);
                    cx.notify();
                });
                cx.stop_propagation();
            })
            .on_action(move |_: &Confirm, window, cx| {
                if let Some(Entry::Item {
                    enabled: true,
                    handler,
                    ..
                }) = confirm
                    .read(cx)
                    .selected
                    .and_then(|i| confirm_entries.get(i))
                {
                    handler(window, cx);
                    if let Some(dismiss) = &confirm_dismiss {
                        dismiss(window, cx);
                    }
                }
                cx.stop_propagation();
            })
            .on_action(move |_: &Cancel, window, cx| {
                if let Some(dismiss) = &cancel_dismiss {
                    dismiss(window, cx);
                }
                cx.stop_propagation();
            });
        for (index, entry) in self.entries.into_iter().enumerate() {
            match entry {
                Entry::Separator => {
                    panel = panel.child(div().border_b_1().border_color(self.tokens.border))
                }
                Entry::Item {
                    id,
                    label,
                    enabled,
                    handler,
                } if enabled => {
                    let dismiss = self.dismiss.clone();
                    panel = panel.child(
                        Button::new(id, label, &self.tokens)
                            .variant(if selection == Some(index) {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click(move |_, window, cx| {
                                handler(window, cx);
                                if let Some(dismiss) = &dismiss {
                                    dismiss(window, cx);
                                }
                            }),
                    );
                }
                Entry::Item { id, label, .. } => {
                    panel = panel.child(
                        div()
                            .id(id)
                            .px(space(Density::default(), Step::Sm))
                            .text_color(self.tokens.muted)
                            .child(label),
                    )
                }
            }
        }
        panel
    }
}

struct ContextState {
    focus: FocusHandle,
    menu_focus: FocusHandle,
    previous: Option<FocusHandle>,
    open: bool,
}
#[derive(IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    element: AnyElement,
    builder: Rc<dyn Fn() -> PopupMenu>,
}
/// Attach a menu to right click, the Menu key, or Shift-F10.
pub fn context_menu(
    element: impl IntoElement,
    id: impl Into<ElementId>,
    builder: impl Fn() -> PopupMenu + 'static,
) -> ContextMenu {
    ContextMenu {
        id: id.into(),
        element: element.into_any_element(),
        builder: Rc::new(builder),
    }
}
impl RenderOnce for ContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| ContextState {
            focus: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            previous: None,
            open: false,
        });
        let focus = state.read(cx).focus.clone().tab_stop(true);
        let open = state.read(cx).open;
        let mouse = state.clone();
        let key = state.clone();
        let open_menu =
            |state: &gpui_kit::Entity<ContextState>, window: &mut Window, cx: &mut App| {
                let previous = window.focused(cx);
                let menu_focus = state.update(cx, |s, cx| {
                    s.previous = previous;
                    s.open = true;
                    cx.notify();
                    s.menu_focus.clone()
                });
                menu_focus.focus(window, cx);
            };
        let mut root = div()
            .id(self.id)
            .relative()
            .track_focus(&focus)
            .child(self.element)
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                open_menu(&mouse, window, cx);
                cx.stop_propagation();
            })
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "menu"
                    || (event.keystroke.key == "f10" && event.keystroke.modifiers.shift)
                {
                    open_menu(&key, window, cx);
                    cx.stop_propagation();
                }
            });
        if open {
            let close = state.clone();
            let menu_focus = state.read(cx).menu_focus.clone();
            let menu = (self.builder)()
                .track_focus(&menu_focus)
                .on_dismiss(move |window, cx| {
                    let previous = close.update(cx, |s, cx| {
                        s.open = false;
                        cx.notify();
                        s.previous.take().unwrap_or_else(|| s.focus.clone())
                    });
                    previous.focus(window, cx);
                });
            root = root.child(div().absolute().top_full().left_0().child(menu));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_navigation_and_builder() {
        for t in crate::desktop::theme::both() {
            let menu = PopupMenu::new("menu", &t)
                .item("a", "A", true, |_, _| {})
                .separator()
                .item("b", "B", false, |_, _| {})
                .item("c", "C", true, |_, _| {});
            assert_eq!(menu.id, gpui_kit::ElementId::from("menu"));
            assert!(menu.tab_stop());
            assert_eq!(menu.move_selection(None, true), Some(0));
            assert_eq!(menu.move_selection(Some(0), true), Some(3));
            assert_eq!(menu.move_selection(Some(3), false), Some(0));
            assert_eq!(menu.focus_ring(true), crate::desktop::theme::focus_ring(&t));
            assert!(menu.focus_ring(false).is_empty());
            assert_eq!(menu.move_selection(None, false), Some(3));
            assert_eq!(menu.move_selection(Some(usize::MAX), true), Some(0));
            let disabled =
                PopupMenu::new("disabled", &t)
                    .separator()
                    .item("x", "X", false, |_, _| {});
            assert_eq!(disabled.move_selection(None, true), None);
            let context = context_menu(div(), "context", move || PopupMenu::new("inner", &t));
            assert_eq!(context.id, gpui_kit::ElementId::from("context"));
            assert_eq!((context.builder)().id, gpui_kit::ElementId::from("inner"));
        }
    }
}
