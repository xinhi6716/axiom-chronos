use super::Workbench;
use gpui_kit::*;

pub struct ExportsPanel;
impl ExportsPanel {
    pub fn new(_: Entity<Workbench>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}
impl Render for ExportsPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("research-exports-panel")
    }
}
