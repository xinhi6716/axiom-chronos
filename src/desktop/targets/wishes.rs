use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("wishes", "wishes", Opening::Screen(Screen::Wishes))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
