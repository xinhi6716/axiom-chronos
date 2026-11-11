use super::*;
use std::prelude::v1::test;

fn run() -> Run {
    Run {
        session: "session".into(), project: "p".into(), cwd: None, wish: "w".into(),
        status: RunStatus::Active, hosted: true, started: "".into(), started_at: None,
        stages: vec![], goals: vec![],
        modules: vec![Module { id: "module-a".into(), purpose: "module".into(), depends: vec![], units: vec![0], contract: None }],
        units: vec![Unit { id: "unit-a".into(), title: "unit".into(), module: 0, state: UnitState::Pending, raw_state: "".into(), attempts: 0, files: UnitFiles::default() }],
        usage: Usage::default(), needs: vec![], log: vec![], decisions: vec![], report: None, elapsed: None, warnings: vec![], notes: vec![],
    }
}

#[test]
fn filter_keys_round_trip() {
    for (filter, key) in [(Filter::All,"all"), (Filter::Active,"active"), (Filter::Review,"review"), (Filter::Failed,"failed")] {
        assert_eq!(filter.to_key(), key);
        assert_eq!(Filter::from_key(Some(key)), filter);
    }
    assert_eq!(Filter::from_key(None), Filter::All);
    assert_eq!(Filter::from_key(Some("bogus")), Filter::All);
}

#[test]
fn targets_use_ids_and_session() {
    let a = run();
    let unit = RowKey::Unit { goal: None, module: None, unit: 0 };
    let module = RowKey::Module { goal: None, module: 0 };
    assert_eq!(menu_target(&a, unit), Some(MenuTarget::Unit { session: "session".into(), unit: "unit-a".into() }));
    assert_eq!(menu_target(&a, module), Some(MenuTarget::Module { session: "session".into(), module: "module-a".into() }));
    assert_eq!(menu_target(&a, RowKey::Section), None);
    assert_eq!(menu_target(&a, RowKey::Goal(0)), None);
    assert_eq!(menu_target(&a, RowKey::Unit { goal: None, module: None, unit: 5 }), None);
    assert_eq!(menu_target(&a, RowKey::Module { goal: None, module: 5 }), None);
    let mut b = a.clone(); b.units[0].id = "unit-b".into();
    assert_ne!(menu_target(&a, unit), menu_target(&b, unit));
    assert_ne!(GlowKey::of(&menu_target(&a, unit).unwrap()), GlowKey::of(&menu_target(&b, unit).unwrap()));
}

#[test]
fn menu_anchor_is_the_bounds_center() {
    let bounds = Bounds { origin: point(px(20.0), px(30.0)), size: size(px(100.0), px(40.0)) };
    assert_eq!(menu_anchor(bounds), point(px(70.0), px(50.0)));
    assert_eq!(menu_anchor(Bounds::default()), point(px(0.0), px(0.0)));
}

fn handles_context_menu(source: &str) -> bool {
    source.contains(".on_action(cx.listener(Self::open_cursor_menu))")
        && source.contains("_: &palette::ContextMenu")
}

#[test]
fn checklist_handles_context_menu_action() {
    assert!(handles_context_menu(include_str!("../todo.rs")));
    assert!(!handles_context_menu(".on_action(cx.listener(Self::other)) // ContextMenu"));
}

#[test]
fn feed_views_cover_planless_and_planned() {
    for planless in [false, true] {
        assert_eq!(view_of(&PanelState::Loading, planless), TodoView::Loading);
        assert_eq!(view_of(&PanelState::Empty, planless), TodoView::Empty);
        assert_eq!(view_of(&PanelState::Ready, planless), if planless { TodoView::Empty } else { TodoView::List });
        assert_eq!(view_of(&PanelState::Failed("bad".into()), planless), TodoView::Failed { message: "bad".into() });
        assert_eq!(view_of(&PanelState::Stale("old".into()), planless), TodoView::Stale { message: "old".into() });
    }
}

#[test]
fn glow_animates_and_reduced_motion_jumps() {
    assert_eq!(glow_target(true), 1.0); assert_eq!(glow_target(false), 0.0);
    let key = GlowKey::Unit("s".into(), "u".into());
    let t = Instant::now(); let mut glow = motion::Glows::new(Pace::Glow);
    glow.set(&key, glow_target(true), t, false);
    let mut mid = Driver::at(t + Pace::Glow.duration() / 2, false);
    let value = glow.value(&key, &mut mid);
    assert!(value > 0.0 && value < 1.0); assert!(mid.wants_frame());
    let mut end = Driver::at(t + Pace::Glow.duration(), false);
    assert_eq!(glow.value(&key, &mut end), 1.0); assert!(!end.wants_frame());
    let mut reduced = motion::Glows::new(Pace::Glow); reduced.set(&key, 1.0, t, true);
    assert_eq!(reduced.value(&key, &mut Driver::at(t, true)), 1.0);
}

fn unlisted<'a>(keys: &'a [String], listed: &[&str]) -> Vec<&'a str> {
    keys.iter().map(String::as_str).filter(|key| !listed.contains(key)).collect()
}

#[test]
fn handled_keys_are_checklist_shortcuts() {
    use super::super::palette::{Area, SHORTCUTS};
    let source = include_str!("../todo.rs");
    let handler = source.split_once("    fn on_key(").unwrap().1.split_once("    fn render_row(").unwrap().0;
    let arms = regex::Regex::new(r#"(?m)^\s*((?:"[a-z0-9-]+"\s*\|\s*)*"[a-z0-9-]+")\s*=>"#).unwrap();
    let literal = regex::Regex::new(r#""([a-z0-9-]+)""#).unwrap();
    let keys: Vec<String> = arms.captures_iter(handler).flat_map(|a| literal.captures_iter(a.get(1).unwrap().as_str()).map(|m| m[1].to_owned()).collect::<Vec<_>>()).collect();
    let listed: Vec<_> = SHORTCUTS.iter().filter(|s| s.area == Area::Checklist).flat_map(|s| s.keys.iter().copied()).collect();
    assert_eq!(keys.len(), 11);
    assert_eq!(unlisted(&keys, &listed), Vec::<&str>::new());
    assert_eq!(unlisted(&["not-listed".into()], &listed), vec!["not-listed"]);
}

// No literal spacing or type sizes are allowed; height, width and border calls are outside the scanner.
const ALLOWLIST: &[(&str, &str)] = &[];
fn raw_scale_literals(source: &str) -> Vec<String> {
    let pattern = regex::Regex::new(r"\.(?:p|px|py|pt|pb|pl|pr|gap|gap_x|gap_y|text_size)\(px\(\s*-?(?:\d+(?:\.\d*)?|\.\d+)\s*\)\)").unwrap();
    pattern.find_iter(source).map(|m| m.as_str()).filter(|s| !ALLOWLIST.iter().any(|(allowed, _)| s == allowed)).map(str::to_owned).collect()
}
#[test]
fn render_spacing_and_type_use_scales() {
    let source = include_str!("../todo.rs");
    for (start, end) in [("    fn render_row(", "    /// How far a row"), ("    fn render_header(", "    fn frame("), ("    fn render_list(", "    fn apply_scene("), ("fn placeholder(", "// HANDOFF OPEN:")] {
        let body = source.split_once(start).unwrap().1.split_once(end).unwrap().0;
        assert_eq!(raw_scale_literals(body), Vec::<String>::new(), "{start}");
    }
    assert_eq!(raw_scale_literals("div().gap(px(7.0))"), vec![".gap(px(7.0))"]);
}
