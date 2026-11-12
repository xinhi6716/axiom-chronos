use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Chat,
    Quick,
    Wish,
    Research,
    Routine,
    Plaza,
}
impl Mode {
    pub const ALL: [Mode; 6] = [
        Self::Chat,
        Self::Quick,
        Self::Wish,
        Self::Research,
        Self::Routine,
        Self::Plaza,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Quick => "quick",
            Self::Wish => "wish",
            Self::Research => "research",
            Self::Routine => "routine",
            Self::Plaza => "plaza",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == value)
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub created: String,
    pub project: Option<PathBuf>,
    pub scope: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Run {
    pub id: String,
    pub session: String,
    pub parent: Option<String>,
    pub mode: Mode,
    pub started: String,
    pub finished: Option<String>,
    pub outcome: Option<String>,
    pub reference: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub seq: u64,
    pub ts: String,
    pub author: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub session: String,
    pub seq: u64,
    pub author: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub session: String,
    pub upto_seq: u64,
    pub thread: Option<String>,
    pub messages: Vec<Message>,
}
