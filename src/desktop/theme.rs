//! Design tokens. Every color a screen uses comes from here, so both themes stay consistent.
use super::model::{CheckStatus, LogLevel, Phase, RunStatus, UnitState};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
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
            Phase::Waiting => self.idle,
            Phase::Running => self.run,
            Phase::Done => self.ok,
            Phase::Failed => self.fail,
        }
    }

    pub fn status(&self, status: RunStatus) -> Hsla {
        match status {
            RunStatus::Active => self.run,
            RunStatus::Waiting => self.review,
            RunStatus::Blocked => self.fail,
            RunStatus::Done => self.ok,
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

/// Applies a mode to both the component library's theme and our tokens.
pub fn apply(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    let t = if cx.theme().is_dark() { DARK } else { LIGHT };
    cx.set_global(t);
    Theme::update(cx, |theme| {
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
        theme.button_primary = t.gold;
        theme.button_primary_hover = t.gold.opacity(0.9);
        theme.button_primary_active = t.gold.opacity(0.8);
        theme.button_primary_foreground = t.canvas;
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
    });
}

#[cfg(test)]
mod tests {
    // Named imports: a glob would bring in gpui's own `test` macro.
    use super::{module_color, Hsla, Rgba, DARK, LIGHT, MODULE_HUES};

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
