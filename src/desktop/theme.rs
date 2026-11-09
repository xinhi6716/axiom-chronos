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
}

impl Global for Tokens {}

const MODULE_HUES: [u32; 10] = [
    0x36d399, 0xf472b6, 0x60a5fa, 0xa78bfa, 0xfb923c, 0xfacc15, 0x22d3ee, 0xf87171, 0x84cc16, 0xe879f9,
];

const DARK: Tokens = Tokens {
    canvas: hex(0x0b1016),
    surface: hex(0x121a24),
    raised: hex(0x18222e),
    border: hex(0x26323f),
    text: hex(0xf4efe3),
    muted: hex(0x8a97a6),
    gold: hex(0xd7bd82),
    ok: hex(0x36d399),
    run: hex(0x60a5fa),
    review: hex(0xfacc15),
    fail: hex(0xf87171),
    idle: hex(0x566675),
};

const LIGHT: Tokens = Tokens {
    canvas: hex(0xf4efe3),
    surface: hex(0xfbf8f1),
    raised: hex(0xffffff),
    border: hex(0xd9d2c1),
    text: hex(0x101a22),
    muted: hex(0x5b5548),
    gold: hex(0x8d6b2d),
    ok: hex(0x0f9d6b),
    run: hex(0x2563eb),
    review: hex(0xb7791f),
    fail: hex(0xc53b3b),
    idle: hex(0x9d9788),
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
