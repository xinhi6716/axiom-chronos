use super::{Event, Source, SourceStatus};
use crate::{
    base::{self, Failure, Res},
    memory::lint::{self, Problem},
};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub fn collect_lint(home: &Path, project_dirs: &[PathBuf]) -> Res<(Vec<Event>, SourceStatus)> {
    let db = home.join("memory/memory.db");
    let mut status = SourceStatus {
        family: "lint".into(),
        state: "unobserved".into(),
        samples: 0,
        note: String::new(),
    };
    if !db.is_file() {
        return Ok((vec![], status));
    }
    // SQLite may create or change the shared-memory file even for a read-only
    // WAL connection. Read a temporary copy in that case, never the home WAL.
    let snapshot = WalSnapshot::new(home, &db)?;
    let db_conn = rusqlite::Connection::open_with_flags(
        snapshot.as_ref().map_or(db.as_path(), |s| s.db.as_path()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(io)?;
    let mut events = Vec::new();
    let dirs: BTreeSet<_> = project_dirs.iter().filter(|p| p.is_dir()).collect();
    for dir in dirs {
        status.samples += 1;
        let project = base::slug_for_dir(dir);
        for (i, problem) in lint_project(&db_conn, home, &project, dir)?
            .into_iter()
            .enumerate()
        {
            let location = db.display().to_string();
            events.push(Event {
                problem: format!("lint.{}", problem.kind),
                title: problem.kind.replace('_', " "),
                count: 1,
                cost_usd: None,
                unpriced: 0,
                detail: format!("current snapshot: {}", problem.message),
                snapshot: true,
                source: Source {
                    family: "lint".into(),
                    location: location.clone(),
                    session: None,
                    project: Some(project.clone()),
                    unit: None,
                    ts_ms: None,
                    key: super::collect::source_key(&[
                        "lint",
                        &location,
                        &dir.display().to_string(),
                        &i.to_string(),
                    ]),
                },
            });
        }
    }
    if status.samples > 0 {
        status.state = "observed".into();
    }
    Ok((events, status))
}

fn io(e: impl std::fmt::Display) -> Failure {
    Failure::new("io", e.to_string())
}

struct WalSnapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl WalSnapshot {
    fn new(home: &Path, db: &Path) -> Res<Option<Self>> {
        let wal = db.with_file_name("memory.db-wal");
        if !wal.is_file() {
            return Ok(None);
        }
        let temp = fs::canonicalize(std::env::temp_dir()).map_err(io)?;
        if temp.starts_with(fs::canonicalize(home).map_err(io)?) {
            return Err(io("temporary directory is inside home"));
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = loop {
            let dir = temp.join(format!(
                "axiom-evolve-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io(e)),
            }
        };
        let snapshot = Self {
            db: dir.join("memory.db"),
            dir,
        };
        fs::copy(db, &snapshot.db).map_err(io)?;
        fs::copy(wal, snapshot.dir.join("memory.db-wal")).map_err(io)?;
        Ok(Some(snapshot))
    }
}

impl Drop for WalSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// Mirrors the deterministic memory lint checks without opening or migrating the home database.
fn lint_project(
    db: &rusqlite::Connection,
    home: &Path,
    project: &str,
    dir: &Path,
) -> Res<Vec<Problem>> {
    use sha2::{Digest, Sha256};
    let mut problems = Vec::new();
    let mut stmt = db.prepare("SELECT id, project_dir, title, summary, body, files FROM entries WHERE project = ? ORDER BY id").map_err(io)?;
    let mut rows = stmt.query([project]).map_err(io)?;
    let mut seen = HashMap::<String, i64>::new();
    while let Some(row) = rows.next().map_err(io)? {
        let id: i64 = row.get(0).map_err(io)?;
        let root: String = row.get(1).map_err(io)?;
        let title: String = row.get(2).map_err(io)?;
        let summary: String = row.get(3).map_err(io)?;
        let body: String = row.get(4).map_err(io)?;
        for (label, text) in [("body", &body), ("summary", &summary)] {
            for mut problem in lint::check_quotes(text, Path::new(&root), label) {
                problem.message = format!("memory {id} {}", problem.message);
                problems.push(problem);
            }
        }
        let root = Path::new(&root);
        if root.is_dir() {
            let files: String = row.get(5).map_err(io)?;
            let files: Vec<serde_json::Value> = serde_json::from_str(&files).map_err(io)?;
            for item in files {
                let path = item
                    .get("path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| io("invalid file path"))?;
                let expected = item
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| io("invalid file hash"))?;
                let path = root.join(path);
                if !path.is_file()
                    || format!("{:x}", Sha256::digest(fs::read(path).map_err(io)?)) != expected
                {
                    problems.push(Problem { kind: "stale_source", message: format!("memory {id} ({title}): a recorded source file changed or vanished since recording") });
                    break;
                }
            }
        }
        let key = lint::normalize(&format!("{title}\n{summary}\n{body}")).to_lowercase();
        if let Some(first) = seen.insert(key, id) {
            problems.push(Problem {
                kind: "duplicate",
                message: format!("memory {id} duplicates memory {first}"),
            });
        }
    }
    let research = home.join("orchestrate").join(project).join("research");
    if let Ok(entries) = fs::read_dir(research) {
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "md"))
            .collect();
        files.sort();
        for file in files {
            let text = fs::read_to_string(&file).map_err(io)?;
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            for mut problem in lint::check_quotes(&text, dir, &name) {
                problem.message = format!("research {}", problem.message);
                problems.push(problem);
            }
        }
    }
    Ok(problems)
}
