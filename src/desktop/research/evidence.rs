use super::Workbench;
use gpui_kit::*;

pub struct EvidencePanel;
impl EvidencePanel {
    pub fn new(_: Entity<Workbench>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}
impl Render for EvidencePanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("research-evidence-panel")
    }
}
