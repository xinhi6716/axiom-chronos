use super::super::{
    i18n::{self, Lang, Table},
    theme::{self, Density, Step, Tokens},
    ux::events::EventSummary,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::{button::Button, v_flex, Sizable as _},
    *,
};

const TABLE: Table = &[
    ("Show raw evidence", "顯示原始證據"),
    ("Hide raw evidence", "隱藏原始證據"),
];

pub fn event_row(
    s: &EventSummary,
    expanded: bool,
    lang: Lang,
    _t: &Tokens,
    d: Density,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let text = match lang {
        Lang::En => &s.en,
        Lang::ZhTw => &s.zh,
    }
    .clone();
    let label = i18n::translate(
        lang,
        TABLE,
        if expanded {
            "Hide raw evidence"
        } else {
            "Show raw evidence"
        },
    );
    v_flex()
        .gap(theme::space(d, Step::Xs))
        .child(
            div()
                .id("event-summary")
                .test_support()
                .child(text.clone().test_text(text)),
        )
        .when_some(s.raw.clone(), |this, raw| {
            this.child(
                Button::new("event-raw-toggle")
                    .small()
                    .tooltip(label)
                    .child(label.test_text(label))
                    .on_click(on_toggle),
            )
            .when(expanded, |this| {
                this.child(
                    div()
                        .id("event-raw")
                        .test_support()
                        .child(raw.clone().test_text(raw)),
                )
            })
        })
}
