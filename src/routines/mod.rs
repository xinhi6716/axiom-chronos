pub mod runner;
pub mod store;

use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    GithubIssue,
    GithubPr,
    CiFailure,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: EventKind,
    pub repo: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub every_minutes: u64,
    pub start_ms: i64,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Routine {
    pub id: String,
    pub project_dir: PathBuf,
    pub wish: String,
    pub enabled: bool,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    #[serde(default)]
    pub triggers: Vec<Trigger>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub kind: EventKind,
    pub repo: String,
    pub title: String,
    pub url: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Cause {
    Schedule {
        due_ms: i64,
    },
    Event {
        id: String,
        kind: EventKind,
        url: String,
    },
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LaunchRequest {
    pub routine: String,
    pub project_dir: PathBuf,
    pub wish: String,
    pub cause: Cause,
}
pub trait WishLauncher: Send + Sync {
    fn launch(&self, request: &LaunchRequest) -> Res<String>;
}

#[derive(Clone, Debug, Default)]
pub struct FakeLauncher {
    requests: Arc<Mutex<Vec<LaunchRequest>>>,
}
impl Serialize for FakeLauncher {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.launched().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for FakeLauncher {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self {
            requests: Arc::new(Mutex::new(Vec::deserialize(deserializer)?)),
        })
    }
}
impl FakeLauncher {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn launched(&self) -> Vec<LaunchRequest> {
        self.requests.lock().unwrap().clone()
    }
}
impl WishLauncher for FakeLauncher {
    fn launch(&self, request: &LaunchRequest) -> Res<String> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request.clone());
        Ok(format!("fake-{}", requests.len()))
    }
}
pub fn validate(routine: &Routine) -> Res<()> {
    if routine.id.is_empty()
        || routine.id.len() > 64
        || !routine
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Failure::new("validation", "invalid routine id"));
    }
    if routine.wish.trim().is_empty() {
        return Err(Failure::new("validation", "wish must not be empty"));
    }
    if routine
        .schedule
        .as_ref()
        .is_some_and(|s| s.every_minutes == 0)
    {
        return Err(Failure::new(
            "validation",
            "schedule interval must be positive",
        ));
    }
    if routine.schedule.is_none() && routine.triggers.is_empty() {
        return Err(Failure::new(
            "validation",
            "a schedule or trigger is required",
        ));
    }
    Ok(())
}
