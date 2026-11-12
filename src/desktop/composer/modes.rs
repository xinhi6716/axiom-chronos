use crate::desktop::{
    i18n::Lang,
    theme::{self, Density, Tokens, TypeSize},
    ux::seams::ModeKind,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    base::{TestSupportExt as _, TestTextExt as _},
    component::h_flex,
    *,
};
use std::rc::Rc;

pub(in crate::desktop) fn suggest(text: &str) -> ModeKind {
    let text = text.trim();
    let lower = text.to_lowercase();
    if [
        "discuss",
        "discussion",
        "debate",
        "vote",
        "討論",
        "辯論",
        "投票",
    ]
    .iter()
    .any(|w| lower.contains(w))
    {
        return ModeKind::Plaza;
    }
    if text.ends_with('?')
        || text.ends_with('？')
        || [
            "what ",
            "why ",
            "how ",
            "who ",
            "when ",
            "where ",
            "什麼",
            "如何",
            "為什麼",
            "是否",
        ]
        .iter()
        .any(|w| lower.starts_with(w))
    {
        return ModeKind::Chat;
    }
    if [
        "small change",
        "quick fix",
        "small fix",
        "one small",
        "小改",
        "小修",
        "修正一個",
        "修改一個",
    ]
    .iter()
    .any(|w| lower.contains(w))
    {
        return ModeKind::QuickTask;
    }
    ModeKind::Wish
}

pub(in crate::desktop) fn mode_picker(
    current: ModeKind,
    suggested: Option<ModeKind>,
    lang: Lang,
    t: &Tokens,
    d: Density,
    on_pick: impl Fn(ModeKind, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_pick = Rc::new(on_pick);
    let mut picker = h_flex()
        .id("mode-picker")
        .gap(theme::space(d, theme::Step::Xs));
    for (mode, id, en, zh) in [
        (ModeKind::Wish, "mode-wish", "Wish", "願望"),
        (ModeKind::Chat, "mode-chat", "Chat", "對話"),
        (ModeKind::QuickTask, "mode-quick", "Quick task", "快速任務"),
        (ModeKind::Plaza, "mode-plaza", "Plaza", "廣場討論"),
    ] {
        let label = if lang == Lang::ZhTw { zh } else { en };
        let hint = if lang == Lang::ZhTw {
            "建議"
        } else {
            "Suggested"
        };
        let callback = on_pick.clone();
        let segment = div()
            .id(id)
            .cursor_pointer()
            .px(theme::space(d, theme::Step::Sm))
            .py(theme::space(d, theme::Step::Xs))
            .rounded(theme::RADIUS)
            .bg(if mode == current { t.raised } else { t.surface })
            .text_color(t.text)
            .text_size(theme::font(d, TypeSize::Body))
            .child(label.test_text(label))
            .when(suggested == Some(mode), |el| {
                el.child(
                    div()
                        .text_color(t.muted)
                        .text_size(theme::font(d, TypeSize::Caption))
                        .child(hint.test_text(hint)),
                )
            })
            .on_click(move |_, window, cx| callback(mode, window, cx))
            .test_support();
        picker = picker.child(segment);
    }
    picker.test_support()
}
