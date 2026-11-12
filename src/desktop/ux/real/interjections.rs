use crate::base::{self, Failure, Res};
use crate::desktop::ux::seams::{
    Amendment, Draft, Interjections, Receipt, ReceiptStage, RunningWork,
};
use crate::orch::events::{self, Event};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

pub trait InterjectTransport: Send + Sync {
    fn interject(&self, session: &str, text: &str) -> Res<()>;
    fn pause(&self, session: &str) -> Res<()>;
}

pub struct ServiceTransport {
    home: PathBuf,
}
impl ServiceTransport {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }
}
impl InterjectTransport for ServiceTransport {
    fn interject(&self, session: &str, text: &str) -> Res<()> {
        let client = crate::service::Client::connect(&self.home)?;
        let key = client
            .snapshot()?
            .wishes
            .into_iter()
            .find(|w| w.session.as_deref() == Some(session))
            .map(|w| w.key)
            .ok_or_else(|| {
                Failure::new("not_found", format!("no hosted wish for session {session}"))
            })?;
        client.interject(&key, text)
    }
    fn pause(&self, _: &str) -> Res<()> {
        crate::service::Client::connect(&self.home)?
            .pause_all()
            .map(|_| ())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    client_id: String,
    digest: String,
    text: String,
    sent_ms: i64,
    replan: bool,
    #[serde(default)]
    ij: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct SessionFile {
    session: String,
    entries: Vec<Entry>,
}

pub struct EngineInterjections {
    home: PathBuf,
    client: Arc<dyn InterjectTransport>,
    gate: Mutex<()>,
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}
fn filename(session: &str) -> String {
    let safe: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Avoid collisions from sanitization and case-insensitive filesystems.
    if safe != session || session.chars().any(|c| c.is_ascii_uppercase()) || safe.is_empty() {
        format!("{safe}-{}", digest(session))
    } else {
        safe
    }
}
fn event_state(events: &[Event], id: &str, sent_ms: i64) -> (ReceiptStage, i64, Vec<String>) {
    let mut result = (ReceiptStage::Sent, sent_ms, vec![]);
    for event in events.iter().filter(|e| e.data["id"].as_str() == Some(id)) {
        match event.kind.as_str() {
            "interjection_received" => result = (ReceiptStage::Received, event.ts_ms, vec![]),
            "interjection_applied" => {
                let units: Vec<String> = event.data["links"]["units"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                result = (
                    ReceiptStage::Applied {
                        first_action: units.first().cloned().unwrap_or_else(|| "replan".into()),
                    },
                    event.ts_ms,
                    units,
                );
            }
            "interjection_declined" => {
                result = (
                    ReceiptStage::Rejected {
                        reason: event.data["reason"].as_str().unwrap_or_default().into(),
                    },
                    event.ts_ms,
                    vec![],
                )
            }
            _ => {}
        }
    }
    result
}
impl EngineInterjections {
    pub fn new(home: PathBuf, client: Arc<dyn InterjectTransport>) -> Self {
        Self {
            home,
            client,
            gate: Mutex::new(()),
        }
    }
    fn dir(&self) -> PathBuf {
        self.home.join("desktop").join("interjections")
    }
    fn path(&self, session: &str) -> PathBuf {
        self.dir().join(format!("{}.json", filename(session)))
    }
    fn load(&self, session: &str) -> Res<SessionFile> {
        let bytes = match std::fs::read(self.path(session)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SessionFile {
                    session: session.into(),
                    entries: vec![],
                })
            }
            Err(e) => return Err(io(e)),
        };
        let file: SessionFile = serde_json::from_slice(&bytes).map_err(io)?;
        if file.session != session {
            return Err(Failure::new("invalid_argument", "session file collision"));
        }
        Ok(file)
    }
    fn save(&self, file: &SessionFile) -> Res<()> {
        std::fs::create_dir_all(self.dir()).map_err(io)?;
        base::atomic_write(
            &self.path(&file.session),
            &serde_json::to_vec(file).map_err(io)?,
        )
        .map_err(io)
    }
    fn events(&self, session: &str) -> Vec<Event> {
        super::session_store(&self.home, session)
            .and_then(|s| events::read_events(&s, 0))
            .unwrap_or_default()
    }
    fn reconcile(&self, file: &mut SessionFile, events: &[Event]) -> Res<()> {
        let mut claimed: HashSet<String> =
            file.entries.iter().filter_map(|e| e.ij.clone()).collect();
        let mut changed = false;
        for entry in &mut file.entries {
            if entry.ij.is_some() {
                continue;
            }
            if let Some(id) = events
                .iter()
                .filter(|e| {
                    e.kind == "interjection_received"
                        && e.data["digest"].as_str() == Some(&entry.digest)
                })
                .filter_map(|e| e.data["id"].as_str())
                .find(|id| !claimed.contains(*id))
            {
                entry.ij = Some(id.to_owned());
                claimed.insert(id.to_owned());
                changed = true;
            }
        }
        if changed {
            self.save(file)?;
        }
        Ok(())
    }
    fn receipt_for(session: &str, entry: &Entry, events: &[Event]) -> Receipt {
        let (stage, at_ms, _) = entry
            .ij
            .as_deref()
            .map(|id| event_state(events, id, entry.sent_ms))
            .unwrap_or((ReceiptStage::Sent, entry.sent_ms, vec![]));
        Receipt {
            client_id: entry.client_id.clone(),
            session: session.into(),
            stage,
            at_ms,
        }
    }
    fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(self.dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        files
    }
}
impl Interjections for EngineInterjections {
    fn send(&self, d: &Draft) -> Res<Receipt> {
        let _gate = self.gate.lock().unwrap();
        if d.pause_first {
            self.client.pause(&d.session)?;
        }
        // The store lock serializes separate adapters as well as threads.
        std::fs::create_dir_all(self.dir()).map_err(io)?;
        let _lock = crate::orch::lock_store(&self.dir())?;
        let mut file = self.load(&d.session)?;
        if file.entries.iter().any(|e| e.client_id == d.client_id) {
            let events = self.events(&d.session);
            self.reconcile(&mut file, &events)?;
            return Ok(Self::receipt_for(
                &d.session,
                file.entries
                    .iter()
                    .find(|e| e.client_id == d.client_id)
                    .unwrap(),
                &events,
            ));
        }
        self.client.interject(&d.session, &d.text)?;
        let entry = Entry {
            client_id: d.client_id.clone(),
            digest: digest(&d.text),
            text: d.text.clone(),
            sent_ms: crate::desktop::ux::wall_ms(),
            replan: d.replan,
            ij: None,
        };
        let receipt = Self::receipt_for(&d.session, &entry, &[]);
        file.entries.push(entry);
        self.save(&file)?;
        Ok(receipt)
    }
    fn receipt(&self, client_id: &str) -> Option<Receipt> {
        let _gate = self.gate.lock().unwrap();
        let _lock = crate::orch::lock_store(&self.dir()).ok()?;
        for path in self.files() {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let Ok(mut file) = serde_json::from_slice::<SessionFile>(&bytes) else {
                continue;
            };
            if !file.entries.iter().any(|e| e.client_id == client_id) {
                continue;
            }
            let events = self.events(&file.session);
            self.reconcile(&mut file, &events).ok()?;
            return file
                .entries
                .iter()
                .find(|e| e.client_id == client_id)
                .map(|e| Self::receipt_for(&file.session, e, &events));
        }
        None
    }
    fn history(&self, session: &str) -> Vec<Amendment> {
        let _gate = self.gate.lock().unwrap();
        if std::fs::create_dir_all(self.dir()).is_err() {
            return vec![];
        }
        let Ok(_lock) = crate::orch::lock_store(&self.dir()) else {
            return vec![];
        };
        let Ok(mut file) = self.load(session) else {
            return vec![];
        };
        let events = self.events(session);
        if self.reconcile(&mut file, &events).is_err() {
            return vec![];
        }
        let claimed: HashSet<_> = file
            .entries
            .iter()
            .filter_map(|e| e.ij.as_deref())
            .collect();
        let mut result: Vec<_> = file
            .entries
            .iter()
            .map(|entry| {
                let (stage, at, affected) = entry
                    .ij
                    .as_deref()
                    .map(|id| event_state(&events, id, entry.sent_ms))
                    .unwrap_or((ReceiptStage::Sent, entry.sent_ms, vec![]));
                Amendment {
                    client_id: Some(entry.client_id.clone()),
                    text: entry.text.clone(),
                    at_ms: Some(at),
                    stage: Some(stage),
                    affected,
                    replan: entry.replan,
                }
            })
            .collect();
        let goal = crate::goal::load(&self.home, session).ok().flatten();
        let mut used = HashSet::new();
        for event in events.iter().filter(|e| e.kind == "interjection_received") {
            let Some(id) = event.data["id"].as_str() else {
                continue;
            };
            let text = goal
                .as_ref()
                .and_then(|g| {
                    g.amendments
                        .iter()
                        .enumerate()
                        .find(|(i, text)| {
                            !used.contains(i)
                                && Some(digest(text).as_str()) == event.data["digest"].as_str()
                        })
                        .map(|(i, text)| {
                            used.insert(i);
                            text.clone()
                        })
                })
                .unwrap_or_default();
            if claimed.contains(id) {
                continue;
            }
            let (stage, at, affected) = event_state(&events, id, event.ts_ms);
            result.push(Amendment {
                client_id: None,
                text,
                at_ms: Some(at),
                stage: Some(stage),
                affected,
                replan: false,
            });
        }
        result
    }
    fn running_work(&self, session: &str) -> RunningWork {
        let goal = crate::goal::load(&self.home, session).ok().flatten();
        let units = super::session_store(&self.home, session)
            .and_then(|s| crate::orch::read_units(&s))
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.state != "verified" && u.superseded_by().is_none())
            .map(|u| u.id)
            .collect();
        RunningWork {
            stage: goal.as_ref().map(|g| g.status.clone()),
            units,
            batch: goal.and_then(|g| g.batch.map(|b| b.progress_file)),
        }
    }
    fn pause_batch(&self, session: &str) -> Res<()> {
        self.client.pause(session)
    }
}
