use axiom::memory::{query_terms, Memory, Record};
use rusqlite::Connection;
use std::fs;

fn fixture() -> (tempfile::TempDir, Record) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("a.py"), "original").unwrap();
    fs::write(project.join("b.py"), "other").unwrap();
    let store = home.join("orchestrate/p");
    fs::create_dir_all(&store).unwrap();
    let brief = store.join("a.md");
    let brief2 = store.join("b.md");
    fs::write(&brief, "GOAL test\nSCOPE a.py missing.py\nVERIFY test\n").unwrap();
    fs::write(&brief2, "GOAL test\nSCOPE b.py\nVERIFY test\n").unwrap();
    fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tx\tverified\t\t\t\t{}\nv\tx\tpending\t\t\t\t{}\n", brief.display(), brief2.display())).unwrap();
    let record = Record {
        project: "p".into(),
        project_dir: project.to_string_lossy().into(),
        unit: "u".into(),
        title: "stats".into(),
        summary: "完成總數".into(),
        body: "body".into(),
        symbols: vec!["stats".into()],
        files: vec!["a.py".into()],
        wish: "統計功能".into(),
    };
    (temp, record)
}
fn setup() -> (tempfile::TempDir, Memory, Record) {
    let (temp, record) = fixture();
    let memory = Memory::open(&temp.path().join("home")).unwrap();
    (temp, memory, record)
}
#[test]
fn record_search_stale_digest_and_validation() {
    let (_temp, memory, mut r) = setup();
    assert_eq!(
        query_terms("統計功能 stats stats"),
        vec!["stats", "統計", "計功", "功能"]
    );
    let result = memory.record(&r).unwrap();
    assert_eq!(
        result["files"],
        serde_json::json!([{ "path": "a.py", "sha256": "0682c5f2076f099c34cfdd15a9e063849ed437a49677e6fcc5b4198c76575be5" }])
    );
    let id = result["id"].as_i64().unwrap();
    assert_eq!(
        memory.search("stats", Some("p"), 5).unwrap()["entries"][0]["id"],
        id
    );
    assert_eq!(
        memory.search("統計", None, 5).unwrap()["entries"][0]["id"],
        id
    );
    assert_eq!(
        memory.search("stats", Some("other"), 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], false);
    fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "changed").unwrap();
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
    fs::remove_file(std::path::Path::new(&r.project_dir).join("a.py")).unwrap();
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
    fs::remove_dir_all(&r.project_dir).unwrap();
    assert!(memory.get(id).unwrap()["entry"]["stale"].is_null());
    let line = format!("- {} one two\n", axiom::base::today());
    assert_eq!(
        memory.digest("p", Some("one\n  two")).unwrap()["digest"],
        line
    );
    assert_eq!(
        memory.digest("p", Some("one two")).unwrap()["digest"],
        line.repeat(2)
    );
    assert_eq!(
        memory.search("?!", None, 5).unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(memory.get(999).unwrap_err().kind, "not_found");
    fs::create_dir_all(&r.project_dir).unwrap();
    r.unit = "v".into();
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
    r.unit = "u".into();
    r.files = vec!["b.py".into()];
    assert_eq!(memory.record(&r).unwrap_err().kind, "outside_scope");
    r.body = vec!["x"; 13].join("\n");
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
}
const PYTHON_SCHEMA: &str = "
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
#[test]
fn existing_database_migrates_without_changing_entries() {
    let (temp, r) = fixture();
    let home = temp.path().join("home");
    fs::create_dir_all(home.join("memory")).unwrap();
    let db = Connection::open(home.join("memory/memory.db")).unwrap();
    db.execute_batch(PYTHON_SCHEMA).unwrap();
    let schema = || {
        let mut stmt = db
            .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };
    let before = schema();
    let gone = temp.path().join("gone").to_string_lossy().into_owned();
    let files = r#"[{"path":"a.py","sha256":"0682c5f2076f099c34cfdd15a9e063849ed437a49677e6fcc5b4198c76575be5"}]"#;
    for i in 1..=3 {
        let dir = if i == 2 { &gone } else { &r.project_dir };
        let stored_files = if i == 1 { "[]" } else { files };
        db.execute("INSERT INTO entries VALUES (?1,'p','u','統計','summary','body','one two','統計',?2,?3,?4)",
            rusqlite::params![i, stored_files, dir, i as f64]).unwrap();
    }
    fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "changed").unwrap();
    let memory = Memory::open(&home).unwrap();
    let hits = memory.search("統計", None, 5).unwrap();
    assert_eq!(
        hits["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (
                v["id"].as_i64().unwrap(),
                v["stale"].clone(),
                v["matched_terms"].as_i64().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            (3, serde_json::json!(true), 1),
            (2, serde_json::Value::Null, 1),
            (1, serde_json::json!(false), 1)
        ]
    );
    for (id, stale, dir, stored_files) in [
        (1, serde_json::json!(false), &r.project_dir, "[]"),
        (2, serde_json::Value::Null, &gone, files),
        (3, serde_json::json!(true), &r.project_dir, files),
    ] {
        assert_eq!(
            memory.get(id).unwrap()["entry"],
            serde_json::json!({"id":id,"project":"p","unit":"u","title":"統計","summary":"summary","body":"body","symbols":["one","two"],"wish":"統計","files":serde_json::from_str::<serde_json::Value>(stored_files).unwrap(),"project_dir":dir,"created":id as f64,"confirmed":id as f64,"corrected":null,"stale":stale})
        );
    }
    assert_eq!(memory.record(&r).unwrap()["id"], 4);
    // Migration only appends the two new columns and adds the two new triggers.
    let after = schema();
    let name = |row: &(String, String, Option<String>)| row.1.clone();
    for old in &before {
        let new = after.iter().find(|r| name(r) == old.1).unwrap();
        if old.1 == "entries" {
            let old_sql = old.2.as_ref().unwrap().trim_end().trim_end_matches(')');
            assert!(new.2.as_ref().unwrap().starts_with(old_sql));
        } else {
            assert_eq!(new, old);
        }
    }
    let added: Vec<_> = after
        .iter()
        .filter(|r| !before.iter().any(|b| b.1 == r.1))
        .map(name)
        .collect();
    assert_eq!(added, vec!["entries_ad", "entries_au"]);
}

#[test]
fn bad_record_input_is_explicit() {
    let (_temp, memory, mut r) = setup();
    r.project = "Bad Slug".into();
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    r.project = "p".into();
    r.title = " ".into();
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    r.title = "title".into();
    r.wish.clear();
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    r.wish = "wish".into();
    let dir = r.project_dir.clone();
    r.files.clear();
    for bad in [
        "relative".to_string(),
        std::path::Path::new(&dir)
            .join("missing-dir")
            .to_string_lossy()
            .into_owned(),
    ] {
        r.project_dir = bad;
        let error = memory.record(&r).unwrap_err();
        assert_eq!(error.kind, "invalid_argument");
        assert!(error.message.contains("project_dir"));
    }
    r.project_dir = dir;
    r.files = vec!["missing.py".into()];
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    for bad in [
        "../a.py".to_string(),
        std::path::Path::new(&r.project_dir)
            .join("a.py")
            .to_string_lossy()
            .into_owned(),
    ] {
        r.files = vec![bad];
        assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    }
}

#[test]
fn search_ranks_match_count_then_creation() {
    let (temp, memory, mut r) = setup();
    r.title = "alpha".into();
    r.summary = "plain".into();
    r.wish = "plain".into();
    r.symbols.clear();
    let first = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "second").unwrap();
    r.title = "alpha beta".into();
    let second = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "third").unwrap();
    let third = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    for (id, created) in [(first, 3), (second, 1), (third, 2)] {
        db.execute("UPDATE entries SET created=?1 WHERE id=?2", (created, id))
            .unwrap();
    }
    let hits = memory.search("alpha beta", Some("p"), 5).unwrap();
    assert_eq!(
        hits["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (
                v["id"].as_i64().unwrap(),
                v["matched_terms"].as_i64().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![(third, 2), (second, 2), (first, 1)]
    );
}

#[test]
fn record_requires_files_and_deduplicates_by_file_set() {
    let (temp, memory, mut r) = setup();
    r.files.clear();
    let error = memory.record(&r).unwrap_err();
    assert_eq!(error.kind, "invalid_argument");
    assert_eq!(error.message, "files are required for a verified unit");
    r.unit = "v".into();
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
    r.unit = "u".into();
    fs::write(
        std::path::Path::new(&r.project_dir).join("missing.py"),
        "other",
    )
    .unwrap();
    r.files = vec!["a.py".into(), "missing.py".into()];
    let first = memory.record(&r).unwrap();
    assert_eq!(first["duplicate"], false);
    r.files.reverse();
    r.files.push("a.py".into());
    let duplicate = memory.record(&r).unwrap();
    assert_eq!(duplicate["status"], "completed");
    assert_eq!(duplicate["id"], first["id"]);
    assert_eq!(duplicate["duplicate"], true);
    assert!(duplicate["error"].is_null());
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM entries", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "changed").unwrap();
    let changed = memory.record(&r).unwrap();
    assert_eq!(changed["duplicate"], false);
    assert_ne!(changed["id"], first["id"]);
    assert_eq!(
        db.query_row("SELECT count(*) FROM entries", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

mod lint_cases {
    use super::*;
    use axiom::memory::lint;

    fn with_source(text: &[u8]) -> (tempfile::TempDir, Memory, Record) {
        let (temp, memory, mut r) = setup();
        fs::write(std::path::Path::new(&r.project_dir).join("a.py"), text).unwrap();
        r.body = "uses stats\n> \"Bush invented hypertext in 1941\" - a.py".into();
        (temp, memory, r)
    }
    #[test]
    fn fabricated_quote_is_refused_naming_the_line() {
        let (_t, memory, r) = with_source(b"def stats(): pass\n");
        let e = memory.record(&r).unwrap_err();
        assert_eq!(e.kind, "fabricated_quote");
        assert!(e.message.contains("body line 2"), "{}", e.message);
        assert!(e.message.contains("a.py"), "{}", e.message);
        assert_eq!(memory.list(Some("p"), 10).unwrap()["entries"].as_array().unwrap().len(), 0);
    }
    #[test]
    fn real_quote_passes_despite_whitespace_and_crlf() {
        let (_t, memory, r) = with_source(b"Bush   invented\r\n  hypertext\tin\r\n1941 .\r\n");
        memory.record(&r).unwrap();
        let (_t, memory, mut r) = with_source(b"Bush invented hypertext in 1941");
        r.body = "> \"Bush  invented\thypertext in 1941\" - a.py".into();
        memory.record(&r).unwrap();
    }
    #[test]
    fn missing_or_escaping_source_is_refused() {
        let (_t, memory, mut r) = with_source(b"x");
        r.body = "> \"x\" - gone.py".into();
        assert_eq!(memory.record(&r).unwrap_err().kind, "missing_source");
        r.body = "> \"x\" - ../a.py".into();
        assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_source");
    }
    #[test]
    fn correct_refuses_a_fabricated_quote() {
        let (_t, memory, mut r) = with_source(b"real text");
        r.body = "> \"real text\" - a.py".into();
        let id = memory.record(&r).unwrap()["id"].as_i64().unwrap();
        let e = memory.correct(id, "> \"made up\" - a.py").unwrap_err();
        assert_eq!(e.kind, "fabricated_quote");
        memory.correct(id, "> \"real  text\" - a.py").unwrap();
    }
    fn kinds(memory: &Memory, r: &Record) -> Vec<&'static str> {
        memory
            .lint("p", std::path::Path::new(&r.project_dir))
            .unwrap()
            .iter()
            .map(|p| p.kind)
            .collect()
    }
    #[test]
    fn lint_reports_changed_source_missing_source_and_bad_quote() {
        let (_t, memory, mut r) = with_source(b"real text");
        r.body = "> \"real text\" - a.py".into();
        memory.record(&r).unwrap();
        assert!(kinds(&memory, &r).is_empty());
        fs::write(std::path::Path::new(&r.project_dir).join("a.py"), "other text").unwrap();
        assert_eq!(kinds(&memory, &r), ["fabricated_quote", "stale_source"]);
        fs::remove_file(std::path::Path::new(&r.project_dir).join("a.py")).unwrap();
        assert_eq!(kinds(&memory, &r), ["missing_source", "stale_source"]);
    }
    #[test]
    fn lint_reports_exact_duplicates_and_research_quotes() {
        let (temp, memory, mut r) = setup();
        memory.record(&r).unwrap();
        // A second unit with the same text is an exact duplicate.
        let store = temp.path().join("home/orchestrate/p");
        let brief = store.join("c.md");
        fs::write(&brief, "GOAL t\nSCOPE b.py\nVERIFY t\n").unwrap();
        let mut units = fs::read_to_string(store.join("units.tsv")).unwrap();
        units.push_str(&format!("w\tx\tverified\t\t\t\t{}\n", brief.display()));
        fs::write(store.join("units.tsv"), units).unwrap();
        r.unit = "w".into();
        r.files = vec!["b.py".into()];
        memory.record(&r).unwrap();
        assert_eq!(kinds(&memory, &r), ["duplicate"]);
        let research = store.join("research");
        fs::create_dir_all(&research).unwrap();
        fs::write(research.join("plan-final.md"), "ok\n> \"original\" - a.py\n> \"nothing like it\" - a.py\n").unwrap();
        let problems = memory.lint("p", std::path::Path::new(&r.project_dir)).unwrap();
        let research: Vec<_> = problems.iter().filter(|p| p.kind == "fabricated_quote").collect();
        assert_eq!(research.len(), 1);
        assert!(research[0].message.contains("plan-final.md line 3"), "{}", research[0].message);
    }
    #[test]
    fn quote_lines_only_match_the_documented_form() {
        let dir = tempfile::tempdir().unwrap();
        assert!(lint::check_quotes("> a plain blockquote - a.py\nnot > \"x\" - a.py", dir.path(), "t").is_empty());
        assert_eq!(lint::normalize(" a\r\n b\t c "), "a b c");
    }
    #[test]
    fn hot_digest_is_bounded_newest_first_kept_and_per_project() {
        let (_t, memory, _r) = setup();
        for n in 0..30 {
            memory.digest("p", Some(&format!("wish number {n}"))).unwrap();
        }
        memory.digest("p", Some(&"x".repeat(1000))).unwrap();
        memory.digest("other", Some("secret of another project")).unwrap();
        let hot = memory.digest("p", None).unwrap();
        let text = hot["digest"].as_str().unwrap();
        assert_eq!(text.lines().count(), 12);
        assert_eq!(hot["omitted"], 19);
        assert!(text.contains("wish number 29") && !text.contains("wish number 18"));
        assert!(text.lines().all(|l| l.chars().count() <= 240));
        assert!(!text.contains("secret of another project"));
        let other = memory.digest("other", None).unwrap();
        assert!(other["digest"].as_str().unwrap().contains("secret"));
        assert!(!other["digest"].as_str().unwrap().contains("wish number"));
    }
}
