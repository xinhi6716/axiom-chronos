#![allow(dead_code)]
mod ux_support;
use axiom::desktop::{demo, i18n::Lang, model::{AppState, Phase, Screen, Stage}, ux::{CouncilView, Member, Seat, Dialogue, Stance, DualCore, FakeDualCore}};
use gpui_kit::TestAppContext;
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
