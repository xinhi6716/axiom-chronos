//! Design tokens. Every color a screen uses comes from here, so both themes stay consistent.
use super::model::{CheckStatus, LogLevel, Phase, RunStatus, UnitState};
use super::motion::mix;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{theme::ThemeToken, ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::*;

#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub gold: Hsla,
    pub ok: Hsla,
    pub run: Hsla,
    pub review: Hsla,
    pub fail: Hsla,
    pub idle: Hsla,
    /// Chart lines and dividers: visible only as structure, never as text.
    pub orbit: Hsla,
}

impl Global for Tokens {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Normal,
    Hover,
    Pressed,
}

pub struct GoldLeaf {
    pub top: Hsla,
    pub bottom: Hsla,
    pub highlight: Hsla,
    pub label: Hsla,
}

// Aged pigments, dark enough to read on parchment and light enough to read on the night sky.
const MODULE_HUES: [u32; 10] = [
    0x53947c, 0xbc7379, 0x6589bc, 0x907dbc, 0xb67948, 0x9a8644, 0x47929b, 0xb96f9a, 0x7c8f4c, 0x76889c,
];

// Sampled from the Chronos portrait: navy night, ivory robes, old gold.
const DARK: Tokens = Tokens {
    canvas: hex(0x0c1826),
    surface: hex(0x122233),
    raised: hex(0x1a2e42),
    border: hex(0x2b4154),
    text: hex(0xf6ead2),
    muted: hex(0xb3b6b4),
    gold: hex(0xdcc28a),
    ok: hex(0x7cc4a4),
    run: hex(0x8fb8de),
    review: hex(0xe0a458),
    fail: hex(0xe58a76),
    idle: hex(0x75879c),
    orbit: hex(0x4a5a66),
};

// The same order drawn as a star chart: parchment and navy ink.
const LIGHT: Tokens = Tokens {
    canvas: hex(0xf6ecd8),
    surface: hex(0xfbf5e8),
    raised: hex(0xfffcf4),
    border: hex(0xdccfb4),
    text: hex(0x142838),
    muted: hex(0x5b5548),
    gold: hex(0x7f5f26),
    ok: hex(0x2c7259),
    run: hex(0x2f5f8f),
    review: hex(0x96590f),
    fail: hex(0xad3d2b),
    idle: hex(0x8a8270),
    orbit: hex(0xc9bda2),
};

const fn hex(rgb: u32) -> Hsla {
    // Const conversion from sRGB to HSLA, because `gpui::rgb` is not const.
    let r = ((rgb >> 16) & 0xff) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xff) as f32 / 255.0;
    let b = (rgb & 0xff) as f32 / 255.0;
    let max = if r > g { if r > b { r } else { b } } else if g > b { g } else { b };
    let min = if r < g { if r < b { r } else { b } } else if g < b { g } else { b };
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return Hsla { h: 0.0, s: 0.0, l, a: 1.0 };
    }
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Hsla { h: h / 6.0, s, l, a: 1.0 }
}

/// Both themes' tokens, for tests elsewhere that check a rule in each.
#[cfg(test)]
pub fn both() -> [Tokens; 2] {
    [DARK, LIGHT]
}

/// The night tokens, whatever the theme: the launch always opens in darkness.
pub fn night() -> Tokens {
    DARK
}

pub fn module_color(index: usize) -> Hsla {
    rgb(MODULE_HUES[index % MODULE_HUES.len()]).into()
}

pub trait Tokened {
    fn tokens(&self) -> Tokens;
}

impl Tokened for App {
    fn tokens(&self) -> Tokens {
        *self.global::<Tokens>()
    }
}

impl Tokens {
    pub fn rail_ink(&self, lit: f32) -> Hsla {
        mix(self.muted, self.gold, lit)
    }

    pub fn gold_leaf(&self, press: Press) -> GoldLeaf {
        let (top, bottom) = match press {
            Press::Normal => (0.20, 0.0),
            Press::Hover => (0.30, 0.10),
            Press::Pressed => (0.0, 0.08),
        };
        GoldLeaf {
            top: mix(self.gold, self.text, top),
            bottom: mix(self.gold, self.text, bottom),
            highlight: mix(self.gold, self.text, 0.4),
            label: if self.surface.l < 0.5 { self.canvas } else { self.surface },
        }
    }

    pub fn unit(&self, state: UnitState) -> Hsla {
        match state {
            UnitState::Pending => self.idle,
            UnitState::Running => self.run,
            UnitState::Built => self.review,
            UnitState::Verified => self.ok,
            UnitState::Failed => self.fail,
        }
    }

    pub fn phase(&self, phase: Phase) -> Hsla {
        match phase {
            Phase::Waiting | Phase::Skipped => self.idle,
            Phase::Running => self.run,
            Phase::Done => self.ok,
            Phase::Failed => self.fail,
        }
    }

    pub fn status(&self, status: RunStatus) -> Hsla {
        match status {
            RunStatus::Active => self.run,
            RunStatus::Waiting | RunStatus::LimitWait => self.review,
            RunStatus::Interrupted | RunStatus::Blocked => self.fail,
            RunStatus::Done => self.ok,
        }
    }

    /// The status color of a run: an ending that was not verified is never drawn as a success.
    pub fn outcome(&self, status: RunStatus, verified_end: bool) -> Hsla {
        if status == RunStatus::Done && !verified_end {
            self.review
        } else {
            self.status(status)
        }
    }

    pub fn log(&self, level: LogLevel) -> Hsla {
        match level {
            LogLevel::Info => self.muted,
            LogLevel::Ok => self.ok,
            LogLevel::Warn => self.review,
            LogLevel::Error => self.fail,
        }
    }

    pub fn check(&self, status: CheckStatus) -> Hsla {
        match status {
            CheckStatus::Ok => self.ok,
            CheckStatus::Warn => self.review,
            CheckStatus::Fail => self.fail,
        }
    }
}

fn leaf_gradient(leaf: &GoldLeaf) -> Background {
    linear_gradient(180.0, linear_color_stop(leaf.top, 0.0), linear_color_stop(leaf.bottom, 1.0))
}

pub(crate) struct PrimaryTokens {
    normal: ThemeToken,
    hover: ThemeToken,
    pressed: ThemeToken,
    foreground: Hsla,
}

pub(crate) fn primary_tokens(t: &Tokens) -> PrimaryTokens {
    let token = |press| {
        let leaf = t.gold_leaf(press);
        ThemeToken::new(mix(leaf.top, leaf.bottom, 0.5), leaf_gradient(&leaf))
    };
    PrimaryTokens {
        normal: token(Press::Normal),
        hover: token(Press::Hover),
        pressed: token(Press::Pressed),
        foreground: t.gold_leaf(Press::Normal).label,
    }
}

/// Restyle the existing button, retaining its identity and behavior. The primary theme
/// supplies the state gradients; the kit derives disabled colors from its normal primary
/// background because Button does not expose its disabled state or a disabled color hook.
pub fn gold_leaf(button: Button, t: &Tokens) -> Button {
    let leaf = t.gold_leaf(Press::Normal);
    // Button replays instance styles after its disabled foreground, so text_color here
    // would override the kit's disabled style; the theme supplies the label instead.
    button
        .primary()
        .border_0()
        .border_t_1()
        .border_color(leaf.highlight)
}

/// The interface font on Windows. The kit's default, `.SystemUIFont`, resolves to the system's
/// icon-title font, which on Chinese Windows is Microsoft JhengHei UI. That font has no glyph for
/// characters such as the subscript in "HfO₂", and at semibold and bold weights gpui draws the
/// fallback glyph DirectWrite substitutes for them as the wrong character. Segoe UI has those
/// glyphs, and Chinese text still falls back to the system's Chinese font for the user's locale.
#[cfg(windows)]
pub const UI_FONT: Option<&str> = Some("Segoe UI");
#[cfg(not(windows))]
pub const UI_FONT: Option<&str> = None;

/// Applies a mode to both the component library's theme and our tokens.
pub fn apply(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    let t = if cx.theme().is_dark() { DARK } else { LIGHT };
    cx.set_global(t);
    Theme::update(cx, |theme| {
        if let Some(font) = UI_FONT {
            theme.font_family = font.into();
        }
        theme.background = t.canvas;
        theme.foreground = t.text;
        theme.border = t.border;
        theme.input = t.border;
        theme.muted = t.raised;
        theme.muted_foreground = t.muted;
        theme.popover = t.raised;
        theme.popover_foreground = t.text;
        theme.primary = t.gold;
        theme.primary_hover = t.gold.opacity(0.9);
        theme.primary_active = t.gold.opacity(0.8);
        theme.primary_foreground = t.canvas;
        let primary = primary_tokens(&t);
        theme.button_primary = primary.normal.color;
        theme.button_primary_hover = primary.hover.color;
        theme.button_primary_active = primary.pressed.color;
        theme.button_primary_foreground = primary.foreground;
        theme.tokens.button_primary = primary.normal;
        theme.tokens.button_primary_hover = primary.hover;
        theme.tokens.button_primary_active = primary.pressed;
        theme.success = t.ok;
        theme.warning = t.review;
        theme.danger = t.fail;
        theme.info = t.run;
        theme.ring = t.gold;
        theme.caret = t.gold;
        theme.selection = t.gold.opacity(0.3);
        theme.sidebar = t.surface;
        theme.sidebar_foreground = t.text;
        theme.sidebar_border = t.border;
        theme.sidebar_accent = t.raised;
        theme.sidebar_accent_foreground = t.text;
        theme.sidebar_primary = t.gold;
        theme.sidebar_primary_foreground = t.canvas;
        theme.accent = t.raised;
        theme.accent_foreground = t.text;
        theme.list_hover = t.raised;
        theme.list_active = t.raised;
        theme.list_active_border = t.gold;
        // Dialogs fade in and out quickly; their content settles with the app's own spring.
        theme.motion.duration_normal = super::motion::Pace::Close.duration();
    });
}

#[cfg(test)]
mod tests {
    // Named imports: a glob would bring in gpui's own `test` macro.
    use super::{hex, linear_color_stop, linear_gradient, mix, module_color, primary_tokens, Hsla, Press, Rgba, DARK, LIGHT, MODULE_HUES};

    fn luminance(c: Hsla) -> f32 {
        let c = Rgba::from(c);
        let line = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * line(c.r) + 0.7152 * line(c.g) + 0.0722 * line(c.b)
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn gold_leaf_labels_meet_aa_at_every_stop() {
        for t in [DARK, LIGHT] {
            for press in [Press::Normal, Press::Hover, Press::Pressed] {
                let leaf = t.gold_leaf(press);
                for bg in [leaf.top, leaf.bottom, mix(leaf.top, leaf.bottom, 0.5)] {
                    let ratio = contrast(leaf.label, bg);
                    assert!(ratio >= 4.5, "{press:?} label {:?} on {bg:?}: {ratio}", leaf.label);
                }
            }
        }
    }

    #[test]
    fn rail_ink_is_visible_on_surface() {
        for t in [DARK, LIGHT] {
            for lit in [0.0, 0.5, 1.0] {
                let ratio = contrast(t.rail_ink(lit), t.surface);
                assert!(ratio >= 3.0, "lit {lit} on {:?}: {ratio}", t.surface);
            }
        }
    }

    #[test]
    fn contrast_rejects_a_low_contrast_pair() {
        assert!(contrast(DARK.gold, DARK.text) < 4.5);
        assert!(contrast(hex(0x000000), hex(0xffffff)) > 4.5);
    }

    #[test]
    fn primary_tokens_match_gold_leaf_and_expected_colours() {
        let expected = [
            [
                Hsla { h: 0.113695, s: 0.544304, l: 0.721176, a: 1.0 },
                Hsla { h: 0.113553, s: 0.549849, l: 0.740392, a: 1.0 },
                Hsla { h: 0.113772, s: 0.541329, l: 0.709647, a: 1.0 },
            ],
            [
                Hsla { h: 0.108279, s: 0.490070, l: 0.306078, a: 1.0 },
                Hsla { h: 0.110417, s: 0.434783, l: 0.288627, a: 1.0 },
                Hsla { h: 0.107302, s: 0.520317, l: 0.316549, a: 1.0 },
            ],
        ];
        for (t, expected) in [DARK, LIGHT].into_iter().zip(expected) {
            let primary = primary_tokens(&t);
            let tokens = [primary.normal, primary.hover, primary.pressed];
            for ((token, press), expected) in tokens.into_iter()
                .zip([Press::Normal, Press::Hover, Press::Pressed]).zip(expected)
            {
                let leaf = t.gold_leaf(press);
                assert_eq!(token.color, mix(leaf.top, leaf.bottom, 0.5));
                for (actual, expected) in [token.color.h, token.color.s, token.color.l, token.color.a]
                    .into_iter().zip([expected.h, expected.s, expected.l, expected.a])
                {
                    assert!((actual - expected).abs() <= 0.005, "{press:?}: {actual} != {expected}");
                }
                assert_eq!(token.background, linear_gradient(180.0,
                    linear_color_stop(leaf.top, 0.0), linear_color_stop(leaf.bottom, 1.0)));
            }
            assert_ne!(primary.normal.color, primary.hover.color);
            assert_ne!(primary.hover.color, primary.pressed.color);
            assert_eq!(primary.foreground, t.gold_leaf(Press::Normal).label);
        }
    }

    #[test]
    fn text_meets_aa_on_every_background_in_both_themes() {
        for t in [DARK, LIGHT] {
            for bg in [t.canvas, t.surface, t.raised] {
                for fg in [t.text, t.muted, t.gold, t.ok, t.run, t.review, t.fail] {
                    assert!(contrast(fg, bg) >= 4.5, "{fg:?} on {bg:?}: {}", contrast(fg, bg));
                }
            }
        }
    }

    #[test]
    fn orbit_lines_are_visible_structure_that_never_competes_with_text() {
        for t in [DARK, LIGHT] {
            for bg in [t.canvas, t.surface, t.raised] {
                let c = contrast(t.orbit, bg);
                assert!((1.4..3.0).contains(&c), "orbit on {bg:?}: {c}");
            }
        }
    }

    #[test]
    fn state_marks_and_module_colors_stay_visible_in_both_themes() {
        for t in [DARK, LIGHT] {
            for bg in [t.canvas, t.surface, t.raised] {
                assert!(contrast(t.idle, bg) >= 3.0, "idle on {bg:?}: {}", contrast(t.idle, bg));
            }
            for i in 0..MODULE_HUES.len() {
                assert!(contrast(module_color(i), t.canvas) >= 3.0, "module {i}");
            }
        }
    }
}
