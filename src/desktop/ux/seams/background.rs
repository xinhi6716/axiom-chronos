use crate::base::{Failure, Res};
use std::sync::Mutex;
#[derive(Clone, Debug, PartialEq)]
pub enum KeepAlive {
    Available,
    Unavailable { reason: String },
}
pub trait BackgroundService: Send + Sync + 'static {
    fn status(&self) -> KeepAlive;
    fn detach(&self, sessions: &[String]) -> Res<()>;
    fn reattach(&self) -> Res<Vec<String>>;
    fn quit(&self, _stop_running: bool) -> Res<()> {
        Err(Failure::new("unsupported", "Quitting is not supported"))
    }
}
pub struct FakeBackground {
    available: bool,
    sessions: Mutex<Vec<String>>,
}
impl FakeBackground {
    pub fn new(available: bool) -> Self {
        Self {
            available,
            sessions: Mutex::new(vec![]),
        }
    }
    pub fn detached(&self) -> Vec<String> {
        self.sessions.lock().unwrap().clone()
    }
}
impl BackgroundService for FakeBackground {
    fn status(&self) -> KeepAlive {
        if self.available {
            KeepAlive::Available
        } else {
            KeepAlive::Unavailable {
                reason: "Background service unavailable".into(),
            }
        }
    }
    fn detach(&self, sessions: &[String]) -> Res<()> {
        if !self.available {
            return Err(Failure::new(
                "unavailable",
                "Background service unavailable",
            ));
        }
        *self.sessions.lock().unwrap() = sessions.to_vec();
        Ok(())
    }
    fn reattach(&self) -> Res<Vec<String>> {
        if !self.available {
            return Err(Failure::new(
                "unavailable",
                "Background service unavailable",
            ));
        }
        Ok(std::mem::take(&mut *self.sessions.lock().unwrap()))
    }
}
