use crate::base::{Failure, Res};
use std::{collections::HashMap, sync::Mutex};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Terminal,
    Browser,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkerSession {
    pub id: String,
    pub run: String,
    pub unit: Option<String>,
    pub title: String,
    pub kind: SessionKind,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub seq: u64,
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connecting,
    Live,
    Lost,
    Closed,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Controller {
    Worker,
    Requested { ticket: String },
    You { ticket: String },
    Refused { reason: String },
}
pub trait SessionStream: Send + Sync + 'static {
    fn sessions(&self) -> Vec<WorkerSession>;
    fn frames(&self, id: &str, after: u64) -> Res<Vec<Frame>>;
    fn connection(&self, id: &str) -> Connection;
    fn controller(&self, id: &str) -> Controller;
    fn request_takeover(&self, id: &str) -> Res<String>;
    fn send_input(&self, id: &str, ticket: &str, text: &str) -> Res<()>;
    fn hand_back(&self, id: &str, ticket: &str) -> Res<()>;
    fn reconnect(&self, id: &str) -> Res<()>;
}
struct Stream {
    session: WorkerSession,
    frames: Vec<Frame>,
    connection: Connection,
    controller: Controller,
    inputs: Vec<String>,
}
#[derive(Default)]
struct State {
    streams: HashMap<String, Stream>,
    next: u64,
}
#[derive(Default)]
pub struct FakeSessions {
    state: Mutex<State>,
}
impl FakeSessions {
    pub fn new(sessions: Vec<WorkerSession>) -> Self {
        let streams = sessions
            .into_iter()
            .map(|session| {
                (
                    session.id.clone(),
                    Stream {
                        session,
                        frames: vec![],
                        connection: Connection::Live,
                        controller: Controller::Worker,
                        inputs: vec![],
                    },
                )
            })
            .collect();
        Self {
            state: Mutex::new(State { streams, next: 0 }),
        }
    }
    pub fn confirm(&self, id: &str) {
        if let Some(s) = self.state.lock().unwrap().streams.get_mut(id) {
            if let Controller::Requested { ticket } = &s.controller {
                s.controller = Controller::You {
                    ticket: ticket.clone(),
                };
            }
        }
    }
    pub fn refuse(&self, id: &str, reason: &str) {
        if let Some(s) = self.state.lock().unwrap().streams.get_mut(id) {
            s.controller = Controller::Refused {
                reason: reason.into(),
            };
        }
    }
    pub fn disconnect(&self, id: &str) {
        if let Some(s) = self.state.lock().unwrap().streams.get_mut(id) {
            s.connection = Connection::Lost;
        }
    }
    pub fn push(&self, id: &str, text: &str) {
        if let Some(s) = self.state.lock().unwrap().streams.get_mut(id) {
            let seq = s.frames.last().map_or(1, |f| f.seq + 1);
            s.frames.push(Frame {
                seq,
                text: text.into(),
            });
        }
    }
    pub fn inputs(&self, id: &str) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .streams
            .get(id)
            .map(|s| s.inputs.clone())
            .unwrap_or_default()
    }
}
impl SessionStream for FakeSessions {
    fn sessions(&self) -> Vec<WorkerSession> {
        let mut v: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .streams
            .values()
            .map(|s| s.session.clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }
    fn frames(&self, id: &str, after: u64) -> Res<Vec<Frame>> {
        self.state
            .lock()
            .unwrap()
            .streams
            .get(id)
            .map(|s| s.frames.iter().filter(|f| f.seq > after).cloned().collect())
            .ok_or_else(|| Failure::new("not_found", "Session not found"))
    }
    fn connection(&self, id: &str) -> Connection {
        self.state
            .lock()
            .unwrap()
            .streams
            .get(id)
            .map(|s| s.connection)
            .unwrap_or(Connection::Closed)
    }
    fn controller(&self, id: &str) -> Controller {
        self.state
            .lock()
            .unwrap()
            .streams
            .get(id)
            .map(|s| s.controller.clone())
            .unwrap_or(Controller::Worker)
    }
    fn request_takeover(&self, id: &str) -> Res<String> {
        let mut s = self.state.lock().unwrap();
        let stream = s
            .streams
            .get(id)
            .ok_or_else(|| Failure::new("not_found", "Session not found"))?;
        if matches!(
            stream.controller,
            Controller::Requested { .. } | Controller::You { .. }
        ) {
            return Err(Failure::new("busy", "Control already requested"));
        }
        s.next += 1;
        let ticket = format!("ticket-{}", s.next);
        s.streams.get_mut(id).unwrap().controller = Controller::Requested {
            ticket: ticket.clone(),
        };
        Ok(ticket)
    }
    fn send_input(&self, id: &str, ticket: &str, text: &str) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        let stream = s
            .streams
            .get_mut(id)
            .ok_or_else(|| Failure::new("not_found", "Session not found"))?;
        if stream.controller
            != (Controller::You {
                ticket: ticket.into(),
            })
        {
            return Err(Failure::new("not_controller", "Control not granted"));
        }
        stream.inputs.push(text.into());
        Ok(())
    }
    fn hand_back(&self, id: &str, ticket: &str) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        let stream = s
            .streams
            .get_mut(id)
            .ok_or_else(|| Failure::new("not_found", "Session not found"))?;
        if stream.controller
            != (Controller::You {
                ticket: ticket.into(),
            })
        {
            return Err(Failure::new("not_controller", "Control not granted"));
        }
        stream.controller = Controller::Worker;
        Ok(())
    }
    fn reconnect(&self, id: &str) -> Res<()> {
        let mut s = self.state.lock().unwrap();
        let stream = s
            .streams
            .get_mut(id)
            .ok_or_else(|| Failure::new("not_found", "Session not found"))?;
        stream.connection = Connection::Live;
        Ok(())
    }
}
