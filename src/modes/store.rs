use super::{runs, Message, Snapshot};
use crate::base::{self, Failure, Res};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

fn error(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

// Canonical paths make independently opened handles share the same mutex.
fn mutex_for(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn file_lock(path: &Path) -> Res<fs::File> {
    // The OS releases the advisory lock on process exit, including crashes.
    // Keep the lock file itself in place so contenders always lock the same file.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(error)?;
    file.lock().map_err(error)?;
    Ok(file)
}
#[derive(Debug)]
pub struct Transcript {
    session: String,
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}
impl Transcript {
    pub fn open(home: &Path, session: &str) -> Res<Self> {
        base::require_slug("session", session)?;
        runs::load_session(home, session)?;
        let dir =
            fs::canonicalize(home.join("modes").join("sessions").join(session)).map_err(error)?;
        let path = dir.join("transcript.jsonl");
        Ok(Self {
            session: session.into(),
            lock: mutex_for(&path),
            path,
        })
    }
    fn locked<T>(&self, f: impl FnOnce() -> Res<T>) -> Res<T> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = file_lock(&self.path.with_file_name("transcript.lock"))?;
        f()
    }
    fn bytes(&self) -> Res<Vec<u8>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(error(e)),
        }
    }
    fn parse(bytes: &[u8]) -> Vec<Message> {
        // A damaged tail becomes an interior line after the next append.
        bytes
            .split(|b| *b == b'\n')
            .filter_map(|line| serde_json::from_slice(line).ok())
            .collect()
    }
    pub fn append(
        &self,
        author: &str,
        kind: &str,
        text: &str,
        thread: Option<&str>,
        data: serde_json::Value,
    ) -> Res<Message> {
        self.locked(|| {
            let bytes = self.bytes()?;
            let seq = Self::parse(&bytes)
                .last()
                .map_or(0, |m| m.seq)
                .checked_add(1)
                .ok_or_else(|| Failure::new("io", "transcript sequence exhausted"))?;
            let message = Message {
                seq,
                ts: base::now_iso(),
                author: author.into(),
                kind: kind.into(),
                text: text.into(),
                thread: thread.map(str::to_owned),
                data,
            };
            let mut record = Vec::new();
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                record.push(b'\n');
            }
            serde_json::to_writer(&mut record, &message).map_err(error)?;
            record.push(b'\n');
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .map_err(error)?;
            file.write_all(&record).map_err(error)?;
            Ok(message)
        })
    }
    pub fn read(&self, after_seq: u64, limit: usize) -> Res<Vec<Message>> {
        self.locked(|| {
            Ok(Self::parse(&self.bytes()?)
                .into_iter()
                .filter(|m| m.seq > after_seq)
                .take(limit)
                .collect())
        })
    }
    pub fn get(&self, seq: u64) -> Res<Option<Message>> {
        self.locked(|| {
            Ok(Self::parse(&self.bytes()?)
                .into_iter()
                .find(|m| m.seq == seq))
        })
    }
    pub fn last_seq(&self) -> Res<u64> {
        self.locked(|| Ok(Self::parse(&self.bytes()?).last().map_or(0, |m| m.seq)))
    }
    pub fn thread(&self, thread: &str) -> Res<Vec<Message>> {
        self.locked(|| {
            Ok(Self::parse(&self.bytes()?)
                .into_iter()
                .filter(|m| m.thread.as_deref() == Some(thread))
                .collect())
        })
    }
    pub fn snapshot(&self, thread: Option<&str>) -> Res<Snapshot> {
        self.locked(|| {
            let mut messages = Self::parse(&self.bytes()?);
            let upto_seq = messages.last().map_or(0, |m| m.seq);
            if let Some(thread) = thread {
                messages.retain(|m| m.thread.as_deref() == Some(thread));
            }
            Ok(Snapshot {
                session: self.session.clone(),
                upto_seq,
                thread: thread.map(str::to_owned),
                messages,
            })
        })
    }
}
