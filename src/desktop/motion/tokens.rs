use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dur {
    Instant,
    Quick,
    Standard,
    Gentle,
    Slow,
}

impl Dur {
    pub const fn get(self) -> Duration {
        Duration::from_millis(match self {
            Self::Instant => 0,
            Self::Quick => 120,
            Self::Standard => 220,
            Self::Gentle => 360,
            Self::Slow => 600,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

impl Spring {
    pub const SNAPPY: Self = Self {
        stiffness: 900.0,
        damping: 60.0,
        mass: 1.0,
    };
    pub const GENTLE: Self = Self {
        stiffness: 625.0,
        damping: 50.0,
        mass: 1.0,
    };
    pub const SOFT: Self = Self {
        stiffness: 400.0,
        damping: 40.0,
        mass: 1.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    Out,
    In,
    InOut,
    Smooth,
}

impl Ease {
    pub fn at(self, x: f32) -> f32 {
        match self {
            Self::Out => super::ease_out(x),
            Self::In => super::ease_in(x),
            Self::InOut => super::ease_in_out(x),
            Self::Smooth => super::smooth(x),
        }
    }
}
