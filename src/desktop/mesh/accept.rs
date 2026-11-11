use super::*;
use super::super::demo::sample_run;
use super::logic::{handled_keys, missing_from_shortcuts};
use std::prelude::v1::test;

#[test]
fn scenes_require_their_run_counts() {
    for (name, needed) in [("mesh-partial", 4), ("mesh-done", 4), ("mesh-blocked", 3), ("mesh-waiting", 2), ("mesh-large", 1), ("unknown", 1)] {
        assert_eq!(scene_runs_needed(name), needed, "{name}");
    }
}

#[test]
fn set_scene_checks_run_count_before_indexing() {
    let source = include_str!("../mesh.rs");
    let scene = source.split_once("fn set_scene(").unwrap().1.split("\n    fn ").next().unwrap();
    assert!(scene.find("scene_runs_needed(").unwrap() < scene.find("runs[").unwrap());
}

#[test]
fn demo_supplies_scene_runs() {
    let mut state = super::super::demo::state();
    let names = ["mesh-large", "mesh-partial", "mesh-done", "mesh-blocked", "mesh-waiting"];
    for name in names {
        assert!(state.runs.len() >= scene_runs_needed(name), "{name}");
    }
    state.runs.clear();
    for name in names {
        assert!(state.runs.len() < scene_runs_needed(name), "{name}");
    }
}

#[test]
fn menu_targets_follow_current_ids() {
    let run = sample_run();
    assert_eq!(menu_target(&run, Node::Unit(0)), Some(MenuTarget::Unit { session: run.session.clone(), unit: run.units[0].id.clone() }));
    assert_eq!(menu_target(&run, Node::Module(0)), Some(MenuTarget::Module { session: run.session.clone(), module: run.modules[0].id.clone() }));
    for node in [Node::Core, Node::Folded(0, UnitState::Pending), Node::Unit(run.units.len()), Node::Module(run.modules.len())] {
        assert_eq!(menu_target(&run, node), None);
    }
    let mut reordered = run.clone();
    assert_ne!(run.units[0].id, run.units[1].id);
    reordered.units.swap(0, 1);
    assert_eq!(menu_target(&reordered, Node::Unit(0)), Some(MenuTarget::Unit { session: reordered.session.clone(), unit: run.units[1].id.clone() }));
    assert_ne!(menu_target(&run, Node::Unit(0)), menu_target(&reordered, Node::Unit(0)));
}

#[test]
fn menu_anchor_and_center() {
    let unit = Some(Node::Unit(1));
    let module = Some(Node::Module(2));
    assert_eq!(menu_anchor_node(unit, module), unit);
    assert_eq!(menu_anchor_node(None, module), module);
    assert_eq!(menu_anchor_node(Some(Node::Core), module), module);
    assert_eq!(menu_anchor_node(Some(Node::Folded(0, UnitState::Pending)), None), None);
    assert_eq!(menu_anchor_node(None, Some(Node::Core)), None);
    assert_eq!(node_center((20., 30.), (10., 15., 100., 40.)), (80., 65.));
}

fn has_context_menu(source: &str) -> bool {
    source.contains(".on_action(cx.listener(Self::open_cursor_menu))")
        && source.contains("_: &palette::ContextMenu")
        && source.contains("context_menu(") && source.contains("MouseButton::Right")
}

#[test]
fn graph_wires_context_menu() {
    assert!(has_context_menu(include_str!("../mesh.rs").split("#[cfg(test)]").next().unwrap()));
    assert!(!has_context_menu("fn unrelated() {}"));
}

#[test]
fn handled_keys_are_graph_shortcuts() {
    use super::super::palette::{Area, SHORTCUTS};
    let keys = handled_keys(include_str!("../mesh.rs"));
    let expected: Vec<String> = ["up", "down", "left", "right", "enter", "escape", "f", "ctrl-=", "ctrl-+", "ctrl--", "ctrl-0"].iter().map(|s| s.to_string()).collect();
    assert_eq!(keys, expected);
    let listed: Vec<_> = SHORTCUTS.iter().filter(|s| s.area == Area::Graph).flat_map(|s| s.keys.iter().copied()).collect();
    assert_eq!(missing_from_shortcuts(&keys, &listed), Vec::<String>::new());
    assert_eq!(missing_from_shortcuts(&["up".into(), "zzz".into()], &["up"]), vec!["zzz"]);
    let sample = "    fn key(\n        match key {\n            \"up\" => {},\n            \"zzz\" => {},\n        }\n    }\n    fn other(";
    assert_eq!(missing_from_shortcuts(&handled_keys(sample), &["up"]), vec!["zzz"]);
}

#[test]
fn glow_target_truth_table() {
    let a = Node::Module(0);
    let b = Node::Module(1);
    for selected in [None, Some(a), Some(b)] {
        for hovered in [None, Some(a), Some(b)] {
            assert_eq!(glow_targets(selected, hovered, a), (if selected == Some(a) { 1. } else { 0. }, if hovered == Some(a) { 1. } else { 0. }));
        }
    }
}

#[test]
fn panel_view_precedence() {
    let failed = PanelState::Failed("failed".into());
    let stale = PanelState::Stale("stale".into());
    assert_eq!(mesh_view(&PanelState::Loading, false, false), MeshView::Loading);
    assert_eq!(mesh_view(&failed, false, false), MeshView::Failed("failed".into()));
    for panel in [&PanelState::Empty, &PanelState::Ready, &stale] {
        assert_eq!(mesh_view(panel, false, false), MeshView::Empty);
        assert_ne!(mesh_view(panel, false, true), MeshView::Graph { stale: Some("stale".into()) });
    }
    for panel in [&PanelState::Loading, &failed, &stale, &PanelState::Ready, &PanelState::Empty] {
        assert_eq!(mesh_view(panel, true, false), MeshView::Planning);
    }
    assert_eq!(mesh_view(&stale, true, true), MeshView::Graph { stale: Some("stale".into()) });
    for panel in [&PanelState::Loading, &failed, &PanelState::Ready, &PanelState::Empty] {
        assert_eq!(mesh_view(panel, true, true), MeshView::Graph { stale: None });
    }
}

#[test]
fn roles_match_only_known_stages() {
    for (stage, role) in [("Astra", Some("astra")), ("Critic", Some("critic")), ("Architect", Some("architect")), ("astra", None), ("Full test", None), ("", None)] {
        assert_eq!(stage_role(stage), role);
    }
}

#[test]
fn glows_request_frames_only_while_moving() {
    let t0 = Instant::now();
    let node = Node::Module(0);
    let mut glows = motion::Glows::new(Pace::Glow);
    glows.set(&node, 1., t0, false);
    let mut mid = Driver::at(t0 + Pace::Glow.duration() / 2, false);
    let value = glows.value(&node, &mut mid);
    assert!(value > 0. && value < 1.);
    assert!(mid.wants_frame());
    let mut end = Driver::at(t0 + Pace::Glow.duration(), false);
    assert_eq!(glows.value(&node, &mut end), 1.);
    assert!(!end.wants_frame());
    let mut reduced = motion::Glows::new(Pace::Glow);
    reduced.set(&node, 1., t0, true);
    let mut driver = Driver::at(t0, true);
    assert_eq!(reduced.value(&node, &mut driver), 1.);
    assert!(!driver.wants_frame());
}

#[test]
fn idle_and_reduced_runs_do_not_animate() {
    let mut run = sample_run();
    run.status = RunStatus::Active;
    run.units[0].state = UnitState::Running;
    assert!(animates(&run, false));
    assert!(!animates(&run, true));
    run.status = RunStatus::Done;
    for unit in &mut run.units { unit.state = UnitState::Verified; }
    assert!(!animates(&run, false));
}

fn has_literal_chrome_sizes(source: &str) -> bool {
    source.contains("text_size(px(") || source.contains("rounded(px(")
}

#[test]
fn production_uses_scaled_text_and_radii() {
    assert!(has_literal_chrome_sizes("div().text_size(px(12.))"));
    assert!(has_literal_chrome_sizes("div().rounded(px(4.))"));
    let production = include_str!("../mesh.rs").split("#[cfg(test)]").next().unwrap();
    assert!(!has_literal_chrome_sizes(production));
}
