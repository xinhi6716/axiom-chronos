use gpui_kit::{App, Global};
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    Slow { ms: u64 },
    Corrupt,
    Missing,
    Stale { age_ms: i64 },
    Offline,
    Panic,
}
#[derive(Clone, Default)]
pub struct FaultPlan(pub HashMap<String, Fault>);
impl Global for FaultPlan {}
pub fn install(plan: FaultPlan, cx: &mut App) {
    cx.set_global(plan);
    crate::desktop::components::panel::install_panic_hook(|panel, cx| {
        active(panel, cx) == Some(Fault::Panic)
    });
}
pub fn active(panel: &str, cx: &App) -> Option<Fault> {
    cx.try_global::<FaultPlan>()
        .and_then(|p| p.0.get(panel))
        .cloned()
}
