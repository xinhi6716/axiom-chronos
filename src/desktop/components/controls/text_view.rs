//! Focusable selectable, wrapped read-only text.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{
    font, space, ButtonVariant, Step, Tokens, TypeSize, REM_BASE,
};
use crate::desktop::theme;
use gpui_kit::base::{
    text::{TextView as MarkdownView, TextViewStyle},
    SelectableText,
};
use gpui_kit::{
    div, rems, App, ClickEvent, ElementId, IntoElement, ParentElement, RenderOnce, SharedString,
    StyleRefinement, Styled, Window,
};
use std::sync::Arc;
struct LinkCallback<E, W, C>(Arc<dyn Fn(&SharedString, &E, &mut W, &mut C) + Send + Sync>);
impl<E, W, C> LinkCallback<E, W, C> {
    fn new(callback: impl Fn(&SharedString, &E, &mut W, &mut C) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }
    fn call(&self, url: &SharedString, event: &E, window: &mut W, cx: &mut C) {
        (self.0)(url, event, window, cx);
    }
}
type LinkHandler = LinkCallback<ClickEvent, Window, App>;
#[derive(IntoElement)]
pub struct TextView {
    id: ElementId,
    text: SharedString,
    size: TypeSize,
    tokens: Tokens,
    markdown: bool,
    on_link_click: Option<LinkHandler>,
}
impl TextView {
    fn resolved_style(&self, d: theme::Density) -> (gpui_kit::Pixels, gpui_kit::Hsla) {
        (font(d, self.size), self.tokens.text)
    }
    fn text_id(&self) -> ElementId {
        (self.id.clone(), "text").into()
    }

    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>, tokens: &Tokens) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            size: TypeSize::Body,
            tokens: *tokens,
            markdown: false,
            on_link_click: None,
        }
    }
    pub fn markdown(
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        tokens: &Tokens,
    ) -> Self {
        let mut view = Self::new(id, text, tokens);
        view.markdown = true;
        view
    }
    pub fn on_link_click(
        mut self,
        callback: impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + Send + Sync + 'static,
    ) -> Self {
        self.on_link_click = Some(LinkCallback::new(callback));
        self
    }
    fn markdown_style(&self, d: theme::Density) -> TextViewStyle {
        TextViewStyle::default()
            .with_foreground(self.tokens.text)
            .with_muted_foreground(self.tokens.muted)
            .with_link(self.tokens.gold)
            .with_selection(self.tokens.gold)
            .with_code_background(self.tokens.raised)
            .with_border(self.tokens.border)
            .with_paragraph_gap(rems(space(d, Step::Sm) / REM_BASE))
            .with_heading(move |level| {
                StyleRefinement::default().text_size(font(
                    d,
                    match level {
                        1 => TypeSize::Heading,
                        2 | 3 => TypeSize::Title,
                        _ => TypeSize::Caption,
                    },
                ))
            })
            .with_code_block(
                StyleRefinement::default()
                    .text_size(font(d, TypeSize::Caption))
                    .px(space(d, Step::Md))
                    .py(space(d, Step::Sm)),
            )
    }
    pub fn size(mut self, size: TypeSize) -> Self {
        self.size = size;
        self
    }
}
impl TextView {
    fn into_toolbar(self, d: theme::Density) -> Toolbar {
        let text_id = self.text_id();
        let (size, color) = self.resolved_style(d);
        let content = if self.markdown {
            let mut view = MarkdownView::markdown(text_id, self.text.clone())
                .selectable(true)
                .style(self.markdown_style(d));
            if let Some(callback) = self.on_link_click {
                view = view.on_link_click(move |url, event, window, cx| {
                    callback.call(url, event, window, cx)
                });
            }
            view.into_any_element()
        } else {
            SelectableText::new(text_id, self.text).into_any_element()
        };
        Toolbar::new(self.id, &self.tokens)
            .variant(ButtonVariant::Ghost)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_color(color)
                    .text_size(size)
                    .child(content),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_uses_tokens_and_stores_link_handler() {
        for t in theme::both() {
            let view = TextView::markdown("markdown", "[link](https://example.com)", &t)
                .on_link_click(|_, _, _, _| {});
            assert!(view.markdown);
            assert!(view.on_link_click.is_some());
            let style = view.markdown_style(theme::Density::Comfortable);
            assert_eq!(style.foreground(), t.text);
            assert_eq!(style.link(), t.gold);
        }
    }
    #[test]
    fn stored_link_handler_receives_url() {
        let received = Arc::new(std::sync::Mutex::new(None));
        let target = received.clone();
        let handler =
            LinkCallback::new(move |url: &SharedString, _: &(), _: &mut (), _: &mut ()| {
                *target.lock().unwrap() = Some(url.clone());
            });
        let url = SharedString::from("https://example.com/path");
        handler.call(&url, &(), &mut (), &mut ());
        assert_eq!(received.lock().unwrap().as_ref(), Some(&url));
    }
    #[test]
    fn resolved_text_style_and_identity() {
        for t in theme::both() {
            for d in [theme::Density::Comfortable, theme::Density::Dense] {
                for size in [
                    TypeSize::Caption,
                    TypeSize::Body,
                    TypeSize::Label,
                    TypeSize::Title,
                    TypeSize::Heading,
                    TypeSize::Display,
                ] {
                    let view = TextView::new("text", "A long text", &t).size(size);
                    assert_eq!(view.resolved_style(d), (font(d, size), t.text));
                    assert_eq!(view.text_id(), (ElementId::from("text"), "text").into());
                    assert_eq!(view.text.as_ref(), "A long text");
                    assert_eq!(view.into_toolbar(d).id, ElementId::from("text"));
                }
            }
        }
    }
}

impl RenderOnce for TextView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_toolbar(theme::density(cx))
    }
}
