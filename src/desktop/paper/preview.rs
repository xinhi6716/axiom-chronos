use super::*;
const TABLE: Table = &[
    ("Open PDF", "開啟 PDF"),
    ("Previous", "上一頁"),
    ("Next", "下一頁"),
];
pub fn render(
    preview: Option<&PreviewView>,
    has_pdf: bool,
    lang: Lang,
    cx: &mut Context<PaperPage>,
) -> Option<AnyElement> {
    let t = cx.tokens();
    let mut content = v_flex();
    let (page, pages) = match preview {
        Some(PreviewView::Ready(p)) => {
            content = content.child(
                v_flex().id("paper-preview").test_support().child(
                    img(Arc::new(Image::from_bytes(ImageFormat::Png, p.png.clone())))
                        .w(px(p.width_px as f32))
                        .h(px(p.height_px as f32)),
                ),
            );
            (p.page, p.pages)
        }
        Some(PreviewView::Unavailable { reason, .. }) => {
            content = content.child(marked("paper-preview-unavailable", reason.clone()));
            (0, 0)
        }
        None => (0, 0),
    };
    content = content.child(
        gpui_kit::component::h_flex()
            .child(
                div()
                    .id("action:paper.preview.previous")
                    .test_support()
                    .child(
                        Button::new(
                            "paper-prev-page",
                            i18n::translate(lang, TABLE, "Previous"),
                            &t,
                        )
                        .variant(if page > 1 {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if page > 1 {
                                this.load_preview(page - 1, cx);
                            }
                        })),
                    ),
            )
            .child(format!("{page} / {pages}").test_text(format!("{page} / {pages}")))
            .child(
                div().id("action:paper.preview.next").test_support().child(
                    Button::new("paper-next-page", i18n::translate(lang, TABLE, "Next"), &t)
                        .variant(if page > 0 && page < pages {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if page > 0 && page < pages {
                                this.load_preview(page + 1, cx);
                            }
                        })),
                ),
            )
            .child(
                div()
                    .id("action:paper.preview.open-pdf")
                    .test_support()
                    .child(
                        Button::new(
                            "paper-open-pdf",
                            i18n::translate(lang, TABLE, "Open PDF"),
                            &t,
                        )
                        .variant(if has_pdf {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if has_pdf {
                                this.action(cx, |s, id| s.open_pdf(&id).map(|_| None));
                            }
                        })),
                    ),
            ),
    );
    Some(content.into_any_element())
}
