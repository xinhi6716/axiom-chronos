use crate::base::{self, Failure, Res};
use crate::orch::{covers, current_verified_unit, evidence, store_for, unit_brief};
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_QUERY: usize = 1024;
const MAX_TERMS: usize = 32;
const MAX_LIMIT: i64 = 50;
const MAX_FIELD: usize = 4096;
const MAX_BODY: usize = 16384;
const MAX_DIGEST: usize = 8192;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY, project TEXT NOT NULL, unit TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL,
    body TEXT NOT NULL, symbols TEXT NOT NULL, wish TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL, created REAL NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    title, summary, body, symbols, wish, content='entries', content_rowid='id', tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, title, summary, body, symbols, wish)
    VALUES (new.id, new.title, new.summary, new.body, new.symbols, new.wish);
END;
";
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn query_terms(query: &str) -> Vec<String> {
    let word = Regex::new(r"[A-Za-z0-9_./-]{2,}").expect("constant regex");
    let cjk = Regex::new(r"[\u{3040}-\u{30ff}\u{3400}-\u{9fff}\u{f900}-\u{faff}\u{1100}-\u{11ff}\u{3130}-\u{318f}\u{a960}-\u{a97f}\u{ac00}-\u{d7ff}\u{20000}-\u{2fa1f}\u{30000}-\u{323af}]+")
        .expect("constant regex");
    let mut terms: Vec<String> = word
        .find_iter(query)
        .map(|m| m.as_str().to_ascii_lowercase())
        .collect();
    for run in cjk.find_iter(query) {
        let chars: Vec<char> = run.as_str().chars().collect();
        if chars.len() == 1 {
            terms.push(chars[0].to_string());
        } else {
            terms.extend(chars.windows(2).map(|w| w.iter().collect()));
        }
    }
    let mut seen = HashSet::new();
    terms.retain(|s| seen.insert(s.clone()));
    terms
}

pub struct Record {
    pub project: String,
    pub project_dir: String,
    pub unit: String,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub symbols: Vec<String>,
    pub files: Vec<String>,
    pub wish: String,
}
pub struct Memory {
    home: PathBuf,
    root: PathBuf,
    db_path: PathBuf,
}
impl Memory {
    pub fn open(home: &Path) -> Res<Self> {
        let root = home.join("memory");
        std::fs::create_dir_all(&root).map_err(err)?;
        let m = Self {
            home: home.to_path_buf(),
            db_path: root.join("memory.db"),
            root,
        };
        m.connect()?.execute_batch(SCHEMA).map_err(err)?;
        Ok(m)
    }
    fn connect(&self) -> Res<Connection> {
        Connection::open(&self.db_path).map_err(err)
    }
    pub fn record(&self, args: &Record) -> Res<Value> {
        base::require_slug("project", &args.project)?;
        for (name, value) in [
            ("unit", &args.unit),
            ("title", &args.title),
            ("summary", &args.summary),
            ("body", &args.body),
            ("wish", &args.wish),
        ] {
            if value.trim().is_empty() {
                return Err(Failure::new(
                    "invalid_argument",
                    format!("{name} is required"),
                ));
            }
        }
        if args.project.len() > 128
            || args.unit.len() > 128
            || args.project_dir.len() > MAX_FIELD
            || [&args.title, &args.summary, &args.wish]
                .iter()
                .any(|v| v.len() > MAX_FIELD)
            || args.body.len() > MAX_BODY
            || args.symbols.len() > 128
            || args.files.len() > 128
            || args
                .symbols
                .iter()
                .chain(&args.files)
                .any(|v| v.len() > MAX_FIELD)
            || args.symbols.iter().map(String::len).sum::<usize>() > MAX_FIELD
            || args.files.iter().map(String::len).sum::<usize>() > MAX_BODY
        {
            return Err(Failure::new(
                "invalid_argument",
                "record exceeds byte or item limits",
            ));
        }
        if args.body.lines().count() > 12 {
            return Err(Failure::new("invalid_argument", "body exceeds 12 lines"));
        }
        let root = Path::new(&args.project_dir);
        if !root.is_absolute() || !root.is_dir() {
            return Err(Failure::new(
                "invalid_argument",
                format!(
                    "project_dir must be an existing absolute directory: {:?}",
                    args.project_dir
                ),
            ));
        }
        let store = store_for(&self.home, &args.project)?;
        let identity = verified_identity(&store, &args.unit, root).map_err(|_| {
            Failure::new(
                "not_verified",
                format!("unit {} lacks current verified evidence", args.unit),
            )
        })?;
        let brief = if args.files.is_empty() {
            None
        } else {
            Some(unit_brief(&store, &args.unit)?)
        };
        let mut files = Vec::new();
        for rel in &args.files {
            let relative = Path::new(rel);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| matches!(c, Component::ParentDir))
            {
                return Err(Failure::new(
                    "invalid_argument",
                    format!("invalid relative file path: {rel}"),
                ));
            }
            if !covers(&brief.as_ref().expect("brief exists").scope, rel) {
                return Err(Failure::new(
                    "outside_scope",
                    format!("file outside unit scope: {rel}"),
                ));
            }
            files.push(json!({"path": rel, "sha256": hash_contained(root, relative)?}));
        }
        let mut db = self.connect()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS memory_evidence (entry_id INTEGER PRIMARY KEY, identity TEXT NOT NULL); CREATE TABLE IF NOT EXISTS memory_symbols (entry_id INTEGER PRIMARY KEY, symbols TEXT NOT NULL)").map_err(err)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let stored_files = serde_json::to_string(&files).map_err(err)?;
        let symbols = format!(
            "@json:{}",
            serde_json::to_string(&args.symbols).map_err(err)?
        );
        let identity = entry_identity(
            &identity,
            &[
                &args.project,
                &args.unit,
                args.title.trim(),
                args.summary.trim(),
                args.body.trim(),
                &symbols,
                args.wish.trim(),
                &stored_files,
                &args.project_dir,
            ],
        );
        let mut existing: Option<i64> = tx.query_row("SELECT id FROM entries WHERE project=? AND unit=? AND title=? AND summary=? AND body=? AND symbols=? AND wish=? AND files=? AND project_dir=? AND id IN (SELECT entry_id FROM memory_symbols) ORDER BY id LIMIT 1", params![args.project,args.unit,args.title.trim(),args.summary.trim(),args.body.trim(),symbols,args.wish.trim(),stored_files,args.project_dir], |r| r.get(0)).optional().map_err(err)?;
        // Legacy whitespace encoding is unambiguous only for nonempty, whitespace-free symbols.
        let mut stored_symbols = symbols.clone();
        if existing.is_none()
            && args
                .symbols
                .iter()
                .all(|s| !s.is_empty() && !s.chars().any(char::is_whitespace))
        {
            let legacy = args.symbols.join(" ");
            existing = tx.query_row("SELECT id FROM entries WHERE project=? AND unit=? AND title=? AND summary=? AND body=? AND symbols=? AND wish=? AND files=? AND project_dir=? AND id NOT IN (SELECT entry_id FROM memory_symbols) ORDER BY id LIMIT 1", params![args.project,args.unit,args.title.trim(),args.summary.trim(),args.body.trim(),legacy,args.wish.trim(),stored_files,args.project_dir], |r| r.get(0)).optional().map_err(err)?;
            if existing.is_some() {
                stored_symbols = legacy;
            }
        }
        let identity = if stored_symbols == symbols {
            identity
        } else {
            entry_identity(
                &verified_identity(&store, &args.unit, root)?,
                &[
                    &args.project,
                    &args.unit,
                    args.title.trim(),
                    args.summary.trim(),
                    args.body.trim(),
                    &stored_symbols,
                    args.wish.trim(),
                    &stored_files,
                    &args.project_dir,
                ],
            )
        };
        let id = if let Some(id) = existing {
            tx.execute("INSERT INTO memory_evidence VALUES (?,?) ON CONFLICT(entry_id) DO UPDATE SET identity=excluded.identity", params![id, identity]).map_err(err)?;
            id
        } else {
            let created = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            tx.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES (?,?,?,?,?,?,?,?,?,?)", params![args.project, args.unit, args.title.trim(), args.summary.trim(), args.body.trim(), symbols, args.wish.trim(), stored_files, args.project_dir, created]).map_err(err)?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memory_symbols VALUES (?,?)",
                params![id, serde_json::to_string(&args.symbols).map_err(err)?],
            )
            .map_err(err)?;
            tx.execute(
                "INSERT INTO memory_evidence VALUES (?,?)",
                params![id, identity],
            )
            .map_err(err)?;
            id
        };
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed", "id":id, "files":files, "error":null}))
    }
    pub fn search(&self, query: &str, project: Option<&str>, limit: i64) -> Res<Value> {
        self.search_impl(query, project, limit, false)
    }
    /// Recall only entries whose recorded evidence, contract, snapshot and files are current.
    pub fn search_fresh(&self, query: &str, project: Option<&str>, limit: i64) -> Res<Value> {
        self.search_impl(query, project, limit, true)
    }
    fn search_impl(
        &self,
        query: &str,
        project: Option<&str>,
        limit: i64,
        fresh: bool,
    ) -> Res<Value> {
        if query.len() > MAX_QUERY || !(1..=MAX_LIMIT).contains(&limit) {
            return Err(Failure::new(
                "invalid_argument",
                "query or result limit exceeded",
            ));
        }
        let terms = query_terms(query);
        if terms.is_empty() || terms.len() > MAX_TERMS {
            return Err(Failure::new(
                "invalid_argument",
                "query has no searchable terms or exceeds term limit",
            ));
        }
        // Trigram FTS cannot index short terms; LIKE intentionally scans for consistent CJK/ASCII semantics.
        let hit = "(e.title LIKE ? ESCAPE '!' OR e.summary LIKE ? ESCAPE '!' OR e.body LIKE ? ESCAPE '!' OR e.symbols LIKE ? ESCAPE '!' OR e.wish LIKE ? ESCAPE '!')";
        let hits = vec![hit; terms.len()].join(" + ");
        let filter = project.filter(|p| !p.is_empty());
        let sql = format!("SELECT * FROM (SELECT e.*, {hits} AS matched FROM entries e) WHERE matched > 0{} ORDER BY matched DESC, created DESC, id DESC", if filter.is_some() { " AND project = ?" } else { "" });
        let mut values: Vec<rusqlite::types::Value> = terms
            .iter()
            .flat_map(|t| {
                let literal = t.replace('!', "!!").replace('%', "!%").replace('_', "!_");
                (0..5).map(move |_| format!("%{literal}%").into())
            })
            .collect();
        if let Some(p) = filter {
            values.push(p.to_string().into());
        }
        let db = self.connect()?;
        let mut stmt = db.prepare(&sql).map_err(err)?;
        let mut rows = stmt.query(params_from_iter(values)).map_err(err)?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let status = self.freshness(&db, row);
            if fresh && status != Some(false) {
                continue;
            }
            entries.push(json!({"id":row.get::<_, i64>("id").map_err(err)?, "project":bounded(&row.get::<_, String>("project").map_err(err)?, 128), "unit":bounded(&row.get::<_, String>("unit").map_err(err)?, 128), "title":bounded(&row.get::<_, String>("title").map_err(err)?, MAX_FIELD), "summary":bounded(&row.get::<_, String>("summary").map_err(err)?, MAX_FIELD), "matched_terms":row.get::<_, i64>("matched").map_err(err)?, "stale":status}));
            if entries.len() == limit as usize {
                break;
            }
        }
        Ok(json!({"status":"completed", "terms":terms, "entries":entries, "error":null}))
    }
    fn freshness(&self, db: &Connection, row: &Row<'_>) -> Option<bool> {
        let id: i64 = row.get("id").ok()?;
        let saved: String = db
            .query_row(
                "SELECT identity FROM memory_evidence WHERE entry_id=?",
                [id],
                |r| r.get(0),
            )
            .ok()?;
        let project: String = row.get("project").ok()?;
        let unit: String = row.get("unit").ok()?;
        let root: String = row.get("project_dir").ok()?;
        if !Path::new(&root).is_dir() {
            return None;
        }
        let store = store_for(&self.home, &project).ok()?;
        let identity = match verified_identity(&store, &unit, Path::new(&root)) {
            Ok(v) => v,
            Err(_) => return Some(true),
        };
        let fields = [
            "project",
            "unit",
            "title",
            "summary",
            "body",
            "symbols",
            "wish",
            "files",
            "project_dir",
        ];
        let values: Vec<String> = fields
            .iter()
            .map(|name| row.get(*name))
            .collect::<Result<_, _>>()
            .ok()?;
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        if entry_identity(&identity, &refs) != saved {
            return Some(true);
        }
        stale(row).ok().flatten()
    }
    pub fn get(&self, id: i64) -> Res<Value> {
        let db = self.connect()?;
        let mut stmt = db
            .prepare("SELECT * FROM entries WHERE id = ?")
            .map_err(err)?;
        let mut rows = stmt.query([id]).map_err(err)?;
        let row = rows
            .next()
            .map_err(err)?
            .ok_or_else(|| Failure::new("not_found", format!("no memory entry {id}")))?;
        let files: String = row.get("files").map_err(err)?;
        let symbols: String = row.get("symbols").map_err(err)?;
        let encoded: Option<String> = db
            .query_row(
                "SELECT symbols FROM memory_symbols WHERE entry_id=?",
                [id],
                |r| r.get(0),
            )
            .ok();
        let decoded: Vec<String> = encoded
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_else(|| symbols.split_whitespace().map(str::to_owned).collect());
        let mut budget = 32768;
        let entry = json!({"id":id, "project":bounded(&row.get::<_, String>("project").map_err(err)?, 128), "unit":bounded(&row.get::<_, String>("unit").map_err(err)?, 128), "title":bounded(&row.get::<_, String>("title").map_err(err)?, MAX_FIELD), "summary":bounded(&row.get::<_, String>("summary").map_err(err)?, MAX_FIELD), "body":bounded(&row.get::<_, String>("body").map_err(err)?, MAX_BODY), "symbols":bound_value(json!(decoded), &mut budget, 0), "wish":bounded(&row.get::<_, String>("wish").map_err(err)?, MAX_FIELD), "files":bound_value(serde_json::from_str::<Value>(&files).map_err(err)?, &mut budget, 0), "project_dir":bounded(&row.get::<_, String>("project_dir").map_err(err)?, MAX_FIELD), "created":row.get::<_, f64>("created").map_err(err)?, "stale":self.freshness(&db, row)});
        Ok(json!({"status":"completed", "entry":entry, "error":null}))
    }
    pub fn digest(&self, project: &str, add: Option<&str>) -> Res<Value> {
        base::require_slug("project", project)?;
        let path = self.root.join(format!("{project}.md"));
        if let Some(add) = add.filter(|s| !s.is_empty()) {
            if add.len() > MAX_FIELD {
                return Err(Failure::new(
                    "invalid_argument",
                    "digest addition exceeds byte limit",
                ));
            }
            let line = add.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(err)?;
            writeln!(f, "- {} {line}", base::today()).map_err(err)?;
        }
        let digest = match std::fs::File::open(path) {
            Ok(mut f) => {
                let size = f.metadata().map_err(err)?.len();
                f.seek(SeekFrom::Start(size.saturating_sub(MAX_DIGEST as u64)))
                    .map_err(err)?;
                let mut bytes = Vec::new();
                f.read_to_end(&mut bytes).map_err(err)?;
                let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
                bounded_tail(&text, MAX_DIGEST)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(err(e)),
        };
        Ok(json!({"status":"completed", "digest":digest, "error":null}))
    }
}
// Cap legacy JSON recursively as well as its strings, without splitting UTF-8.
fn bound_value(value: Value, budget: &mut usize, depth: usize) -> Value {
    if depth >= 8 || *budget == 0 {
        return Value::Null;
    }
    *budget -= 1;
    match value {
        Value::String(s) => {
            let s = bounded(&s, (*budget).min(MAX_FIELD));
            *budget -= s.len();
            Value::String(s)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(128)
                .take_while(|_| *budget > 0)
                .collect::<Vec<_>>()
                .into_iter()
                .map(|v| bound_value(v, budget, depth + 1))
                .collect(),
        ),
        Value::Object(items) => {
            let mut out = serde_json::Map::new();
            for (key, value) in items.into_iter().take(128) {
                if *budget == 0 {
                    break;
                }
                let key = bounded(&key, (*budget).min(MAX_FIELD));
                *budget -= key.len();
                out.insert(key, bound_value(value, budget, depth + 1));
            }
            Value::Object(out)
        }
        v => v,
    }
}
fn entry_identity(evidence: &str, fields: &[&str]) -> String {
    let bytes = serde_json::to_vec(&(evidence, fields)).expect("strings serialize");
    format!("{:x}", Sha256::digest(bytes))
}
fn bounded(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}
fn bounded_tail(s: &str, max: usize) -> String {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}
fn verified_identity(store: &Path, unit: &str, root: &Path) -> Res<String> {
    if !current_verified_unit(store, unit, root)? {
        return Err(Failure::new(
            "not_verified",
            "evidence, contract, dependencies or snapshot are not current",
        ));
    }
    let rows = evidence(store, unit)?;
    let data = serde_json::to_vec(&rows).map_err(err)?;
    Ok(format!("{:x}", Sha256::digest(data)))
}
fn hash_contained(root: &Path, relative: &Path) -> Res<String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Failure::new(
            "invalid_argument",
            "invalid relative file path",
        ));
    }
    let directory = open_directory(root).map_err(err)?;
    let mut f = std::fs::File::open(root.join(relative)).map_err(err)?;
    if !f.metadata().map_err(err)?.is_file() || !handle_is_contained(&directory, &f).map_err(err)? {
        return Err(Failure::new(
            "invalid_argument",
            "file resolves outside project_dir or is not a regular file",
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 16384];
    loop {
        let n = f.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(windows)]
fn open_directory(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x02000000)
        .open(path)
}

#[cfg(target_os = "linux")]
fn open_directory(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

// Resolve the opened handles, not the path used to open the file. No file
// bytes are read until containment has been established on that same handle.
#[cfg(target_os = "linux")]
fn handle_is_contained(root: &std::fs::File, file: &std::fs::File) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    let path = |f: &std::fs::File| std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd()));
    let root = path(root)?;
    let file = path(file)?;
    Ok(root.is_absolute() && file.is_absolute() && file.starts_with(root))
}

#[cfg(windows)]
fn handle_is_contained(root: &std::fs::File, file: &std::fs::File) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFinalPathNameByHandleW(
            handle: *mut std::ffi::c_void,
            path: *mut u16,
            len: u32,
            flags: u32,
        ) -> u32;
    }
    fn final_path(f: &std::fs::File) -> std::io::Result<Vec<u16>> {
        let mut buf = vec![0u16; 512];
        loop {
            let n = unsafe {
                GetFinalPathNameByHandleW(f.as_raw_handle(), buf.as_mut_ptr(), buf.len() as u32, 0)
            };
            if n == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if n < buf.len() as u32 {
                buf.truncate(n as usize);
                return Ok(buf);
            }
            buf.resize(n as usize + 1, 0);
        }
    }
    let mut root = final_path(root)?;
    let file = final_path(file)?;
    if root.last() != Some(&(b'\\' as u16)) {
        root.push(b'\\' as u16);
    }
    // Windows directories can be case-sensitive. A case-insensitive prefix
    // comparison could otherwise accept a different sibling directory.
    Ok(file.len() > root.len() && file.starts_with(&root))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open_directory(_: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure file containment is unsupported on this platform",
    ))
}
#[cfg(not(any(windows, target_os = "linux")))]
fn handle_is_contained(_: &std::fs::File, _: &std::fs::File) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure file containment is unsupported on this platform",
    ))
}
fn stale(row: &Row<'_>) -> Res<Option<bool>> {
    let root: String = row.get("project_dir").map_err(err)?;
    let root = Path::new(&root);
    if !root.is_dir() {
        return Ok(None);
    }
    let files: String = row.get("files").map_err(err)?;
    let files: Vec<Value> = serde_json::from_str(&files).map_err(err)?;
    for item in files {
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| err("invalid file path"))?;
        let expected = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| err("invalid file hash"))?;
        if hash_contained(root, Path::new(path)).map_or(true, |hash| hash != expected) {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod containment_tests {
    use super::*;

    #[test]
    fn checks_opened_handles_and_rejects_escapes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("inside"), "inside").unwrap();
        std::fs::write(outside.join("secret"), "secret").unwrap();
        let dir = open_directory(&root).unwrap();
        let inside = std::fs::File::open(root.join("inside")).unwrap();
        let secret = std::fs::File::open(outside.join("secret")).unwrap();
        assert!(handle_is_contained(&dir, &inside).unwrap());
        assert!(!handle_is_contained(&dir, &secret).unwrap());
        assert!(hash_contained(&root, Path::new("inside")).is_ok());
        assert!(hash_contained(&root, Path::new("../outside/secret")).is_err());

        #[cfg(unix)]
        let link = std::os::unix::fs::symlink(outside.join("secret"), root.join("link"));
        #[cfg(windows)]
        let link = std::os::windows::fs::symlink_file(outside.join("secret"), root.join("link"));
        if link.is_ok() {
            assert!(hash_contained(&root, Path::new("link")).is_err());
        } else {
            // Windows may disallow symlink creation without developer mode.
            #[cfg(unix)]
            link.unwrap();
        }
    }
}
