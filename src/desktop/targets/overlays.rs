use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("palette", "overlays", Opening::Overlay("palette")),
 entry("composer", "overlays", Opening::Overlay("composer")),
 entry("dialog.close", "overlays", Opening::Dialog("close")),
 entry("dialog.control", "overlays", Opening::Dialog("control")),
 entry("dialog.interjection", "overlays", Opening::Dialog("interjection")),
 entry("dialog.composer", "overlays", Opening::Dialog("composer")),
 entry("dialog.evolve", "overlays", Opening::Dialog("evolve"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
