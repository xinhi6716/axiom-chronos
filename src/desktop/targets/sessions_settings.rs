use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("settings", "sessions_settings", Opening::Screen(Screen::Doctor)),
 entry("session", "sessions_settings", Opening::Page("session"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
