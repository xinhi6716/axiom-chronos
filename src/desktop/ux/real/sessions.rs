use crate::{
    base::{Failure, Res},
    desktop::ux::seams::{
        Connection, Controller, Frame, SessionKind, SessionStream, WorkerSession,
    },
    service::{client::Client, Snapshot, WireEvent},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const REASON: &str = "The service protocol does not support taking control or sending input.";
pub(super) struct Entry {
    session: WorkerSession,
    pub(super) frames: Vec<Frame>,
    closed: bool,
    log: Vec<String>,
    gap: Option<u64>,
}
#[derive(Default)]
pub(super) struct State {
    pub(super) entries: HashMap<String, Entry>,
    instance: Option<String>,
    cursor: u64,
    #[cfg(test)]
    pub(super) snapshot_count: usize,
    max_seq: u64,
    connection: Option<Connection>,
}
impl State {
    #[cfg(test)]
    pub(super) fn force_cursor(&mut self, cursor: u64) {
        self.cursor = cursor;
    }
    pub(super) fn frames(&self, id: &str, after: u64) -> Res<Vec<Frame>> {
        let entry = self.entries.get(id)
            .ok_or_else(|| Failure::new("not_found", "Session not found"))?;
        if entry.frames.last().is_some_and(|frame| after < frame.seq)
            && entry.gap.is_some_and(|boundary| after <= boundary)
        {
            return Err(Failure::new("gap", "The service log was truncated; some session frames may be missing."));
        }
        Ok(entry.frames.iter().filter(|f| f.seq > after).cloned().collect())
    }
    pub(super) fn snapshot(&mut self, snapshot: &Snapshot) {
        #[cfg(test)]
        { self.snapshot_count += 1; }
        let previous_cursor = self.cursor;
        let changed = self
            .instance
            .as_deref()
            .is_some_and(|i| i != snapshot.instance);
        self.instance = Some(snapshot.instance.clone());
        self.cursor = snapshot.cursor;
        let mut old = std::mem::take(&mut self.entries);
        for wish in &snapshot.wishes {
            let Some(id) = &wish.session else { continue };
            let previous = old.remove(id);
            let (mut frames, previous_log, mut gap) =
                previous.map(|e| (e.frames, e.log, e.gap)).unwrap_or_default();
            let log: Vec<String> = wish
                .log
                .iter()
                .filter(|r| r.source == "assistant")
                .map(|r| r.text.clone())
                .collect();
            // Logs are bounded by the service; retain their largest overlapping suffix.
            let overlap = if changed {
                0
            } else {
                (0..=previous_log.len().min(log.len()))
                    .rev()
                    .find(|&n| previous_log[previous_log.len() - n..] == log[..n])
                    .unwrap_or(0)
            };
            // The cursor is global, so even a large advance can consist of
            // unrelated events. A repeating log can have multiple plausible
            // overlaps, however, and a capacity-sized advance could have
            // replaced it entirely.
            if !changed && !previous_log.is_empty() && wish.log.len() == 2000
                && snapshot.cursor > previous_cursor
                && (overlap == 0
                    || (snapshot.cursor - previous_cursor >= wish.log.len() as u64
                        && (1..overlap).any(|n| previous_log[previous_log.len() - n..] == log[..n])))
            {
                let boundary = frames.last().map_or(0, |frame| frame.seq);
                gap = Some(gap.map_or(boundary, |old| old.max(boundary)));
            }
            for text in &log[overlap..] {
                self.max_seq += 1;
                frames.push(Frame {
                    seq: self.max_seq,
                    text: text.clone(),
                });
            }
            self.entries.insert(
                id.clone(),
                Entry {
                    session: WorkerSession {
                        id: id.clone(),
                        run: wish.handle.clone(),
                        unit: None,
                        title: wish.text.clone(),
                        kind: SessionKind::Terminal,
                    },
                    frames,
                    closed: wish.state == "ended"
                        || (wish.ended_at.is_some() && wish.state != "waiting_for_space"),
                    log,
                    gap,
                },
            );
        }
    }
    pub(super) fn event(&mut self, event: &WireEvent) -> bool {
        if self.instance.as_deref() != Some(&event.instance) {
            return false;
        }
        if event.seq <= self.cursor {
            return true;
        }
        if event.seq != self.cursor + 1 {
            return false;
        }
        self.cursor = event.seq;
        if let Some(entry) = event
            .session
            .as_ref()
            .and_then(|id| self.entries.get_mut(id))
        {
            if event.kind == "text" {
                if let Some(text) = event.data["text"].as_str() {
                    self.max_seq += 1;
                    entry.frames.push(Frame {
                        seq: self.max_seq,
                        text: text.into(),
                    });
                    entry.log.push(text.into());
                    if entry.log.len() > 2000 {
                        entry.log.remove(0);
                    }
                }
            }
        }
        true
    }
}
enum Command {
    Reconnect,
    Stop,
}
pub struct ServiceSessions {
    home: PathBuf,
    pub(super) state: Arc<Mutex<State>>,
    worker: Mutex<Option<(mpsc::Sender<Command>, JoinHandle<()>)>>,
}
impl ServiceSessions {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            state: Arc::new(Mutex::new(State::default())),
            worker: Mutex::new(None),
        }
    }
    fn start(&self) {
        let mut worker = self.worker.lock().unwrap();
        if worker.is_none() {
            let (tx, rx) = mpsc::channel();
            let home = self.home.clone();
            let state = self.state.clone();
            *worker = Some((tx, thread::spawn(move || run(home, state, rx))));
        }
    }
}
fn run(home: PathBuf, state: Arc<Mutex<State>>, commands: mpsc::Receiver<Command>) {
    loop {
        state.lock().unwrap().connection = Some(Connection::Connecting);
        let connected = Client::connect(&home)
            .and_then(|client| client.snapshot().map(|snapshot| (client, snapshot)));
        if let Ok((client, snapshot)) = connected {
            let after = {
                let mut s = state.lock().unwrap();
                s.snapshot(&snapshot);
                s.cursor
            };
            let mut after = after;
            let subscription = loop {
                match client.subscribe(after) {
                    Ok(sub) => break Some(sub),
                    Err(e) if e.kind == "gap" => match client.snapshot() {
                        Ok(snapshot) => {
                            let mut s = state.lock().unwrap();
                            s.snapshot(&snapshot);
                            s.cursor = snapshot.cursor;
                            after = s.cursor;
                        }
                        Err(_) => break None,
                    },
                    Err(_) => break None,
                }
            };
            if let Some(subscription) = subscription {
                state.lock().unwrap().connection = Some(Connection::Live);
                loop {
                    match commands.try_recv() {
                        Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => return,
                        Ok(Command::Reconnect) => break,
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    match subscription.recv_timeout(Duration::from_millis(100)) {
                        Ok(Some(event)) => {
                            let refresh = {
                                let s = state.lock().unwrap();
                                event.kind != "text"
                                    || event
                                        .session
                                        .as_ref()
                                        .is_some_and(|id| !s.entries.contains_key(id))
                                    || s.instance.as_deref() != Some(&event.instance)
                            };
                            if refresh || !state.lock().unwrap().event(&event) {
                                if let Ok(snapshot) = client.snapshot() {
                                    state.lock().unwrap().snapshot(&snapshot);
                                }
                                // Re-subscribe at the snapshot cursor: it already includes
                                // everything through that point, including events in flight.
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
            }
        }
        state.lock().unwrap().connection = Some(Connection::Lost);
        match commands.recv_timeout(Duration::from_millis(500)) {
            Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Ok(Command::Reconnect) | Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}
impl Drop for ServiceSessions {
    fn drop(&mut self) {
        if let Some((tx, worker)) = self.worker.get_mut().unwrap().take() {
            let _ = tx.send(Command::Stop);
            let _ = worker.join();
        }
    }
}
fn unsupported() -> Failure {
    Failure::new("unsupported", REASON)
}
impl SessionStream for ServiceSessions {
    fn sessions(&self) -> Vec<WorkerSession> {
        self.start();
        let mut v: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|e| e.session.clone())
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }
    fn frames(&self, id: &str, after: u64) -> Res<Vec<Frame>> {
        self.start();
        self.state.lock().unwrap().frames(id, after)
    }
    fn connection(&self, id: &str) -> Connection {
        self.start();
        let s = self.state.lock().unwrap();
        match s.entries.get(id) {
            Some(e) if e.closed => Connection::Closed,
            Some(_) => s.connection.unwrap_or(Connection::Connecting),
            None if s.instance.is_none() => s.connection.unwrap_or(Connection::Connecting),
            None => Connection::Closed,
        }
    }
    fn controller(&self, _: &str) -> Controller {
        Controller::Refused {
            reason: REASON.into(),
        }
    }
    fn request_takeover(&self, _: &str) -> Res<String> {
        Err(unsupported())
    }
    fn send_input(&self, _: &str, _: &str, _: &str) -> Res<()> {
        Err(unsupported())
    }
    fn hand_back(&self, _: &str, _: &str) -> Res<()> {
        Err(unsupported())
    }
    fn reconnect(&self, id: &str) -> Res<()> {
        self.start();
        if !self.state.lock().unwrap().entries.contains_key(id) {
            return Err(Failure::new("not_found", "Session not found"));
        }
        if let Some((tx, _)) = self.worker.lock().unwrap().as_ref() {
            tx.send(Command::Reconnect)
                .map_err(|_| Failure::new("service_unavailable", "Subscription stopped"))?;
        }
        Ok(())
    }
}
