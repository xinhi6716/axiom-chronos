use super::{Filter, RowKey};
use super::super::model::{MenuTarget, PanelState, Run};
use gpui_kit::{point, Bounds, Pixels, Point, SharedString};

pub(super) fn menu_anchor(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(bounds.origin.x + bounds.size.width / 2.0, bounds.origin.y + bounds.size.height / 2.0)
}

impl Filter {
    pub(super) fn to_key(self) -> &'static str {
        match self { Self::All => "all", Self::Active => "active", Self::Review => "review", Self::Failed => "failed" }
    }

    pub(super) fn from_key(key: Option<&str>) -> Self {
        match key { Some("active") => Self::Active, Some("review") => Self::Review, Some("failed") => Self::Failed, _ => Self::All }
    }
}

pub(super) fn menu_target(run: &Run, key: RowKey) -> Option<MenuTarget> {
    match key {
        RowKey::Unit { unit, .. } => Some(MenuTarget::Unit { session: run.session.clone(), unit: run.units.get(unit)?.id.clone() }),
        RowKey::Module { module, .. } => Some(MenuTarget::Module { session: run.session.clone(), module: run.modules.get(module)?.id.clone() }),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum GlowKey {
    Unit(SharedString, SharedString),
    Module(SharedString, SharedString),
}

impl GlowKey {
    pub(super) fn of(target: &MenuTarget) -> Option<Self> {
        match target {
            MenuTarget::Unit { session, unit } => Some(Self::Unit(session.clone(), unit.clone())),
            MenuTarget::Module { session, module } => Some(Self::Module(session.clone(), module.clone())),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum TodoView { Loading, Empty, List, Failed { message: SharedString }, Stale { message: SharedString } }

pub(super) fn view_of(panel: &PanelState, planless: bool) -> TodoView {
    match panel {
        PanelState::Loading => TodoView::Loading,
        PanelState::Empty => TodoView::Empty,
        PanelState::Ready if planless => TodoView::Empty,
        PanelState::Ready => TodoView::List,
        PanelState::Failed(message) => TodoView::Failed { message: message.clone() },
        PanelState::Stale(message) => TodoView::Stale { message: message.clone() },
    }
}

pub(super) fn glow_target(selected: bool) -> f32 { if selected { 1.0 } else { 0.0 } }
