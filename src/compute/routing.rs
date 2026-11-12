use super::{colab, local, CliJob, ComputeRequest, JobOutcome, Target};
use crate::{
    base::Res,
    seams::{Clock, PersonApproval},
    usage::extensions::{self, ExtRecord},
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalCapacity {
    pub vram_gb: f64,
    pub max_minutes: f64,
}
pub trait LocalGpu: Send + Sync {
    fn capacity(&self) -> Option<LocalCapacity>;
    fn start(&self, req: &ComputeRequest) -> Res<Box<dyn CliJob>>;
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RouteReason {
    LocalFits,
    VramTooSmall,
    TimeTooShort,
    LocalUnavailable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub target: Target,
    pub reason: RouteReason,
}
pub fn route(req: &ComputeRequest, local: Option<&LocalCapacity>) -> Route {
    let reason = match local {
        None => RouteReason::LocalUnavailable,
        Some(cap) if !(cap.vram_gb >= req.vram_gb) => RouteReason::VramTooSmall,
        Some(cap) if !(cap.max_minutes >= req.est_minutes) => RouteReason::TimeTooShort,
        Some(_) => RouteReason::LocalFits,
    };
    Route {
        target: if reason == RouteReason::LocalFits {
            Target::Local
        } else {
            Target::Colab
        },
        reason,
    }
}
pub struct Compute<'a> {
    pub home: &'a Path,
    pub local: &'a dyn LocalGpu,
    pub colab: &'a dyn super::ColabCli,
    pub approval: &'a dyn PersonApproval,
    pub clock: &'a dyn Clock,
    pub cancel: &'a AtomicBool,
    pub poll: Duration,
}
pub fn run_job(c: &Compute, req: &ComputeRequest) -> Res<JobOutcome> {
    req.limits.validate()?;
    let target = route(req, c.local.capacity().as_ref()).target;
    let outcome = match target {
        Target::Local => local::run(c, req)?,
        Target::Colab => colab::run(c, req)?,
    };
    extensions::append(
        c.home,
        &ExtRecord {
            provider: match target {
                Target::Local => "gpu_local",
                Target::Colab => "colab",
            }
            .into(),
            project: Some(req.project.clone()),
            job: Some(req.job_id.clone()),
            gpu_seconds: outcome.gpu_seconds,
            compute_units: outcome.compute_units,
            ts_ms: c.clock.now_ms(),
            ..Default::default()
        },
    )?;
    Ok(outcome)
}
#[doc(hidden)]
pub struct FakeLocalGpu {
    capacity: Mutex<Option<LocalCapacity>>,
    duration: Mutex<Duration>,
    exit_code: Mutex<i32>,
    calls: Mutex<Vec<ComputeRequest>>,
    kills: Arc<AtomicUsize>,
}
impl FakeLocalGpu {
    pub fn new(capacity: Option<LocalCapacity>) -> Self {
        Self {
            capacity: Mutex::new(capacity),
            duration: Mutex::new(Duration::ZERO),
            exit_code: Mutex::new(0),
            calls: Mutex::new(Vec::new()),
            kills: Arc::new(AtomicUsize::new(0)),
        }
    }
    pub fn set_capacity(&self, capacity: Option<LocalCapacity>) {
        *self.capacity.lock().unwrap() = capacity;
    }
    pub fn set_duration(&self, duration: Duration) {
        *self.duration.lock().unwrap() = duration;
    }
    pub fn set_exit_code(&self, code: i32) {
        *self.exit_code.lock().unwrap() = code;
    }
    pub fn calls(&self) -> Vec<ComputeRequest> {
        self.calls.lock().unwrap().clone()
    }
    pub fn kills(&self) -> usize {
        self.kills.load(Ordering::SeqCst)
    }
}
impl LocalGpu for FakeLocalGpu {
    fn capacity(&self) -> Option<LocalCapacity> {
        *self.capacity.lock().unwrap()
    }
    fn start(&self, req: &ComputeRequest) -> Res<Box<dyn CliJob>> {
        self.calls.lock().unwrap().push(req.clone());
        Ok(Box::new(FakeJob {
            start: Instant::now(),
            duration: *self.duration.lock().unwrap(),
            code: *self.exit_code.lock().unwrap(),
            killed: false,
            kills: self.kills.clone(),
        }))
    }
}
struct FakeJob {
    start: Instant,
    duration: Duration,
    code: i32,
    killed: bool,
    kills: Arc<AtomicUsize>,
}
impl CliJob for FakeJob {
    fn poll(&mut self) -> Res<Option<super::CliOutput>> {
        Ok(
            (self.killed || self.start.elapsed() >= self.duration).then(|| super::CliOutput {
                code: if self.killed { -1 } else { self.code },
                stdout: String::new(),
                stderr: String::new(),
            }),
        )
    }
    fn kill(&mut self) {
        if !self.killed {
            self.kills.fetch_add(1, Ordering::SeqCst);
            self.killed = true;
        }
    }
}
