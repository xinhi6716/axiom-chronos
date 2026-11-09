use crate::base::{proc, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const BATCH_POLL: Duration = Duration::from_secs(5);
const STOP_CHECK: Duration = Duration::from_millis(200);
const PROGRESS_TAIL: u64 = 4096;
const PROGRESS_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    pub progress_file: String,
    pub marker_file: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchEnd {
    Finished,
    Died,
}

impl Batch {
    pub(super) fn validate(&self) -> Res<()> {
        for (field, path) in [
            ("progress_file", &self.progress_file),
            ("marker_file", &self.marker_file),
        ] {
            if !Path::new(path).is_absolute() {
                return Err(Failure::new(
                    "invalid_argument",
                    format!("{field} must be an absolute path: {path:?}"),
                ));
            }
        }
        if self.pid == 0 {
            return Err(Failure::new(
                "invalid_argument",
                "pid must be a positive process id",
            ));
        }
        Ok(())
    }
    pub(super) fn outcome(&self, end: BatchEnd) -> String {
        match end {
            BatchEnd::Finished => format!(
                "The detached batch (pid {}) finished: its completion marker {} exists. Check its results against the wish before closing the goal.",
                self.pid, self.marker_file
            ),
            BatchEnd::Died => format!(
                "The detached batch (pid {}) died without writing its completion marker {}. Treat this as a failure: diagnose why from its progress file {} and its logs before anything else, and do not count its work as done.",
                self.pid, self.marker_file, self.progress_file
            ),
        }
    }
}

fn progress_text(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(PROGRESS_TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let whole_json = (start == 0)
        .then(|| serde_json::from_str::<serde_json::Value>(&text).ok())
        .flatten();
    let shown = match whole_json {
        Some(value) => value.to_string(),
        None => text
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())?
            .to_owned(),
    };
    Some(shown.chars().take(PROGRESS_CHARS).collect())
}

/// Waits on a detached batch without calling any model, reporting its progress as it changes.
/// `None` means `stop` was raised first, so the batch may still be running.
pub fn wait_for_batch(
    batch: &Batch,
    stop: &AtomicBool,
    sleep: &mut dyn FnMut(Duration),
    progress: &mut dyn FnMut(&str),
) -> Option<BatchEnd> {
    let mut shown = None;
    loop {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        // Checked before the marker: a process seen dead here has written every marker it ever will.
        let alive = proc::pid_alive(batch.pid);
        if let Some(text) = progress_text(Path::new(&batch.progress_file)) {
            if shown.as_ref() != Some(&text) {
                progress(&text);
                shown = Some(text);
            }
        }
        if Path::new(&batch.marker_file).exists() {
            return Some(BatchEnd::Finished);
        }
        if !alive {
            return Some(BatchEnd::Died);
        }
        sleep(BATCH_POLL);
    }
}

/// The sleeper for [`wait_for_batch`] outside tests: a stop ends the sleep within a fraction of a second.
pub fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
    let end = Instant::now() + total;
    while !stop.load(Ordering::SeqCst) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(STOP_CHECK));
    }
}
