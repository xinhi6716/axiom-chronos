#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_button_preserves_value_tooltip_and_target_size() {
        let t = crate::desktop::theme::night();
        let button = CopyButton::new("copy", "value", "Copy value", &t);
        assert_eq!(button.value.as_ref(), "value");
        assert_eq!(button.tooltip.as_ref(), "Copy value");
        assert_eq!(button.target_size(), crate::desktop::theme::TARGET_MIN);
    }
}
// Clipboard action with an accessible, token-styled icon button.
use super::icon_button::IconButton;
use crate::desktop::components::tokens::{ButtonVariant, Tokens};
use crate::desktop::theme;
use gpui_kit::component::IconName;
use gpui_kit::{
    App, ClipboardItem, ElementId, IntoElement, Pixels, RenderOnce, SharedString, Window,
};

#[derive(IntoElement)]
pub struct CopyButton {
    id: ElementId,
    value: SharedString,
    tooltip: SharedString,
    tokens: Tokens,
}
impl CopyButton {
    pub fn new(
        id: impl Into<ElementId>,
        value: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            tooltip: tooltip.into(),
            tokens: *tokens,
        }
    }
    fn target_size(&self) -> Pixels {
        theme::TARGET_MIN
    }
}
impl RenderOnce for CopyButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let size = self.target_size();
        let value = self.value;
        IconButton::new(self.id, IconName::Copy, self.tooltip.clone(), &self.tokens)
            .variant(ButtonVariant::Ghost)
            .tooltip(self.tooltip)
            .min_size(size)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()))
            })
    }
}
