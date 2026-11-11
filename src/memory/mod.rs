pub mod lint;
use crate::base::{self, Failure, Res};
use crate::orch::{covers, read_units, store_for, unit_brief};
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, Row};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashSet},
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    id INTEGER PRIMARY KEY, project TEXT NOT NULL, unit TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL,
    body TEXT NOT NULL, symbols TEXT NOT NULL, wish TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL, created REAL NOT NULL,
    confirmed REAL, corrected REAL);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    title, summary, body, symbols, wish, content='entries', content_rowid='id', tokenize='trigram');
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, title, summary, body, symbols, wish)
    VALUES (new.id, new.title, new.summary, new.body, new.symbols, new.wish);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, summary, body, symbols, wish)
    VALUES ('delete', old.id, old.title, old.summary, old.body, old.symbols, old.wish);
END;
CREATE TRIGGER IF NOT EXISTS entries_au AFTER UPDATE OF title, summary, body, symbols, wish ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, title, summary, body, symbols, wish)
    VALUES ('delete', old.id, old.title, old.summary, old.body, old.symbols, old.wish);
    INSERT INTO entries_fts(rowid, title, summary, body, symbols, wish)
    VALUES (new.id, new.title, new.summary, new.body, new.symbols, new.wish);
END;
";
fn err(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

pub fn query_terms(query: &str) -> Vec<String> {
    let word = Regex::new(r"[A-Za-z0-9_./-]{2,}").expect("constant regex");
    let cjk = Regex::new(r"[\u{3040}-\u{30ff}\u{3400}-\u{9fff}\u{f900}-\u{faff}]+")
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
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryPriorRow {
    pub body: String,
    pub summary: String,
    pub corrected: Option<f64>,
    pub confirmed: f64,
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
        m.migrate()?;
        Ok(m)
    }
    /// Creates or upgrades the schema under one write lock, so concurrent opens
    /// (several processes, old or new database) serialize instead of racing.
    fn migrate(&self) -> Res<()> {
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        tx.execute_batch(SCHEMA).map_err(err)?;
        let have: HashSet<String> = {
            let mut stmt = tx
                .prepare("SELECT name FROM pragma_table_info('entries')")
                .map_err(err)?;
            let names = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?;
            names
        };
        for column in ["confirmed", "corrected"] {
            if !have.contains(column) {
                tx.execute(&format!("ALTER TABLE entries ADD COLUMN {column} REAL"), [])
                    .map_err(err)?;
            }
        }
        tx.execute(
            "UPDATE entries SET confirmed = created WHERE confirmed IS NULL",
            [],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }
    fn connect(&self) -> Res<Connection> {
        let db = Connection::open(&self.db_path).map_err(err)?;
        db.busy_timeout(std::time::Duration::from_secs(30))
            .map_err(err)?;
        Ok(db)
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
        refuse_bad_quotes(&[("body", &args.body), ("summary", &args.summary)], root)?;
        let store = store_for(&self.home, &args.project)?;
        if !read_units(&store)
            .map(|units| {
                units
                    .iter()
                    .any(|u| u.id == args.unit && u.state == "verified")
            })
            .unwrap_or(false)
        {
            return Err(Failure::new(
                "not_verified",
                format!("unit {} is not verified", args.unit),
            ));
        }
        if args.files.is_empty() {
            return Err(Failure::new(
                "invalid_argument",
                "files are required for a verified unit",
            ));
        }
        let brief = unit_brief(&store, &args.unit)?;
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
            if !covers(&brief.scope, rel) {
                return Err(Failure::new(
                    "outside_scope",
                    format!("file outside unit scope: {rel}"),
                ));
            }
            let path = root.join(rel);
            if !path.is_file() {
                return Err(Failure::new(
                    "invalid_argument",
                    format!("file not found in project_dir: {rel}"),
                ));
            }
            files.push(json!({"path": rel, "sha256": hash(&path)?}));
        }
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let wanted = file_set(&files)?;
        let duplicate = {
            let mut stmt = tx
                .prepare("SELECT id, files FROM entries WHERE project = ? AND unit = ? ORDER BY id")
                .map_err(err)?;
            let mut rows = stmt.query(params![args.project, args.unit]).map_err(err)?;
            let mut found = None;
            while let Some(row) = rows.next().map_err(err)? {
                let stored: String = row.get(1).map_err(err)?;
                let stored: Vec<Value> = serde_json::from_str(&stored).map_err(err)?;
                if file_set(&stored)? == wanted {
                    found = Some(row.get::<_, i64>(0).map_err(err)?);
                    break;
                }
            }
            found
        };
        if let Some(id) = duplicate {
            // Recording the same verified files again re-confirms the entry.
            tx.execute(
                "UPDATE entries SET confirmed = ? WHERE id = ?",
                params![now(), id],
            )
            .map_err(err)?;
            tx.commit().map_err(err)?;
            return Ok(
                json!({"status":"completed", "id":id, "files":files, "duplicate":true, "error":null}),
            );
        }
        let created = now();
        tx.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created, confirmed) VALUES (?,?,?,?,?,?,?,?,?,?,?)", params![args.project, args.unit, args.title.trim(), args.summary.trim(), args.body.trim(), args.symbols.join(" "), args.wish.trim(), serde_json::to_string(&files).map_err(err)?, args.project_dir, created, created]).map_err(err)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed", "id":id, "files":files, "duplicate":false, "error":null}))
    }
    pub fn search(&self, query: &str, project: Option<&str>, limit: i64) -> Res<Value> {
        let terms = query_terms(query);
        if terms.is_empty() {
            return Err(Failure::new(
                "invalid_argument",
                "query has no searchable terms",
            ));
        }
        let hit = "(f.title LIKE ? OR f.summary LIKE ? OR f.body LIKE ? OR f.symbols LIKE ? OR f.wish LIKE ?)";
        let hits = vec![hit; terms.len()].join(" + ");
        let filter = project.filter(|p| !p.is_empty());
        let sql = format!("SELECT * FROM (SELECT e.*, {hits} AS matched FROM entries e JOIN entries_fts f ON f.rowid = e.id) WHERE matched > 0{} ORDER BY matched DESC, created DESC LIMIT ?", if filter.is_some() { " AND project = ?" } else { "" });
        let mut values: Vec<rusqlite::types::Value> = terms
            .iter()
            .flat_map(|t| (0..5).map(move |_| format!("%{t}%").into()))
            .collect();
        if let Some(p) = filter {
            values.push(p.to_string().into());
        }
        values.push(limit.into());
        let db = self.connect()?;
        let mut stmt = db.prepare(&sql).map_err(err)?;
        let mut rows = stmt.query(params_from_iter(values)).map_err(err)?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            entries.push(json!({"id":row.get::<_, i64>("id").map_err(err)?, "project":row.get::<_, String>("project").map_err(err)?, "unit":row.get::<_, String>("unit").map_err(err)?, "title":row.get::<_, String>("title").map_err(err)?, "summary":row.get::<_, String>("summary").map_err(err)?, "matched_terms":row.get::<_, i64>("matched").map_err(err)?, "stale":stale(row)?}));
        }
        Ok(json!({"status":"completed", "terms":terms, "entries":entries, "error":null}))
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
        let entry = json!({"id":id, "project":row.get::<_, String>("project").map_err(err)?, "unit":row.get::<_, String>("unit").map_err(err)?, "title":row.get::<_, String>("title").map_err(err)?, "summary":row.get::<_, String>("summary").map_err(err)?, "body":row.get::<_, String>("body").map_err(err)?, "symbols":symbols.split_whitespace().collect::<Vec<_>>(), "wish":row.get::<_, String>("wish").map_err(err)?, "files":serde_json::from_str::<Value>(&files).map_err(err)?, "project_dir":row.get::<_, String>("project_dir").map_err(err)?, "created":row.get::<_, f64>("created").map_err(err)?, "confirmed":confirmed(row)?, "corrected":row.get::<_, Option<f64>>("corrected").map_err(err)?, "stale":stale(row)?});
        Ok(json!({"status":"completed", "entry":entry, "error":null}))
    }
    /// Entries newest first, in full, for browsing and managing what was learned.
    pub fn list(&self, project: Option<&str>, limit: i64) -> Res<Value> {
        let filter = project.filter(|p| !p.is_empty());
        let sql = format!(
            "SELECT * FROM entries{} ORDER BY created DESC, id DESC LIMIT ?",
            if filter.is_some() {
                " WHERE project = ?"
            } else {
                ""
            }
        );
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(p) = filter {
            values.push(p.to_string().into());
        }
        values.push(limit.max(0).into());
        let db = self.connect()?;
        let mut stmt = db.prepare(&sql).map_err(err)?;
        let mut rows = stmt.query(params_from_iter(values)).map_err(err)?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            entries.push(json!({"id":row.get::<_, i64>("id").map_err(err)?, "project":row.get::<_, String>("project").map_err(err)?, "title":row.get::<_, String>("title").map_err(err)?, "summary":row.get::<_, String>("summary").map_err(err)?, "body":row.get::<_, String>("body").map_err(err)?, "wish":row.get::<_, String>("wish").map_err(err)?, "created":row.get::<_, f64>("created").map_err(err)?, "confirmed":confirmed(row)?, "corrected":row.get::<_, Option<f64>>("corrected").map_err(err)?, "stale":stale(row)?}));
        }
        Ok(json!({"status":"completed", "entries":entries, "error":null}))
    }
    /// Removes the entry; the delete trigger removes its search row.
    pub fn delete(&self, id: i64) -> Res<Value> {
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        if tx
            .execute("DELETE FROM entries WHERE id = ?", [id])
            .map_err(err)?
            == 0
        {
            return Err(Failure::new("not_found", format!("no memory entry {id}")));
        }
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed", "id":id, "error":null}))
    }
    pub fn prior(&self, id: i64) -> Res<MemoryPriorRow> {
        let db = self.connect()?;
        let mut stmt = db.prepare("SELECT * FROM entries WHERE id = ?").map_err(err)?;
        let mut rows = stmt.query([id]).map_err(err)?;
        let row = rows.next().map_err(err)?
            .ok_or_else(|| Failure::new("not_found", format!("no memory entry {id}")))?;
        Ok(MemoryPriorRow {
            body: row.get("body").map_err(err)?,
            summary: row.get("summary").map_err(err)?,
            corrected: row.get("corrected").map_err(err)?,
            confirmed: confirmed(row)?,
        })
    }
    pub fn restore(&self, id: i64, prior: &MemoryPriorRow) -> Res<()> {
        let mut db = self.connect()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(err)?;
        if tx.execute(
            "UPDATE entries SET body = ?, summary = ?, corrected = ?, confirmed = ? WHERE id = ?",
            params![prior.body, prior.summary, prior.corrected, prior.confirmed, id],
        ).map_err(err)? == 0 {
            return Err(Failure::new("not_found", format!("no memory entry {id}")));
        }
        tx.commit().map_err(err)
    }
    /// Replaces the entry's text: `text` becomes the body and its first line the
    /// summary, so neither search nor briefs can surface the old text. The title is kept.
    pub fn correct(&self, id: i64, text: &str) -> Res<Value> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Failure::new("invalid_argument", "text is required"));
        }
        if text.lines().count() > 12 {
            return Err(Failure::new("invalid_argument", "text exceeds 12 lines"));
        }
        let summary: String = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(text)
            .trim()
            .chars()
            .take(200)
            .collect();
        let at = now();
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let dir: Option<String> = tx
            .query_row("SELECT project_dir FROM entries WHERE id = ?", [id], |r| r.get(0))
            .ok();
        if let Some(dir) = dir {
            refuse_bad_quotes(&[("text", text)], Path::new(&dir))?;
        }
        if tx
            .execute(
                "UPDATE entries SET body = ?, summary = ?, corrected = ?, confirmed = ? WHERE id = ?",
                params![text, summary, at, at, id],
            )
            .map_err(err)?
            == 0
        {
            return Err(Failure::new("not_found", format!("no memory entry {id}")));
        }
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed", "id":id, "corrected":at, "error":null}))
    }
    pub fn digest(&self, project: &str, add: Option<&str>) -> Res<Value> {
        base::require_slug("project", project)?;
        let path = self.root.join(format!("{project}.md"));
        if let Some(add) = add.filter(|s| !s.is_empty()) {
            let line = add.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(err)?;
            // One write per line: appends from separate processes stay whole.
            f.write_all(
                format!(
                    "- {} {line}
",
                    base::today()
                )
                .as_bytes(),
            )
            .map_err(err)?;
        }
        let all = match std::fs::read_to_string(path) {
            Ok(s) => s.replace("\r\n", "\n"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(err(e)),
        };
        // Hot memory: only this project's file, newest HOT_LINES lines, each cut
        // short. Older history and full entries stay behind memory_search / memory_get.
        let lines: Vec<&str> = all.lines().filter(|l| !l.trim().is_empty()).collect();
        let omitted = lines.len().saturating_sub(HOT_LINES);
        let digest: String = lines[omitted..]
            .iter()
            .map(|l| format!("{}\n", l.chars().take(HOT_LINE_CHARS).collect::<String>()))
            .collect();
        Ok(json!({"status":"completed", "digest":digest, "omitted":omitted, "error":null}))
    }
    /// Deterministic checks of one project's memory and research output.
    pub fn lint(&self, project: &str, project_dir: &Path) -> Res<Vec<lint::Problem>> {
        base::require_slug("project", project)?;
        let mut problems = Vec::new();
        let db = self.connect()?;
        let mut stmt = db
            .prepare("SELECT * FROM entries WHERE project = ? ORDER BY id")
            .map_err(err)?;
        let mut rows = stmt.query([project]).map_err(err)?;
        let mut seen: std::collections::HashMap<String, i64> = Default::default();
        while let Some(row) = rows.next().map_err(err)? {
            let id: i64 = row.get("id").map_err(err)?;
            let dir: String = row.get("project_dir").map_err(err)?;
            let (title, summary, body): (String, String, String) = (
                row.get("title").map_err(err)?,
                row.get("summary").map_err(err)?,
                row.get("body").map_err(err)?,
            );
            for (label, text) in [("body", &body), ("summary", &summary)] {
                for mut p in lint::check_quotes(text, Path::new(&dir), label) {
                    p.message = format!("memory {id} {}", p.message);
                    problems.push(p);
                }
            }
            if stale(row)? == Some(true) {
                problems.push(lint::Problem {
                    kind: "stale_source",
                    message: format!("memory {id} ({title}): a recorded source file changed or vanished since recording"),
                });
            }
            let key = lint::normalize(&format!("{title}\n{summary}\n{body}")).to_lowercase();
            if let Some(first) = seen.insert(key, id) {
                problems.push(lint::Problem {
                    kind: "duplicate",
                    message: format!("memory {id} duplicates memory {first}"),
                });
            }
        }
        let research = store_for(&self.home, project)?.join("research");
        if let Ok(dir) = std::fs::read_dir(&research) {
            let mut files: Vec<PathBuf> = dir
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect();
            files.sort();
            for file in files {
                let text = std::fs::read_to_string(&file).map_err(err)?;
                let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
                for mut p in lint::check_quotes(&text, project_dir, &name) {
                    p.message = format!("research {}", p.message);
                    problems.push(p);
                }
            }
        }
        Ok(problems)
    }
}
const HOT_LINES: usize = 12;
const HOT_LINE_CHARS: usize = 240;
fn refuse_bad_quotes(parts: &[(&str, &str)], root: &Path) -> Res<()> {
    let problems: Vec<lint::Problem> = parts
        .iter()
        .flat_map(|(label, text)| lint::check_quotes(text, root, label))
        .collect();
    let Some(first) = problems.first() else {
        return Ok(());
    };
    Err(Failure::new(
        first.kind,
        problems
            .iter()
            .map(|p| p.message.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    ))
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn confirmed(row: &Row<'_>) -> Res<f64> {
    Ok(row
        .get::<_, Option<f64>>("confirmed")
        .map_err(err)?
        .unwrap_or(row.get::<_, f64>("created").map_err(err)?))
}

#[cfg(test)]
mod prior_tests {
    use super::*;

    #[test]
    fn prior_and_restore_preserve_all_fields_and_report_missing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("a.py"), "original").unwrap();
        let store = home.join("orchestrate/p");
        std::fs::create_dir_all(&store).unwrap();
        let brief = store.join("a.md");
        std::fs::write(&brief, "GOAL test\nSCOPE a.py\nVERIFY test\n").unwrap();
        std::fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tx\tverified\t\t\t\t{}\n", brief.display())).unwrap();
        let memory = Memory::open(&home).unwrap();
        let id = memory.record(&Record {
            project: "p".into(), project_dir: project.to_string_lossy().into(), unit: "u".into(),
            title: "title".into(), summary: "old summary".into(), body: "old body".into(),
            symbols: vec![], files: vec!["a.py".into()], wish: "wish".into(),
        }).unwrap()["id"].as_i64().unwrap();
        let original = memory.prior(id).unwrap();
        assert_eq!(original.corrected, None);
        memory.correct(id, "first correction").unwrap();
        let corrected = memory.prior(id).unwrap();
        assert!(corrected.corrected.is_some());
        memory.correct(id, "second correction").unwrap();
        memory.restore(id, &corrected).unwrap();
        assert_eq!(memory.prior(id).unwrap(), corrected);
        memory.restore(id, &original).unwrap();
        let listed = memory.list(None, 10).unwrap();
        let entry = &listed["entries"][0];
        assert_eq!(entry["body"], "old body");
        assert_eq!(entry["summary"], "old summary");
        assert!(entry["corrected"].is_null());
        assert_eq!(entry["confirmed"].as_f64(), Some(original.confirmed));
        assert_eq!(memory.prior(-1).unwrap_err().kind, "not_found");
        assert_eq!(memory.restore(-1, &original).unwrap_err().kind, "not_found");
    }
}
fn file_set(files: &[Value]) -> Res<BTreeSet<(&str, &str)>> {
    files
        .iter()
        .map(|item| {
            let path = item
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| err("invalid file path"))?;
            let sha = item
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| err("invalid file hash"))?;
            Ok((path, sha))
        })
        .collect()
}
fn hash(path: &Path) -> Res<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(std::fs::read(path).map_err(err)?)
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
        let path = root.join(path);
        if !path.is_file() || hash(&path)? != expected {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}
