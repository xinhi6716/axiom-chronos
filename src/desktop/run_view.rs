use super::model::*;
use super::theme::Tokened as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use super::{control::Control, detail::Detail, mesh::Mesh, todo::Todo};

pub struct RunView {
    state: Entity<AppState>,
    todo: Entity<Todo>,
    mesh: Entity<Mesh>,
    detail: Entity<Detail>,
    control: Entity<Control>,
}

impl RunView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            todo: cx.new(|cx| Todo::new(state.clone(), window, cx)),
            mesh: cx.new(|cx| Mesh::new(state.clone(), window, cx)),
            detail: cx.new(|cx| Detail::new(state.clone(), window, cx)),
            control: cx.new(|cx| Control::new(state.clone(), window, cx)),
            state,
        }
    }
}

impl Render for RunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.tokens();
        let wish = self.state.read(cx).run().map(|r| r.wish.clone()).unwrap_or_default();
        v_flex()
            .size_full()
            .child(div().p_4().text_color(t.text).child(wish))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().w(px(320.)).h_full().child(self.todo.clone()))
                    .child(div().flex_1().min_w_0().h_full().child(self.mesh.clone()))
                    .child(div().w(px(400.)).h_full().child(self.detail.clone())),
            )
            .child(div().h(px(120.)).child(self.control.clone()))
    }
}
