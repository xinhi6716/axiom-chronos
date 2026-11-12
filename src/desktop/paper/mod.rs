// p8: begin stub
use crate::desktop::model::AppState;
use gpui_kit::*;

pub struct PaperPage;

impl PaperPage {
    pub fn new(
        _: Entity<AppState>,
        _: SharedString,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Self {
        Self
    }
}

impl Render for PaperPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
// p8: end stub
