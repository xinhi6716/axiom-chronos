#![allow(dead_code)]
mod ux_support;
use axiom::desktop::{demo, i18n::Lang, model::{AppState, Phase, Screen, Stage}, theme, ux::{CelestialDriver, CouncilView, Member, Seat, Dialogue, Stance, DualCore, FakeDualCore}};
use gpui_kit::{div, AppContext as _, Context, IntoElement as _, Render, TestAppContext, Window, WindowOptions};
use gpui_kit::test::TestWindowExt as _;
use std::sync::Arc;
use ux_support::{open, capture, shows};

fn state(council: Option<CouncilView>, dual: DualCore, graph: bool) -> AppState {
    let mut s = demo::state();
    s.reduce_motion = true;
    let r = &mut s.runs[s.current.unwrap()];
    if !graph { r.modules.clear(); r.units.clear(); r.goals.clear(); r.stages = vec![Stage { name: "Astra".into(), phase: Phase::Running, note: None }]; }
    let session = r.session.clone();
    s.ux.council.clear();
    if let Some(c) = council { s.ux.council.insert(session.clone(), c); }
    let fake = FakeDualCore::new(); fake.set(&session, dual);
    s.ux.seams.dual_core = Some(Arc::new(fake));
    s
}
fn check(cx: &mut TestAppContext, lang: Lang) {
    let active = CouncilView { active: true, round: 1, rounds_max: Some(3), seats: vec![(Member::Astra, Seat::Speaking), (Member::Fable, Seat::Degraded { status: "unavailable".into(), reason: "timeout".into() }), (Member::Argon, Seat::Waiting)], speaker: Some(Member::Astra), dialogue: vec![Dialogue { from: Member::Astra, to: Member::Fable, stance: Stance::Dissent }], ..Default::default() };
    let (window, _) = open(cx, lang, state(Some(active.clone()), DualCore::Off, false), Screen::Run);
    let lines = capture(cx, window);
    assert!(shows(&lines, "council-orbit", if lang == Lang::En { "Council · round 1" } else { "議會 · 第 1 輪" }), "{lines:?}");
    for name in ["Astra", "Fable", "Argon"] { assert!(shows(&lines, "council-seat", name), "{name}: {lines:?}"); }
    assert!(shows(&lines, "council-seat", if lang == Lang::En { "Astra · Speaking" } else { "Astra · 發言中" }));
    assert!(shows(&lines, "council-seat", if lang == Lang::En { "Degraded" } else { "狀態異常" }));
    let mut inactive = active; inactive.active = false;
    let (window, _) = open(cx, lang, state(Some(inactive), DualCore::Off, false), Screen::Run);
    let lines = capture(cx, window);
    assert!(shows(&lines, "research-emblem", if lang == Lang::En { "Advanced research" } else { "進階研究" }));
    assert!(!lines.iter().any(|l| l.path.iter().any(|id| id == "council-orbit")));
    let (window, _) = open(cx, lang, state(None, DualCore::Off, false), Screen::Run);
    let lines = capture(cx, window);
    assert!(shows(&lines, "astra-planning", if lang == Lang::En { "Astra is planning" } else { "Astra 正在規劃" }));
    assert!(!lines.iter().any(|l| l.path.iter().any(|id| id == "council-orbit" || id == "dual-core")));
    for (core, en, zh) in [
        (DualCore::Deciding { topic: "Topic".into() }, "Deciding", "雙核研議中"),
        (DualCore::Agreed { topic: "Topic".into() }, "Agreed", "雙核一致"),
        (DualCore::Split { topic: "Topic".into() }, "Awaiting Fable", "等待 Fable 裁決"),
        (DualCore::Adjudicated { topic: "Topic".into(), by: "Fable".into() }, "Ruled by Fable", "由 Fable 裁決"),
    ] {
        let (window, _) = open(cx, lang, state(None, core, false), Screen::Run);
        let lines = capture(cx, window);
        assert!(shows(&lines, "dual-core", if lang == Lang::En { en } else { zh }), "{lines:?}");
    }
    let (window, _) = open(cx, lang, state(None, DualCore::Agreed { topic: "Topic".into() }, true), Screen::Run);
    assert!(shows(&capture(cx, window), "dual-core", if lang == Lang::En { "Agreed" } else { "雙核一致" }));
}
#[gpui_kit::test] fn english(cx: &mut TestAppContext) { check(cx, Lang::En); }
#[gpui_kit::test] fn traditional_chinese(cx: &mut TestAppContext) { check(cx, Lang::ZhTw); }

enum Subject {
    Council(CouncilView, bool),
    Emblem(CouncilView),
    Core(DualCore),
}
struct CelestialFixture { subject: Subject, lang: Lang }
impl Render for CelestialFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui_kit::IntoElement {
        let t = theme::night();
        let d = theme::Density::default();
        let mut driver = CelestialDriver::new(true, cx);
        match &self.subject {
            Subject::Council(c, flash) => c.element(&mut driver, self.lang, &t, d, *flash),
            Subject::Emblem(c) => c.research_emblem(self.lang, &t, d),
            Subject::Core(core) => core.element(&mut driver, self.lang, &t, d)
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}

fn direct(cx: &mut TestAppContext, subject: Subject, lang: Lang, check: impl FnOnce(&mut Window)) {
    let (window, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|_| CelestialFixture { subject, lang })
        })
    }).expect("open celestial fixture");
    cx.update_window(window, |_, window, cx| window.render_frame(cx)).expect("render");
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| check(window)).expect("inspect");
}

fn has_text(window: &Window, id: &'static str, expected: &str) -> bool {
    gpui_kit::base::test_support::texts(window).iter().any(|text| {
        text.visible() && text.text().contains(expected) && text.path().iter().any(|part| match part {
            gpui_kit::ElementId::Name(name) | gpui_kit::ElementId::NamedInteger(name, _) => name.as_ref() == id,
            _ => false,
        })
    })
}

#[gpui_kit::test]
fn celestial_elements_render_their_data(cx: &mut TestAppContext) {
    for active in [false, true] {
        for flash in [false, true] {
            let council = CouncilView { active, speaker: Some(Member::Argon), ..Default::default() };
            direct(cx, Subject::Council(council, flash), Lang::En, |window| {
                assert_eq!(window.try_find("council-orbit").is_some(), active);
                assert_eq!(window.try_find("astra-planning").is_some(), !active);
                assert_eq!(window.try_find("council-lightning").is_some(), active && flash);
                assert!(window.try_find("research-emblem").is_some());
                if active {
                    assert!(window.try_find(("council-seat", 2_usize)).is_some());
                    assert!(has_text(window, "council-seat", "Argon · Speaking"));
                    assert!(has_text(window, "council-orbit", "Council · round"));
                } else {
                    assert!(has_text(window, "astra-planning", "Astra is planning"));
                }
            });
        }
    }
    direct(cx, Subject::Emblem(CouncilView::default()), Lang::ZhTw, |window| {
        assert!(window.try_find("research-emblem").is_some());
        assert!(has_text(window, "research-emblem", "進階研究"));
    });
    for (core, en, zh) in [
        (DualCore::Deciding { topic: "Topic".into() }, "Deciding", "雙核研議中"),
        (DualCore::Agreed { topic: "Topic".into() }, "Agreed", "雙核一致"),
        (DualCore::Split { topic: "Topic".into() }, "Awaiting Fable", "等待 Fable 裁決"),
        (DualCore::Adjudicated { topic: "Topic".into(), by: "Argon".into() }, "Ruled by Argon", "由 Argon 裁決"),
    ] {
        for (lang, label) in [(Lang::En, en), (Lang::ZhTw, zh)] {
            direct(cx, Subject::Core(core.clone()), lang, |window| {
                assert!(window.try_find("dual-core").is_some());
                assert!(has_text(window, "dual-core", label), "missing {label}");
            });
        }
    }
    direct(cx, Subject::Core(DualCore::Off), Lang::En, |window| {
        assert!(window.try_find("dual-core").is_none());
    });
}
