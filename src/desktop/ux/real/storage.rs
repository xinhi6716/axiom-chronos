//! Real StorageService over the 2a ledger, retention and disk probe; owned by module
//! service-storage-seam.
use crate::{
    base::{Failure, Res},
    desktop::ux::seams::{
        CleanupPlan, CleanupProgress, DiskSpace, StorageItem, StorageReport, StorageService,
    },
    storage::{
        preflight,
        retention::{self, Action, AuditRecord, Mode, Planned},
        Entry, Ledger, Policy,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn item_of(entry: &Entry, policy: &Policy, now: i64) -> StorageItem {
    StorageItem {
        id: path_text(&entry.path),
        category: entry.class.as_str().into(),
        project: Some(entry.owner.project.clone()),
        wish: entry.owner.wish.clone(),
        path: path_text(&entry.path),
        bytes: entry.size_bytes.unwrap_or(0),
        measured: entry.size_bytes.is_some(),
        reclaimable: retention::is_reclaimable(entry, policy, now),
    }
}

/// The ledger exposes no revision counter, so fingerprint the current entries, including changes
/// to eligibility and measurement, without treating leases as ledger-entry changes.
fn revision(entries: &[Entry]) -> u64 {
    let mut hash = Sha256::new();
    for entry in entries {
        hash.update(serde_json::to_vec(entry).expect("ledger entries are serializable"));
        hash.update([0]);
    }
    u64::from_be_bytes(hash.finalize()[..8].try_into().unwrap())
}

fn plan_id(paths: &[String]) -> String {
    let mut sorted = paths.to_vec();
    sorted.sort();
    let mut hash = Sha256::new();
    for path in &sorted {
        hash.update(path.as_bytes());
        hash.update([0]);
    }
    hash.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct StoredPlan {
    version: u64,
    paths: Vec<String>,
    items: Vec<Planned>,
}

struct Job {
    paths: Vec<String>,
    audit_offset: usize,
    audit_end: Arc<AtomicUsize>,
    finished: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

pub struct LedgerStorage {
    pub home: PathBuf,
    plans: Mutex<HashMap<String, StoredPlan>>,
    jobs: Mutex<HashMap<String, Job>>,
}

impl LedgerStorage {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            plans: Mutex::default(),
            jobs: Mutex::default(),
        }
    }

    fn audit_len(&self) -> usize {
        fs::metadata(retention::audit_log(&self.home))
            .map(|m| m.len() as usize)
            .unwrap_or(0)
    }
}

impl StorageService for LedgerStorage {
    fn report(&self) -> Res<StorageReport> {
        let ledger = Ledger::open(&self.home)?;
        let policy = Policy::load(&self.home)?;
        let now = now_ms();
        let items = ledger
            .entries()?
            .iter()
            .map(|entry| item_of(entry, &policy, now))
            .collect();
        Ok(StorageReport {
            items,
            space: self.space(),
        })
    }

    fn preview(&self, ids: &[String]) -> Res<CleanupPlan> {
        let ledger = Ledger::open(&self.home)?;
        let policy = Policy::load(&self.home)?;
        let now = now_ms();
        let entries = ledger.entries()?;
        let planned = retention::plan(&self.home, &policy, Mode::Routine, now)?;
        let mut items = Vec::new();
        let mut planned_items = Vec::new();
        for item in planned.items.iter().filter(|p| p.action == Action::Delete) {
            let id = path_text(&item.path);
            if !ids.contains(&id) {
                continue;
            }
            if let Some(entry) = entries.iter().find(|e| path_text(&e.path) == id) {
                items.push(item_of(entry, &policy, now));
                planned_items.push(item.clone());
            }
        }
        let paths: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
        let id = plan_id(&paths);
        let version = revision(&entries);
        let bytes = items
            .iter()
            .filter(|i| i.measured)
            .fold(0u64, |sum, i| sum.saturating_add(i.bytes));
        self.plans.lock().unwrap().insert(
            id.clone(),
            StoredPlan {
                version,
                paths,
                items: planned_items,
            },
        );
        Ok(CleanupPlan {
            id,
            version,
            items,
            bytes,
        })
    }

    fn clean(&self, plan_id: &str, version: u64, confirmed: bool) -> Res<String> {
        if !confirmed {
            return Err(Failure::new("unconfirmed", "Confirmation required"));
        }
        let stale = || Failure::new("stale", "Storage changed since the preview");
        let (stored_version, stored_paths, stored_items) = {
            let plans = self.plans.lock().unwrap();
            let plan = plans.get(plan_id).ok_or_else(stale)?;
            (plan.version, plan.paths.clone(), plan.items.clone())
        };
        let ledger = Ledger::open(&self.home)?;
        if stored_version != version || revision(&ledger.entries()?) != version {
            return Err(stale());
        }
        let policy = Policy::load(&self.home)?;
        let now = now_ms();
        let mut current = retention::plan(&self.home, &policy, Mode::Routine, now)?;
        let mut run = retention::Plan::default();
        let mut protection = Vec::new();
        for path in &stored_paths {
            let used = ledger.in_use(Path::new(path))?;
            let position = current
                .items
                .iter()
                .position(|p| p.action == Action::Delete && path_text(&p.path) == *path);
            match position {
                _ if used => {
                    // Keep the entry leased until execute has audited its protected outcome.
                    protection.push(ledger.lease(Path::new(path))?);
                    run.items.push(
                        stored_items
                            .iter()
                            .find(|item| path_text(&item.path) == *path)
                            .ok_or_else(stale)?
                            .clone(),
                    );
                }
                Some(index) => run.items.push(current.items.remove(index)),
                None => return Err(stale()),
            }
        }
        let mut jobs = self.jobs.lock().unwrap();
        let job_id = format!("clean-{}-{}", plan_id, jobs.len() + 1);
        let finished = Arc::new(AtomicBool::new(false));
        let audit_end = Arc::new(AtomicUsize::new(0));
        let error = Arc::new(Mutex::new(None));
        jobs.insert(
            job_id.clone(),
            Job {
                paths: stored_paths,
                audit_offset: self.audit_len(),
                audit_end: audit_end.clone(),
                finished: finished.clone(),
                error: error.clone(),
            },
        );
        drop(jobs);
        let home = self.home.clone();
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            if let Err(e) = retention::execute(&home, &run, now, &cancel) {
                *error.lock().unwrap() = Some(e.message);
            }
            let end = fs::metadata(retention::audit_log(&home))
                .map(|m| m.len() as usize)
                .unwrap_or(0);
            audit_end.store(end, Ordering::SeqCst);
            drop(protection);
            finished.store(true, Ordering::SeqCst);
        });
        self.plans.lock().unwrap().remove(plan_id);
        Ok(job_id)
    }

    fn progress(&self, job: &str) -> Res<CleanupProgress> {
        let jobs = self.jobs.lock().unwrap();
        let job = jobs
            .get(job)
            .ok_or_else(|| Failure::new("not_found", "Job not found"))?;
        let finished = job.finished.load(Ordering::SeqCst);
        let mut done = Vec::new();
        let mut failed = Vec::new();
        let log = fs::read(retention::audit_log(&self.home)).unwrap_or_default();
        let end = if finished {
            job.audit_end.load(Ordering::SeqCst)
        } else {
            log.len()
        };
        let log = log.get(job.audit_offset..end).unwrap_or_default();
        for line in String::from_utf8_lossy(log).lines() {
            let Ok(record) = serde_json::from_str::<AuditRecord>(line) else {
                continue;
            };
            if !job.paths.contains(&record.path) || failed.iter().any(|(p, _)| *p == record.path) {
                continue;
            }
            if record.outcome == "done" {
                done.push(record.path);
            } else {
                let outcome = if record.outcome == "skipped: in use" {
                    "protected".into()
                } else {
                    record.outcome
                };
                failed.push((record.path, outcome));
            }
        }
        if finished {
            let reason = job
                .error
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| "not processed".into());
            for path in &job.paths {
                if !done.contains(path) && !failed.iter().any(|(p, _)| p == path) {
                    failed.push((path.clone(), reason.clone()));
                }
            }
        }
        let remaining = job.paths.len().saturating_sub(done.len() + failed.len());
        Ok(CleanupProgress {
            done,
            failed,
            remaining,
            finished,
        })
    }

    fn space(&self) -> Option<DiskSpace> {
        let free_bytes = preflight::os_probe().free_bytes(&self.home).ok()?;
        // The probe reports free space only, and a zero total means the total is unknown.
        Some(DiskSpace {
            free_bytes,
            total_bytes: 0,
        })
    }
}
