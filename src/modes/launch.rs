use super::{Mode, Run, Snapshot, Source};
use crate::base::Res;
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LaunchKind {
    Quick,
    Wish,
}
impl LaunchKind {
    pub fn mode(self) -> Mode {
        match self {
            Self::Quick => Mode::Quick,
            Self::Wish => Mode::Wish,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LaunchRequest {
    pub kind: LaunchKind,
    pub session: String,
    pub parent_run: String,
    pub goal: String,
    pub scope: Vec<String>,
    pub verify: Option<String>,
    pub review: bool,
    pub context: Option<Snapshot>,
    pub sources: Vec<Source>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Launched {
    pub run: Run,
    pub detail: serde_json::Value,
}
pub trait Launcher {
    fn launch(&self, req: &LaunchRequest) -> Res<Launched>;
}
pub fn start_child(home: &Path, req: &LaunchRequest) -> Res<Run> {
    super::runs::start_run(
        home,
        &req.session,
        req.kind.mode(),
        Some(&req.parent_run),
        None,
    )
}
