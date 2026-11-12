use crate::base::{Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub mod colab;
pub mod consent;
pub mod limits;
pub mod local;
pub mod routing;
pub mod session;

pub fn compute_dir(home: &Path) -> PathBuf {
    home.join("compute")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JobLimits {
    pub wall_clock_s: u64,
    pub compute_units: f64,
}
impl JobLimits {
    pub fn validate(&self) -> Res<()> {
        if self.wall_clock_s == 0 || !self.compute_units.is_finite() || self.compute_units <= 0.0 {
            return Err(Failure::new(
                "invalid_limits",
                "limits must be finite and greater than zero",
            ));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ComputeRequest {
    pub job_id: String,
    pub project: String,
    pub project_dir: PathBuf,
    pub argv: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub gpu: String,
    pub vram_gb: f64,
    pub est_minutes: f64,
    pub limits: JobLimits,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Local,
    Colab,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobEnd {
    Completed,
    Failed,
    WallClockCap,
    ComputeUnitCap,
    Cancelled,
    Killed,
    ConsentMissing,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct JobOutcome {
    pub job_id: String,
    pub target: Target,
    pub end: JobEnd,
    pub exit_code: Option<i32>,
    pub gpu_seconds: f64,
    pub compute_units: f64,
    pub session_closed: Option<bool>,
    pub uploads: u64,
    pub detail: String,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CliOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}
pub trait CliJob: Send {
    fn poll(&mut self) -> Res<Option<CliOutput>>;
    fn kill(&mut self);
}
pub trait ColabCli: Send + Sync {
    fn run(&self, argv: &[String], cwd: &Path, timeout: Duration) -> Res<CliOutput>;
    fn start(&self, argv: &[String], cwd: &Path) -> Res<Box<dyn CliJob>>;
}
