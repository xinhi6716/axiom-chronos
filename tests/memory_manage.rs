#[path = "common/race.rs"]
mod race;

use axiom::memory::{Memory, Record};
use race::{child, children};
use rusqlite::Connection;
use std::{fs, path::Path};

const OLD_SCHEMA: &str = "
CREATE TABLE entries (
    id INTEGER PRIMARY KEY, project TEXT NOT NULL, unit TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL,
    body TEXT NOT NULL, symbols TEXT NOT NULL, wish TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL, created REAL NOT NULL);
CREATE VIRTUAL TABLE entries_fts USING fts5(
    title, summary, body, symbols, wish, content='entries', content_rowid='id', tokenize='trigram');
CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, title, summary, body, symbols, wish)
    VALUES (new.id, new.title, new.summary, new.body, new.symbols, new.wish);
END;
";

fn db_path(home: &Path) -> std::path::PathBuf {
    home.join("memory/memory.db")
}

/// A database exactly as the previous version created it, holding `n` entries.
fn old_database(home: &Path, n: usize) {
    fs::create_dir_all(home.join("memory")).unwrap();
    let db = Connection::open(db_path(home)).unwrap();
    db.execute_batch(OLD_SCHEMA).unwrap();
    for i in 0..n {
        db.execute(
            "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES ('p', ?, ?, 'old summary', 'legacyneedle body', '', 'wish', '[]', '/nowhere', ?)",
            rusqlite::params![format!("u{i}"), format!("title {i}"), 1000.0 + i as f64],
        )
        .unwrap();
    }
}

fn fts_hits(home: &Path, phrase: &str) -> i64 {
    Connection::open(db_path(home))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM entries_fts WHERE entries_fts MATCH ?",
            [format!("\"{phrase}\"")],
            |r| r.get(0),
        )
        .unwrap()
}

fn ids(v: &serde_json::Value) -> Vec<i64> {
    v["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_i64().unwrap())
        .collect()
}

/// A home with a verified unit `u` (scope a.py) in project `p`, and one recorded entry.
fn recorded() -> (tempfile::TempDir, Memory, i64) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("a.py"), "original").unwrap();
    let store = home.join("orchestrate/p");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("a.md");
    fs::write(&brief, "GOAL test\nSCOPE a.py\nVERIFY test\n").unwrap();
    fs::write(
        store.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tx\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    let memory = Memory::open(&home).unwrap();
    let r = memory
        .record(&Record {
            project: "p".into(),
            project_dir: project.to_string_lossy().into(),
            unit: "u".into(),
            title: "cache layer".into(),
            summary: "alphasummary".into(),
            body: "alphabody detail".into(),
            symbols: vec![],
            files: vec!["a.py".into()],
            wish: "speed up".into(),
        })
        .unwrap();
    let id = r["id"].as_i64().unwrap();
    (temp, memory, id)
}

#[test]
fn list_returns_full_entries_newest_first_with_staleness() {
    let (temp, memory, id) = recorded();
    let all = memory.list(None, 10).unwrap();
    let e = &all["entries"][0];
    assert_eq!(e["id"], id);
    assert_eq!(e["project"], "p");
    assert_eq!(e["title"], "cache layer");
    assert_eq!(e["summary"], "alphasummary");
    assert_eq!(e["body"], "alphabody detail");
    assert_eq!(e["wish"], "speed up");
    assert_eq!(e["stale"], false);
    assert!(e["corrected"].is_null());
    assert!(e["created"].as_f64().unwrap() > 0.0);
    assert_eq!(e["confirmed"], e["created"]);

    fs::write(temp.path().join("project/a.py"), "changed").unwrap();
    assert_eq!(memory.list(None, 10).unwrap()["entries"][0]["stale"], true);

    // Newest first, project filter, limit.
    let db = Connection::open(db_path(&temp.path().join("home"))).unwrap();
    for (unit, project, created) in [("n1", "q", 5.0e9), ("n2", "p", 5.0e9 + 1.0)] {
        db.execute(
            "INSERT INTO entries (project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES (?, ?, 't', 's', 'b', '', 'w', '[]', '/nowhere', ?)",
            rusqlite::params![project, unit, created],
        )
        .unwrap();
    }
    let listed = memory.list(None, 10).unwrap();
    assert_eq!(ids(&listed), vec![3, 2, id]);
    assert_eq!(listed["entries"][0]["stale"], serde_json::Value::Null);
    assert_eq!(ids(&memory.list(Some("p"), 10).unwrap()), vec![3, id]);
    assert_eq!(ids(&memory.list(None, 1).unwrap()), vec![3]);
}

#[test]
fn correct_replaces_text_for_search_get_and_index() {
    let (temp, memory, id) = recorded();
    let home = temp.path().join("home");
    assert_eq!(ids(&memory.search("alphabody", None, 5).unwrap()), vec![id]);
    assert_eq!(fts_hits(&home, "alphabody"), 1);

    memory
        .correct(id, "betafix replaced the cache\nsecond line")
        .unwrap();

    assert!(ids(&memory.search("alphabody", None, 5).unwrap()).is_empty());
    assert!(ids(&memory.search("alphasummary", None, 5).unwrap()).is_empty());
    assert_eq!(ids(&memory.search("betafix", None, 5).unwrap()), vec![id]);
    assert_eq!(fts_hits(&home, "alphabody"), 0);
    assert_eq!(fts_hits(&home, "betafix"), 1);

    let got = memory.get(id).unwrap();
    assert_eq!(
        got["entry"]["body"],
        "betafix replaced the cache\nsecond line"
    );
    assert_eq!(got["entry"]["summary"], "betafix replaced the cache");
    assert_eq!(got["entry"]["title"], "cache layer");
    let listed = memory.list(None, 5).unwrap();
    let e = &listed["entries"][0];
    assert!(e["corrected"].as_f64().unwrap() >= e["created"].as_f64().unwrap());
    assert_eq!(e["confirmed"], e["corrected"]);
}

#[test]
fn correct_rejects_empty_missing_and_overlong() {
    let (_t, memory, id) = recorded();
    for bad in ["", "  \n\t "] {
        assert_eq!(
            memory.correct(id, bad).unwrap_err().kind,
            "invalid_argument"
        );
    }
    let long = (0..13)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        memory.correct(id, &long).unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        memory.correct(id + 99, "x text").unwrap_err().kind,
        "not_found"
    );
    // Rejected corrections leave the entry untouched.
    assert_eq!(memory.get(id).unwrap()["entry"]["body"], "alphabody detail");
    assert!(memory.list(None, 1).unwrap()["entries"][0]["corrected"].is_null());
}

#[test]
fn delete_removes_entry_and_search_row() {
    let (temp, memory, id) = recorded();
    let home = temp.path().join("home");
    assert_eq!(fts_hits(&home, "alphabody"), 1);
    memory.delete(id).unwrap();
    assert!(ids(&memory.search("alphabody", None, 5).unwrap()).is_empty());
    assert_eq!(fts_hits(&home, "alphabody"), 0);
    assert_eq!(memory.get(id).unwrap_err().kind, "not_found");
    assert!(memory.list(None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(memory.delete(id).unwrap_err().kind, "not_found");
}

#[test]
fn old_schema_database_migrates_keeping_every_entry() {
    let t = tempfile::tempdir().unwrap();
    old_database(t.path(), 3);
    let memory = Memory::open(t.path()).unwrap();
    let listed = memory.list(None, 10).unwrap();
    assert_eq!(ids(&listed), vec![3, 2, 1]);
    // Confirmed falls back to creation for entries that predate the column.
    assert_eq!(listed["entries"][2]["confirmed"], 1000.0);
    assert!(listed["entries"][2]["corrected"].is_null());
    assert_eq!(
        ids(&memory.search("legacyneedle", None, 10).unwrap()).len(),
        3
    );

    // The new triggers work on the migrated database.
    memory.correct(2, "freshtext only").unwrap();
    memory.delete(3).unwrap();
    assert_eq!(
        ids(&memory.search("legacyneedle", None, 10).unwrap()),
        vec![1]
    );
    assert_eq!(fts_hits(t.path(), "legacyneedle"), 1);
    assert_eq!(ids(&memory.search("freshtext", None, 10).unwrap()), vec![2]);
    // Reopening an already migrated database is a no-op.
    let again = Memory::open(t.path()).unwrap();
    assert_eq!(ids(&again.list(None, 10).unwrap()), vec![2, 1]);
}

#[test]
fn many_processes_opening_an_old_database_migrate_it_and_keep_entries() {
    const OPENERS: usize = 8;
    if let Some((dir, i)) = child() {
        let memory = Memory::open(&dir).unwrap();
        assert_eq!(
            memory.list(None, 100).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        // Each of the first four corrects its own entry while the others still open.
        if i < 4 {
            memory
                .correct(i as i64 + 1, &format!("racetext{i}"))
                .unwrap();
        }
        return;
    }
    let t = tempfile::tempdir().unwrap();
    old_database(t.path(), 4);
    children(
        "many_processes_opening_an_old_database_migrate_it_and_keep_entries",
        t.path(),
        OPENERS,
    );
    let memory = Memory::open(t.path()).unwrap();
    assert_eq!(ids(&memory.list(None, 100).unwrap()).len(), 4);
    assert!(ids(&memory.search("legacyneedle", None, 10).unwrap()).is_empty());
    for i in 0..4 {
        assert_eq!(
            ids(&memory.search(&format!("racetext{i}"), None, 10).unwrap()),
            vec![i + 1]
        );
    }
}
