use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("mini", "mini", Opening::Window("mini")),
 entry("focus.graph", "mini", Opening::Window("focus.graph")),
 entry("focus.checklist", "mini", Opening::Window("focus.checklist"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
