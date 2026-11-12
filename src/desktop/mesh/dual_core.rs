use super::{i18n, ornament, theme, Lang, Table, Tokens};
use super::super::ux::DualCore;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use gpui_kit::base::TestTextExt as _;

const TABLE: Table = &[("Agreed", "雙核一致"), ("Awaiting Fable", "等待 Fable 裁決"), ("Deciding", "雙核研議中"), ("Ruled by {by}", "由 {by} 裁決")];
fn status(s: &DualCore, lang: Lang) -> Option<(String, bool)> {
    let (key, amber) = match s {
        DualCore::Off => return None,
        DualCore::Deciding { .. } => ("Deciding", false),
        DualCore::Agreed { .. } => ("Agreed", false),
        DualCore::Split { .. } => ("Awaiting Fable", true),
        DualCore::Adjudicated { .. } => ("Ruled by {by}", false),
    };
    let mut text = i18n::translate(lang, TABLE, key).to_owned();
    if let DualCore::Adjudicated { by, .. } = s { text = text.replace("{by}", by); }
    Some((text, amber))
}
pub fn dual_core(s: &DualCore, _phase: f32, lang: Lang, t: &Tokens, d: theme::Density) -> Option<AnyElement> {
    let (label, amber) = status(s, lang)?;
    let topic = match s { DualCore::Off => unreachable!(), DualCore::Deciding { topic } | DualCore::Agreed { topic } | DualCore::Split { topic } | DualCore::Adjudicated { topic, .. } => topic.clone() };
    let color = if amber { t.review } else { t.gold };
    Some(v_flex().id("dual-core").test_support().items_center().gap(theme::space(d, theme::Step::Xs))
        .child(h_flex().items_center().gap(theme::space(d, theme::Step::Sm))
            .child(ornament::hourglass(t, Some(0.5), t.gold, 26.))
            .child(ornament::celestial(t, ornament::Celestial::Star, 26.)))
        .child(div().px(theme::space(d, theme::Step::Sm)).bg(color.opacity(0.18)).text_color(color).text_size(theme::font(d, theme::TypeSize::Caption)).child(label.clone().test_text(label)))
        .child(div().max_w(px(180.)).text_center().text_color(t.muted).text_size(theme::font(d, theme::TypeSize::Caption)).child(topic))
        .into_any_element())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test] fn state_labels_are_localized_and_off_is_absent() {
        assert!(status(&DualCore::Off, Lang::En).is_none());
        assert_eq!(status(&DualCore::Split { topic: "x".into() }, Lang::ZhTw).unwrap(), ("等待 Fable 裁決".into(), true));
        assert_eq!(status(&DualCore::Adjudicated { topic: "x".into(), by: "Fable".into() }, Lang::En).unwrap().0, "Ruled by Fable");
    }
}
