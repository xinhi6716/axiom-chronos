//! A focusable empty-state message with optional caller-supplied content.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{
    font, space, ButtonVariant, Density, Step, Tokens, TypeSize,
};
use gpui_kit::{
    div, App, ElementId, FocusHandle, IntoElement, ParentElement, RenderOnce, SharedString, Styled,
    Window,
};

#[derive(IntoElement)]
pub struct EmptyState {
    toolbar: Toolbar,
    variant: ButtonVariant,
    tokens: Tokens,
    title: SharedString,
    header: Option<gpui_kit::AnyElement>,
    icon: Option<gpui_kit::AnyElement>,
    line: Option<gpui_kit::AnyElement>,
    action: Option<gpui_kit::AnyElement>,
}
impl EmptyState {
    pub fn new(
        id: impl Into<ElementId>,
        message: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            variant: ButtonVariant::Ghost,
            tokens: *tokens,
            title: message.into(),
            header: None,
            icon: None,
            line: None,
            action: None,
        }
    }
    pub fn header(mut self, value: impl IntoElement) -> Self {
        self.header = Some(value.into_any_element());
        self
    }
    pub fn media(self, value: impl IntoElement) -> Self {
        self.icon(value)
    }
    pub fn description(self, value: impl IntoElement) -> Self {
        self.line(value)
    }
    pub fn content(self, value: impl IntoElement) -> Self {
        self.action(value)
    }
    pub fn icon(mut self, value: impl IntoElement) -> Self {
        self.icon = Some(value.into_any_element());
        self
    }
    pub fn title(mut self, value: impl Into<SharedString>) -> Self {
        self.title = value.into();
        self
    }
    pub fn line(mut self, value: impl IntoElement) -> Self {
        self.line = Some(value.into_any_element());
        self
    }
    pub fn action(mut self, value: impl IntoElement) -> Self {
        self.action = Some(value.into_any_element());
        self
    }
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Header,
    Icon,
    Title,
    Line,
    Action,
}
impl EmptyState {
    fn into_slots(self) -> (Toolbar, Vec<(Slot, gpui_kit::AnyElement)>) {
        let mut slots = Vec::new();
        if let Some(header) = self.header {
            slots.push((Slot::Header, header));
        }
        if let Some(icon) = self.icon {
            slots.push((Slot::Icon, icon));
        }
        slots.push((
            Slot::Title,
            div()
                .text_size(font(Density::Comfortable, TypeSize::Title))
                .text_color(self.tokens.text)
                .child(self.title)
                .into_any_element(),
        ));
        if let Some(line) = self.line {
            slots.push((
                Slot::Line,
                div()
                    .text_size(font(Density::Comfortable, TypeSize::Body))
                    .text_color(self.tokens.muted)
                    .child(line)
                    .into_any_element(),
            ));
        }
        if let Some(action) = self.action {
            slots.push((Slot::Action, action));
        }
        (self.toolbar.variant(self.variant), slots)
    }
    pub(super) fn into_toolbar(self) -> Toolbar {
        let (toolbar, slots) = self.into_slots();
        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(space(Density::Comfortable, Step::Sm))
            .children(slots.into_iter().map(|(_, element)| element));
        toolbar.child(content)
    }
}
impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.into_toolbar()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::button_colors;
    #[test]
    fn identity_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let empty = EmptyState::new("empty", "No items", &t).variant(ButtonVariant::Danger);
            assert_eq!(empty.toolbar.id, ElementId::from("empty"));
            assert_eq!(button_colors(empty.variant, &t).background, t.fail);
        }
    }
}

#[cfg(test)]
mod slots_tests {
    use super::*;
    #[test]
    fn toolbar_slot_order() {
        for t in crate::desktop::theme::both() {
            let empty = EmptyState::new("empty", "Title", &t)
                .icon("icon")
                .line("line")
                .action("action");
            let (toolbar, slots) = empty.into_slots();
            assert_eq!(
                slots.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
                [Slot::Icon, Slot::Title, Slot::Line, Slot::Action]
            );
            assert_eq!(toolbar.id, ElementId::from("empty"));
            let (_, slots) = EmptyState::new("empty", "Title", &t).into_slots();
            assert_eq!(
                slots.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
                [Slot::Title]
            );
        }
    }
    #[test]
    fn slots_are_retained() {
        for t in crate::desktop::theme::both() {
            let empty = EmptyState::new("empty", "Title", &t)
                .icon("icon")
                .title("New title")
                .line("detail")
                .action("action");
            assert!(empty.icon.is_some() && empty.line.is_some() && empty.action.is_some());
            assert_eq!(empty.title.as_ref(), "New title");
        }
    }
}

#[cfg(test)]
mod wishes_slots_tests {
    use super::*;
    #[test]
    fn named_slots_render_in_order() {
        for t in crate::desktop::theme::both() {
            let empty = EmptyState::new("empty", "title", &t)
                .header("header")
                .media("media")
                .description("description")
                .content("content");
            let (_, slots) = empty.into_slots();
            assert_eq!(
                slots.iter().map(|(slot, _)| *slot).collect::<Vec<_>>(),
                [
                    Slot::Header,
                    Slot::Icon,
                    Slot::Title,
                    Slot::Line,
                    Slot::Action
                ]
            );
        }
    }
}
