//! A controlled splitter: mouse drag and arrow keys report pixel deltas to the owner.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{
    ButtonVariant, Tokens, SPLIT_HANDLE_LENGTH, SPLIT_HANDLE_WIDTH, SPLIT_KEY_STEP,
};
use gpui_kit::{
    div, App, ElementId, FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Point, RenderOnce, Styled, Window,
};
use std::rc::Rc;

type Resize = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}
fn key_delta(axis: Axis, key: &str) -> Option<Pixels> {
    match (axis, key) {
        (Axis::Vertical, "left") | (Axis::Horizontal, "up") => Some(-SPLIT_KEY_STEP),
        (Axis::Vertical, "right") | (Axis::Horizontal, "down") => Some(SPLIT_KEY_STEP),
        _ => None,
    }
}
fn coordinate(axis: Axis, point: Point<Pixels>) -> Pixels {
    match axis {
        Axis::Vertical => point.x,
        Axis::Horizontal => point.y,
    }
}
pub(super) fn drag_start(position: Pixels) -> Option<Pixels> {
    position.as_f32().is_finite().then_some(position)
}
// Ignore invalid pointer positions without poisoning the next valid transition.
pub(super) fn drag_transition(
    last: Option<Pixels>,
    position: Pixels,
    dragging: bool,
) -> (Option<Pixels>, Option<Pixels>) {
    if !dragging {
        return (None, None);
    }
    if !position.as_f32().is_finite() {
        return (last, None);
    }
    match last.filter(|p| p.as_f32().is_finite()) {
        Some(previous) => {
            let delta = position - previous;
            (Some(position), delta.as_f32().is_finite().then_some(delta))
        }
        None => (None, None),
    }
}
fn dispatch_resize(delta: Option<Pixels>, callback: Option<impl FnOnce(Pixels)>) -> bool {
    if let (Some(delta), Some(callback)) = (delta, callback) {
        callback(delta);
        true
    } else {
        false
    }
}
fn dispatch_key(axis: Axis, key: &str, callback: Option<impl FnOnce(Pixels)>) -> bool {
    dispatch_resize(key_delta(axis, key), callback)
}
#[derive(IntoElement)]
pub struct SplitHandle {
    pub(super) toolbar: Toolbar,
    axis: Axis,
    on_resize: Option<Resize>,
}
impl SplitHandle {
    pub fn new(id: impl Into<ElementId>, tokens: &Tokens) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            axis: Axis::Vertical,
            on_resize: None,
        }
    }
    /// A vertical handle adjusts horizontally (the default).
    pub fn vertical(mut self) -> Self {
        self.axis = Axis::Vertical;
        self
    }
    /// A horizontal handle adjusts vertically.
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.toolbar = self.toolbar.track_focus(focus);
        self
    }
    pub fn on_resize(mut self, callback: impl Fn(Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.on_resize = Some(Rc::new(callback));
        self
    }
}
impl RenderOnce for SplitHandle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let state = window.use_keyed_state((self.toolbar.id.clone(), "split-drag"), cx, |_, _| {
            None::<Pixels>
        });
        let down = state.clone();
        let moved = state.clone();
        let up = state.clone();
        let out = state;
        let key_callback = self.on_resize.clone();
        let mouse_callback = self.on_resize;
        let grip = match axis {
            Axis::Vertical => div().w(SPLIT_HANDLE_WIDTH).h(SPLIT_HANDLE_LENGTH),
            Axis::Horizontal => div().w(SPLIT_HANDLE_LENGTH).h(SPLIT_HANDLE_WIDTH),
        };
        div()
            .id((self.toolbar.id.clone(), "split-events"))
            .child(self.toolbar.variant(ButtonVariant::Secondary).child(grip))
            .on_key_down(move |event, window, cx| {
                let callback = key_callback
                    .as_ref()
                    .map(|callback| |delta| callback(delta, window, cx));
                if dispatch_key(axis, &event.keystroke.key, callback) {
                    cx.stop_propagation();
                }
            })
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                let position = coordinate(axis, event.position);
                down.update(cx, |last, _| {
                    *last = drag_start(position);
                });
            })
            .on_mouse_move(move |event, window, cx| {
                let position = coordinate(axis, event.position);
                let delta = moved.update(cx, |last, _| {
                    let (next, delta) = drag_transition(*last, position, event.dragging());
                    *last = next;
                    delta
                });
                let callback = mouse_callback
                    .as_ref()
                    .map(|callback| |delta| callback(delta, window, cx));
                dispatch_resize(delta, callback);
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up.update(cx, |last, _| *last = None);
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                out.update(cx, |last, _| *last = None);
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::components::tokens::button_colors;
    #[test]
    fn drag_moves_only_after_mouse_down_and_stops_on_release() {
        use gpui_kit::px;
        assert_eq!(drag_start(px(f32::NAN)), None);
        assert_eq!(drag_start(px(f32::INFINITY)), None);
        assert_eq!(drag_start(px(f32::NEG_INFINITY)), None);
        let down = drag_start(px(10.));
        let (last, delta) = drag_transition(down, px(15.), true);
        assert_eq!((last, delta), (Some(px(15.)), Some(px(5.))));
        let mut received = Vec::new();
        assert!(dispatch_resize(delta, Some(|value| received.push(value))));
        assert_eq!(received, [px(5.)]);
        let (last, delta) = drag_transition(last, px(12.), true);
        assert_eq!((last, delta), (Some(px(12.)), Some(px(-3.))));
        assert_eq!(drag_transition(last, px(f32::NAN), true), (last, None));
        assert_eq!(drag_transition(last, px(f32::INFINITY), true), (last, None));
        assert_eq!(drag_transition(last, px(20.), false), (None, None));
        assert_eq!(drag_transition(None, px(20.), true), (None, None));
        assert_eq!(
            drag_transition(Some(px(f32::NAN)), px(20.), true),
            (None, None)
        );
    }
    #[test]
    fn key_dispatch_forwards_only_the_mapped_delta() {
        let mut values = Vec::new();
        assert!(dispatch_key(
            Axis::Vertical,
            "left",
            Some(|delta| values.push(delta))
        ));
        assert!(dispatch_key(
            Axis::Horizontal,
            "down",
            Some(|delta| values.push(delta))
        ));
        assert!(!dispatch_key(
            Axis::Horizontal,
            "left",
            Some(|delta| values.push(delta))
        ));
        assert!(!dispatch_key(Axis::Horizontal, "up", None::<fn(Pixels)>));
        assert_eq!(values, [-SPLIT_KEY_STEP, SPLIT_KEY_STEP]);
    }
    #[test]
    fn identity_colours_keyboard_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let handle = SplitHandle::new("split", &t).horizontal();
            assert_eq!(handle.toolbar.id, ElementId::from("split"));
            assert_eq!(
                button_colors(ButtonVariant::Secondary, &t).background,
                t.raised
            );
            assert_eq!(SplitHandle::new("default", &t).axis, Axis::Vertical);
            assert_eq!(
                SplitHandle::new("vertical", &t)
                    .horizontal()
                    .vertical()
                    .axis,
                Axis::Vertical
            );
            assert_eq!(handle.axis, Axis::Horizontal);
            assert_eq!(key_delta(Axis::Vertical, "left"), Some(-SPLIT_KEY_STEP));
            assert_eq!(key_delta(Axis::Vertical, "right"), Some(SPLIT_KEY_STEP));
            assert_eq!(key_delta(Axis::Horizontal, "up"), Some(-SPLIT_KEY_STEP));
            assert_eq!(key_delta(handle.axis, "down"), Some(SPLIT_KEY_STEP));
            assert_eq!(key_delta(handle.axis, "left"), None);
        }
    }
}
