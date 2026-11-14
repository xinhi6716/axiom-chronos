use crate::desktop::{i18n::{self, Lang, Table}, motion, ornament, theme, viz};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Member {
    Astra,
    Fable,
    Argon,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Seat {
    Waiting,
    Speaking,
    Done,
    Degraded { status: String, reason: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Agree,
    Dissent,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Dialogue {
    pub from: Member,
    pub to: Member,
    pub stance: Stance,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CouncilView {
    pub active: bool,
    pub phase: SharedString,
    pub round: u32,
    pub rounds_max: Option<u32>,
    pub seats: Vec<(Member, Seat)>,
    pub speaker: Option<Member>,
    pub dialogue: Vec<Dialogue>,
    pub agreed: u32,
    pub majority: u32,
    pub disputed: u32,
    pub spent_usd: Option<f64>,
    pub cap_usd: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum DualCore {
    Off,
    Deciding {
        topic: SharedString,
    },
    Agreed {
        topic: SharedString,
    },
    Split {
        topic: SharedString,
    },
    Adjudicated {
        topic: SharedString,
        by: SharedString,
    },
}

pub use crate::desktop::ux::seams::dual_core::{DualCoreSource, FakeDualCore};
pub use crate::desktop::motion::Driver as CelestialDriver;

const TABLE: Table = &[
    ("Council · round {n} of {max} · {agreed} agreed · {disputed} disputed · ${spent} of ${cap}", "議會 · 第 {n} 輪（共 {max} 輪）· 同意 {agreed} · 爭議 {disputed} · 已用 ${spent}／${cap}"),
    ("Degraded", "狀態異常"), ("Speaking", "發言中"),
    ("Astra is planning", "Astra 正在規劃"), ("Advanced research", "進階研究"),
    ("Agreed", "雙核一致"), ("Awaiting Fable", "等待 Fable 裁決"),
    ("Deciding", "雙核研議中"), ("Ruled by {by}", "由 {by} 裁決"),
];
const EMBLEM_SIDE: f32 = 92.0;
const GLYPH_SIDE: f32 = 20.0;
const CARD_GLYPH_SIDE: f32 = 28.0;
const INACTIVE_ALPHA: f32 = 0.35;
const ACTIVE_ALPHA: f32 = 1.0;
const CHIP_ALPHA: f32 = 0.18;
const TOPIC_WIDTH: f32 = 180.0;
const FLASH_GLOW: f32 = 0.8;

pub(crate) fn member(m: Member) -> (&'static str, ornament::Celestial, usize) {
    match m {
        Member::Astra => ("Astra", ornament::Celestial::Star, 0),
        Member::Fable => ("Fable", ornament::Celestial::Jupiter, 1),
        Member::Argon => ("Argon", ornament::Celestial::Gemini, 2),
    }
}

impl CouncilView {
    fn known_members(&self) -> Vec<Member> {
        let mut members = Vec::new();
        for m in self.seats.iter().map(|(m, _)| *m)
            .chain(self.speaker)
            .chain(self.dialogue.iter().flat_map(|link| [link.from, link.to])) {
            if !members.contains(&m) { members.push(m); }
        }
        members
    }

    pub fn orbit_members(&self, lang: Lang) -> Vec<viz::OrbitMember> {
        self.known_members().into_iter().map(|m| {
            let (name, _, _) = member(m);
            let seat = self.seats.iter().find(|(who, _)| *who == m).map(|(_, seat)| seat);
            let (state, suffix) = match seat {
                Some(Seat::Speaking) => (viz::OrbitState::Speaking, Some("Speaking")),
                Some(Seat::Waiting) => (viz::OrbitState::Waiting, None),
                Some(Seat::Degraded { .. }) => (viz::OrbitState::Degraded, Some("Degraded")),
                Some(Seat::Done) => (viz::OrbitState::Done, None),
                None if self.speaker == Some(m) => (viz::OrbitState::Speaking, Some("Speaking")),
                None => (viz::OrbitState::Idle, None),
            };
            let label = suffix.map_or_else(|| name.to_owned(), |s| format!("{name} · {}", i18n::translate(lang, TABLE, s)));
            let links = self.dialogue.iter().filter(|link| link.from == m || link.to == m).count();
            let weight = if self.dialogue.is_empty() { 0.0 } else { links as f32 / self.dialogue.len() as f32 };
            viz::OrbitMember { id: name.to_owned(), label, state, weight }
        }).collect()
    }

    pub fn line(&self, lang: Lang) -> String {
        let number = |n: Option<f64>| n.map_or_else(|| "—".to_owned(), |n| format!("{n:.2}"));
        i18n::translate(lang, TABLE, "Council · round {n} of {max} · {agreed} agreed · {disputed} disputed · ${spent} of ${cap}")
            .replace("{n}", &self.round.to_string())
            .replace("{max}", &self.rounds_max.map_or_else(|| "—".to_owned(), |n| n.to_string()))
            .replace("{agreed}", &self.agreed.to_string()).replace("{disputed}", &self.disputed.to_string())
            .replace("{spent}", &number(self.spent_usd)).replace("{cap}", &number(self.cap_usd))
    }

    pub fn moving(&self) -> bool { self.active }

    fn lightning(&self, extreme_flash: bool) -> bool { self.active && extreme_flash }

    /// The caller derives `extreme_flash` from `state.ux.extreme_flash`
    /// and sets `driver.view_visible(...)` according to panel visibility.
    pub fn element(&self, driver: &mut motion::Driver, lang: Lang, t: &theme::Tokens, d: theme::Density, extreme_flash: bool) -> AnyElement {
        let content = if self.active {
            let members = self.orbit_members(lang);
            let mut seats = h_flex().items_center().gap(theme::space(d, theme::Step::Sm));
            for (m, item) in self.known_members().into_iter().zip(&members) {
                let (_, glyph, index) = member(m);
                seats = seats.child(v_flex().id(("council-seat", index)).test_support().items_center()
                    .text_size(theme::font(d, theme::TypeSize::Caption)).text_color(t.text)
                    .child(ornament::celestial(t, glyph, GLYPH_SIDE)).child(item.label.clone().test_text(item.label.clone())));
            }
            let line = self.line(lang);
            v_flex().id("council-orbit").test_support().items_center().gap(theme::space(d, theme::Step::Sm))
                .child(h_flex().items_center().child(viz::orbit("council-orbit-viz", &members, driver, t, d))
                    .when(self.lightning(extreme_flash), |e| e.child(div().id("council-lightning").test_support().text_color(t.review)
                        .shadow(ornament::gold_glow(t, FLASH_GLOW)).child("⚡"))))
                .child(seats).child(div().text_center().text_size(theme::font(d, theme::TypeSize::Body))
                    .text_color(t.muted).child(line.clone().test_text(line))).into_any_element()
        } else {
            ornament::frame(t, t.border, t.surface).id("astra-planning").test_support().flex().items_center()
                .gap(theme::space(d, theme::Step::Md)).p(theme::space(d, theme::Step::Xl))
                .child(ornament::celestial(t, ornament::Celestial::Star, CARD_GLYPH_SIDE))
                .child(div().text_size(theme::font(d, theme::TypeSize::Title)).text_color(t.text)
                    .child(i18n::translate(lang, TABLE, "Astra is planning").test_text(i18n::translate(lang, TABLE, "Astra is planning")))).into_any_element()
        };
        h_flex().items_center().gap(theme::space(d, theme::Step::Md)).child(content)
            .child(self.research_emblem(lang, t, d)).into_any_element()
    }

    pub fn research_emblem(&self, lang: Lang, t: &theme::Tokens, d: theme::Density) -> AnyElement {
        let label = i18n::translate(lang, TABLE, "Advanced research");
        ornament::frame(t, t.border, t.surface).id("research-emblem").test_support()
            .w(px(EMBLEM_SIDE)).h(px(EMBLEM_SIDE)).flex().flex_col().items_center().justify_center()
            .opacity(if self.active { ACTIVE_ALPHA } else { INACTIVE_ALPHA })
            .child(ornament::emblem(ornament::Emblem::StarBook, CARD_GLYPH_SIDE, t.gold))
            .child(div().text_center().text_size(theme::font(d, theme::TypeSize::Caption)).text_color(t.muted).child(label.test_text(label)))
            .into_any_element()
    }
}

impl DualCore {
    pub fn star(&self) -> Option<viz::BinaryStar> {
        let (phase, agreement) = match self {
            Self::Off => return None,
            Self::Deciding { .. } => (viz::StarPhase::Approaching, 0.0),
            Self::Agreed { .. } | Self::Adjudicated { .. } => (viz::StarPhase::Agreed, 1.0),
            Self::Split { .. } => (viz::StarPhase::Disagreed, 0.0),
        };
        Some(viz::BinaryStar { a_label: "Astra".into(), b_label: "Argon".into(), phase, agreement })
    }

    /// The caller sets `driver.view_visible(...)` according to panel visibility.
    pub fn element(&self, driver: &mut motion::Driver, lang: Lang, t: &theme::Tokens, d: theme::Density) -> Option<AnyElement> {
        let star = self.star()?;
        let (key, topic, amber) = match self {
            Self::Off => return None,
            Self::Deciding { topic } => ("Deciding", topic, false),
            Self::Agreed { topic } => ("Agreed", topic, false),
            Self::Split { topic } => ("Awaiting Fable", topic, true),
            Self::Adjudicated { topic, .. } => ("Ruled by {by}", topic, false),
        };
        let mut label = i18n::translate(lang, TABLE, key).to_owned();
        if let Self::Adjudicated { by, .. } = self { label = label.replace("{by}", by); }
        let color = if amber { t.review } else { t.gold };
        Some(v_flex().id("dual-core").test_support().items_center().gap(theme::space(d, theme::Step::Xs))
            .child(viz::binary_star("dual-core-viz", &star, driver, t, d))
            .child(div().px(theme::space(d, theme::Step::Sm)).bg(color.opacity(CHIP_ALPHA)).text_color(color)
                .text_size(theme::font(d, theme::TypeSize::Caption)).child(label.clone().test_text(label)))
            .child(div().max_w(px(TOPIC_WIDTH)).text_center().text_color(t.muted)
                .text_size(theme::font(d, theme::TypeSize::Caption)).child(topic.clone())).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
    use std::time::Instant;

    #[test]
    fn seats_and_links_are_real() {
        let mut v = CouncilView::default();
        assert!(v.orbit_members(Lang::En).is_empty());
        v.speaker = Some(Member::Argon);
        let speaker = v.orbit_members(Lang::En);
        assert_eq!(speaker.len(), 1);
        assert_eq!(speaker[0].state, viz::OrbitState::Speaking);
        assert_eq!(speaker[0].label, "Argon · Speaking");
        assert_eq!(speaker[0].id, "Argon");
        v.seats = vec![(Member::Argon, Seat::Done)];
        assert_eq!(v.orbit_members(Lang::En)[0].state, viz::OrbitState::Done);
        v.seats.clear();
        v.speaker = None;
        for (seat, expected) in [
            (Seat::Speaking, viz::OrbitState::Speaking),
            (Seat::Waiting, viz::OrbitState::Waiting),
            (Seat::Degraded { status: "s".into(), reason: "r".into() }, viz::OrbitState::Degraded),
            (Seat::Done, viz::OrbitState::Done),
        ] {
            v.seats = vec![(Member::Astra, seat)];
            let members = v.orbit_members(Lang::En);
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].state, expected);
            assert_eq!(members[0].weight, 0.0);
        }
        v.seats = vec![(Member::Astra, Seat::Speaking), (Member::Fable, Seat::Degraded { status: "".into(), reason: "".into() }), (Member::Argon, Seat::Waiting)];
        v.dialogue = vec![Dialogue { from: Member::Astra, to: Member::Fable, stance: Stance::Agree }, Dialogue { from: Member::Fable, to: Member::Argon, stance: Stance::Dissent }];
        let members = v.orbit_members(Lang::ZhTw);
        assert_eq!(members.iter().map(|m| m.weight).collect::<Vec<_>>(), vec![0.5, 1.0, 0.5]);
        assert_eq!(members[0].label, "Astra · 發言中");
        assert_eq!(members[1].label, "Fable · 狀態異常");
    }

    #[test]
    fn dual_star_maps_all_states() {
        assert!(DualCore::Off.star().is_none());
        for (core, phase, agreement) in [
            (DualCore::Deciding { topic: "x".into() }, viz::StarPhase::Approaching, 0.0),
            (DualCore::Agreed { topic: "x".into() }, viz::StarPhase::Agreed, 1.0),
            (DualCore::Split { topic: "x".into() }, viz::StarPhase::Disagreed, 0.0),
            (DualCore::Adjudicated { topic: "x".into(), by: "Fable".into() }, viz::StarPhase::Agreed, 1.0),
        ] {
            let star = core.star().unwrap();
            assert_eq!((star.phase, star.agreement), (phase, agreement));
            assert_eq!((star.a_label.as_str(), star.b_label.as_str()), ("Astra", "Argon"));
        }
    }

    #[test]
    fn line_and_activity() {
        let mut v = CouncilView { round: 2, rounds_max: Some(3), agreed: 2, disputed: 1, spent_usd: Some(1.25), ..Default::default() };
        assert_eq!(v.line(Lang::En), "Council · round 2 of 3 · 2 agreed · 1 disputed · $1.25 of $—");
        assert_eq!(v.line(Lang::ZhTw), "議會 · 第 2 輪（共 3 輪）· 同意 2 · 爭議 1 · 已用 $1.25／$—");
        assert!(!v.moving());
        assert!(!v.lightning(false));
        assert!(!v.lightning(true));
        v.active = true;
        assert!(v.moving());
        assert!(!v.lightning(false));
        assert!(v.lightning(true));
    }

    #[test]
    fn driver_respects_visibility_and_reduce_motion() {
        let v = CouncilView { active: true, seats: vec![(Member::Astra, Seat::Speaking)], ..Default::default() };
        let t = theme::night();
        let now = Instant::now();
        let mut visible = motion::Driver::at(now, false);
        let _ = v.element(&mut visible, Lang::En, &t, theme::Density::default(), false);
        assert!(visible.wants_frame());
        let mut hidden = motion::Driver::at(now, false);
        hidden.view_visible(false);
        let _ = v.element(&mut hidden, Lang::En, &t, theme::Density::default(), false);
        assert!(!hidden.wants_frame());
        let mut reduced = motion::Driver::at(now, true);
        let _ = v.element(&mut reduced, Lang::En, &t, theme::Density::default(), false);
        assert!(!reduced.wants_frame());
        let inactive = CouncilView { active: false, ..v };
        for (visible, reduce_motion) in [(true, false), (false, false), (true, true)] {
            let mut driver = motion::Driver::at(now, reduce_motion);
            driver.view_visible(visible);
            let _ = inactive.element(&mut driver, Lang::En, &t, theme::Density::default(), true);
            assert!(!driver.wants_frame());
        }
        let core = DualCore::Deciding { topic: "x".into() };
        let mut visible = motion::Driver::at(now, false);
        assert!(core.element(&mut visible, Lang::En, &t, theme::Density::default()).is_some());
        assert!(visible.wants_frame());
        for (visible, reduce_motion) in [(false, false), (true, true)] {
            let mut driver = motion::Driver::at(now, reduce_motion);
            driver.view_visible(visible);
            assert!(core.element(&mut driver, Lang::En, &t, theme::Density::default()).is_some());
            assert!(!driver.wants_frame());
        }
    }
}
