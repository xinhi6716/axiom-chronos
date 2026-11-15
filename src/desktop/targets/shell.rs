use super::*;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("run", "shell", Opening::Screen(Screen::Run)),
 entry("memory", "shell", Opening::Screen(Screen::Memory)),
 entry("usage", "shell", Opening::Screen(Screen::Usage))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }

pub(crate) fn open(t: &Target, s: CaptureState, state: &gpui_kit::Entity<AppState>, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> Result<(), Unregistered> {
 let _ = (state, window, cx);
 Err(super::unavailable(t, s))
}
