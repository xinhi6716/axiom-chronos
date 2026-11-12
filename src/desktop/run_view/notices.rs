use super::*;
use crate::desktop::{
    live::events::notice_copy,
    ux::events::{Notice, NoticeAction, NoticeKind},
};
use gpui_kit::base::TestTextExt as _;
use std::rc::Rc;

pub(super) fn notice_card(
    n: &Notice,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_action: Rc<dyn Fn(NoticeAction, &mut Window, &mut App)>,
) -> AnyElement {
    let id = match n.kind {
        NoticeKind::MaxTurns { .. } => "notice-max-turns",
        NoticeKind::DegradedCouncil { .. } => "notice-degraded-council",
        NoticeKind::ScopeFailure { .. } => "notice-scope-failure",
    };
    let copy = notice_copy(n, lang);
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .p(theme::space(d, Step::Sm))
        .gap(theme::space(d, Step::Sm))
        .border_1()
        .border_color(t.border)
        .text_size(theme::font(d, TypeSize::Caption))
        .text_color(t.text)
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(copy.title.clone().test_text(copy.title)),
        )
        .child(copy.cause.clone().test_text(copy.cause))
        .child(copy.effect.clone().test_text(copy.effect))
        .when_some(copy.action, |card, (action, label)| {
            card.child(
                Button::new("notice-action")
                    .small()
                    .accessibility_label(label.clone())
                    .child(label.clone().test_text(label))
                    .on_click(move |_, window, cx| on_action(action.clone(), window, cx)),
            )
        })
        .into_any_element()
}
