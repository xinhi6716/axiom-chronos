use crate::base::{self, Failure, Res};
use crate::orch::{covers, evidence, read_units, snapshot, store_for, unit_brief};
use regex::Regex;
use rusqlite::{params, params_from_iter, Connection, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    io::Write,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const RUN_MANIFEST_VERSION: &str = "axiom-run-inputs-v2";

/// Run-level memory covers the complete verified worktree, including ignored
/// files, directories and platform-specific input streams. Legacy unit memory
/// remains a list of explicitly selected files and plain SHA-256 values.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunManifest {
    version: String,
    source_digest: String,
    review_digest: String,
    entries: BTreeMap<String, String>,
}

fn source_matches_manifest(source: &str, entries: &BTreeMap<String, String>) -> bool {
    crate::orch::checkpoint::current_source_digest(source)
        && source.rsplit_once(':').is_some_and(|(_, expected)| {
            crate::orch::evidence::input_manifest_digest(entries) == expected
        })
}

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
    /// Run-level memory has the checkpoint's independent verification as its
    /// authority, rather than a model's claim that a unit passed.
    pub fn record_run(&self, project: &str, run_id: &str, content: &str) -> Res<Value> {
        self.record_run_inner(project, run_id, content, None)
    }
    /// Bind a managed memory callback to the same currently verified review
    /// that was supplied to its role. Keep `record_run` for existing callers.
    pub fn record_run_with_evidence(
        &self,
        project: &str,
        run_id: &str,
        content: &str,
        evidence_id: &str,
    ) -> Res<Value> {
        self.record_run_inner(project, run_id, content, Some(evidence_id))
    }
    fn record_run_inner(
        &self,
        project: &str,
        run_id: &str,
        content: &str,
        evidence_id: Option<&str>,
    ) -> Res<Value> {
        base::require_slug("project", project)?;
        let store = store_for(&self.home, project)?;
        let c = crate::orch::checkpoint::load_run(&store, run_id)?;
        // Historical private evaluation used for rework cannot authorize a
        // new memory record, even when the source bytes are unchanged.
        crate::orch::checkpoint::ensure_rework_history_safe(&store, &c)?;
        if !matches!(
            c.phase,
            crate::orch::checkpoint::Phase::Memory | crate::orch::checkpoint::Phase::ReadyToCommit
        ) || !crate::orch::checkpoint::verified_outcome(&c)
            || content.trim().is_empty()
            || evidence_id.is_some_and(|expected| {
                crate::orch::checkpoint::verified_review(&c)
                    .is_none_or(|reference| reference.digest != expected)
            })
        {
            return Err(Failure::new(
                "not_verified",
                "memory requires current independently verified outcomes",
            ));
        }
        let entries = crate::orch::lifecycle::contents(&c.repo.canonical_root)?;
        let source = c
            .checks
            .values()
            .next()
            .map(|check| check.snapshot.source_digest.as_str())
            .ok_or_else(|| Failure::new("not_verified", "verified source snapshot missing"))?;
        if !source_matches_manifest(source, &entries) {
            return Err(Failure::new(
                "not_verified",
                "worktree changed between verification and memory capture",
            ));
        }
        let review = crate::orch::checkpoint::verified_review(&c)
            .ok_or_else(|| Failure::new("not_verified", "verified review missing"))?;
        if evidence_id.is_some_and(|expected| expected != review.digest) {
            return Err(Failure::new(
                "not_verified",
                "verified review changed during memory capture",
            ));
        }
        let files = RunManifest {
            version: RUN_MANIFEST_VERSION.into(),
            source_digest: source.into(),
            review_digest: review.digest.clone(),
            entries,
        };
        let wish = std::fs::read_to_string(&c.intent.path).map_err(err)?;
        let files = serde_json::to_string(&files).map_err(err)?;
        let root = c.repo.canonical_root.to_string_lossy().into_owned();
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let existing = tx.query_row("SELECT id FROM entries WHERE project=? AND unit=? AND files=? AND project_dir=? LIMIT 1", params![project,run_id,files,root], |r| r.get::<_,i64>(0));
        let id = match existing {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                tx.execute("INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES(?,?,?,?,?,?,?,?,?,?)", params![project,run_id,run_id,content,content,"",wish,files,root,SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()]).map_err(err)?;
                tx.last_insert_rowid()
            }
            Err(e) => return Err(err(e)),
        };
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed","id":id,"run_id":run_id}))
    }
    pub fn record(&self, args: &Record) -> Res<Value> {
        base::require_slug("project", &args.project)?;
        base::require_slug("unit", &args.unit)?;
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
        let root = std::fs::canonicalize(root).map_err(err)?;
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
            let resolved = std::fs::canonicalize(&path).map_err(err)?;
            if !resolved.starts_with(&root) {
                return Err(Failure::new(
                    "outside_scope",
                    "file resolves outside worktree",
                ));
            }
            files.push(json!({"path": rel, "sha256": hash(&resolved)?}));
        }
        let provenance: Value = serde_json::from_slice(
            &std::fs::read(
                store
                    .join("reports")
                    .join(format!("{}.repo.json", args.unit)),
            )
            .map_err(|_| {
                Failure::new("missing_evidence", "unit repository provenance is missing")
            })?,
        )
        .map_err(err)?;
        let expected = snapshot(&root, &brief.scope);
        let rows = evidence(&store, &args.unit)?;
        let needs_accept = crate::orch::needs_accept(&store, &args.unit)?;
        if crate::orch::derive_state(&rows, needs_accept) != "verified" {
            return Err(Failure::new(
                "not_verified",
                "current unit evidence has not passed independent review and required acceptance",
            ));
        }
        let valid = rows.iter().rposition(|r| r.kind == "run").is_some_and(|i| {
            let run = &rows[i];
            !expected.is_empty()
                && run.exit_code == 0
                && run.commit == expected
                && ["scope", "verify"].iter().all(|kind| {
                    rows[i + 1..]
                        .iter()
                        .any(|r| r.kind == *kind && r.exit_code == 0 && r.commit == expected)
                })
                && (!needs_accept
                    || rows[i + 1..]
                        .iter()
                        .rev()
                        .find(|r| r.kind == "accept")
                        .is_some_and(|r| r.exit_code == 0 && r.commit == expected))
        });
        if provenance.get("repo").and_then(Value::as_str) != root.to_str()
            || provenance.get("unit").and_then(Value::as_str) != Some(args.unit.as_str())
            || provenance.get("snapshot").and_then(Value::as_str) != Some(expected.as_str())
            || !valid
        {
            return Err(Failure::new(
                "missing_evidence",
                "repository or current verified evidence does not match",
            ));
        }
        let mut db = self.connect()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let files_json = serde_json::to_string(&files).map_err(err)?;
        let canonical_root = root.to_string_lossy().into_owned();
        let symbols = args.symbols.join(" ");
        let existing = tx.query_row(
            "SELECT id FROM entries WHERE project=? AND unit=? AND title=? AND summary=? AND body=? AND symbols=? AND wish=? AND files=? AND project_dir=? LIMIT 1",
            params![args.project, args.unit, args.title.trim(), args.summary.trim(), args.body.trim(), symbols, args.wish.trim(), files_json, canonical_root],
            |row| row.get::<_, i64>(0),
        );
        match existing {
            Ok(id) => {
                return Ok(json!({"status":"completed", "id":id, "files":files, "error":null}))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(e) => return Err(err(e)),
        }
        tx.execute("INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES (?,?,?,?,?,?,?,?,?,?)", params![args.project, args.unit, args.title.trim(), args.summary.trim(), args.body.trim(), symbols, args.wish.trim(), files_json, canonical_root, created]).map_err(err)?;
        let id = tx.last_insert_rowid();
        tx.commit().map_err(err)?;
        Ok(json!({"status":"completed", "id":id, "files":files, "error":null}))
    }
    pub fn search(&self, query: &str, project: Option<&str>, limit: i64) -> Res<Value> {
        self.search_inner(query, project, None, limit, false)
    }

    /// Return only usable summaries from this exact canonical repository.
    /// Project, repository and freshness are applied before the result limit.
    pub fn search_reusable(
        &self,
        query: &str,
        project: &str,
        repo: &Path,
        limit: i64,
    ) -> Res<Value> {
        base::require_slug("project", project)?;
        if !repo.is_absolute() || !repo.is_dir() {
            return Err(Failure::new(
                "invalid_argument",
                "reusable memory requires an existing absolute repository",
            ));
        }
        let root = repo.canonicalize().map_err(err)?;
        let root = root
            .to_str()
            .ok_or_else(|| Failure::new("invalid_argument", "repository path is not Unicode"))?;
        self.search_inner(query, Some(project), Some(root), limit, true)
    }

    fn search_inner(
        &self,
        query: &str,
        project: Option<&str>,
        repo: Option<&str>,
        limit: i64,
        fresh_only: bool,
    ) -> Res<Value> {
        let terms = query_terms(query);
        if terms.is_empty() {
            if fresh_only {
                return Ok(
                    json!({"status":"completed", "terms":terms, "entries":[], "error":null}),
                );
            }
            return Err(Failure::new(
                "invalid_argument",
                "query has no searchable terms",
            ));
        }
        if fresh_only && limit <= 0 {
            return Ok(json!({"status":"completed", "terms":terms, "entries":[], "error":null}));
        }
        let hit = "(f.title LIKE ? OR f.summary LIKE ? OR f.body LIKE ? OR f.symbols LIKE ? OR f.wish LIKE ?)";
        let hits = vec![hit; terms.len()].join(" + ");
        let filter = project.filter(|p| !p.is_empty());
        let sql = format!(
            "SELECT * FROM (SELECT e.*, {hits} AS matched FROM entries e JOIN entries_fts f ON f.rowid = e.id) WHERE matched > 0{}{} ORDER BY matched DESC, created DESC{}",
            if filter.is_some() { " AND project = ?" } else { "" },
            if repo.is_some() { " AND project_dir = ?" } else { "" },
            if fresh_only { "" } else { " LIMIT ?" },
        );
        let mut values: Vec<rusqlite::types::Value> = terms
            .iter()
            .flat_map(|t| (0..5).map(move |_| format!("%{t}%").into()))
            .collect();
        if let Some(p) = filter {
            values.push(p.to_string().into());
        }
        if let Some(root) = repo {
            values.push(root.to_string().into());
        }
        if !fresh_only {
            values.push(limit.into());
        }
        let db = self.connect()?;
        let mut stmt = db.prepare(&sql).map_err(err)?;
        let mut rows = stmt.query(params_from_iter(values)).map_err(err)?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let eligibility = match self.rework_eligibility(row) {
                Ok(value) => value,
                // A single unreadable entry cannot block optional reuse.
                Err(_) if fresh_only => continue,
                Err(error) => return Err(error),
            };
            if fresh_only
                && eligibility
                    .as_ref()
                    .is_some_and(|v| v["rework_policy_eligible"] != true)
            {
                continue;
            }
            let staleness = match stale(row) {
                Ok(value) => value,
                // One malformed or unreadable legacy entry cannot block the
                // optional reuse of other entries, and is never marked fresh.
                Err(_) if fresh_only => None,
                Err(error) => return Err(error),
            };
            if fresh_only && staleness != Some(false) {
                continue;
            }
            let mut entry = json!({"id":row.get::<_, i64>("id").map_err(err)?, "project":row.get::<_, String>("project").map_err(err)?, "unit":row.get::<_, String>("unit").map_err(err)?, "title":row.get::<_, String>("title").map_err(err)?, "summary":row.get::<_, String>("summary").map_err(err)?, "matched_terms":row.get::<_, i64>("matched").map_err(err)?, "stale":staleness});
            if let Some(policy) = eligibility {
                entry["rework_policy_eligible"] = policy["rework_policy_eligible"].clone();
                entry["rework_policy_reason"] = policy["rework_policy_reason"].clone();
            }
            entries.push(entry);
            if fresh_only && entries.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
                break;
            }
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
        let mut entry = json!({"id":id, "project":row.get::<_, String>("project").map_err(err)?, "unit":row.get::<_, String>("unit").map_err(err)?, "title":row.get::<_, String>("title").map_err(err)?, "summary":row.get::<_, String>("summary").map_err(err)?, "body":row.get::<_, String>("body").map_err(err)?, "symbols":symbols.split_whitespace().collect::<Vec<_>>(), "wish":row.get::<_, String>("wish").map_err(err)?, "files":serde_json::from_str::<Value>(&files).map_err(err)?, "project_dir":row.get::<_, String>("project_dir").map_err(err)?, "created":row.get::<_, f64>("created").map_err(err)?, "stale":stale(row)?});
        if let Some(policy) = self.rework_eligibility(row)? {
            entry["rework_policy_eligible"] = policy["rework_policy_eligible"].clone();
            entry["rework_policy_reason"] = policy["rework_policy_reason"].clone();
        }
        Ok(json!({"status":"completed", "entry":entry, "error":null}))
    }

    /// Check only run identity, stored source/review metadata and the rework
    /// policy. This does not rehash the current worktree or rerun verification.
    fn rework_eligibility(&self, row: &Row<'_>) -> Res<Option<Value>> {
        let files: String = row.get("files").map_err(err)?;
        let value: Value = match serde_json::from_str(&files) {
            Ok(value) => value,
            Err(_) => return Ok(None), // stale() handles malformed legacy rows
        };
        let run_id: String = row.get("unit").map_err(err)?;
        if value.is_array() {
            return Ok(run_id
                .starts_with("run-")
                .then(|| rework_policy(None, "legacy_run_provenance_unavailable")));
        }
        let Ok(manifest) = serde_json::from_value::<RunManifest>(value) else {
            return Ok(Some(rework_policy(None, "memory_manifest_unavailable")));
        };
        if manifest.version != RUN_MANIFEST_VERSION
            || !sha256_text(&manifest.review_digest)
            || !source_matches_manifest(&manifest.source_digest, &manifest.entries)
        {
            return Ok(Some(rework_policy(None, "memory_manifest_invalid")));
        }
        let project: String = row.get("project").map_err(err)?;
        let root: String = row.get("project_dir").map_err(err)?;
        let checkpoint = store_for(&self.home, &project).and_then(|store| {
            crate::orch::checkpoint::load_run(&store, &run_id).map(|c| (store, c))
        });
        let Ok((store, c)) = checkpoint else {
            return Ok(Some(rework_policy(None, "source_run_unavailable")));
        };
        if c.repo.canonical_root.as_path() != Path::new(&root)
            || c.checks.is_empty()
            || c.checks
                .values()
                .any(|f| f.snapshot.source_digest != manifest.source_digest)
            || !c
                .receipts
                .iter()
                .any(|(key, r)| key.starts_with("review-") && r.digest == manifest.review_digest)
        {
            return Ok(Some(rework_policy(None, "source_identity_mismatch")));
        }
        match crate::orch::checkpoint::ensure_rework_history_safe(&store, &c) {
            Ok(()) => Ok(Some(rework_policy(Some(true), "rework_history_allowed"))),
            Err(failure) if failure.kind == "independent_evaluation_used" => Ok(Some(
                rework_policy(Some(false), "independent_evaluation_used"),
            )),
            Err(_) => Ok(Some(rework_policy(None, "rework_provenance_unavailable"))),
        }
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
            writeln!(f, "- {} {line}", base::today()).map_err(err)?;
        }
        let digest = match std::fs::read_to_string(path) {
            Ok(s) => s.replace("\r\n", "\n"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(err(e)),
        };
        Ok(json!({"status":"completed", "digest":digest, "error":null}))
    }
}
fn hash(path: &Path) -> Res<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(std::fs::read(path).map_err(err)?)
    ))
}

fn sha256_text(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn rework_policy(eligible: Option<bool>, reason: &str) -> Value {
    json!({"rework_policy_eligible":eligible,"rework_policy_reason":reason})
}

fn stale(row: &Row<'_>) -> Res<Option<bool>> {
    let root: String = row.get("project_dir").map_err(err)?;
    let root = Path::new(&root);
    if !root.is_dir() {
        return Ok(None);
    }
    let files: String = row.get("files").map_err(err)?;
    let files: Value = serde_json::from_str(&files).map_err(err)?;
    if files.is_object() {
        let Ok(manifest) = serde_json::from_value::<RunManifest>(files) else {
            return Ok(None);
        };
        if manifest.version != RUN_MANIFEST_VERSION
            || !sha256_text(&manifest.review_digest)
            || !source_matches_manifest(&manifest.source_digest, &manifest.entries)
        {
            return Ok(None);
        }
        // Unreadable or unsafe input cannot be treated as fresh.
        return match crate::orch::lifecycle::contents(root) {
            Ok(current) => Ok(Some(current != manifest.entries)),
            Err(_) => Ok(None),
        };
    }
    let Some(files) = files.as_array() else {
        return Ok(None);
    };
    let unit: String = row.get("unit").map_err(err)?;
    // Old run-level entries lack a versioned complete manifest. Empty legacy
    // unit lists similarly cannot establish freshness after a source change.
    if files.is_empty() || unit.starts_with("run-") {
        return Ok(None);
    }
    for item in files {
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| err("invalid file path"))?;
        let expected = item
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| err("invalid file hash"))?;
        if !sha256_text(expected) {
            return Ok(None);
        }
        let path = root.join(path);
        if !path.is_file()
            || !std::fs::canonicalize(&path).is_ok_and(|p| p.starts_with(root))
            || hash(&path)? != expected
        {
            return Ok(Some(true));
        }
    }
    Ok(Some(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};

    fn repository(path: &Path) {
        fs::create_dir_all(path).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
        fs::write(path.join("a.txt"), "before").unwrap();
    }

    // Synthetic database rows exercise compatibility and ranking; these are
    // not a substitute for a verified run's memory-writing gate.
    fn insert(
        memory: &Memory,
        project: &str,
        unit: &str,
        repo: &Path,
        summary: &str,
        files: &Value,
        created: i64,
    ) -> i64 {
        let root = repo.canonicalize().unwrap();
        let db = memory.connect().unwrap();
        db.execute(
            "INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![project, unit, "needle", summary, "", "", "needle", files.to_string(), root.to_str().unwrap(), created],
        ).unwrap();
        db.last_insert_rowid()
    }

    // Synthetic checkpoint metadata only: no private cases or complete
    // verifier evidence are required to exercise rework policy classification.
    fn seed_policy_checkpoint(
        memory: &Memory,
        repo: &Path,
        run_id: &str,
        manifest: &RunManifest,
        private: bool,
        reworked: bool,
    ) {
        use crate::orch::checkpoint::{
            self, ArtifactRef, Attempt, AttemptOutcome, AttemptState, FrozenCheck, RepoIdentity,
            RunSeed, Snapshot,
        };
        let store = store_for(&memory.home, "p").unwrap();
        fs::create_dir_all(&store).unwrap();
        let reference = |name: &str, bytes: &[u8]| {
            let path = store.join(format!("{run_id}-{name}"));
            fs::write(&path, bytes).unwrap();
            ArtifactRef {
                path,
                digest: format!("{:x}", Sha256::digest(bytes)),
            }
        };
        let intent = reference("intent", b"synthetic intent");
        let mut c = checkpoint::create_run(
            &store,
            &RunSeed {
                run_id: run_id.into(),
                repo: RepoIdentity {
                    canonical_root: repo.canonicalize().unwrap(),
                    identity_digest: "fixture".into(),
                },
                intent,
            },
        )
        .unwrap();
        let plan = reference("plan", br#"{"checks":[{"id":"public"}]}"#);
        let snapshot = Snapshot {
            source_digest: manifest.source_digest.clone(),
            plan_digest: plan.digest.clone(),
            checks_digest: "fixture".into(),
        };
        c.plan = Some(plan.clone());
        for id in if private {
            vec!["public", "independent-private"]
        } else {
            vec!["public"]
        } {
            c.checks.insert(
                id.into(),
                FrozenCheck {
                    check_id: id.into(),
                    program: "fixture".into(),
                    args: vec![],
                    env: BTreeMap::new(),
                    timeout_s: 1,
                    snapshot: snapshot.clone(),
                    spec_digest: "fixture".into(),
                },
            );
        }
        let review = reference("review", b"synthetic review");
        assert_eq!(review.digest, manifest.review_digest);
        c.receipts.insert("review-fixture".into(), review);
        if private {
            let prep = reference(
                "prep",
                br#"{"structured_output":{"status":"ready","exposure":"private","checks":[{"id":"independent-private"}]}}"#,
            );
            c.receipts.insert("evaluation-0".into(), prep.clone());
            c.attempts.insert(
                "prep-attempt".into(),
                Attempt {
                    attempt_id: "prep-attempt".into(),
                    step_id: "evaluation-prep-0".into(),
                    input_digest: "fixture".into(),
                    state: AttemptState::Terminal,
                    generation: None,
                    thread_id: None,
                    turn_id: None,
                    raw_result: Some(prep),
                    outcome: Some(AttemptOutcome::Completed),
                    last_event_sequence: 0,
                },
            );
        }
        if reworked {
            c.receipts.insert("rework-1".into(), plan);
        }
        fs::write(
            store.join("runs").join(run_id).join("checkpoint.json"),
            serde_json::to_vec(&c).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn run_manifest_is_bound_to_verified_inputs_and_detects_complete_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        repository(&repo);
        fs::write(repo.join(".gitignore"), "ignored/\n").unwrap();
        fs::create_dir(repo.join("ignored")).unwrap();
        fs::write(repo.join("ignored/input"), "before").unwrap();
        fs::create_dir(repo.join("empty")).unwrap();

        let entries = crate::orch::lifecycle::contents(&repo).unwrap();
        let source = crate::orch::snapshot(&repo, &[".".into()]);
        assert!(source_matches_manifest(&source, &entries));
        let mut different = entries.clone();
        different.insert("unreviewed".into(), "different input".into());
        assert!(!source_matches_manifest(&source, &different));
        assert!(!source_matches_manifest("tree:legacy", &entries));

        let manifest = RunManifest {
            version: RUN_MANIFEST_VERSION.into(),
            source_digest: source,
            review_digest: format!("{:x}", Sha256::digest(b"synthetic review")),
            entries,
        };
        let memory = Memory::open(&dir.path().join("home")).unwrap();
        seed_policy_checkpoint(&memory, &repo, "run-fixture", &manifest, false, false);
        let id = insert(
            &memory,
            "p",
            "run-fixture",
            &repo,
            "usable",
            &serde_json::to_value(&manifest).unwrap(),
            1,
        );
        let mut corrupt = serde_json::to_value(&manifest).unwrap();
        corrupt["entries"]["unreviewed"] = json!("different input");
        let corrupt_id = insert(&memory, "p", "run-corrupt", &repo, "invalid", &corrupt, 2);
        assert!(memory.get(corrupt_id).unwrap()["entry"]["stale"].is_null());
        assert_eq!(memory.get(id).unwrap()["entry"]["stale"], false);
        assert_eq!(
            memory.get(id).unwrap()["entry"]["files"]["version"],
            RUN_MANIFEST_VERSION
        );
        assert_eq!(
            memory.search_reusable("needle", "p", &repo, 3).unwrap()["entries"][0]["id"],
            id
        );

        // Same-length ignored content changes must invalidate, without mtime shortcuts.
        fs::write(repo.join("ignored/input"), "AFTER!").unwrap();
        assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
        assert_eq!(
            memory.search_reusable("needle", "p", &repo, 3).unwrap()["entries"],
            json!([])
        );
        fs::write(repo.join("ignored/input"), "before").unwrap();
        fs::write(repo.join("ignored/added"), "new").unwrap();
        assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
        fs::remove_file(repo.join("ignored/added")).unwrap();
        fs::remove_dir(repo.join("empty")).unwrap();
        assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
        fs::create_dir(repo.join("empty")).unwrap();
        assert_eq!(memory.get(id).unwrap()["entry"]["stale"], false);

        #[cfg(windows)]
        {
            let stream = repo.join("a.txt:axiom-memory-test");
            fs::write(&stream, "stream").unwrap();
            assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
            fs::remove_file(stream).unwrap();
        }
    }

    #[test]
    fn reusable_run_history_respects_private_rework_without_relabeling_source_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        repository(&repo);
        let memory = Memory::open(&dir.path().join("home")).unwrap();
        let manifest = RunManifest {
            version: RUN_MANIFEST_VERSION.into(),
            source_digest: crate::orch::snapshot(&repo, &[".".into()]),
            review_digest: format!("{:x}", Sha256::digest(b"synthetic review")),
            entries: crate::orch::lifecycle::contents(&repo).unwrap(),
        };
        let files = serde_json::to_value(&manifest).unwrap();
        for (run, private, reworked) in [
            ("run-private-first", true, false),
            ("run-private-rework", true, true),
            ("run-public-rework", false, true),
            ("run-unknown-rework", true, true),
        ] {
            seed_policy_checkpoint(&memory, &repo, run, &manifest, private, reworked);
        }
        let store = store_for(&memory.home, "p").unwrap();
        let unknown_path = store.join("runs/run-unknown-rework/checkpoint.json");
        let mut unknown: crate::orch::checkpoint::Checkpoint =
            serde_json::from_slice(&fs::read(&unknown_path).unwrap()).unwrap();
        unknown.receipts.remove("evaluation-0");
        fs::write(&unknown_path, serde_json::to_vec(&unknown).unwrap()).unwrap();

        let first = insert(
            &memory,
            "p",
            "run-private-first",
            &repo,
            "private first pass",
            &files,
            1,
        );
        let blocked = insert(
            &memory,
            "p",
            "run-private-rework",
            &repo,
            "private rework history",
            &files,
            2,
        );
        let public = insert(
            &memory,
            "p",
            "run-public-rework",
            &repo,
            "public rework",
            &files,
            3,
        );
        let missing = insert(
            &memory,
            "p",
            "run-unknown-rework",
            &repo,
            "unknown provenance",
            &files,
            4,
        );
        let legacy = insert(
            &memory,
            "p",
            "legacy-unit",
            &repo,
            "valid legacy",
            &json!([{"path":"a.txt","sha256":hash(&repo.join("a.txt")).unwrap()}]),
            5,
        );

        let saved = memory.get(blocked).unwrap();
        assert_eq!(
            saved["entry"]["stale"], false,
            "source freshness is separate"
        );
        assert_eq!(saved["entry"]["rework_policy_eligible"], false);
        assert_eq!(
            saved["entry"]["rework_policy_reason"],
            "independent_evaluation_used"
        );
        assert_eq!(saved["entry"]["body"], "", "history is preserved");
        let unknown_entry = memory.get(missing).unwrap();
        assert_eq!(unknown_entry["entry"]["stale"], false);
        assert!(unknown_entry["entry"]["rework_policy_eligible"].is_null());
        assert_eq!(
            unknown_entry["entry"]["rework_policy_reason"],
            "rework_provenance_unavailable"
        );
        assert_eq!(
            memory.get(first).unwrap()["entry"]["rework_policy_eligible"],
            true
        );
        assert_eq!(
            memory.get(public).unwrap()["entry"]["rework_policy_eligible"],
            true
        );

        let hits = memory.search_reusable("needle", "p", &repo, 10).unwrap();
        let ids: Vec<i64> = hits["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_i64().unwrap())
            .collect();
        assert!(ids.contains(&first));
        assert!(ids.contains(&public));
        assert!(ids.contains(&legacy));
        assert!(!ids.contains(&blocked));
        assert!(!ids.contains(&missing));

        let before: i64 = memory
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            memory
                .record_run("p", "run-private-rework", "must not be recorded")
                .unwrap_err()
                .kind,
            "independent_evaluation_used"
        );
        let after: i64 = memory
            .connect()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            memory.get(blocked).unwrap()["entry"]["summary"],
            "private rework history"
        );
    }

    #[test]
    fn reusable_search_filters_repository_and_unknown_or_stale_before_limit() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let other = dir.path().join("other");
        repository(&first);
        repository(&other);
        let memory = Memory::open(&dir.path().join("home")).unwrap();
        let good = json!([{"path":"a.txt","sha256":hash(&first.join("a.txt")).unwrap()}]);
        let bad = json!([{"path":"a.txt","sha256":"0".repeat(64)}]);
        let expected = insert(&memory, "p", "unit-good", &first, "right repo", &good, 1);
        for i in 0..3 {
            insert(
                &memory,
                "p",
                &format!("unit-stale-{i}"),
                &first,
                "stale",
                &bad,
                10 + i,
            );
            insert(
                &memory,
                "p",
                &format!("unit-other-{i}"),
                &other,
                "other repo secret",
                &good,
                20 + i,
            );
        }
        insert(
            &memory,
            "other-project",
            "unit-x",
            &first,
            "other project",
            &good,
            30,
        );
        insert(
            &memory,
            "p",
            "unit-empty",
            &first,
            "no file provenance",
            &json!([]),
            40,
        );
        insert(
            &memory,
            "p",
            "run-old",
            &first,
            "old run provenance unknown",
            &good,
            50,
        );
        let db = memory.connect().unwrap();
        db.execute(
            "INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params!["p", "bad-json", "needle", "corrupt", "", "", "needle", "not-json", first.canonicalize().unwrap().to_str().unwrap(), 100_i64],
        ).unwrap();
        db.execute(
            "INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params!["p", "bad-blob", "needle", "unreadable", "", "", "needle", rusqlite::types::Value::Blob(vec![0xff]), first.canonicalize().unwrap().to_str().unwrap(), 101_i64],
        ).unwrap();
        drop(db);

        let result = memory.search_reusable("needle", "p", &first, 3).unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], expected);
        assert_eq!(entries[0]["summary"], "right repo");
        assert!(!result.to_string().contains("other repo secret"));
        assert_eq!(
            memory.search_reusable("+", "p", &first, 3).unwrap()["entries"],
            json!([])
        );
        assert_eq!(
            memory.search_reusable("x", "p", &first, 3).unwrap()["entries"],
            json!([])
        );
        assert_eq!(
            memory.search("+", Some("p"), 3).unwrap_err().kind,
            "invalid_argument"
        );
        assert!(memory.search("needle", Some("p"), 20).is_err());
    }
}
