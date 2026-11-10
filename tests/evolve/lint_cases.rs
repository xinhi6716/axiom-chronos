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
fn old_database_migrates_without_changing_other_files() {
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
    // Memory::open migrates old schemas; compare all files outside memory/ byte-for-byte.
    let outside = |root: &Path| {
        tree(root)
            .into_iter()
            .filter(|(p, _)| !p.starts_with("home/memory"))
            .collect::<BTreeMap<_, _>>()
    };
    let before = outside(tmp.path());
    let (_, status) = collect_lint(&home, &[project]).unwrap();
    assert_eq!(status.samples, 1);
    assert_eq!(before, outside(tmp.path()));
    let conn = rusqlite::Connection::open(&db).unwrap();
    let (confirmed, corrected): (f64, Option<f64>) = conn
        .query_row(
            "SELECT confirmed, corrected FROM entries WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(confirmed, 123.0);
    assert_eq!(corrected, None);
}
