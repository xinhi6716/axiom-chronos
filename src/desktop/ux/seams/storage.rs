use crate::base::{Failure, Res};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiskSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StorageItem {
    pub id: String,
    pub category: String,
    pub project: Option<String>,
    pub wish: Option<String>,
    pub path: String,
    pub bytes: u64,
    pub measured: bool,
    pub reclaimable: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct StorageReport {
    pub items: Vec<StorageItem>,
    pub space: Option<DiskSpace>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CleanupPlan {
    pub id: String,
    pub version: u64,
    pub items: Vec<StorageItem>,
    pub bytes: u64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CleanupProgress {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub remaining: usize,
    pub finished: bool,
}
pub trait StorageService: Send + Sync + 'static {
    fn report(&self) -> Res<StorageReport>;
    fn preview(&self, ids: &[String]) -> Res<CleanupPlan>;
    fn clean(&self, plan_id: &str, version: u64, confirmed: bool) -> Res<String>;
    fn progress(&self, job: &str) -> Res<CleanupProgress>;
    fn space(&self) -> Option<DiskSpace>;
}
#[derive(Default)]
struct State {
    items: Vec<StorageItem>,
    space: Option<DiskSpace>,
    revision: u64,
    next: u64,
    plans: HashMap<String, CleanupPlan>,
    jobs: HashMap<String, (Vec<String>, CleanupProgress)>,
    fail: HashSet<String>,
    deleted: Vec<String>,
}
#[derive(Default)]
pub struct FakeStorage {
    state: Mutex<State>,
}
impl FakeStorage {
    pub fn new(items: Vec<StorageItem>, space: Option<DiskSpace>) -> Self {
        Self {
            state: Mutex::new(State {
                items,
                space,
                ..Default::default()
            }),
        }
    }
    pub fn touch(&self) {
        self.state.lock().unwrap().revision += 1;
    }
    pub fn fail_item(&self, id: &str) {
        self.state.lock().unwrap().fail.insert(id.into());
    }
    pub fn deleted(&self) -> Vec<String> {
        self.state.lock().unwrap().deleted.clone()
    }
    pub fn set_space(&self, space: DiskSpace) {
        self.state.lock().unwrap().space = Some(space);
    }
    pub fn step(&self) {
        let mut s = self.state.lock().unwrap();
        let job = s
            .jobs
            .iter()
            .filter(|(_, (_, p))| !p.finished)
            .map(|(id, _)| id.clone())
            .min();
        if let Some(id) = job {
            let item = {
                let (queue, _) = s.jobs.get_mut(&id).unwrap();
                if queue.is_empty() {
                    None
                } else {
                    Some(queue.remove(0))
                }
            };
            if let Some(item) = item {
                let failed = s.fail.contains(&item);
                if !failed {
                    s.items.retain(|i| i.id != item);
                    s.deleted.push(item.clone());
                    s.revision += 1;
                }
                let (queue, p) = s.jobs.get_mut(&id).unwrap();
                if failed {
                    p.failed.push((item, "Could not remove item".into()));
                } else {
                    p.done.push(item);
                }
                p.remaining = queue.len();
                p.finished = queue.is_empty();
            }
        }
    }
}
impl StorageService for FakeStorage {
    fn report(&self) -> Res<StorageReport> {
        let s = self.state.lock().unwrap();
        Ok(StorageReport {
            items: s.items.clone(),
            space: s.space,
        })
    }
    fn preview(&self, ids: &[String]) -> Res<CleanupPlan> {
        let mut s = self.state.lock().unwrap();
        let items: Vec<_> = s
            .items
            .iter()
            .filter(|i| i.reclaimable && ids.contains(&i.id))
            .cloned()
            .collect();
        s.next += 1;
        let plan = CleanupPlan {
            id: format!("plan-{}", s.next),
            version: s.revision,
            bytes: items.iter().filter(|i| i.measured).map(|i| i.bytes).sum(),
            items,
        };
        s.plans.insert(plan.id.clone(), plan.clone());
        Ok(plan)
    }
    fn clean(&self, id: &str, version: u64, confirmed: bool) -> Res<String> {
        let mut s = self.state.lock().unwrap();
        if !confirmed {
            return Err(Failure::new("unconfirmed", "Confirmation required"));
        }
        let plan = s
            .plans
            .get(id)
            .cloned()
            .ok_or_else(|| Failure::new("not_found", "Plan not found"))?;
        if version != plan.version || version != s.revision {
            return Err(Failure::new("stale", "Storage changed"));
        }
        s.next += 1;
        let job = format!("job-{}", s.next);
        let queue: Vec<_> = plan.items.iter().map(|i| i.id.clone()).collect();
        let p = CleanupProgress {
            remaining: queue.len(),
            finished: queue.is_empty(),
            ..Default::default()
        };
        s.jobs.insert(job.clone(), (queue, p));
        Ok(job)
    }
    fn progress(&self, id: &str) -> Res<CleanupProgress> {
        self.state
            .lock()
            .unwrap()
            .jobs
            .get(id)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| Failure::new("not_found", "Job not found"))
    }
    fn space(&self) -> Option<DiskSpace> {
        self.state.lock().unwrap().space
    }
}
