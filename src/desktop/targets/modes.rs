use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("storage", "modes", Opening::Page("storage")),
 entry("chat", "modes", Opening::Page("chat")),
 entry("quick", "modes", Opening::Page("quick")),
 entry("plaza", "modes", Opening::Page("plaza")),
 entry("routine", "modes", Opening::Page("routine"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }

pub(crate) fn open(t: &Target, s: CaptureState, state: &gpui_kit::Entity<AppState>, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> Result<(), Unregistered> {
 let _ = (state, window, cx);
 Err(super::unavailable(t, s))
}
