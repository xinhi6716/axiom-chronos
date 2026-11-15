//! Virtualized rows with uniform or variable heights.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{space, Density, Step, Tokens};
use gpui_kit::{
    div, list, px, AnyElement, App, ElementId, IntoElement, ListAlignment, ListOffset, ListState,
    Div, ParentElement, Pixels, RenderOnce, Styled, Window,
};
use std::{ops::Range, rc::Rc};

#[derive(Clone)]
pub struct VirtualListHandle {
    heights: Rc<Vec<f32>>,
    offsets: Rc<Vec<f32>>,
    state: ListState,
}
impl VirtualListHandle {
    pub fn uniform(count: usize, height: f32) -> Self {
        Self::variable(vec![height; count])
    }
    pub fn variable(heights: Vec<f32>) -> Self {
        let heights: Vec<_> = heights
            .into_iter()
            .map(|h| if h.is_finite() && h > 0. { h } else { 0. })
            .collect();
        let mut offsets = Vec::with_capacity(heights.len() + 1);
        offsets.push(0.);
        for &h in &heights {
            offsets.push(offsets.last().copied().unwrap_or(0.) + h);
        }
        let state = ListState::new(
            heights.len(),
            ListAlignment::Top,
            space(Density::Comfortable, Step::Lg),
        );
        if let Some(&height) = heights.first().filter(|&&h| h > 0.) {
            state.reset_with_uniform_height(heights.len(), px(height));
        }
        Self {
            heights: Rc::new(heights),
            offsets: Rc::new(offsets),
            state,
        }
    }
    pub fn visible_range(&self, offset: f32, viewport: f32, overscan: usize) -> Range<usize> {
        let n = self.heights.len();
        if n == 0 || self.offsets[n] <= 0. || !viewport.is_finite() || viewport <= 0. {
            return 0..0;
        }
        let offset = if offset.is_finite() {
            offset.max(0.)
        } else {
            0.
        };
        let start = self
            .offsets
            .partition_point(|&v| v <= offset)
            .saturating_sub(1)
            .min(n);
        let end = self
            .offsets
            .partition_point(|&v| v < offset + viewport)
            .min(n);
        start.saturating_sub(overscan)..end.saturating_add(overscan).min(n)
    }
    pub fn scroll_to(&mut self, index: usize, viewport: f32) {
        if index >= self.heights.len() {
            return;
        }
        let viewport = if viewport.is_finite() {
            viewport.max(0.)
        } else {
            0.
        };
        let mut offset = self.offset();
        let top = self.offsets[index];
        let bottom = self.offsets[index + 1];
        if top < offset {
            offset = top;
        } else if bottom > offset + viewport {
            offset = (bottom - viewport).max(0.);
        }
        let first = self
            .offsets
            .partition_point(|&v| v <= offset)
            .saturating_sub(1)
            .min(self.heights.len() - 1);
        self.state.scroll_to(ListOffset {
            item_ix: first,
            offset_in_item: px(offset - self.offsets[first]),
        });
    }
    pub fn offset(&self) -> f32 {
        let top = self.state.logical_scroll_top();
        self.offsets
            .get(top.item_ix)
            .copied()
            .unwrap_or_else(|| *self.offsets.last().unwrap_or(&0.))
            + top.offset_in_item.as_f32()
    }
}
#[derive(IntoElement)]
pub struct VirtualList {
    id: ElementId,
    handle: VirtualListHandle,
    tokens: Tokens,
    render: Rc<dyn Fn(usize) -> AnyElement>,
    row_width: Option<Pixels>,
}
impl VirtualList {
    pub fn new<E: IntoElement>(
        id: impl Into<ElementId>,
        handle: VirtualListHandle,
        tokens: &Tokens,
        render: impl Fn(usize) -> E + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            handle,
            tokens: *tokens,
            render: Rc::new(move |i| render(i).into_any_element()),
            row_width: None,
        }
    }
    pub fn row_width(mut self, width: Pixels) -> Self {
        self.row_width = Some(width);
        self
    }
}
fn build_row(
    render: &Rc<dyn Fn(usize) -> AnyElement>,
    heights: &[f32],
    range: &Range<usize>,
    index: usize,
    width: Option<Pixels>,
) -> Div {
    let mut row = div().h(px(heights[index]));
    if let Some(width) = width {
        row = row.w(width);
    }
    if range.contains(&index) {
        row.child(render(index))
    } else {
        row
    }
}
fn render_range(handle: &VirtualListHandle, window_height: f32, offset: f32) -> Range<usize> {
    let viewport = window_height.max(space(Density::Comfortable, Step::Lg).as_f32());
    handle.visible_range(offset, viewport, 1)
}
impl RenderOnce for VirtualList {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        // The window height bounds the list viewport even on its first layout and after resizing.
        let range = render_range(
            &self.handle,
            window.viewport_size().height.as_f32(),
            self.handle.offset(),
        );
        let render = self.render;
        let heights = self.handle.heights.clone();
        let row_width = self.row_width;
        let rows = list(self.handle.state, move |i, _, _| {
            build_row(&render, &heights, &range, i, row_width).into_any_element()
        })
        .size_full();
        Toolbar::new(self.id, &self.tokens).child(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_width_is_optional_and_applied_to_all_rows() {
        let tokens = crate::desktop::theme::night();
        let handle = VirtualListHandle::uniform(2, 20.);
        let list = VirtualList::new("test", handle.clone(), &tokens, |_| div());
        assert_eq!(list.row_width, None);
        let list = list.row_width(px(120.));
        assert_eq!(list.row_width, Some(px(120.)));
        let range = 0..1;
        for index in 0..2 {
            let mut row = build_row(&list.render, &handle.heights, &range, index, list.row_width);
            assert_eq!(row.style().size.width, Some(px(120.).into()));
            let mut default_row = build_row(&list.render, &handle.heights, &range, index, None);
            assert_eq!(default_row.style().size.width, None);
        }
    }
    #[test]
    fn render_range_uses_window_height_floor_and_offset() {
        let floor = space(Density::Comfortable, Step::Lg).as_f32();
        let h = VirtualListHandle::uniform(100, floor);
        assert_eq!(render_range(&h, 0., 0.), 0..2);
        assert_eq!(render_range(&h, floor * 2., 0.), 0..3);
        assert_eq!(render_range(&h, floor * 2., floor * 5.), 4..8);
    }
    #[test]
    fn zero_height_rows_are_not_visible() {
        let h = VirtualListHandle::uniform(100_000, 0.);
        assert_eq!(h.visible_range(0., 40., 1), 0..0);
    }
    #[test]
    fn scroll_boundaries_and_empty_viewport() {
        let mut h = VirtualListHandle::uniform(100_000, 20.);
        h.scroll_to(500, 40.);
        h.scroll_to(3, 40.);
        assert_eq!(h.offset(), 60.);
        h.scroll_to(99_999, 40.);
        assert_eq!(h.offset(), 1_999_960.);
        assert_eq!(h.visible_range(h.offset(), 40., 1), 99_997..100_000);
        assert_eq!(h.visible_range(0., 0., 1), 0..0);
        let mut empty = VirtualListHandle::uniform(0, 20.);
        empty.scroll_to(0, 40.);
        assert_eq!(empty.offset(), 0.);
        assert_eq!(empty.visible_range(0., 40., 1), 0..0);
    }
    #[test]
    fn builds_only_rows_in_visible_range() {
        let called = Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = called.clone();
        let render: Rc<dyn Fn(usize) -> AnyElement> = Rc::new(move |i| {
            received.borrow_mut().push(i);
            div().into_any_element()
        });
        let h = VirtualListHandle::uniform(100_000, 20.);
        let range = h.visible_range(1_000_000., 40., 1);
        assert_eq!(range, 49_999..50_003);
        for i in 0..100_000 {
            let _ = build_row(&render, &h.heights, &range, i, None);
        }
        assert_eq!(called.borrow().len(), range.len());
        assert_eq!(*called.borrow(), range.collect::<Vec<_>>());
    }
    #[test]
    fn window_and_scroll() {
        let mut h = VirtualListHandle::uniform(100_000, 20.);
        assert_eq!(h.visible_range(100., 40., 1), 4..8);
        h.scroll_to(500, 40.);
        assert_eq!(h.visible_range(h.offset(), 40., 1), 498..502);
        let h = VirtualListHandle::variable(vec![10., 20., 30., 40.]);
        assert_eq!(h.visible_range(10., 20., 1), 0..3);
    }
}
