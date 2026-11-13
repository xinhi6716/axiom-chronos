use super::*;
use crate::desktop::{
    model::facade::usage::{council_member, CouncilMember},
    ornament::{self, Celestial},
    theme::Tokens,
};
use crate::paper::PassId;
const TABLE: Table = &[
    ("OK", "完成"),
    ("Missing", "缺失"),
    ("Quota", "額度不足"),
    ("Timeout", "逾時"),
    ("Budget cut", "預算截斷"),
    ("Malformed", "格式錯誤"),
    ("Failed", "失敗"),
];
pub(super) fn glyph(pass: PassId) -> (&'static str, Celestial) {
    let m = match pass {
        PassId::AstraDraft => CouncilMember::Astra,
        PassId::FableRevise | PassId::FableStyle => CouncilMember::Fable,
        PassId::GeminiRevise => return ("Gemini", Celestial::Gemini),
    };
    let (name, glyph, _) = council_member(m);
    (name, glyph)
}
pub fn render(status: Option<&PaperStatus>, lang: Lang, t: &Tokens) -> Vec<AnyElement> {
    let Some(s) = status else { return vec![] };
    PassId::ALL
        .into_iter()
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
                .find(|r| r.pass == p)
                .map(|r| {
                    i18n::translate(
                        lang,
                        TABLE,
                        match r.status {
                            crate::council::Status::Ok => "OK",
                            crate::council::Status::Missing => "Missing",
                            crate::council::Status::Quota => "Quota",
                            crate::council::Status::Timeout => "Timeout",
                            crate::council::Status::BudgetCut => "Budget cut",
                            crate::council::Status::Malformed => "Malformed",
                            crate::council::Status::Failed => "Failed",
                        },
                    )
                    .to_owned()
                })
                .unwrap_or_else(|| "—".into());
            gpui_kit::component::h_flex()
                .id(("paper-pass-slot", p.number() as usize))
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
