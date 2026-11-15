use axiom::graph::{
    code::{
        health,
        index::sync::{sync_with_hook, sync_with_versions},
        refs_at, sync, SyncOptions,
    },
    GraphStore,
};
use std::{fs, path::Path, process::Command};
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(d.path())
        .status()
        .unwrap()
        .success());
    d
}
fn write(p: &Path, s: &str) {
    fs::write(p, s).unwrap()
}
fn setup() -> (tempfile::TempDir, tempfile::TempDir, GraphStore) {
    let home = tempfile::tempdir().unwrap();
    let repo = repo();
    let store = GraphStore::open(home.path()).unwrap();
    (home, repo, store)
}
#[test]
fn changes_and_no_reparse() {
    let (_home, repo, store) = setup();
    let p = repo.path();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph/code/index/main.py"),
        p.join("main.py"),
    )
    .unwrap();
    let a = sync(&store, p, None, &SyncOptions::default()).unwrap();
    let get = || {
        refs_at(&store, &a.scope, "main.py", 4)
            .unwrap()
            .into_iter()
            .find(|r| r.name == "work")
            .unwrap()
    };
    assert_ne!(get().resolution, "resolved");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph/code/index/lib.py"),
        p.join("lib.py"),
    )
    .unwrap();
    let b = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(b.parsed, vec!["lib.py"]);
    assert!(b.reresolved > 0);
    assert_eq!(get().resolution, "resolved");
    let c = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert!(c.parsed.is_empty());
    assert_eq!(c.reresolved, 0);
    write(&p.join("lib.py"), "def wark():\n    pass\n");
    let d = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(d.parsed, vec!["lib.py"]);
    assert_ne!(get().resolution, "resolved");
    assert_eq!(get().reason, "target removed");
    fs::rename(p.join("lib.py"), p.join("other.py")).unwrap();
    let e = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(e.removed, vec!["lib.py"]);
    assert_eq!(e.parsed, vec!["other.py"]);
    assert_eq!(get().reason, "target removed");
    fs::remove_file(p.join("other.py")).unwrap();
    let f = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(f.removed, vec!["other.py"]);
}
#[test]
fn deleted_target_and_race_and_versions() {
    let (home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("main.py"), "from lib import work\nwork()\n");
    write(&p.join("lib.py"), "def work():\n    pass\n");
    let a = sync(&store, p, None, &SyncOptions::default()).unwrap();
    fs::remove_file(p.join("lib.py")).unwrap();
    sync(&store, p, None, &SyncOptions::default()).unwrap();
    let r = refs_at(&store, &a.scope, "main.py", 2)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "work")
        .unwrap();
    assert_eq!(r.resolution, "unresolved");
    assert_eq!(r.reason, "target removed");
    write(&p.join("unrelated.py"), "def unrelated(): pass\n");
    let unrelated = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(unrelated.reresolved, 0);
    let still_removed = refs_at(&store, &a.scope, "main.py", 2)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "work")
        .unwrap();
    assert_eq!(still_removed.reason, "target removed");
    fs::remove_file(p.join("unrelated.py")).unwrap();
    // Editing the importer ends the special reason even when the target is still absent.
    write(
        &p.join("main.py"),
        "from lib import work\nwork()  # edited\n",
    );
    sync(&store, p, None, &SyncOptions::default()).unwrap();
    let edited = refs_at(&store, &a.scope, "main.py", 2)
        .unwrap()
        .into_iter()
        .find(|r| r.name == "work")
        .unwrap();
    assert_ne!(edited.reason, "target removed");
    let before_race = refs_at(&store, &a.scope, "main.py", 2).unwrap();
    write(&p.join("main.py"), "def changed():\n    pass\n");
    let race = sync_with_hook(&store, p, None, &SyncOptions::default(), || {
        write(&p.join("main.py"), "def changed_again():\n    pass\n")
    })
    .unwrap();
    assert!(race.partial);
    assert!(race.partial_reason.is_some());
    assert_eq!(health(home.path(), p).unwrap().state, "stale");
    let after_race = refs_at(&store, &a.scope, "main.py", 2).unwrap();
    assert_eq!(
        serde_json::to_value(&after_race).unwrap(),
        serde_json::to_value(&before_race).unwrap()
    );
    assert!(!after_race.is_empty());
    write(&p.join("second.py"), "def second(): pass\n");
    sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert!(sync(&store, p, None, &SyncOptions::default())
        .unwrap()
        .parsed
        .is_empty());
    let v = sync_with_versions(
        &store,
        p,
        None,
        &SyncOptions::default(),
        "new-parser",
        "new-resolver",
    )
    .unwrap();
    assert_eq!(v.parsed, vec!["main.py", "second.py"]);
}
#[test]
fn filters_and_read_only_health() {
    let (home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("ok.py"), "def ok(): pass\n");
    write(&p.join("gen.py"), "# @generated\ndef no(): pass\n");
    write(&p.join("binary.py"), "hi\0there");
    write(&p.join(".gitignore"), "ignored.py\n");
    write(&p.join("ignored.py"), "def no(): pass\n");
    fs::create_dir(p.join("node_modules")).unwrap();
    write(&p.join("node_modules/no.js"), "function no(){}\n");
    let outside = tempfile::tempdir().unwrap();
    write(&outside.path().join("outside.py"), "def no(): pass\n");
    #[cfg(unix)]
    let link = std::os::unix::fs::symlink(outside.path().join("outside.py"), p.join("escape.py"));
    #[cfg(windows)]
    let link =
        std::os::windows::fs::symlink_file(outside.path().join("outside.py"), p.join("escape.py"));
    let r = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(r.parsed, vec!["ok.py"]);
    if let Err(e) = &link {
        eprintln!("skip escaping symlink check: {e}");
        let (candidates, skipped) = axiom::graph::code::index::files::discover(p).unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|c| c.path.as_str())
                .collect::<Vec<_>>(),
            vec!["ok.py"]
        );
        assert!(skipped
            .iter()
            .any(|(path, reason)| path == "gen.py" && reason == "generated"));
        assert!(skipped
            .iter()
            .any(|(path, reason)| path == "binary.py" && reason == "binary"));
    } else {
        assert!(r
            .skipped
            .iter()
            .any(|(p, s)| p == "escape.py" && s.contains("escapes")));
    }
    assert_eq!(health(home.path(), p).unwrap().state, "ready");
    let empty = tempfile::tempdir().unwrap();
    assert_eq!(health(empty.path(), p).unwrap().state, "missing");
    assert!(!empty.path().join("memory/memory.db").exists());
}
#[test]
fn worktree_scopes_are_separate() {
    let (home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("a.py"), "def a(): pass\n");
    assert!(Command::new("git")
        .args(["add", "a.py"])
        .current_dir(p)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args([
            "-c",
            "user.name=Index Test",
            "-c",
            "user.email=index@example.invalid",
            "commit",
            "-qm",
            "initial"
        ])
        .current_dir(p)
        .status()
        .unwrap()
        .success());
    let parent = tempfile::tempdir().unwrap();
    let second = parent.path().join("second");
    assert!(Command::new("git")
        .args(["worktree", "add", "-qb", "index-test"])
        .arg(&second)
        .current_dir(p)
        .status()
        .unwrap()
        .success());
    write(&second.join("a.py"), "def b(): pass\n");
    let a = sync(&store, p, Some(p), &SyncOptions::default()).unwrap();
    let b = sync(&store, &second, Some(&second), &SyncOptions::default()).unwrap();
    assert_eq!(a.scope.project_key, b.scope.project_key);
    assert_ne!(a.scope.id, b.scope.id);
    assert_eq!(a.parsed, vec!["a.py"]);
    assert_eq!(b.parsed, vec!["a.py"]);
    assert_eq!(health(home.path(), p).unwrap().state, "ready");
    assert_eq!(health(home.path(), p).unwrap().files, 1);
    write(&second.join("b.py"), "def b(): pass\n");
    sync(&store, &second, Some(&second), &SyncOptions::default()).unwrap();
    assert_eq!(health(home.path(), p).unwrap().files, 2);
    assert!(sync(&store, p, Some(p), &SyncOptions::default())
        .unwrap()
        .parsed
        .is_empty());
    assert_eq!(health(home.path(), &second).unwrap().files, 1);
}
#[test]
fn partial_keeps_old_rows_and_health_reports_it() {
    let (home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("a.py"), "def a(): pass\n");
    let first = sync(&store, p, None, &SyncOptions::default()).unwrap();
    write(&p.join("b.py"), "def b(): pass\n");
    let mut opts = SyncOptions::default();
    opts.limits.files = 1;
    let r = sync(&store, p, None, &opts).unwrap();
    assert!(r.partial);
    assert!(r.removed.is_empty());
    assert_eq!(health(home.path(), p).unwrap().state, "partial");
    let again = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(again.scope.id, first.scope.id);
    assert_eq!(again.parsed, vec!["b.py"]);
    assert_eq!(health(home.path(), p).unwrap().state, "ready");
}

#[test]
fn broken_files_do_not_block_or_reparse() {
    let (_home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("a.rs"), "fn broken( {");
    fs::write(p.join("b.py"), [0xff, 0xfe]).unwrap();
    write(&p.join("c.py"), "def good(): pass\n");
    let first = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert!(!first.partial);
    assert_eq!(first.parsed, vec!["a.rs", "b.py", "c.py"]);
    assert_eq!(first.parse_failures.len(), 2);
    let second = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert!(!second.partial);
    assert!(second.parsed.is_empty());
    assert_eq!(second.unchanged, 3);
    assert_eq!(second.parse_failures.len(), 2);
}
#[test]
fn all_limits_and_deadline_preserve_rows() {
    let (home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("old.py"), "def old(): pass\nold()\n");
    let first = sync(&store, p, None, &SyncOptions::default()).unwrap();
    let old = refs_at(&store, &first.scope, "old.py", 2).unwrap();
    let row_counts = || {
        let db = rusqlite::Connection::open_with_flags(
            home.path().join("memory/memory.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        [
            "SELECT count(*) FROM g_files",
            "SELECT count(*) FROM g_nodes WHERE kind IN ('file','symbol')",
            "SELECT count(*) FROM g_edges",
            "SELECT count(*) FROM g_refs",
        ]
        .map(|sql| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap())
    };
    let counts_before = row_counts();
    assert!(!old.is_empty());
    fs::remove_file(p.join("old.py")).unwrap();
    write(&p.join("new.py"), "def new(): pass\nnew()\n");
    for which in ["files", "file_bytes", "nodes", "edges", "deadline"] {
        let mut opts = SyncOptions::default();
        match which {
            "files" => opts.limits.files = 0,
            "file_bytes" => opts.limits.file_bytes = 0,
            "nodes" => opts.limits.nodes = 0,
            "edges" => opts.limits.edges = 0,
            _ => {
                opts.deadline = Some(std::time::Instant::now() - std::time::Duration::from_secs(1))
            }
        }
        let report = sync(&store, p, None, &opts).unwrap();
        assert!(report.partial, "{which}");
        assert!(report.partial_reason.is_some(), "{which}");
        assert!(report.removed.is_empty(), "{which}");
        let retained = refs_at(&store, &first.scope, "old.py", 2).unwrap();
        assert_eq!(retained.len(), old.len(), "{which}");
        assert_eq!(health(home.path(), p).unwrap().files, 1, "{which}");
        assert_eq!(row_counts(), counts_before, "{which}");
    }
}
#[test]
fn unrelated_refs_are_not_reresolved() {
    let (_home, repo, store) = setup();
    let p = repo.path();
    write(&p.join("main.py"), "from lib import work\nwork()\n");
    write(&p.join("lib.py"), "def work(): pass\n");
    write(&p.join("other.py"), "def other(): pass\nother()\n");
    let first = sync(&store, p, None, &SyncOptions::default()).unwrap();
    let dependent = refs_at(&store, &first.scope, "main.py", 2).unwrap().len();
    assert!(dependent > 0);
    assert!(!refs_at(&store, &first.scope, "other.py", 2)
        .unwrap()
        .is_empty());
    write(&p.join("lib.py"), "def work():\n    pass\n");
    let next = sync(&store, p, None, &SyncOptions::default()).unwrap();
    assert_eq!(next.reresolved, dependent);
}
#[test]
fn corrupt_health_is_reported_without_writing() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join("memory")).unwrap();
    fs::write(home.path().join("memory/memory.db"), b"not sqlite").unwrap();
    let p = tempfile::tempdir().unwrap();
    assert_eq!(health(home.path(), p.path()).unwrap().state, "corrupt");
}
