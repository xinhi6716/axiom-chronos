use crate::base::{self, Failure, Res};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Worktree,
    BuildCache,
    UnitStore,
    Scratch,
    EngineCopy,
    Logs,
    Evidence,
    ResearchCache,
    Download,
}
impl Class {
    pub const ALL: [Class; 9] = [
        Self::Worktree,
        Self::BuildCache,
        Self::UnitStore,
        Self::Scratch,
        Self::EngineCopy,
        Self::Logs,
        Self::Evidence,
        Self::ResearchCache,
        Self::Download,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::BuildCache => "build_cache",
            Self::UnitStore => "unit_store",
            Self::Scratch => "scratch",
            Self::EngineCopy => "engine_copy",
            Self::Logs => "logs",
            Self::Evidence => "evidence",
            Self::ResearchCache => "research_cache",
            Self::Download => "download",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub project: String,
    pub wish: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    Present,
    Deleted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: i64,
    pub path: PathBuf,
    pub class: Class,
    pub owner: Owner,
    pub created_ms: i64,
    pub last_used_ms: i64,
    pub size_bytes: Option<u64>,
    pub measured_ms: Option<i64>,
    pub state: EntryState,
    pub meta: Value,
}
pub struct Ledger {
    conn: Connection,
    db: PathBuf,
}
pub struct Lease {
    conn: Connection,
    id: i64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self
            .conn
            .execute("DELETE FROM ledger_leases WHERE id=?1", [self.id]);
    }
}
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("storage", e.to_string())
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
/// Canonicalize the existing prefix, preserving a possibly nonexistent suffix.
pub fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut clean = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::CurDir => (),
            Component::ParentDir => {
                clean.pop();
            }
            _ => clean.push(c.as_os_str()),
        }
    }
    let mut prefix = clean.as_path();
    let mut suffix = Vec::new();
    while !prefix.exists() {
        if let Some(n) = prefix.file_name() {
            suffix.push(n.to_os_string());
        } else {
            break;
        }
        match prefix.parent() {
            Some(p) => prefix = p,
            None => break,
        }
    }
    let mut result = fs::canonicalize(prefix).unwrap_or_else(|_| prefix.to_path_buf());
    for part in suffix.into_iter().rev() {
        result.push(part);
    }
    #[cfg(windows)]
    {
        let s = result.to_string_lossy();
        let s = if let Some(x) = s.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{x}")
        } else if let Some(x) = s.strip_prefix(r"\\?\") {
            x.to_string()
        } else {
            s.to_string()
        };
        result = PathBuf::from(s);
    }
    result
}
fn key(path: &Path) -> String {
    let s = normalize(path).to_string_lossy().into_owned();
    #[cfg(windows)]
    let s = s.to_lowercase();
    s
}
fn path_starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    {
        PathBuf::from(path.to_string_lossy().to_lowercase())
            .starts_with(PathBuf::from(base.to_string_lossy().to_lowercase()))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}
fn is_link(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}
fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
fn merge(old: &mut Value, new: Value) {
    if let (Value::Object(a), Value::Object(b)) = (&mut *old, &new) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    } else {
        *old = new;
    }
}
fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let class: String = row.get(2)?;
    let meta: String = row.get(10)?;
    let state: String = row.get(9)?;
    let invalid = |msg: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                msg.to_owned(),
            )),
        )
    };
    Ok(Entry {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        class: Class::parse(&class).ok_or_else(|| invalid("invalid class"))?,
        owner: Owner {
            project: row.get(3)?,
            wish: row.get(4)?,
        },
        created_ms: row.get(5)?,
        last_used_ms: row.get(6)?,
        size_bytes: row.get(7)?,
        measured_ms: row.get(8)?,
        state: match state.as_str() {
            "present" => EntryState::Present,
            "deleted" => EntryState::Deleted,
            _ => return Err(invalid("invalid state")),
        },
        meta: serde_json::from_str(&meta).map_err(|_| invalid("invalid metadata"))?,
    })
}
const COLUMNS: &str =
    "id,path,class,project,wish,created_ms,last_used_ms,size_bytes,measured_ms,state,meta";
impl Ledger {
    pub fn open(home: &Path) -> Res<Self> {
        fs::create_dir_all(super::dir(home)).map_err(err)?;
        let db = super::dir(home).join("ledger.sqlite3");
        let conn = Connection::open(&db).map_err(err)?;
        conn.busy_timeout(Duration::from_secs(10)).map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS ledger_entries (id INTEGER PRIMARY KEY, path TEXT NOT NULL, path_key TEXT NOT NULL UNIQUE, class TEXT NOT NULL, project TEXT NOT NULL, wish TEXT, created_ms INTEGER NOT NULL, last_used_ms INTEGER NOT NULL, size_bytes INTEGER, measured_ms INTEGER, state TEXT NOT NULL, meta TEXT NOT NULL); CREATE TABLE IF NOT EXISTS ledger_leases (id INTEGER PRIMARY KEY, path TEXT NOT NULL, pid INTEGER NOT NULL);").map_err(err)?;
        Ok(Self { conn, db })
    }
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
    fn transaction<T>(&self, f: impl FnOnce() -> Res<T>) -> Res<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
        match f() {
            Ok(v) => match self.conn.execute_batch("COMMIT") {
                Ok(()) => Ok(v),
                Err(e) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    Err(err(e))
                }
            },
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    fn get_key(&self, k: &str) -> Res<Option<Entry>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM ledger_entries WHERE path_key=?1"),
                [k],
                decode,
            )
            .optional()
            .map_err(err)
    }
    fn invalidate_ancestors(&self, path: &Path) -> Res<()> {
        for e in self.entries()? {
            if !same_path(&e.path, path) && path_starts_with(path, &e.path) {
                self.conn
                    .execute(
                        "UPDATE ledger_entries SET size_bytes=NULL, measured_ms=NULL WHERE id=?1",
                        [e.id],
                    )
                    .map_err(err)?;
            }
        }
        Ok(())
    }
    pub fn register(&self, path: &Path, class: Class, owner: &Owner, meta: Value) -> Res<Entry> {
        if owner.project != super::SYSTEM_PROJECT {
            base::require_slug("project", &owner.project)?;
        }
        let normalized = normalize(path);
        let k = key(&normalized);
        self.transaction(|| {
            let t = now();
            if let Some(mut existing) = self.get_key(&k)? {
                if existing.class != class || existing.owner != *owner { return Err(Failure::new("ownership_conflict", format!("ownership conflict: {}", path.display()))); }
                if existing.state == EntryState::Deleted { self.invalidate_ancestors(&normalized)?; self.conn.execute("UPDATE ledger_entries SET size_bytes=NULL, measured_ms=NULL WHERE id=?1", [existing.id]).map_err(err)?; }
                merge(&mut existing.meta, meta);
                self.conn.execute("UPDATE ledger_entries SET last_used_ms=?1, state='present', meta=?2 WHERE path_key=?3", params![t, existing.meta.to_string(), k]).map_err(err)?;
            } else { self.invalidate_ancestors(&normalized)?; self.conn.execute("INSERT INTO ledger_entries(path,path_key,class,project,wish,created_ms,last_used_ms,state,meta) VALUES(?1,?2,?3,?4,?5,?6,?6,'present',?7)", params![normalized.to_string_lossy(), k, class.as_str(), owner.project, owner.wish, t, meta.to_string()]).map_err(err)?; }
            self.get_key(&k)?.ok_or_else(|| err("entry disappeared"))
        })
    }
    pub fn touch(&self, path: &Path, now_ms: i64) -> Res<()> {
        self.conn
            .execute(
                "UPDATE ledger_entries SET last_used_ms=?1 WHERE path_key=?2 AND state='present'",
                params![now_ms, key(path)],
            )
            .map_err(err)?;
        Ok(())
    }
    pub fn get(&self, path: &Path) -> Res<Option<Entry>> {
        self.get_key(&key(path))
    }
    pub fn entries(&self) -> Res<Vec<Entry>> {
        let mut s = self
            .conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM ledger_entries WHERE state='present' ORDER BY path"
            ))
            .map_err(err)?;
        let rows = s.query_map([], decode).map_err(err)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(err)
    }
    pub fn owner_of(&self, path: &Path) -> Res<Option<Entry>> {
        let p = normalize(path);
        Ok(self
            .entries()?
            .into_iter()
            .filter(|e| path_starts_with(&p, &e.path))
            .max_by_key(|e| e.path.components().count()))
    }
    pub fn measure(&self, path: &Path, now_ms: i64) -> Res<Option<u64>> {
        let p = normalize(path);
        let entry = match self.get(&p)? {
            Some(e) if e.state == EntryState::Present => e,
            _ => return Ok(None),
        };
        if fs::symlink_metadata(path).is_ok_and(|m| is_link(&m)) {
            return Ok(Some(0));
        }
        match fs::symlink_metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(0)),
            Err(e) => return Err(err(e)),
            Ok(_) => (),
        }
        let descendants: Vec<PathBuf> = self
            .entries()?
            .into_iter()
            .filter(|e| e.id != entry.id && path_starts_with(&e.path, &p))
            .map(|e| e.path)
            .collect();
        fn walk(p: &Path, excludes: &[PathBuf]) -> Res<u64> {
            if excludes.iter().any(|x| same_path(x, p)) {
                return Ok(0);
            }
            let m = fs::symlink_metadata(p).map_err(err)?;
            if is_link(&m) {
                return Ok(0);
            }
            if m.is_file() {
                return Ok(m.len());
            }
            if !m.is_dir() {
                return Ok(0);
            }
            let mut total = 0u64;
            for item in fs::read_dir(p).map_err(err)? {
                let item = item.map_err(err)?;
                total = total
                    .checked_add(walk(&item.path(), excludes)?)
                    .ok_or_else(|| err("size overflow"))?;
            }
            Ok(total)
        }
        let size = walk(&p, &descendants)?;
        let stored = i64::try_from(size).map_err(err)?;
        self.transaction(|| {
            if !self.get_key(&key(&p))?.is_some_and(|e| e.id == entry.id && e.state == EntryState::Present) {
                return Ok(None);
            }
            // A registration during the walk changes which bytes belong to this entry.
            let current: Vec<PathBuf> = self.entries()?.into_iter()
                .filter(|e| e.id != entry.id && path_starts_with(&e.path, &p))
                .map(|e| e.path).collect();
            if current != descendants {
                return Ok(None);
            }
            self.conn.execute("UPDATE ledger_entries SET size_bytes=?1, measured_ms=?2 WHERE id=?3 AND state='present'", params![stored, now_ms, entry.id]).map_err(err)?;
            Ok(Some(size))
        })
    }
    pub fn measure_stale(&self, max_age_ms: i64, budget: Duration, now_ms: i64) -> Res<usize> {
        let start = Instant::now();
        let mut n = 0;
        for e in self.entries()? {
            if start.elapsed() >= budget {
                break;
            }
            if e.measured_ms
                .is_none_or(|t| now_ms.saturating_sub(t) >= max_age_ms)
            {
                match self.measure(&e.path, now_ms) {
                    Ok(Some(_)) => n += 1,
                    Ok(None) => (),
                    Err(error) => {
                        n += 1;
                        if let Some(home) = self.db.parent().and_then(Path::parent) {
                            super::log_error(
                                home,
                                &Failure::new(
                                    &error.kind,
                                    format!("{}: {}", e.path.display(), error.message),
                                ),
                            );
                        }
                    }
                }
            }
        }
        Ok(n)
    }
    pub fn mark_deleted(&self, path: &Path, now_ms: i64) -> Res<()> {
        let k = key(path);
        self.transaction(|| {
            if self
                .get_key(&k)?
                .is_some_and(|e| e.state == EntryState::Present)
            {
                self.invalidate_ancestors(&normalize(path))?;
            }
            self.conn
                .execute(
                    "UPDATE ledger_entries SET state='deleted', last_used_ms=?1 WHERE path_key=?2",
                    params![now_ms, k],
                )
                .map_err(err)?;
            Ok(())
        })
    }
    pub fn set_meta(&self, path: &Path, meta: Value) -> Res<()> {
        let k = key(path);
        self.transaction(|| {
            if let Some(mut e) = self.get_key(&k)? {
                merge(&mut e.meta, meta);
                self.conn
                    .execute(
                        "UPDATE ledger_entries SET meta=?1 WHERE path_key=?2",
                        params![e.meta.to_string(), k],
                    )
                    .map_err(err)?;
            }
            Ok(())
        })
    }
    pub fn lease(&self, path: &Path) -> Res<Lease> {
        let conn = Connection::open(&self.db).map_err(err)?;
        conn.busy_timeout(Duration::from_secs(10)).map_err(err)?;
        conn.execute(
            "INSERT INTO ledger_leases(path,pid) VALUES(?1,?2)",
            params![key(path), std::process::id()],
        )
        .map_err(err)?;
        let id = conn.last_insert_rowid();
        Ok(Lease { conn, id })
    }
    pub fn in_use(&self, path: &Path) -> Res<bool> {
        let p = PathBuf::from(key(path));
        let mut stmt = self
            .conn
            .prepare("SELECT id,path,pid FROM ledger_leases")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            })
            .map_err(err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(err)?;
        drop(stmt);
        let mut used = false;
        for (id, path, pid) in rows {
            if !base::proc::pid_alive(pid) {
                self.conn
                    .execute(
                        "DELETE FROM ledger_leases WHERE id=?1 AND pid=?2",
                        params![id, pid],
                    )
                    .map_err(err)?;
            } else if path_starts_with(Path::new(&path), &p) {
                used = true;
            }
        }
        Ok(used)
    }
    pub fn reconcile(&self, home: &Path) -> Res<usize> {
        fn children(p: &Path) -> Res<Vec<PathBuf>> {
            match fs::read_dir(p) {
                Ok(rd) => {
                    let mut v = Vec::new();
                    for x in rd {
                        let x = x.map_err(err)?;
                        if x.file_type().map_err(err)?.is_dir() {
                            v.push(x.path());
                        }
                    }
                    v.sort();
                    Ok(v)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err(err(e)),
            }
        }
        let sys = Owner {
            project: super::SYSTEM_PROJECT.into(),
            wish: None,
        };
        let mut count = 0;
        let adopt = |p: &Path, c: Class, o: &Owner, count: &mut usize| -> Res<()> {
            if self.get(p)?.is_none() {
                *count += 1;
            }
            self.register(p, c, o, serde_json::json!({}))?;
            Ok(())
        };
        for p in children(&home.join("engine"))? {
            adopt(&p, Class::EngineCopy, &sys, &mut count)?;
        }
        for p in children(&home.join("build-cache"))? {
            adopt(&p, Class::BuildCache, &sys, &mut count)?;
        }
        let logs = home.join("logs");
        if logs.is_dir() {
            adopt(&logs, Class::Logs, &sys, &mut count)?;
        }
        for name in [
            "goals",
            "memory",
            "codex-slots",
            "council-running",
            "council-switch",
            "evolve",
            "hook",
            "service",
            "storage",
        ] {
            let p = home.join(name);
            if p.is_dir() {
                adopt(&p, Class::UnitStore, &sys, &mut count)?;
            }
        }
        for project in children(&home.join("orchestrate"))? {
            let name = project
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if name != super::SYSTEM_PROJECT && base::require_slug("project", &name).is_err() {
                continue;
            }
            let owner = Owner {
                project: name.clone(),
                wish: None,
            };
            adopt(&project, Class::UnitStore, &owner, &mut count)?;
            for store in children(&project)? {
                let wish = store
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let o = Owner {
                    project: name.clone(),
                    wish: Some(wish),
                };
                adopt(&store, Class::UnitStore, &o, &mut count)?;
                for (name, class) in [
                    ("scratch", Class::Scratch),
                    ("reports", Class::Evidence),
                    ("receipts", Class::Evidence),
                ] {
                    let p = store.join(name);
                    if p.is_dir() {
                        adopt(&p, class, &o, &mut count)?;
                    }
                }
            }
        }
        Ok(count)
    }
    pub fn unregistered(&self, home: &Path) -> Res<Vec<PathBuf>> {
        let entries = self.entries()?;
        let mut out = Vec::new();
        fn children(path: &Path) -> Res<Vec<PathBuf>> {
            match fs::read_dir(path) {
                Ok(rd) => rd
                    .map(|x| x.map(|entry| entry.path()).map_err(err))
                    .collect(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(e) => Err(err(e)),
            }
        }
        for p in children(home)? {
            let n = normalize(&p);
            if entries.iter().any(|e| path_starts_with(&n, &e.path)) {
                continue;
            }
            if entries.iter().any(|e| path_starts_with(&e.path, &n)) {
                // Do not follow links while inspecting partially covered directories.
                if fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir() && !is_link(&m)) {
                    for child in children(&p)? {
                        let n = normalize(&child);
                        if !entries
                            .iter()
                            .any(|e| path_starts_with(&n, &e.path) || path_starts_with(&e.path, &n))
                        {
                            out.push(child);
                        }
                    }
                }
            } else {
                out.push(p);
            }
        }
        out.sort();
        Ok(out)
    }
}
