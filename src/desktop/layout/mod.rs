pub const DEFAULT_WINDOW: (f32, f32) = (1440., 900.);
pub const MIN_WINDOW: (f32, f32) = (800., 600.);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Breakpoint {
    Narrow,
    Compact,
    Regular,
    Wide,
    Ultra,
}
impl Breakpoint {
    pub fn of(width: f32) -> Self {
        if !width.is_finite() || width < 960. {
            Self::Narrow
        } else if width < 1280. {
            Self::Compact
        } else if width < 1920. {
            Self::Regular
        } else if width < 2560. {
            Self::Wide
        } else {
            Self::Ultra
        }
    }
}
#[cfg(test)]
mod accept;
pub mod grid;
pub mod persist;
pub mod split;
pub mod window;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn window_sizes_are_shared() {
        let source = include_str!("../mod.rs");
        assert!(source.contains("layout::DEFAULT_WINDOW") && source.contains("layout::MIN_WINDOW"));
        for literal in ["1440.", "900.", "1100.", "700.", "800.", "600."] {
            assert!(!source.contains(literal));
        }
        assert_eq!(DEFAULT_WINDOW, (1440., 900.));
        assert_eq!(MIN_WINDOW, (800., 600.));
    }
}
