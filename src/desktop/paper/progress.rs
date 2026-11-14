use super::*;
use crate::desktop::{
    model::facade::usage::{council_member, CouncilMember},
    ornament::{self, Celestial},
    theme::Tokens,
};

const TABLE: Table = &[
    ("OK", "完成"),
    ("Missing", "缺失"),
    ("Quota", "額度不足"),
    ("Timeout", "逾時"),
    ("Budget cut", "預算截斷"),
    ("Malformed", "格式錯誤"),
    ("Failed", "失敗"),
];
pub(super) fn glyph(pass: u8) -> (&'static str, Celestial) {
    let m = match pass {
        1 => CouncilMember::Astra,
        2 | 4 => CouncilMember::Fable,
        3 => return ("Gemini", Celestial::Gemini),
        _ => return ("", Celestial::Star),
    };
    let (name, glyph, _) = council_member(m);
    (name, glyph)
}
pub fn render(status: Option<&PaperView>, lang: Lang, t: &Tokens) -> Vec<AnyElement> {
    let Some(s) = status else { return vec![] };
    (1u8..=4)
        .map(|p| {
            let (name, g) = glyph(p);
            let symbol = match g {
                Celestial::Star => "Star",
                Celestial::Jupiter => "Jupiter",
                Celestial::Gemini => "Gemini",
                _ => "",
            };
            let label = format!("{name} · {symbol}");
            let state = s
                .passes
                .iter()
                .find(|r| r.number == p)
                .map(|r| {
                    i18n::translate(
                        lang,
                        TABLE,
                        match r.status.as_str() {
                            "Ok" => "OK",
                            "Missing" => "Missing",
                            "Quota" => "Quota",
                            "Timeout" => "Timeout",
                            "BudgetCut" => "Budget cut",
                            "Malformed" => "Malformed",
                            _ => "Failed",
                        },
                    )
                    .to_owned()
                })
                .unwrap_or_else(|| "—".into());
            gpui_kit::component::h_flex()
                .id(("paper-pass-slot", p as usize))
                .gap(crate::desktop::theme::space(
                    crate::desktop::theme::Density::default(),
                    crate::desktop::theme::Step::Xs,
                ))
                .child(ornament::celestial(
                    t,
                    g,
                    f32::from(crate::desktop::theme::space(
                        crate::desktop::theme::Density::default(),
                        crate::desktop::theme::Step::Lg,
                    )),
                ))
                .child(
                    div()
                        .id("paper-pass")
                        .test_support()
                        .child(label.clone().test_text(label)),
                )
                .child(state)
                .into_any_element()
        })
        .collect()
}
