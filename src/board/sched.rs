use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
struct Run {
    revision: u64,
    dirty: bool,
    flight: Option<u64>,
    next: Instant,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    min_gap: Duration,
    runs: HashMap<String, Run>,
}
impl Scheduler {
    pub fn new(min_gap: Duration) -> Self {
        Self {
            min_gap,
            runs: HashMap::new(),
        }
    }
    pub fn observe(&mut self, session: &str, revision: u64, now: Instant) {
        match self.runs.get_mut(session) {
            Some(run) if run.revision != revision => {
                run.revision = revision;
                run.dirty = true;
            }
            Some(_) => {}
            None => {
                self.runs.insert(
                    session.into(),
                    Run {
                        revision,
                        dirty: true,
                        flight: None,
                        next: now,
                    },
                );
            }
        }
    }
    pub fn due(&mut self, now: Instant) -> Vec<(String, u64)> {
        let mut due = Vec::new();
        for (session, run) in &mut self.runs {
            if run.dirty && run.flight.is_none() && run.next <= now {
                run.flight = Some(run.revision);
                due.push((session.clone(), run.revision));
            }
        }
        due.sort_unstable();
        due
    }
    pub fn finished(&mut self, session: &str, revision: u64, now: Instant) -> bool {
        let Some(run) = self.runs.get_mut(session) else {
            return false;
        };
        if run.flight.take() != Some(revision) {
            return false;
        }
        run.next = now + self.min_gap;
        run.dirty = run.revision != revision;
        !run.dirty
    }
    pub fn forget(&mut self, session: &str) {
        self.runs.remove(session);
    }
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        self.runs
            .values()
            .filter(|r| r.dirty && r.flight.is_none())
            .map(|r| r.next.max(now))
            .min()
    }
}
