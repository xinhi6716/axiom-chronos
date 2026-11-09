use super::model::*;
use super::theme::Tokened as _;
use gpui_kit::*;

pub struct MemoryView {
    state: Entity<AppState>,
}

impl MemoryView {
    pub fn new(state: Entity<AppState>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }
}

impl Render for MemoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let _ = self.state.read(cx);
        div().size_full().p_4().text_color(t.muted).child("Memory")
    }
}
