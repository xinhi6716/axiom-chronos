use axiom::{
    base,
    evolve::{collect::collect, lint::collect_lint, Inputs},
    memory::Memory,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        out.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            if metadata.is_dir() {
                None
            } else {
                Some(fs::read(path).unwrap())
            },
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
#[test]
fn missing_database_is_unobserved_without_creating_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let (events, status) = collect_lint(&home, &[project]).unwrap();
    assert!(events.is_empty());
    assert_eq!(status.state, "unobserved");
    assert!(!home.exists());
}
#[test]
fn current_lint_finds_bad_research_quote_and_collect_uses_all_in_window_goals() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("a.txt"), "actual").unwrap();
    Memory::open(&home).unwrap();
    let slug = base::slug_for_dir(&project);
    let research = home.join("orchestrate").join(&slug).join("research");
    fs::create_dir_all(&research).unwrap();
    fs::write(research.join("plan.md"), "> \"invented\" - a.txt\n").unwrap();
    let (events, status) = collect_lint(&home, &[project.clone(), project.clone()]).unwrap();
    assert_eq!(status.samples, 1);
    assert_eq!(status.state, "observed");
    assert!(events.iter().any(|e| e.problem == "lint.fabricated_quote"
        && e.snapshot
        && e.detail.contains("current snapshot")));
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(
        home.join("goals/s.json"),
        serde_json::json!({"wish":"x", "status":"done", "cwd":project}).to_string(),
    )
    .unwrap();
    let input = Inputs {
        home,
        projects: tmp.path().join("projects"),
        now_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
        since_days: 7,
    };
    // Check the already-migrated database and all other files byte-for-byte.
    let before = tree(&input.home);
    let result = collect(&input).unwrap();
    assert_eq!(before, tree(&input.home));
    assert!(result
        .events
        .iter()
        .any(|e| e.problem == "lint.fabricated_quote"));
}

#[test]
fn old_database_is_read_without_migration_or_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("keep.txt"), "project unchanged").unwrap();
    fs::create_dir_all(home.join("memory")).unwrap();
    fs::create_dir_all(home.join("other")).unwrap();
    fs::write(home.join("other/keep.txt"), "unchanged").unwrap();
    let db = home.join("memory/memory.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE entries (id INTEGER PRIMARY KEY, project TEXT NOT NULL, unit TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL, body TEXT NOT NULL, symbols TEXT NOT NULL, wish TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL, created REAL NOT NULL); INSERT INTO entries VALUES (1, 'p', 'u', 't', 's', 'b', '', '', '', '', 123);").unwrap();
    }
    let before = tree(tmp.path());
    let (_, status) = collect_lint(&home, &[project]).unwrap();
    assert_eq!(status.samples, 1);
    assert_eq!(before, tree(tmp.path()));
    let input = Inputs {
        home: home.clone(),
        projects: tmp.path().join("projects"),
        now_ms: 1_700_000_600_000,
        since_days: 7,
    };
    axiom::evolve::preview(&input).unwrap();
    assert_eq!(before, tree(tmp.path()));
    let conn = rusqlite::Connection::open(&db).unwrap();
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('entries')")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(!columns.iter().any(|c| c == "confirmed" || c == "corrected"));
}

#[test]
fn read_only_lint_matches_memory_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("source.txt"), "real").unwrap();
    let memory = Memory::open(&home).unwrap();
    let slug = base::slug_for_dir(&project);
    let conn = rusqlite::Connection::open(home.join("memory/memory.db")).unwrap();
    for id in 1..=2 {
        conn.execute("INSERT INTO entries (id, project, unit, title, summary, body, symbols, wish, files, project_dir, created) VALUES (?1, ?2, 'u', 'title', 'summary', ?3, '', '', ?4, ?5, 1)", rusqlite::params![id, slug, "> \"invented\" - source.txt", r#"[{"path":"missing","sha256":"x"}]"#, project.to_string_lossy()]).unwrap();
    }
    drop(conn);
    let expected: Vec<_> = memory
        .lint(&slug, &project)
        .unwrap()
        .into_iter()
        .map(|p| (p.kind.to_owned(), p.message))
        .collect();
    let before = tree(&home);
    let (events, _) = collect_lint(&home, &[project]).unwrap();
    let actual: Vec<_> = events
        .into_iter()
        .map(|e| {
            (
                e.problem.strip_prefix("lint.").unwrap().to_owned(),
                e.detail
                    .strip_prefix("current snapshot: ")
                    .unwrap()
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(before, tree(&home));
}

#[test]
fn preview_reads_wal_without_creating_home_shared_memory() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("source.txt"), "actual").unwrap();
    let original = tmp.path().join("original.db");
    let conn = rusqlite::Connection::open(&original).unwrap();
    conn.execute_batch("CREATE TABLE entries (id INTEGER PRIMARY KEY, project TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL, body TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL); PRAGMA journal_mode=WAL;").unwrap();
    conn.execute(
        "INSERT INTO entries VALUES (1, ?1, 'title', 'summary', ?2, '[]', ?3)",
        rusqlite::params![
            base::slug_for_dir(&project),
            "> \"invented\" - source.txt",
            project.to_string_lossy()
        ],
    )
    .unwrap();
    fs::create_dir_all(home.join("memory")).unwrap();
    fs::copy(&original, home.join("memory/memory.db")).unwrap();
    fs::copy(
        tmp.path().join("original.db-wal"),
        home.join("memory/memory.db-wal"),
    )
    .unwrap();
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(
        home.join("goals/s.json"),
        serde_json::json!({"wish":"x", "status":"done", "cwd":project}).to_string(),
    )
    .unwrap();
    let input = Inputs {
        home: home.clone(),
        projects: tmp.path().join("projects"),
        now_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64,
        since_days: 7,
    };
    let before = tree(&home);
    let preview = axiom::evolve::preview(&input).unwrap();
    assert!(preview
        .digest
        .problems
        .iter()
        .any(|p| p.id == "lint.fabricated_quote"));
    assert_eq!(tree(&home), before);
}
