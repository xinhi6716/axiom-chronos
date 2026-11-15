use super::{readonly::ToolRequest, Mode};
use crate::base::Res;
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Member {
    Opus,
    Astra,
    Fable,
    Argon,
}
impl Member {
    pub const ALL: [Self; 4] = [Self::Opus, Self::Astra, Self::Fable, Self::Argon];
    pub fn name(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Astra => "astra",
            Self::Fable => "fable",
            Self::Argon => "argon",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|member| member.name().eq_ignore_ascii_case(value))
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Ask {
    pub member: Member,
    pub mode: Mode,
    pub session: String,
    pub prompt: String,
    pub context: String,
    pub tools: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reply {
    pub text: String,
    pub tool_requests: Vec<ToolRequest>,
    pub cost_usd: Option<f64>,
    pub tokens: Option<u64>,
}
pub trait Provider {
    fn ask(&self, ask: &Ask) -> Res<Reply>;
}
pub const MAX_CALLS_PER_TURN: usize = 3;
