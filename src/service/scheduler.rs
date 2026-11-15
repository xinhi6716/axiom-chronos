use super::{
    launcher::{ServiceLauncher, StartPath},
    Client, StartRequest,
};
use crate::{
    base::Res,
    routines::{self, store::Store, WishLauncher},
    seams::{self, ApprovalRequest, Decision, PersonApproval},
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

pub struct SchedulerConfig {
    pub clock: Arc<dyn seams::Clock>,
    pub launcher: Option<Arc<dyn WishLauncher>>,
    pub approval: Arc<dyn PersonApproval>,
    pub interval: Duration,
}
impl SchedulerConfig {
    pub fn production(home: &Path) -> Self {
        Self {
            clock: Arc::new(seams::SystemClock),
            launcher: None,
            approval: Arc::new(EnabledRoutineApproval::new(home)),
            interval: Duration::from_secs(60),
        }
    }
}

pub struct EnabledRoutineApproval {
    home: PathBuf,
}
impl EnabledRoutineApproval {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }
}
impl PersonApproval for EnabledRoutineApproval {
    fn decide(&self, request: &ApprovalRequest) -> Decision {
        let allowed = request.action == "launch_wish"
            && request.subject.strip_prefix("routine:").is_some_and(|id| {
                Store::open(&self.home)
                    .and_then(|store| store.routines())
                    .is_ok_and(|routines| routines.iter().any(|r| r.id == id && r.enabled))
            });
        if allowed {
            Decision::Approved
        } else {
            Decision::Denied
        }
    }
}

struct ConnectAtLaunch {
    home: PathBuf,
}
impl StartPath for ConnectAtLaunch {
    fn start(&self, request: StartRequest) -> Res<String> {
        Client::connect(&self.home)?.start(request)
    }
}

pub(super) fn spawn(
    home: PathBuf,
    config: SchedulerConfig,
    stop: mpsc::Receiver<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let launcher = config.launcher.unwrap_or_else(|| {
            Arc::new(ServiceLauncher {
                home: home.clone(),
                start: Arc::new(ConnectAtLaunch { home: home.clone() }),
            })
        });
        loop {
            match stop.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let mut stopped = false;
            let result = Store::open(&home).and_then(|store| {
                routines::runner::tick_while(
                    &store,
                    &*config.clock,
                    &[],
                    &*launcher,
                    &*config.approval,
                    || {
                        stopped = !matches!(stop.try_recv(), Err(mpsc::TryRecvError::Empty));
                        !stopped
                    },
                )
            });
            if stopped {
                break;
            }
            if let Err(error) = result {
                let _ = std::fs::write(home.join("service/scheduler-error.txt"), error.to_string());
            }
            match stop.recv_timeout(config.interval) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => break,
            }
        }
    })
}
