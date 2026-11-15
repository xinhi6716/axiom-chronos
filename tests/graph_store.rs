use axiom::{
    graph::{
        self,
        store::{migrate_failing_after, project_key},
        *,
    },
    memory::{Memory, Record},
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
fn old(home: &Path, project: &Path) {
    fs::create_dir_all(home.join("memory")).unwrap();
    let db = Connection::open(home.join("memory/memory.db")).unwrap();
    db.execute_batch("CREATE TABLE entries (id INTEGER PRIMARY KEY, project TEXT NOT NULL, unit TEXT NOT NULL, title TEXT NOT NULL, summary TEXT NOT NULL, body TEXT NOT NULL, symbols TEXT NOT NULL, wish TEXT NOT NULL, files TEXT NOT NULL, project_dir TEXT NOT NULL, created REAL NOT NULL, confirmed REAL, corrected REAL); CREATE VIRTUAL TABLE entries_fts USING fts5(title,summary,body,symbols,wish,content='entries',content_rowid='id',tokenize='trigram'); CREATE TRIGGER entries_ai AFTER INSERT ON entries BEGIN INSERT INTO entries_fts(rowid,title,summary,body,symbols,wish) VALUES(new.id,new.title,new.summary,new.body,new.symbols,new.wish); END; CREATE TRIGGER entries_ad AFTER DELETE ON entries BEGIN INSERT INTO entries_fts(entries_fts,rowid,title,summary,body,symbols,wish) VALUES('delete',old.id,old.title,old.summary,old.body,old.symbols,old.wish); END; CREATE TRIGGER entries_au AFTER UPDATE OF title,summary,body,symbols,wish ON entries BEGIN INSERT INTO entries_fts(entries_fts,rowid,title,summary,body,symbols,wish) VALUES('delete',old.id,old.title,old.summary,old.body,old.symbols,old.wish); INSERT INTO entries_fts(rowid,title,summary,body,symbols,wish) VALUES(new.id,new.title,new.summary,new.body,new.symbols,new.wish); END;").unwrap();
    db.execute("INSERT INTO entries VALUES(42,'p','u','oldtitle','oldsummary','oldbody','','wish','[]',?,1,1,NULL)",[project.to_string_lossy().as_ref()]).unwrap();
}
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let p = t.path().join("project");
    fs::create_dir_all(&p).unwrap();
    (t, home, p)
}
fn source() -> SourceSpec {
    SourceSpec {
        kind: "code".into(),
        locator: "source".into(),
        sha256: "hash".into(),
        version: "1".into(),
        state: "active".into(),
    }
}
fn node(key: &str, kind: &str) -> NodeSpec {
    NodeSpec {
        key: key.into(),
        kind: kind.into(),
        label: key.into(),
        summary: String::new(),
        attrs: json!({}),
        entry_id: None,
        state: "active".into(),
    }
}
fn reference(scope: i64, key: &str) -> NodeRef {
    NodeRef {
        scope,
        key: key.into(),
    }
}
fn edge(scope: i64, a: &str, b: &str, k: &str) -> EdgeSpec {
    EdgeSpec {
        src: reference(scope, a),
        dst: reference(scope, b),
        kind: k.into(),
        locator: String::new(),
        weight: 500,
        state: "active".into(),
        attrs: json!({}),
    }
}
#[test]
fn migration_old_database_and_idempotent() {
    let (_t, h, p) = fixture();
    old(&h, &p);
    let db = Connection::open(h.join("memory/memory.db")).unwrap();
    let schema: i64 = db
        .query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap();
    let store = GraphStore::open(&h).unwrap();
    let n = store.node_by_entry(42).unwrap().unwrap();
    assert_eq!(n.key, "entry:42");
    assert_eq!(n.label, "oldtitle");
    assert_eq!(
        Memory::open(&h).unwrap().get(42).unwrap()["entry"]["body"],
        "oldbody"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM entries_fts WHERE entries_fts MATCH 'oldbody'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let after: i64 = db
        .query_row("PRAGMA schema_version", [], |r| r.get(0))
        .unwrap();
    assert!(after > schema);
    GraphStore::open(&h).unwrap();
    assert_eq!(
        db.query_row("PRAGMA schema_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        after
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM g_nodes WHERE entry_id=42", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
}
#[test]
fn migration_rolls_back() {
    for step in ["schema", "backfill", "verify", "metadata"] {
        let (_t, h, p) = fixture();
        old(&h, &p);
        let path = h.join("memory/memory.db");
        assert!(migrate_failing_after(&path, step).is_err());
        let c = Connection::open(path).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name GLOB 'g_*'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT body FROM entries WHERE id=42", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "oldbody"
        );
    }
}
#[test]
fn concurrent_migration() {
    let (_t, h, p) = fixture();
    old(&h, &p);
    let hs = [h.clone(), h.clone()];
    let threads: Vec<_> = hs
        .into_iter()
        .map(|h| std::thread::spawn(move || GraphStore::open(&h).is_ok()))
        .collect();
    assert!(threads.into_iter().all(|x| x.join().unwrap()));
    let c = Connection::open(h.join("memory/memory.db")).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM g_meta", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM g_nodes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn batch_validation_and_replace() {
    let (_t, h, p) = fixture();
    let s = GraphStore::open(&h).unwrap();
    let scope = s.scope(&p, None).unwrap().id;
    let mut b = Batch {
        scope,
        source: source(),
        nodes: vec![node("file:a.rs", "file"), node("sym:a.rs#fn:a", "symbol")],
        edges: vec![edge(scope, "file:a.rs", "sym:a.rs#fn:a", "defines")],
        replace_source: true,
    };
    let r = s.write(&b).unwrap();
    assert_eq!((r.nodes_inserted, r.edges_inserted), (2, 1));
    let r = s.write(&b).unwrap();
    assert_eq!(
        (r.nodes_inserted, r.nodes_updated, r.edges_unchanged),
        (0, 0, 1)
    );
    for variant in 0..5 {
        let mut bad = b.clone();
        bad.nodes.push(node("file:new.rs", "file"));
        match variant {
            0 => bad.nodes[0].kind = "unknown".into(),
            1 => bad.edges[0].kind = "calls".into(),
            2 => {
                let other = p.join("other");
                fs::create_dir_all(&other).unwrap();
                let x = s.scope(&other, None).unwrap().id;
                s.write(&Batch {
                    scope: x,
                    source: source(),
                    nodes: vec![node("file:x", "file")],
                    edges: vec![],
                    replace_source: false,
                })
                .unwrap();
                bad.edges[0].dst = reference(x, "file:x");
            }
            3 => bad.nodes[0].key = "file:/absolute".into(),
            _ => bad.nodes[0].attrs = json!("bad"),
        };
        assert_eq!(s.write(&bad).unwrap_err().kind, "invalid_argument");
        assert!(s.node(&reference(scope, "file:new.rs")).unwrap().is_none());
    }
    b.edges.clear();
    assert_eq!(s.write(&b).unwrap().edges_removed, 1);
}
#[test]
fn scope_isolation() {
    let (t, h, p) = fixture();
    let other = t.path().join("other/project");
    fs::create_dir_all(&other).unwrap();
    let s = GraphStore::open(&h).unwrap();
    let a = s.scope(&p, None).unwrap();
    let b = s.scope(&other, None).unwrap();
    let w = s.scope(&p, Some(&other)).unwrap();
    assert_ne!(a.project_key, b.project_key);
    assert_ne!(a.id, w.id);
    assert_eq!(s.scopes_of(&a.project_key).unwrap().len(), 2);
    assert_ne!(project_key(&p), project_key(&other));
}
#[test]
fn fts_and_dirty_entry() {
    let (_t, h, p) = fixture();
    old(&h, &p);
    let m = Memory::open(&h).unwrap();
    let s = GraphStore::open(&h).unwrap();
    let scope = s.scope(&p, None).unwrap().id;
    let b = Batch {
        scope,
        source: source(),
        nodes: vec![node("file:needle.rs", "file")],
        edges: vec![],
        replace_source: false,
    };
    s.write(&b).unwrap();
    assert_eq!(s.fts(&[scope], "needle", 10).unwrap().len(), 1);
    assert_eq!(s.fts(&[scope], "oldbody", 10).unwrap().len(), 1);
    m.correct(42, "replacement text").unwrap();
    assert!(s.fts(&[scope], "oldbody", 10).unwrap().is_empty());
    let c = Connection::open(h.join("memory/memory.db")).unwrap();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    c.execute(
        "UPDATE entries SET title='newtitle',body='rawupdate' WHERE id=42",
        [],
    )
    .unwrap();
    assert!(s.fts(&[scope], "oldtitle", 10).unwrap().is_empty());
    assert_eq!(s.node_by_entry(42).unwrap().unwrap().label, "newtitle");
    c.execute("DELETE FROM entries WHERE id=42", []).unwrap();
    assert!(s.fts(&[scope], "rawupdate", 10).unwrap().is_empty());
    let n = s.node(&reference(scope, "entry:42")).unwrap().unwrap();
    assert_eq!(n.state, "tombstone");
    assert_eq!(n.entry_id, None);
}
#[test]
fn existing_never_creates() {
    let (_t, h, p) = fixture();
    assert!(GraphStore::open_existing(&h).unwrap().is_none());
    assert!(!h.exists());
    old(&h, &p);
    assert!(!GraphStore::open_existing(&h)
        .unwrap()
        .unwrap()
        .ready()
        .unwrap());
}
#[test]
fn key_roundtrip() {
    let k = keys::symbol("src\\x.rs", "fn", "main", 2);
    assert_eq!(k, "sym:src/x.rs#fn:main~2");
    assert_eq!(
        keys::split_symbol(&k),
        Some(("src/x.rs".into(), "fn".into(), "main".into(), 2))
    );
    assert_eq!(keys::entry(2), "entry:2");
    assert_eq!(keys::wish("w"), "wish:w");
    assert_eq!(keys::unit("w", "u"), "unit:w/u");
    assert_eq!(keys::module("w", "m"), "module:w/m");
    assert_eq!(keys::finding("w", "u", 1), "finding:w/u#1");
    assert_eq!(keys::decision("w", "u", 2), "decision:w/u#2");
    assert_eq!(keys::scope_claim("w", "u", 3), "scope_claim:w/u#3");
    assert_eq!(keys::artifact("a\\b"), "artifact:a/b");
    assert_eq!(keys::feedback("hash"), "feedback:hash");
    assert_eq!(keys::project_index("p"), "project_index:p");
    assert_eq!(keys::file("a\\b"), "file:a/b");
}
fn record_setup(h: &Path, p: &Path) -> Record {
    fs::write(p.join("a.rs"), "source").unwrap();
    let dir = h.join("orchestrate/p");
    fs::create_dir_all(&dir).unwrap();
    let brief = dir.join("u.md");
    fs::write(&brief, "GOAL test\nSCOPE a.rs\nVERIFY test\n").unwrap();
    fs::write(
        dir.join("units.tsv"),
        format!(
            "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu\tx\tverified\t\t\t\t{}\n",
            brief.display()
        ),
    )
    .unwrap();
    Record {
        project: "p".into(),
        project_dir: p.to_string_lossy().into(),
        unit: "u".into(),
        title: "title".into(),
        summary: "summary".into(),
        body: "body".into(),
        symbols: vec![],
        files: vec!["a.rs".into()],
        wish: "wish".into(),
    }
}
#[test]
fn typed_record_is_atomic() {
    let (_t, h, p) = fixture();
    let r = record_setup(&h, &p);
    let m = Memory::open(&h).unwrap();
    let typed = TypedWrite {
        nodes: vec![node("unit:w/u", "unit")],
        links: vec![LinkSpec {
            from: None,
            kind: "records".into(),
            to: "unit:w/u".into(),
        }],
    };
    let first = m.record_typed(&r, &typed).unwrap();
    assert_eq!(first["graph"], json!({"nodes":1,"links":1}));
    let second = TypedWrite {
        nodes: vec![node("unit:w/v", "unit")],
        links: vec![LinkSpec {
            from: None,
            kind: "records".into(),
            to: "unit:w/v".into(),
        }],
    };
    assert_eq!(m.record_typed(&r, &second).unwrap()["duplicate"], true);
    assert_eq!(m.record_typed(&r, &second).unwrap()["duplicate"], true);
    let s = GraphStore::open(&h).unwrap();
    let id = s
        .node_by_entry(first["id"].as_i64().unwrap())
        .unwrap()
        .unwrap()
        .id;
    assert_eq!(s.edges(id, Direction::Out, 10).unwrap().len(), 2);
    let plain = m.record(&r).unwrap();
    assert!(plain.get("graph").is_none());
    for invalid in [
        TypedWrite {
            nodes: vec![node("unit:w/u", "unit"); 33],
            links: vec![],
        },
        TypedWrite {
            nodes: vec![],
            links: vec![
                LinkSpec {
                    from: None,
                    kind: "records".into(),
                    to: "unit:w/u".into()
                };
                65
            ],
        },
        TypedWrite {
            nodes: vec![NodeSpec {
                label: "x".repeat(65536),
                ..node("unit:w/big", "unit")
            }],
            links: vec![],
        },
        TypedWrite {
            nodes: vec![],
            links: vec![LinkSpec {
                from: None,
                kind: "records".into(),
                to: "unit:missing".into(),
            }],
        },
    ] {
        assert_eq!(
            m.record_typed(&r, &invalid).unwrap_err().kind,
            "invalid_argument"
        );
    }
    let c = Connection::open(h.join("memory/memory.db")).unwrap();
    c.execute("DELETE FROM entries", []).unwrap();
    let bad = TypedWrite {
        nodes: vec![],
        links: vec![LinkSpec {
            from: None,
            kind: "records".into(),
            to: "unit:missing".into(),
        }],
    };
    assert!(m.record_typed(&r, &bad).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM entries", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn state_change_atomic() {
    let (_t, h, p) = fixture();
    let s = GraphStore::open(&h).unwrap();
    let scope = s.scope(&p, None).unwrap().id;
    s.write(&Batch {
        scope,
        source: source(),
        nodes: vec![node("file:x", "file")],
        edges: vec![],
        replace_source: false,
    })
    .unwrap();
    let mut c = ChangeRecord {
        scope,
        actor: "user".into(),
        op: "state".into(),
        target: "file:x".into(),
        reason: "reason".into(),
        evidence: json!({}),
        before: json!("active"),
        after: json!("stale"),
    };
    s.set_state(&reference(scope, "file:x"), "stale", &c)
        .unwrap();
    assert_eq!(s.changes(scope, 10).unwrap().len(), 1);
    c.actor.clear();
    assert!(s
        .set_state(&reference(scope, "file:x"), "retired", &c)
        .is_err());
    assert_eq!(
        s.node(&reference(scope, "file:x")).unwrap().unwrap().state,
        "stale"
    );
}
#[test]
fn lint_reports_bad_graph() {
    let (_t, h, p) = fixture();
    let s = GraphStore::open(&h).unwrap();
    let a = s.scope(&p, None).unwrap().id;
    assert!(graph::lint::check(&s, &p).unwrap().is_empty());
    let other = p.join("other");
    fs::create_dir_all(&other).unwrap();
    let b = s.scope(&other, None).unwrap().id;
    let c = Connection::open(h.join("memory/memory.db")).unwrap();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    c.execute(
        "INSERT INTO g_sources VALUES(1,?,'typed','x','h','1','stale',0)",
        [a],
    )
    .unwrap();
    c.execute(
        "INSERT INTO g_sources VALUES(2,?,'code','missing','h','1','retired',0)",
        [a],
    )
    .unwrap();
    c.execute(
        "INSERT INTO g_nodes VALUES(1,?,'file:a','file',NULL,'','','{}',1,'active',0)",
        [a],
    )
    .unwrap();
    c.execute(
        "INSERT INTO g_nodes VALUES(2,?,'file:b','file',NULL,'','','{}',1,'active',0)",
        [b],
    )
    .unwrap();
    c.execute_batch("INSERT INTO g_edges VALUES(1,999,'defines',1,'',1,'active','{}'); INSERT INTO g_edges VALUES(1,2,'calls',1,'',1,'active','{}'); INSERT INTO g_aliases VALUES(1,2); INSERT INTO g_aliases VALUES(2,1);").unwrap();
    for i in 3..36 {
        c.execute(
            "INSERT INTO g_nodes VALUES(?,?,?,'file',NULL,'','','{}',1,'active',0)",
            params![i, a, format!("file:{i}")],
        )
        .unwrap();
    }
    let kinds: Vec<_> = graph::lint::check(&s, &p)
        .unwrap()
        .into_iter()
        .map(|x| x.kind)
        .collect();
    for kind in [
        "missing_endpoint",
        "bad_kind_pair",
        "cross_project",
        "alias_cycle",
        "stale_source",
        "removed_source",
        "over_limit",
    ] {
        assert!(kinds.contains(&kind), "missing {kind}: {kinds:?}");
    }
}
