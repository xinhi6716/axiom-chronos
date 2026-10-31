use axiom::memory::{query_terms, Memory, Record};
use rusqlite::Connection;
use std::fs;

fn fixture() -> (tempfile::TempDir, Record) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    fs::write(project.join("a.py"), "original").unwrap();
    fs::write(project.join("b.py"), "other").unwrap();
    let store = home.join("orchestrate/p");
    fs::create_dir_all(store.join("inbox")).unwrap();
    let brief = store.join("a.md");
    let brief2 = store.join("b.md");
    fs::write(&brief, "GOAL test\nSCOPE a.py\nVERIFY echo ok\n").unwrap();
    fs::write(&brief2, "GOAL test\nSCOPE b.py\nVERIFY test\n").unwrap();
    fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tx\tverified\t\t\t\t{}\nv\tx\tpending\t\t\t\t{}\n", brief.display(), brief2.display())).unwrap();
    axiom::orch::begin_unit(&store, "u", "terra", &project, false).unwrap();
    axiom::orch::finish_unit(
        &store,
        "u",
        &project,
        "terra",
        "gpt-6.1-terra",
        &serde_json::json!({"status":"completed","file_changes":[]}),
    )
    .unwrap();
    axiom::orch::record_review(&store, &["u".into()], "gpt-6.1-sol", true, &project).unwrap();
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
    assert_eq!(memory.record(&r).unwrap()["id"], id);
    assert_eq!(
        memory.search_fresh("stats", None, 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    r.files = vec!["b.py".into()];
    assert_eq!(memory.record(&r).unwrap_err().kind, "outside_scope");
    r.files = vec!["a.py".into()];
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
    assert!(memory.search_fresh("stats", None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
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
fn existing_database_is_not_changed() {
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
    assert!(memory.search_fresh("統計", None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
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
            (3, serde_json::Value::Null, 1),
            (2, serde_json::Value::Null, 1),
            (1, serde_json::Value::Null, 1)
        ]
    );
    for (id, stale, dir, stored_files) in [
        (1, serde_json::Value::Null, &r.project_dir, "[]"),
        (2, serde_json::Value::Null, &gone, files),
        (3, serde_json::Value::Null, &r.project_dir, files),
    ] {
        assert_eq!(
            memory.get(id).unwrap()["entry"],
            serde_json::json!({"id":id,"project":"p","unit":"u","title":"統計","summary":"summary","body":"body","symbols":["one","two"],"wish":"統計","files":serde_json::from_str::<serde_json::Value>(stored_files).unwrap(),"project_dir":dir,"created":id as f64,"stale":stale})
        );
    }
    assert_eq!(schema(), before);
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
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
    assert_eq!(memory.record(&r).unwrap_err().kind, "outside_scope");
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
    r.files.clear();
    r.title = "alpha".into();
    r.summary = "plain".into();
    r.wish = "plain".into();
    r.symbols.clear();
    let first = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    r.title = "alpha beta".into();
    let second = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    r.body = "distinct".into();
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
fn evidence_tampering_limits_and_literal_queries() {
    let (temp, memory, mut r) = setup();
    r.title = "prefix_a 記憶功能".into();
    r.files.clear();
    let id = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    r.title = "prefixZa".into();
    memory.record(&r).unwrap(); // Would also match prefix_a if '_' were a LIKE wildcard.
    r.title = "prefix_a 記憶功能".into();
    assert_eq!(
        memory.search("prefix_a", None, 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        memory.search("prefix_b", None, 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        memory.search("記憶", None, 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for (query, limit) in [
        ("x".repeat(1025), 1),
        (
            (0..33)
                .map(|i| format!("word{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            1,
        ),
        ("stats".into(), 0),
        ("stats".into(), 51),
    ] {
        assert_eq!(
            memory.search(&query, None, limit).unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert!(memory.search(&"x".repeat(1024), None, 50).is_ok());
    assert!(memory
        .search(
            &(0..32)
                .map(|i| format!("word{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            None,
            50
        )
        .is_ok());
    r.title = "boundary-only".into();
    r.body = "x".repeat(16384);
    assert!(memory.record(&r).is_ok());
    r.body = "x".repeat(16385);
    assert_eq!(memory.record(&r).unwrap_err().kind, "invalid_argument");
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    db.execute("UPDATE entries SET body='tampered' WHERE id=?", [id])
        .unwrap();
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
    assert!(memory.search_fresh("prefix_a", None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    db.execute("UPDATE entries SET body='body' WHERE id=?", [id])
        .unwrap();
    let store = temp.path().join("home/orchestrate/p");
    fs::write(
        store.join("a.md"),
        "GOAL tampered\nSCOPE a.py\nVERIFY test\n",
    )
    .unwrap();
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
    assert!(memory.search_fresh("prefix_a", None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        memory
            .record(&Record {
                body: "body".into(),
                ..r
            })
            .unwrap_err()
            .kind,
        "not_verified"
    );
}

#[test]
fn directly_seeded_probe_rows_are_not_equivalent_to_admitted_memory() {
    let (temp, memory, mut r) = setup();
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    // The host probe inserts directly into entries, not through record; even a
    // valid project directory and matching file hashes cannot supply admission evidence.
    let files = serde_json::json!([{ "path": "a.py", "sha256": "0682c5f2076f099c34cfdd15a9e063849ed437a49677e6fcc5b4198c76575be5" }]).to_string();
    for (unit, title) in [
        ("literal", "literal_needle"),
        ("decoy", "literalXneedle"),
        ("cjk", "本地記憶庫"),
    ] {
        db.execute("INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES('p',?1,?2,'summary','body','','wish',?3,?4,1)", rusqlite::params![unit, title, files, r.project_dir]).unwrap();
    }
    for (query, unit) in [("literal_needle", "literal"), ("記憶", "cjk")] {
        let hits = memory.search(query, Some("p"), 5).unwrap();
        assert_eq!(hits["entries"].as_array().unwrap().len(), 1);
        assert_eq!(hits["entries"][0]["unit"], unit);
        assert!(hits["entries"][0]["stale"].is_null());
        assert!(memory.search_fresh(query, Some("p"), 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    assert!(
        memory.search_fresh("unfindable_qz", Some("p"), 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        memory
            .search_fresh("common", Some("p"), -1)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );

    // The same search terms work through fresh recall when record has admitted
    // the entry against current verified evidence; seeded rows remain excluded.
    r.title = "literal_needle 本地記憶庫".into();
    let id = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    for query in ["literal_needle", "記憶"] {
        let hits = memory.search_fresh(query, Some("p"), 5).unwrap();
        assert_eq!(hits["entries"].as_array().unwrap().len(), 1);
        assert_eq!(hits["entries"][0]["id"], id);
        assert_eq!(hits["entries"][0]["stale"], false);
    }
}

#[test]
fn digest_keeps_history_but_bounds_context() {
    let (temp, memory, _) = setup();
    assert!(memory.digest("p", Some(&"x".repeat(4096))).is_ok());
    assert_eq!(
        memory
            .digest("p", Some(&"x".repeat(4097)))
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    for _ in 0..4 {
        memory.digest("p", Some(&"字".repeat(1000))).unwrap();
    }
    let response = memory.digest("p", None).unwrap();
    assert!(response["digest"].as_str().unwrap().len() <= 8192);
    assert!(
        fs::metadata(temp.path().join("home/memory/p.md"))
            .unwrap()
            .len()
            > 8192
    );
}

#[test]
fn verified_tsv_without_evidence_is_rejected() {
    let (temp, memory, r) = setup();
    fs::remove_file(
        temp.path()
            .join("home/orchestrate/p/reports/u.evidence.tsv"),
    )
    .unwrap();
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
}

#[test]
fn symbols_unicode_and_legacy_bounds() {
    let (temp, memory, mut r) = setup();
    r.files.clear();
    r.title = "한글 𠀀𠀁".into();
    r.symbols = vec!["one two".into(), "".into(), "é".into()];
    let id = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    assert_eq!(
        memory.get(id).unwrap()["entry"]["symbols"],
        serde_json::json!(r.symbols)
    );
    assert_eq!(memory.record(&r).unwrap()["id"], id);
    r.symbols = vec!["one".into(), "two".into(), "é".into()];
    assert_ne!(memory.record(&r).unwrap()["id"], id);
    for q in ["한글", "𠀀𠀁"] {
        assert_eq!(
            memory.search_fresh(q, None, 5).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    assert!(memory.search("다른", None, 5).unwrap()["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    let files = serde_json::json!((0..200)
        .map(|_| serde_json::json!({"path":"字".repeat(2000), "sha256":"a".repeat(64)}))
        .collect::<Vec<_>>())
    .to_string();
    db.execute("INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES('p','u','legacy',?1,?2,'one two','wish',?3,?4,1)", rusqlite::params!["字".repeat(2000), "字".repeat(7000), files, r.project_dir]).unwrap();
    let entry = memory.get(db.last_insert_rowid()).unwrap()["entry"].clone();
    assert!(entry["summary"].as_str().unwrap().len() <= 4096);
    assert!(entry["body"].as_str().unwrap().len() <= 16384);
    assert!(entry["files"].as_array().unwrap().len() <= 128);
    assert!(entry.to_string().len() < 100_000);
    assert_eq!(entry["symbols"], serde_json::json!(["one", "two"]));
}

#[test]
fn fresh_refills_after_unadmitted_and_revocation() {
    let (temp, memory, mut r) = setup();
    r.files.clear();
    r.title = "refill_marker".into();
    let id = memory.record(&r).unwrap()["id"].as_i64().unwrap();
    let db = Connection::open(temp.path().join("home/memory/memory.db")).unwrap();
    db.execute("INSERT INTO entries(project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES('p','u','refill_marker','s','b','','w','[]',?1,99999999999)", [&r.project_dir]).unwrap();
    assert_ne!(
        memory.search("refill_marker", None, 1).unwrap()["entries"][0]["id"],
        id
    );
    assert_eq!(
        memory.search_fresh("refill_marker", None, 1).unwrap()["entries"][0]["id"],
        id
    );
    fs::remove_file(
        temp.path()
            .join("home/orchestrate/p/reports/u.evidence.tsv"),
    )
    .unwrap();
    assert_ne!(memory.get(id).unwrap()["entry"]["stale"], false);
    assert!(
        memory.search_fresh("refill_marker", None, 1).unwrap()["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(memory.record(&r).unwrap_err().kind, "not_verified");
}

#[test]
fn independently_invalidated_dependency_rejects_admitted_memory() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let project = temp.path().join("project");
    let store = home.join("orchestrate/p");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(store.join("inbox")).unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .status()
        .unwrap()
        .success());
    fs::write(project.join("a.py"), "consumer").unwrap();
    fs::write(project.join("b.py"), "dependency").unwrap();
    fs::write(
        store.join("a.md"),
        "GOAL consumer\nSCOPE a.py\nVERIFY echo ok\n",
    )
    .unwrap();
    fs::write(
        store.join("b.md"),
        "GOAL dependency\nSCOPE b.py\nVERIFY echo ok\n",
    )
    .unwrap();
    fs::write(store.join("units.tsv"), format!("id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nd\tdep\tpending\t\t\t\t{}\nu\tconsumer\tpending\t\t\t\t{}\n", store.join("b.md").display(), store.join("a.md").display())).unwrap();
    fs::write(store.join("modules.md"), "## dep\nPURPOSE x\nPATHS b.py\nDEPENDS none\nCONTRACT x\nACCEPT test `echo ok`\n## consumer\nPURPOSE x\nPATHS a.py\nDEPENDS dep\nCONTRACT x\nACCEPT test `echo ok`\n").unwrap();
    for (unit, module) in [("d", "dep"), ("u", "consumer")] {
        axiom::orch::begin_unit(&store, unit, "terra", &project, false).unwrap();
        axiom::orch::finish_unit(
            &store,
            unit,
            &project,
            "terra",
            "gpt-6.1-terra",
            &serde_json::json!({"status":"completed","file_changes":[]}),
        )
        .unwrap();
        axiom::orch::record_accept(&store, module, &project).unwrap();
        axiom::orch::record_review(&store, &[unit.into()], "gpt-6.1-sol", true, &project).unwrap();
    }
    let memory = Memory::open(&home).unwrap();
    let record = Record {
        project: "p".into(),
        project_dir: project.to_string_lossy().into(),
        unit: "u".into(),
        title: "dependency_marker".into(),
        summary: "summary".into(),
        body: "body".into(),
        symbols: vec![],
        files: vec!["a.py".into()],
        wish: "wish".into(),
    };
    let id = memory.record(&record).unwrap()["id"].as_i64().unwrap();
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], false);
    assert_eq!(
        memory.search_fresh("dependency_marker", None, 5).unwrap()["entries"][0]["id"],
        id
    );
    // Only the dependency changes; the consumer's scope and evidence remain untouched.
    fs::write(project.join("b.py"), "dependency changed").unwrap();
    assert_eq!(
        fs::read_to_string(project.join("a.py")).unwrap(),
        "consumer"
    );
    assert_eq!(memory.get(id).unwrap()["entry"]["stale"], true);
    assert!(
        memory.search_fresh("dependency_marker", None, 5).unwrap()["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
