use super::{i18n, ornament, theme, Lang, Table, Tokens};
use super::super::ux::{CouncilView, Member, Seat, Stance};
use gpui_kit::component::v_flex;
use gpui_kit::*;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use std::f32::consts::TAU;

const TABLE: Table = &[
    ("Council · round {n} of {max} · {agreed} agreed · {disputed} disputed · ${spent} of ${cap}", "議會 · 第 {n} 輪（共 {max} 輪）· 同意 {agreed} · 爭議 {disputed} · 已用 ${spent}／${cap}"),
    ("Degraded", "狀態異常"),
    ("Speaking", "發言中"),
    ("Astra is planning", "Astra 正在規劃"),
    ("Advanced research", "進階研究"),
];

pub fn council_line(v: &CouncilView, lang: Lang) -> String {
    let number = |n: Option<f64>| n.map_or_else(|| "—".to_owned(), |n| format!("{n:.2}"));
    i18n::translate(lang, TABLE, "Council · round {n} of {max} · {agreed} agreed · {disputed} disputed · ${spent} of ${cap}")
        .replace("{n}", &v.round.to_string())
        .replace("{max}", &v.rounds_max.map_or_else(|| "—".to_owned(), |n| n.to_string()))
        .replace("{agreed}", &v.agreed.to_string())
        .replace("{disputed}", &v.disputed.to_string())
        .replace("{spent}", &number(v.spent_usd))
        .replace("{cap}", &number(v.cap_usd))
}

pub fn orbit_moving(v: &CouncilView) -> bool { v.active }

// p8: begin
pub(crate) fn member(m: Member) -> (&'static str, ornament::Celestial, usize) {
    match m {
        Member::Astra => ("Astra", ornament::Celestial::Star, 0),
        Member::Fable => ("Fable", ornament::Celestial::Jupiter, 1),
        Member::Argon => ("Argon", ornament::Celestial::Gemini, 2),
    }
}
// p8: end
fn position(index: usize, phase: f32) -> (f32, f32) {
    let a = phase + index as f32 * TAU / 3.0 - TAU / 4.0;
    (150. + 105. * a.cos(), 112. + 70. * a.sin())
}

pub fn council_orbit(v: &CouncilView, phase: f32, lang: Lang, t: &Tokens, d: theme::Density) -> AnyElement {
    let t = *t;
    let links = v.dialogue.clone();
    let seats = v.seats.clone();
    let speaker = v.speaker;
    let line = council_line(v, lang);
    let drawing = canvas(|_, _, _| (), move |bounds, _, window, _| {
        let x = f32::from(bounds.origin.x); let y = f32::from(bounds.origin.y);
        let ring: Vec<_> = (0..64).map(|i| { let (px_, py_) = position(0, i as f32 * TAU / 64.); point(px(x + px_), px(y + py_)) }).collect();
        super::stroke_points(window, &ring, true, 1., t.orbit);
        for link in &links {
            let (ax, ay) = position(member(link.from).2, phase);
            let (bx, by) = position(member(link.to).2, phase);
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(point(px(x + ax), px(y + ay)));
            path.curve_to(point(px(x + bx), px(y + by)), point(px(x + 150.), px(y + 112.)));
            if let Ok(path) = path.build() { window.paint_path(path, if link.stance == Stance::Agree { t.gold } else { t.review }); }
        }
    }).absolute().top_0().left_0().w(px(300.)).h(px(224.));
    let mut orbit = div().relative().w(px(300.)).h(px(224.)).child(drawing)
        .child(div().absolute().left(px(132.)).top(px(94.)).child(ornament::hourglass(&t, Some(0.5), t.gold, 36.)));
    for (m, seat) in seats {
        let (name, glyph, index) = member(m);
        let (x, y) = position(index, phase);
        let is_speaker = speaker == Some(m);
        let degraded = matches!(seat, Seat::Degraded { .. });
        let label = if degraded { format!("{name} · {}", i18n::translate(lang, TABLE, "Degraded")) } else if is_speaker { format!("{name} · {}", i18n::translate(lang, TABLE, "Speaking")) } else { name.to_owned() };
        orbit = orbit.child(v_flex().id(("council-seat", index)).test_support().absolute().left(px(x - 48.)).top(px(y - 18.))
            .w(px(96.)).items_center().text_center().text_size(theme::font(d, theme::TypeSize::Caption))
            .text_color(if degraded { t.fail } else if is_speaker { t.gold } else { t.text })
            .when(is_speaker, |e| e.shadow(ornament::gold_glow(&t, 0.8)))
            .child(ornament::celestial(&t, glyph, 20.)).child(label.clone().test_text(label)));
    }
    v_flex().id("council-orbit").test_support().items_center().gap(theme::space(d, theme::Step::Sm))
        .child(orbit).child(div().text_center().text_size(theme::font(d, theme::TypeSize::Body)).text_color(t.muted).child(line.clone().test_text(line))).into_any_element()
}

pub fn research_emblem(active: bool, lang: Lang, t: &Tokens, side: f32) -> AnyElement {
    let t = *t;
    div().id("research-emblem").test_support().relative().w(px(side)).h(px(side))
        .opacity(if active { 1. } else { 0.35 })
        .child(super::super::superswitch::backdrop(super::super::superswitch::Look::Lit, t))
        .child(div().absolute().bottom_0().w_full().text_center().text_size(theme::font(theme::Density::default(), theme::TypeSize::Caption)).text_color(t.muted).child(i18n::translate(lang, TABLE, "Advanced research").test_text(i18n::translate(lang, TABLE, "Advanced research"))))
        .into_any_element()
}

pub fn astra_card(lang: Lang, t: &Tokens, d: theme::Density) -> AnyElement {
    ornament::frame(t, t.border, t.surface).id("astra-planning").test_support().flex().items_center()
        .gap(theme::space(d, theme::Step::Md)).p(theme::space(d, theme::Step::Xl))
        .child(ornament::celestial(t, ornament::Celestial::Star, 28.))
        .child(div().text_size(theme::font(d, theme::TypeSize::Title)).text_color(t.text).child(i18n::translate(lang, TABLE, "Astra is planning").test_text(i18n::translate(lang, TABLE, "Astra is planning"))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    #[test] fn line_formats_known_and_unknown_values() {
        let v = CouncilView { round: 2, rounds_max: Some(3), agreed: 2, disputed: 1, spent_usd: Some(1.25), ..Default::default() };
        assert_eq!(council_line(&v, Lang::En), "Council · round 2 of 3 · 2 agreed · 1 disputed · $1.25 of $—");
        assert_eq!(council_line(&v, Lang::ZhTw), "議會 · 第 2 輪（共 3 輪）· 同意 2 · 爭議 1 · 已用 $1.25／$—");
        assert!(council_line(&CouncilView::default(), Lang::En).contains("of —"));
    }
    #[test] fn motion_tracks_activity() {
        let mut v = CouncilView::default(); assert!(!orbit_moving(&v)); v.active = true; assert!(orbit_moving(&v));
    }
}
