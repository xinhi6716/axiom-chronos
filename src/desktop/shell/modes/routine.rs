use crate::desktop::model::AppState;
use gpui_kit::*;
pub struct RoutinePage;
impl RoutinePage { pub fn new(_: Entity<AppState>, _: &mut Window, _: &mut Context<Self>) -> Self { Self } }
impl Render for RoutinePage { fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div().id("routine.page").child("Routines") } }
