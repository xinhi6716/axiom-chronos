use super::*;
const TABLE: Table = &[
    ("Open PDF", "開啟 PDF"),
    ("Previous", "上一頁"),
    ("Next", "下一頁"),
];
pub fn render(
    preview: Option<&Preview>,
    lang: Lang,
    cx: &mut Context<PaperPage>,
) -> Option<AnyElement> {
    match preview? {
        Preview::Ready(p) => {
            let page = p.page;
            let pages = p.pages;
            Some(
                v_flex()
                    .id("paper-preview")
                    .test_support()
                    .child(
                        img(Arc::new(Image::from_bytes(ImageFormat::Png, p.png.clone())))
                            .w(px(p.width_px as f32))
                            .h(px(p.height_px as f32)),
                    )
                    .child(
                        gpui_kit::component::h_flex()
                            .child(
                                Button::new("paper-prev-page")
                                    .label(i18n::translate(lang, TABLE, "Previous"))
                                    .disabled(page <= 1)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.load_preview(page - 1, cx)
                                    })),
                            )
                            .child(format!("{page} / {pages}"))
                            .child(
                                Button::new("paper-next-page")
                                    .label(i18n::translate(lang, TABLE, "Next"))
                                    .disabled(page >= pages)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.load_preview(page + 1, cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            )
        }
        Preview::Unavailable { reason, .. } => Some(
            v_flex()
                .child(marked("paper-preview-unavailable", reason.clone()))
                .child(
                    Button::new("paper-open-pdf")
                        .label(i18n::translate(lang, TABLE, "Open PDF"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.action(cx, |s, id| s.open_pdf(&id).map(|_| None))
                        })),
                )
                .into_any_element(),
        ),
    }
}
