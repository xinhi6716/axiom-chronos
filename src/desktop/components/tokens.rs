//! Shared design tokens for desktop components.
pub use crate::desktop::motion::{Curve, Dur, Ease, Pace, Spring};
pub use crate::desktop::theme::{
    focus_ring, font, gold_leaf, lift_shadow, row_height, selection_glow, space, Density, Step,
    Tokened, Tokens, TypeSize,
};
use gpui_kit::{point, px, BoxShadow, Hsla, Pixels, Point};

pub const PROGRESS_HEIGHT: Pixels = px(8.);
pub const PROGRESS_RING_SIZE: Pixels = px(40.);
pub const PROGRESS_RING_STROKE: Pixels = px(4.);
pub const STATUS_DOT_SIZE: Pixels = px(8.);
pub const SPLIT_HANDLE_WIDTH: Pixels = px(12.);
pub const SPLIT_HANDLE_LENGTH: Pixels = px(40.);
pub const SPLIT_KEY_STEP: Pixels = px(8.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Running,
    Success,
    Warning,
    Error,
}

pub fn status_color(status: Status, t: &Tokens) -> Hsla {
    match status {
        Status::Idle => t.idle,
        Status::Running => t.run,
        Status::Success => t.ok,
        Status::Warning => t.review,
        Status::Error => t.fail,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Radius {
    Sm,
    Md,
    Lg,
    Pill,
}

pub fn radius(value: Radius) -> Pixels {
    px(match value {
        Radius::Sm => 2.,
        Radius::Md => 4.,
        Radius::Lg => 8.,
        Radius::Pill => 999.,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Flat,
    Raised,
    Overlay,
    Modal,
}

pub fn elevation(value: Elevation, tokens: &Tokens) -> Vec<BoxShadow> {
    let (offset, blur, alpha) = match value {
        Elevation::Flat => return Vec::new(),
        Elevation::Raised => (2., 6., 0.12),
        Elevation::Overlay => (4., 12., 0.20),
        Elevation::Modal => (8., 24., 0.30),
    };
    let alpha = alpha * if tokens.surface.l < 0.5 { 1.0 } else { 0.7 };
    vec![BoxShadow {
        color: Hsla {
            h: 0.,
            s: 0.,
            l: 0.,
            a: alpha,
        },
        offset: point(px(0.), px(offset)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glass {
    Subtle,
    Card,
    Hero,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassStyle {
    pub fill: Hsla,
    pub border: Hsla,
    pub inner: Hsla,
}

pub fn glass(value: Glass, tokens: &Tokens) -> GlassStyle {
    let (fill, border, inner) = match value {
        Glass::Subtle => (0.75, 0.50, 0.08),
        Glass::Card => (0.88, 0.70, 0.12),
        Glass::Hero => (0.96, 0.90, 0.18),
    };
    GlassStyle {
        fill: tokens.surface.opacity(fill),
        border: tokens.border.opacity(border),
        inner: tokens.text.opacity(inner),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glow {
    Rest,
    Active,
    LongRunning,
}
pub fn glow(value: Glow, level: f32, tokens: &Tokens) -> Vec<BoxShadow> {
    let level = if level.is_nan() {
        0.
    } else {
        level.clamp(0., 1.)
    };
    if level <= 0. {
        return Vec::new();
    }
    let (blur, spread, alpha) = match value {
        Glow::Rest => (6., 1., 0.14),
        Glow::Active => (12., 2., 0.30),
        Glow::LongRunning => (18., 3., 0.40),
    };
    vec![BoxShadow {
        color: tokens.gold.opacity(alpha * level),
        offset: Point::default(),
        blur_radius: px(blur * level),
        spread_radius: px(spread * level),
        inset: false,
    }]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Primary,
    Secondary,
    Ghost,
    Danger,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonColors {
    pub background: Hsla,
    pub foreground: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
}
pub fn button_colors(variant: ButtonVariant, t: &Tokens) -> ButtonColors {
    use crate::desktop::motion::mix;
    let (background, foreground) = match variant {
        ButtonVariant::Primary => (
            t.gold,
            t.gold_leaf(crate::desktop::theme::Press::Normal).label,
        ),
        ButtonVariant::Secondary => (t.raised, t.text),
        ButtonVariant::Ghost => (t.surface.opacity(0.), t.text),
        ButtonVariant::Danger => (
            t.fail,
            if t.surface.l < 0.5 {
                t.canvas
            } else {
                t.surface
            },
        ),
    };
    let (hover, pressed) = if variant == ButtonVariant::Ghost {
        (t.raised, t.border)
    } else {
        (
            mix(background, foreground, 0.12),
            mix(background, foreground, 0.22),
        )
    };
    ButtonColors {
        background,
        foreground,
        hover,
        pressed,
    }
}
