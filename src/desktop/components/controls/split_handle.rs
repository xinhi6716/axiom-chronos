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
/// A split group. The static key must be exactly `<target>.<split.id>`.
/// Construct through `SplitHandle::resizable`; store the returned group between renders.
#[derive(IntoElement)]
pub struct Resizable {
    id: ElementId,
    key: &'static str,
    split: crate::desktop::layout::split::Split,
    available: f32,
    sizes: std::rc::Rc<std::cell::RefCell<Vec<f32>>>,
    tokens: Tokens,
    children: Vec<gpui_kit::AnyElement>,
    axis: Axis,
    persist: bool,
}
impl Resizable {
    pub fn new(
        id: impl Into<ElementId>,
        key: &'static str,
        target: &str,
        split: crate::desktop::layout::split::Split,
        available: f32,
        saved: Option<&[f32]>,
        tokens: &Tokens,
    ) -> Option<Self> {
        if key != format!("{}.{}", target, split.id)
            || target.is_empty()
            || key.len() > 48
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        {
            return None;
        }
        let sizes = split.sizes(available, saved);
        Some(Self {
            id: id.into(),
            key,
            split,
            available,
            sizes: std::rc::Rc::new(std::cell::RefCell::new(sizes)),
            tokens: *tokens,
            children: Vec::new(),
            axis: Axis::Vertical,
            persist: false,
        })
    }
    pub fn key(&self) -> &'static str {
        self.key
    }
    pub fn sizes(&self) -> Vec<f32> {
        self.sizes.borrow().clone()
    }
    pub fn resize(&mut self, handle: usize, delta: Pixels) {
        let next = self
            .split
            .resize(&self.sizes.borrow(), handle, delta.as_f32(), self.available);
        *self.sizes.borrow_mut() = next;
    }
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }
}
fn restore_from_store(
    id: impl Into<ElementId>,
    key: &'static str,
    target: &str,
    split: crate::desktop::layout::split::Split,
    available: f32,
    tokens: &Tokens,
    store: Option<&crate::desktop::layout::persist::LayoutState>,
) -> Option<Resizable> {
    let store = store?;
    let saved = crate::desktop::layout::persist::sizes_of(store, key);
    let mut group = Resizable::new(id, key, target, split, available, saved.as_deref(), tokens)?;
    group.persist = true;
    Some(group)
}
fn apply_resize(
    persist: bool,
    store: Option<&mut crate::desktop::layout::persist::LayoutState>,
    key: &'static str,
    split: &crate::desktop::layout::split::Split,
    sizes: &std::cell::RefCell<Vec<f32>>,
    index: usize,
    delta: Pixels,
    available: f32,
) {
    let next = split.resize(&sizes.borrow(), index, delta.as_f32(), available);
    *sizes.borrow_mut() = next.clone();
    if let (true, Some(store)) = (persist, store) {
        crate::desktop::layout::persist::remember_state(store, key, next);
    }
}
impl SplitHandle {
    /// Restore and persist sizes using the store previously installed with `layout::persist::init`.
    /// Returns None when no store has been initialized.
    pub fn resizable(
        id: impl Into<ElementId>,
        key: &'static str,
        target: &str,
        split: crate::desktop::layout::split::Split,
        available: f32,
        tokens: &Tokens,
        cx: &mut App,
    ) -> Option<Resizable> {
        let store = cx.try_global::<crate::desktop::layout::persist::LayoutState>();
        restore_from_store(id, key, target, split, available, tokens, store)
    }
}
impl RenderOnce for Resizable {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut group = div().id(self.id.clone()).flex();
        if self.axis == Axis::Horizontal {
            group = group.flex_col();
        }
        let len = self.split.panes.len();
        for (index, child) in self.children.into_iter().take(len).enumerate() {
            let size = self.sizes.borrow().get(index).copied().unwrap_or_default();
            let pane = match self.axis {
                Axis::Vertical => div().w(gpui_kit::px(size)),
                Axis::Horizontal => div().h(gpui_kit::px(size)),
            }
            .child(child);
            group = group.child(pane);
            if index + 1 < len {
                let sizes = self.sizes.clone();
                let split = self.split.clone();
                let available = self.available;
                let key = self.key;
                let persist = self.persist;
                let mut handle =
                    SplitHandle::new((self.id.clone(), index.to_string()), &self.tokens);
                if self.axis == Axis::Horizontal {
                    handle = handle.horizontal();
                }
                group = group.child(handle.on_resize(move |delta, _, cx| {
                    let store = if persist
                        && cx
                            .try_global::<crate::desktop::layout::persist::LayoutState>()
                            .is_some()
                    {
                        Some(cx.global_mut::<crate::desktop::layout::persist::LayoutState>())
                    } else {
                        None
                    };
                    apply_resize(persist, store, key, &split, &sizes, index, delta, available);
                    cx.refresh_windows();
                }));
            }
        }
        group
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

#[cfg(test)]
mod group_tests {
    use super::*;
    #[test]
    fn injected_store_validation_and_keyboard_resize() {
        use crate::desktop::layout::{
            persist,
            split::{Pane, Split},
        };
        for t in crate::desktop::theme::both() {
            let home = tempfile::tempdir().unwrap();
            let mut store = persist::init_state(home.path(), None);
            let split = Split {
                id: "main",
                panes: vec![
                    Pane {
                        min: 10.,
                        max: 90.,
                        default: 50.,
                        collapsible: false
                    };
                    2
                ],
            };
            for (key, target) in [
                ("wrong.main", "run"),
                ("run.Main", "run"),
                ("run_main.main", "run_main"),
                (
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.main",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
            ] {
                assert!(
                    Resizable::new("group", key, target, split.clone(), 100., None, &t).is_none()
                );
            }
            let sizes = std::cell::RefCell::new(vec![50., 50.]);
            assert!(dispatch_key(
                Axis::Vertical,
                "right",
                Some(|delta| {
                    apply_resize(
                        true,
                        Some(&mut store),
                        "run.main",
                        &split,
                        &sizes,
                        0,
                        delta,
                        100.,
                    );
                })
            ));
            assert_eq!(persist::sizes_of(&store, "run.main"), Some(vec![58., 42.]));
            assert_eq!(persist::sizes_of(&store, "main"), None);
            let group = restore_from_store(
                "group",
                "run.main",
                "run",
                split.clone(),
                100.,
                &t,
                Some(&store),
            )
            .unwrap();
            assert_eq!(group.sizes(), vec![58., 42.]);
            persist::flush_state(&store);
            assert_eq!(
                persist::load(home.path()).panels.get("run.main"),
                Some(&vec![58., 42.])
            );
        }
    }
    #[test]
    fn resize_store_branches() {
        use crate::desktop::layout::{
            persist,
            split::{Pane, Split},
        };
        let home = tempfile::tempdir().unwrap();
        let mut store = persist::init_state(home.path(), None);
        let split = Split {
            id: "main",
            panes: vec![
                Pane {
                    min: 10.,
                    max: 90.,
                    default: 50.,
                    collapsible: false
                };
                2
            ],
        };
        let sizes = std::cell::RefCell::new(vec![50., 50.]);
        apply_resize(
            true,
            Some(&mut store),
            "run.main",
            &split,
            &sizes,
            0,
            gpui_kit::px(8.),
            100.,
        );
        assert_eq!(*sizes.borrow(), vec![58., 42.]);
        assert_eq!(persist::sizes_of(&store, "run.main"), Some(vec![58., 42.]));
        apply_resize(
            false,
            Some(&mut store),
            "run.main",
            &split,
            &sizes,
            0,
            gpui_kit::px(5.),
            100.,
        );
        assert_eq!(*sizes.borrow(), vec![63., 37.]);
        assert_eq!(persist::sizes_of(&store, "run.main"), Some(vec![58., 42.]));
        apply_resize(
            true,
            None,
            "run.main",
            &split,
            &sizes,
            0,
            gpui_kit::px(5.),
            100.,
        );
        assert_eq!(*sizes.borrow(), vec![68., 32.]);
        assert_eq!(persist::sizes_of(&store, "run.main"), Some(vec![58., 42.]));
    }
    #[test]
    fn store_lookup_requires_store_and_matching_key() {
        use crate::desktop::layout::{
            persist,
            split::{Pane, Split},
        };
        let home = tempfile::tempdir().unwrap();
        let mut store = persist::init_state(home.path(), None);
        let split = Split {
            id: "main",
            panes: vec![
                Pane {
                    min: 10.,
                    max: 90.,
                    default: 50.,
                    collapsible: false
                };
                2
            ],
        };
        let t = crate::desktop::theme::both().into_iter().next().unwrap();
        persist::remember_state(&mut store, "run.main", vec![60., 40.]);
        assert!(
            restore_from_store("group", "run.main", "run", split.clone(), 100., &t, None).is_none()
        );
        assert!(restore_from_store(
            "group",
            "other.main",
            "run",
            split.clone(),
            100.,
            &t,
            Some(&store)
        )
        .is_none());
        let group =
            restore_from_store("group", "run.main", "run", split, 100., &t, Some(&store)).unwrap();
        assert!(group.persist);
        assert_eq!(group.sizes(), vec![60., 40.]);
    }
    #[test]
    fn key_resize_and_restore() {
        for t in crate::desktop::theme::both() {
            let split = crate::desktop::layout::split::Split {
                id: "main",
                panes: vec![
                    crate::desktop::layout::split::Pane {
                        min: 10.,
                        max: 90.,
                        default: 50.,
                        collapsible: false
                    };
                    2
                ],
            };
            let mut group = Resizable::new(
                "group",
                "run.main",
                "run",
                split,
                100.,
                Some(&[60., 40.]),
                &t,
            )
            .unwrap();
            assert_eq!(group.key(), "run.main");
            assert_eq!(group.sizes(), &[60., 40.]);
            group.resize(0, SPLIT_KEY_STEP);
            assert_eq!(group.sizes(), &[68., 32.]);
            let home =
                std::env::temp_dir().join(format!("axiom-resizable-test-{}", std::process::id()));
            let mut store = crate::desktop::layout::persist::init_state(&home, None);
            assert!(crate::desktop::layout::persist::remember_state(
                &mut store,
                group.key(),
                group.sizes()
            ));
            crate::desktop::layout::persist::flush_state(&store);
            let stored = crate::desktop::layout::persist::load(&home);
            let restored = Resizable::new(
                "restored",
                "run.main",
                "run",
                group.split.clone(),
                100.,
                stored.panels.get("run.main").map(Vec::as_slice),
                &t,
            )
            .unwrap();
            assert_eq!(restored.sizes(), &[68., 32.]);
            let _ = std::fs::remove_file(home.join("desktop/layout.json"));
            let _ = std::fs::remove_dir(home.join("desktop"));
            let _ = std::fs::remove_dir(home);
        }
    }
}
