use super::*;
use gpui_kit::AppContext as _;
pub(super) fn targets() -> Vec<Target> { vec![
 entry("settings", "sessions_settings", Opening::Screen(Screen::Doctor)),
 entry("session", "sessions_settings", Opening::Page("session"))
] }
pub(super) fn prepare(t: &Target, s: CaptureState, state: &mut AppState) -> Result<(), Unregistered> { super::prepare_basic(t,s,state) }

pub(crate) fn open(t: &Target, s: CaptureState, state: &gpui_kit::Entity<AppState>, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> Result<(), Unregistered> {
 let _ = (state, window, cx);
 Err(super::unavailable(t, s))
}

pub(crate) fn session_page(state: gpui_kit::Entity<AppState>, id: gpui_kit::SharedString, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> gpui_kit::AnyView {
 cx.new(|cx| crate::desktop::detail::SessionPage::new(state, id, window, cx)).into()
}
