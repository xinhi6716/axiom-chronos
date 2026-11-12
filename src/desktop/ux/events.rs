#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Ok,
    Warn,
    Error,
    Unknown,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EventSummary {
    pub tone: Tone,
    pub en: String,
    pub zh: String,
    pub raw: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeKind {
    MaxTurns {
        turn_limit: Option<u32>,
    },
    DegradedCouncil {
        member: String,
        phase: String,
        round: u64,
        status: String,
        reason: String,
    },
    ScopeFailure {
        unit: String,
        allowed: Vec<String>,
        outside: Vec<String>,
        outside_count: usize,
        ignored: Vec<(String, String)>,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub key: String,
    pub kind: NoticeKind,
    pub at_ms: Option<i64>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum NoticeAction {
    Resume,
    OpenCouncil,
    OpenUnit(String),
}
