use super::{Settling, Spring};
use std::time::Instant;

/// A damped oscillator, evaluated analytically from its last retarget.
#[derive(Clone, Copy, Debug)]
pub struct SpringValue {
    from: f32,
    to: f32,
    velocity: f32,
    start: Option<Instant>,
    spring: Spring,
}

impl SpringValue {
    pub fn new(value: f32, spring: Spring) -> Self {
        Self {
            from: value,
            to: value,
            velocity: 0.0,
            start: None,
            spring,
        }
    }

    pub fn set(&mut self, to: f32, now: Instant) {
        if self.to == to {
            return;
        }
        let (value, velocity) = if self.settled(now) {
            (self.to, 0.0)
        } else {
            self.sample(now)
        };
        self.from = value;
        self.velocity = velocity;
        self.to = to;
        self.start = Some(now);
    }

    /// Retarget, or immediately snap when motion is reduced.
    pub fn set_reduced(&mut self, to: f32, now: Instant, reduce: bool) {
        if reduce {
            self.from = to;
            self.to = to;
            self.velocity = 0.0;
            self.start = None;
        } else {
            self.set(to, now);
        }
    }

    pub fn value(&self, now: Instant) -> f32 {
        if self.settled(now) {
            self.to
        } else {
            self.sample(now).0
        }
    }

    pub fn value_reduced(&self, now: Instant, reduce: bool) -> f32 {
        if reduce {
            self.to
        } else {
            self.value(now)
        }
    }

    pub fn settled(&self, now: Instant) -> bool {
        if self.start.is_none() {
            return true;
        }
        let (value, velocity) = self.sample(now);
        (value - self.to).abs() < 0.001 && velocity.abs() < 0.001
    }

    fn sample(&self, now: Instant) -> (f32, f32) {
        let Some(start) = self.start else {
            return (self.to, 0.0);
        };
        let t = now.saturating_duration_since(start).as_secs_f32();
        let x = self.from - self.to;
        let v = self.velocity;
        let mass = self.spring.mass.max(f32::EPSILON);
        let omega2 = self.spring.stiffness.max(0.0) / mass;
        let alpha = self.spring.damping.max(0.0) / (2.0 * mass);
        let discriminant = alpha * alpha - omega2;
        let (position, velocity) = if discriminant.abs() < 1e-4 {
            let b = v + alpha * x;
            let decay = (-alpha * t).exp();
            ((x + b * t) * decay, (b - alpha * (x + b * t)) * decay)
        } else if discriminant < 0.0 {
            let omega = (-discriminant).sqrt();
            let b = (v + alpha * x) / omega;
            let (sin, cos) = (omega * t).sin_cos();
            let decay = (-alpha * t).exp();
            let y = x * cos + b * sin;
            (
                decay * y,
                decay * (-alpha * y - x * omega * sin + b * omega * cos),
            )
        } else {
            let root = discriminant.sqrt();
            let r1 = -alpha + root;
            let r2 = -alpha - root;
            let a = (v - r2 * x) / (r1 - r2);
            let b = x - a;
            let e1 = (r1 * t).exp();
            let e2 = (r2 * t).exp();
            (a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        };
        (self.to + position, velocity)
    }
}

impl Settling for SpringValue {
    fn settled(&self, now: Instant, reduce: bool) -> bool {
        reduce || self.settled(now)
    }
}
