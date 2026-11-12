use super::JobLimits;
use crate::{base::Res, seams::Clock};
use std::sync::Mutex;

pub trait CuMeter: Send + Sync {
    fn used(&self) -> Res<f64>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapHit {
    WallClock,
    ComputeUnits,
}
pub struct CapWatch<'a> {
    clock: &'a dyn Clock,
    start_ms: i64,
    limits: JobLimits,
    meter: Option<&'a dyn CuMeter>,
    last_cu: f64,
}
impl<'a> CapWatch<'a> {
    pub fn new(clock: &'a dyn Clock, limits: &JobLimits, meter: Option<&'a dyn CuMeter>) -> Self {
        Self {
            clock,
            start_ms: clock.now_ms(),
            limits: limits.clone(),
            meter,
            last_cu: 0.0,
        }
    }
    pub fn check(&mut self) -> Res<Option<CapHit>> {
        if self.elapsed_s() >= self.limits.wall_clock_s as f64 {
            return Ok(Some(CapHit::WallClock));
        }
        if let Some(meter) = self.meter {
            let value = meter.used()?;
            if !value.is_finite() || value < 0.0 {
                return Err(crate::base::Failure::new(
                    "invalid_usage",
                    "invalid compute-unit reading",
                ));
            }
            self.last_cu = value;
            if value >= self.limits.compute_units {
                return Ok(Some(CapHit::ComputeUnits));
            }
        }
        Ok(None)
    }
    pub fn elapsed_s(&self) -> f64 {
        self.clock.now_ms().saturating_sub(self.start_ms).max(0) as f64 / 1000.0
    }
    pub fn last_cu(&self) -> f64 {
        self.last_cu
    }
}
#[derive(Default)]
pub struct FakeMeter {
    value: Mutex<f64>,
}
impl FakeMeter {
    pub fn new(value: f64) -> Self {
        Self {
            value: Mutex::new(value),
        }
    }
    pub fn set(&self, value: f64) {
        *self.value.lock().unwrap() = value;
    }
}
impl CuMeter for FakeMeter {
    fn used(&self) -> Res<f64> {
        Ok(*self.value.lock().unwrap())
    }
}
