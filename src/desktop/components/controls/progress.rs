//! Determinate progress bar or ring. Values are fractions between zero and one.
use super::toolbar::Toolbar;
use crate::desktop::components::tokens::{
    ButtonVariant, Tokens, PROGRESS_HEIGHT, PROGRESS_RING_SIZE, PROGRESS_RING_STROKE,
};
use gpui_kit::component::plot::shape::{Arc, ArcData};
use gpui_kit::{
    canvas, div, relative, App, ElementId, FocusHandle, IntoElement, ParentElement, RenderOnce,
    Styled, Window,
};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Bar,
    Ring,
}
pub(super) fn fraction(value: f32) -> f32 {
    if value.is_nan() {
        0.
    } else {
        value.clamp(0., 1.)
    }
}
pub(super) fn ring_sweep(value: f32) -> f32 {
    fraction(value) * TAU
}
#[derive(IntoElement)]
pub struct Progress {
    pub(super) toolbar: Toolbar,
    tokens: Tokens,
    value: f32,
    shape: Shape,
}
impl Progress {
    pub fn new(id: impl Into<ElementId>, value: f32, tokens: &Tokens) -> Self {
        Self {
            toolbar: Toolbar::new(id, tokens),
            tokens: *tokens,
            value: fraction(value),
            shape: Shape::Bar,
        }
    }
    pub fn bar(id: impl Into<ElementId>, value: f32, tokens: &Tokens) -> Self {
        Self::new(id, value, tokens)
    }
    pub fn ring(id: impl Into<ElementId>, value: f32, tokens: &Tokens) -> Self {
        let mut p = Self::new(id, value, tokens);
        p.shape = Shape::Ring;
        p
    }
    pub fn value(mut self, value: f32) -> Self {
        self.value = fraction(value);
        self
    }
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.toolbar = self.toolbar.track_focus(focus);
        self
    }
}
impl RenderOnce for Progress {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let value = fraction(self.value);
        let sweep = ring_sweep(value);
        let t = self.tokens;
        let indicator = match self.shape {
            Shape::Bar => div()
                .w_full()
                .h(PROGRESS_HEIGHT)
                .rounded_full()
                .overflow_hidden()
                .bg(t.border)
                .child(div().w(relative(fraction(value))).h_full().bg(t.gold))
                .into_any_element(),
            Shape::Ring => canvas(
                move |bounds, _, _| bounds,
                move |_, bounds, window, _| {
                    let radius = (bounds.size.width.min(bounds.size.height) - PROGRESS_RING_STROKE)
                        .as_f32()
                        / 2.;
                    let half = PROGRESS_RING_STROKE.as_f32() / 2.;
                    let arc = Arc::new()
                        .inner_radius(radius - half)
                        .outer_radius(radius + half);
                    arc.paint(
                        &ArcData::new(&(), 0, 1., 0., TAU),
                        t.border,
                        &bounds,
                        window,
                    );
                    if value > 0. {
                        arc.paint(
                            &ArcData::new(&(), 1, value, 0., sweep),
                            t.gold,
                            &bounds,
                            window,
                        );
                    }
                },
            )
            .size(PROGRESS_RING_SIZE)
            .into_any_element(),
        };
        self.toolbar.variant(ButtonVariant::Ghost).child(indicator)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bar_width_is_clamped_fraction() {
        assert_eq!(fraction(0.5), 0.5);
        assert_eq!(fraction(-1.), 0.);
        assert_eq!(fraction(2.), 1.);
        assert_eq!(fraction(f32::NAN), 0.);
        assert_eq!(fraction(f32::INFINITY), 1.);
        assert_eq!(fraction(f32::NEG_INFINITY), 0.);
    }
    #[test]
    fn identity_values_colours_focus_and_motion() {
        for t in crate::desktop::theme::both() {
            let p = Progress::ring("progress", 0.5, &t);
            assert_eq!(p.toolbar.id, ElementId::from("progress"));
            assert_eq!(p.shape, Shape::Ring);
            assert_eq!(p.value, 0.5);
            assert_eq!(p.tokens.gold, t.gold);
            assert_eq!(p.tokens.border, t.border);
            assert_eq!(Progress::bar("bar", 2., &t).value, 1.);
            assert_eq!(fraction(-1.), 0.);
            assert_eq!(fraction(f32::NAN), 0.);
            assert_eq!(fraction(f32::INFINITY), 1.);
            assert_eq!(fraction(f32::NEG_INFINITY), 0.);
            assert_eq!(ring_sweep(p.value), TAU / 2.);
            assert_eq!(ring_sweep(-1.), 0.);
            assert_eq!(ring_sweep(2.), TAU);
            assert_eq!(ring_sweep(f32::NAN), 0.);
        }
    }
}
