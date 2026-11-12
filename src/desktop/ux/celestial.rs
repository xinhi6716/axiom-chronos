use gpui_kit::SharedString;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Member {
    Astra,
    Fable,
    Argon,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Seat {
    Waiting,
    Speaking,
    Done,
    Degraded { status: String, reason: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Agree,
    Dissent,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Dialogue {
    pub from: Member,
    pub to: Member,
    pub stance: Stance,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CouncilView {
    pub active: bool,
    pub phase: SharedString,
    pub round: u32,
    pub rounds_max: Option<u32>,
    pub seats: Vec<(Member, Seat)>,
    pub speaker: Option<Member>,
    pub dialogue: Vec<Dialogue>,
    pub agreed: u32,
    pub majority: u32,
    pub disputed: u32,
    pub spent_usd: Option<f64>,
    pub cap_usd: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum DualCore {
    Off,
    Deciding {
        topic: SharedString,
    },
    Agreed {
        topic: SharedString,
    },
    Split {
        topic: SharedString,
    },
    Adjudicated {
        topic: SharedString,
        by: SharedString,
    },
}

pub use super::seams::dual_core::{DualCoreSource, FakeDualCore};
