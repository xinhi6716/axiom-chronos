use super::Workbench;
use gpui_kit::*;

pub struct SourcesPanel;
impl SourcesPanel {
    pub fn new(_: Entity<Workbench>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}
impl Render for SourcesPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("research-sources-panel")
    }
}
