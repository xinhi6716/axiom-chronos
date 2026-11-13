use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("run", "shell", Opening::Screen(Screen::Run)),
 entry("memory", "shell", Opening::Screen(Screen::Memory)),
 entry("usage", "shell", Opening::Screen(Screen::Usage))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }
