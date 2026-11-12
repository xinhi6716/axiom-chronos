use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StartRequest {
    pub wish: String,
    pub cwd: String,
    #[serde(default)]
    pub creative: bool,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub advanced_research: bool,
    #[serde(default)]
    pub codex_max_parallel: Option<u64>,
    #[serde(default)]
    pub need_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub instance: String,
    pub cursor: u64,
    pub login_start: bool,
    pub wishes: Vec<HostedWish>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostedWish {
    pub key: String,
    pub handle: String,
    pub session: Option<String>,
    pub text: String,
    pub cwd: String,
    pub started_unix: u64,
    pub state: String,
    pub limit_until: Option<i64>,
    pub ended_at: Option<String>,
    pub ending: String,
    pub disk_line: Option<String>,
    pub log: Vec<LogRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    pub time_ms: i64,
    pub source: String,
    pub text: String,
    pub level: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEvent {
    pub seq: u64,
    pub instance: String,
    pub handle: String,
    pub session: Option<String>,
    pub kind: String,
    pub data: Value,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_round_trip() {
        let event = WireEvent {
            seq: 2,
            instance: "i".into(),
            handle: "h".into(),
            session: Some("s".into()),
            kind: "text".into(),
            data: serde_json::json!({"text":"hi"}),
        };
        let v = serde_json::to_value(event).unwrap();
        let e: WireEvent = serde_json::from_value(v).unwrap();
        assert_eq!(e.seq, 2);
        assert_eq!(e.data["text"], "hi");
    }
}
