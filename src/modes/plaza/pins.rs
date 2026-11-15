//! Stored plaza conclusions and thread promotion.
use crate::{
    base::{self, Failure, Res},
    modes::{
        launch::{LaunchKind, Launcher},
        plaza::Room,
        promotion::{self, Promotion},
        store::Transcript,
        Source,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PinKind {
    Decision,
    Memory,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pin {
    pub id: String,
    pub session: String,
    pub thread: Option<String>,
    pub seq: u64,
    pub kind: PinKind,
    pub text: String,
    pub author: String,
    pub pinned_by: String,
    pub sources: Vec<Source>,
    pub ts: String,
}

pub trait ConclusionSink {
    fn accept(&self, pin: &Pin) -> Res<()>;
}

static PINS_LOCK: Mutex<()> = Mutex::new(());

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

fn path(home: &Path, session: &str) -> Res<PathBuf> {
    base::require_slug("session", session)?;
    Ok(home
        .join("modes")
        .join("sessions")
        .join(session)
        .join("pins.jsonl"))
}

fn read(path: &Path) -> Res<Vec<Pin>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(error(e)),
    };
    bytes
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).map_err(error))
        .collect()
}

pub fn pins(home: &Path, session: &str) -> Res<Vec<Pin>> {
    let path = path(home, session)?;
    let _guard = PINS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read(&path)
}

pub fn pin(
    home: &Path,
    session: &str,
    seq: u64,
    kind: PinKind,
    pinned_by: &str,
    sink: &dyn ConclusionSink,
) -> Res<Pin> {
    let path = path(home, session)?;
    let _guard = PINS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let transcript = Transcript::open(home, session)?;
    let message = transcript
        .get(seq)?
        .ok_or_else(|| Failure::new("not_found", "message not found"))?;
    if let Some(existing) = read(&path)?
        .into_iter()
        .find(|pin| pin.seq == seq && pin.kind == kind && pin.session == session)
    {
        return Ok(existing);
    }
    let mut sources = vec![Source {
        session: session.into(),
        seq,
        author: message.author.clone(),
    }];
    if let Some(entries) = message
        .data
        .get("sources")
        .and_then(|value| value.as_array())
    {
        for entry in entries {
            if let Ok(source) = serde_json::from_value::<Source>(entry.clone()) {
                if !sources.contains(&source) {
                    sources.push(source);
                }
            }
        }
    }
    let pin = Pin {
        id: base::unique(),
        session: session.into(),
        thread: message.thread.clone(),
        seq,
        kind,
        text: message.text,
        author: message.author,
        pinned_by: pinned_by.into(),
        sources,
        ts: base::now_iso(),
    };
    sink.accept(&pin)?;
    let mut record = serde_json::to_vec(&pin).map_err(error)?;
    record.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(error)?
        .write_all(&record)
        .map_err(error)?;
    transcript.append(
        pinned_by,
        "pin",
        &pin.text,
        pin.thread.as_deref(),
        json!({"pin": pin.id, "seq": seq, "kind": kind}),
    )?;
    Ok(pin)
}

pub fn promote_thread(
    home: &Path,
    room: &Room,
    thread: &str,
    kind: LaunchKind,
    goal: &str,
    verify: Option<&str>,
    launcher: &dyn Launcher,
) -> Res<Promotion> {
    base::require_slug("thread", thread)?;
    let snapshot = Transcript::open(home, &room.session.id)?.snapshot(Some(thread))?;
    promotion::promote(
        home,
        &room.session,
        &room.run,
        kind,
        goal,
        verify,
        snapshot,
        launcher,
    )
}
