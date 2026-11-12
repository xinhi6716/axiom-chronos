use gpui_kit::SharedString;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
    pub last_seq: u64,
    pub last_activity_ms: Option<i64>,
    pub last_seen_ms: Option<i64>,
    pub open: Vec<OpenActivity>,
    pub claimed: Vec<SharedString>,
    pub running_units: Vec<SharedString>,
    pub council_round: Option<u32>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OpenActivity {
    pub id: SharedString,
    pub module: SharedString,
    pub actor: SharedString,
    pub phase: SharedString,
    pub since_ms: i64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pulse {
    Unknown,
    Live { last_activity_ms: Option<i64> },
    Lost { last_seen_ms: i64 },
}
pub const LOST_AFTER_MS: i64 = 120_000;
