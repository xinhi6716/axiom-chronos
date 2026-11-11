use super::theme::Density;
use super::{LanguageChoice, ThemeChoice};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusPanel {
    Graph,
    Checklist,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowMode {
    #[default]
    Full,
    Focus(FocusPanel),
    Mini,
}
impl WindowMode {
    pub fn toggle_focus(self, panel: FocusPanel) -> Self {
        if self == Self::Focus(panel) {
            Self::Full
        } else {
            Self::Focus(panel)
        }
    }
    pub fn toggle_mini(self) -> Self {
        if self == Self::Mini {
            Self::Full
        } else {
            Self::Mini
        }
    }
    pub fn back_to_full(self) -> Self {
        Self::Full
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoardChoice {
    #[default]
    Luna,
    Haiku,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnWidths {
    pub checklist: f32,
    pub details: f32,
}
impl ColumnWidths {
    /// Ranges are expected to be non-empty and have finite endpoints.
    pub fn clamped(
        self,
        available: f32,
        checklist: Range<f32>,
        details: Range<f32>,
        graph_min: f32,
    ) -> Self {
        // Range has an exclusive end; keep even an oversized saved width inside it.
        let upper = |end: f32| {
            if end == 0.0 {
                -f32::from_bits(1)
            } else if end > 0.0 {
                f32::from_bits(end.to_bits() - 1)
            } else {
                f32::from_bits(end.to_bits() + 1)
            }
        };
        let bound = |v: f32, range: &Range<f32>| {
            if v.is_finite() {
                v.clamp(range.start, upper(range.end))
            } else {
                range.start
            }
        };
        let mut a = bound(self.checklist, &checklist);
        let mut b = bound(self.details, &details);
        let available = if available.is_finite() {
            available
        } else {
            0.0
        };
        let graph_min = if graph_min.is_finite() {
            graph_min.max(0.0)
        } else {
            0.0
        };
        let excess = (a as f64 + b as f64 - (available as f64 - graph_min as f64)).max(0.0);
        let room_a = (a as f64 - checklist.start as f64).max(0.0);
        let room_b = (b as f64 - details.start as f64).max(0.0);
        let room = room_a + room_b;
        if room > 0.0 {
            let fraction = (excess / room).min(1.0);
            a = (a as f64 - room_a * fraction) as f32;
            b = (b as f64 - room_b * fraction) as f32;
        }
        Self {
            checklist: a.clamp(checklist.start, upper(checklist.end)),
            details: b.clamp(details.start, upper(details.end)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewMemory {
    pub filter: Option<String>,
    pub sort: Option<String>,
    pub scroll: f32,
    pub selected: Option<String>,
}
impl ViewMemory {
    pub(super) fn sanitize(&mut self) {
        for text in [&mut self.filter, &mut self.sort, &mut self.selected] {
            if text.as_ref().is_some_and(|s| s.chars().count() > 200) {
                *text = None;
            }
        }
        if !self.scroll.is_finite() || self.scroll < 0.0 {
            self.scroll = 0.0;
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoardTokens {
    pub input: u64,
    pub output: u64,
    pub unreported: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingPrior {
    Theme(ThemeChoice),
    Language(LanguageChoice),
    ReduceMotion(Option<bool>),
    PlainWords(bool),
    Density(Density),
    Mode(WindowMode),
    MiniOnTop(bool),
    Board(BoardChoice),
}
