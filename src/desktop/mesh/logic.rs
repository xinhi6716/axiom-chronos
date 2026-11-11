use super::{MenuTarget, Node, PanelState, Run, SharedString};

pub(super) fn scene_runs_needed(name: &str) -> usize {
    match name {
        "mesh-partial" | "mesh-done" => 4,
        "mesh-blocked" => 3,
        "mesh-waiting" => 2,
        _ => 1,
    }
}

pub(super) fn menu_target(run: &Run, node: Node) -> Option<MenuTarget> {
    match node {
        Node::Unit(i) => Some(MenuTarget::Unit { session: run.session.clone(), unit: run.units.get(i)?.id.clone() }),
        Node::Module(i) => Some(MenuTarget::Module { session: run.session.clone(), module: run.modules.get(i)?.id.clone() }),
        Node::Core | Node::Folded(..) => None,
    }
}

pub(super) fn menu_anchor_node(cursor: Option<Node>, selected: Option<Node>) -> Option<Node> {
    let has_menu = |node: &Node| matches!(node, Node::Unit(_) | Node::Module(_));
    cursor.filter(has_menu).or_else(|| selected.filter(has_menu))
}

pub(super) fn node_center(origin: (f32, f32), rect: (f32, f32, f32, f32)) -> (f32, f32) {
    (origin.0 + rect.0 + rect.2 / 2.0, origin.1 + rect.1 + rect.3 / 2.0)
}

#[cfg(test)]
pub(super) fn handled_keys(source: &str) -> Vec<String> {
    let Some((_, source)) = source.split_once("    fn key(") else { return vec![] };
    let handler = source.split("\n    fn ").next().unwrap_or(source);
    let arms = regex::Regex::new(r#"(?m)^\s*(\([^\n]*?\)|"[^"]+"(?:\s*\|\s*"[^"]+")*)(?:\s+if\s+[^=]+)?\s*=>"#).unwrap();
    let literals = regex::Regex::new(r#""([^"]+)""#).unwrap();
    arms.captures_iter(handler).flat_map(|arm| {
        let pattern = arm.get(1).unwrap().as_str();
        let prefix = if pattern.trim_end().ends_with(", true)") { "ctrl-" } else { "" };
        literals.captures_iter(pattern).map(|key| format!("{prefix}{}", &key[1])).collect::<Vec<_>>()
    }).collect()
}

#[cfg(test)]
pub(super) fn missing_from_shortcuts(handled: &[String], listed: &[&str]) -> Vec<String> {
    handled.iter().filter(|key| {
        !listed.contains(&key.as_str()) && !key.strip_prefix("ctrl-").is_some_and(|rest| listed.contains(&format!("cmd-{rest}").as_str()))
    }).cloned().collect()
}

pub(super) fn glow_targets(selected: Option<Node>, hovered: Option<Node>, node: Node) -> (f32, f32) {
    (if selected == Some(node) { 1.0 } else { 0.0 }, if hovered == Some(node) { 1.0 } else { 0.0 })
}

#[derive(Debug, PartialEq)]
pub(super) enum MeshView {
    Loading,
    Empty,
    Planning,
    Failed(SharedString),
    Graph { stale: Option<SharedString> },
}

pub(super) fn mesh_view(panel: &PanelState, has_run: bool, has_modules: bool) -> MeshView {
    if !has_run {
        return match panel {
            PanelState::Loading => MeshView::Loading,
            PanelState::Failed(message) => MeshView::Failed(message.clone()),
            _ => MeshView::Empty,
        };
    }
    if !has_modules { return MeshView::Planning; }
    MeshView::Graph { stale: match panel { PanelState::Stale(message) => Some(message.clone()), _ => None } }
}

pub(super) fn stage_role(stage: &str) -> Option<&'static str> {
    match stage { "Astra" => Some("astra"), "Critic" => Some("critic"), "Architect" => Some("architect"), _ => None }
}
