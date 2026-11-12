use super::Workbench;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::*;

pub struct DatasetPanel;
impl DatasetPanel {
    pub fn new(_: Entity<Workbench>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self
    }
}
impl Render for DatasetPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("research-dataset-panel").test_support()
    }
}
