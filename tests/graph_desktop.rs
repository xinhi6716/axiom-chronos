use axiom::graph::{
    desktop::{memory_refs, module_refs, reads, MAX_NEIGHBOURS},
    ingest::sync_orch,
    keys, Batch, EdgeSpec, GraphStore, NodeRef, NodeSpec, SourceSpec,
};
use rusqlite::Connection;
use serde_json::json;
use std::fs;

fn has_polling(source: &str) -> bool {
    ["thread::", "sleep", "Instant", "SystemTime", "spawn"]
        .iter()
        .any(|s| source.contains(s))
}

#[test]
fn desktop_graph_refs_are_on_demand() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let db = home.join("memory/memory.db");
    let wish = temp.path().join("w-1234567890123-abcdef");
    assert!(memory_refs(&home, &[1]).is_empty());
    assert!(module_refs(&home, &wish).is_empty());
    assert!(!db.exists());

    fs::create_dir_all(db.parent().unwrap()).unwrap();
    Connection::open(&db).unwrap();
    assert!(memory_refs(&home, &[1]).is_empty());
    assert!(module_refs(&home, &wish).is_empty());
    fs::remove_file(&db).unwrap();

    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    let store = GraphStore::open(&home).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute("INSERT INTO entries (id,project,unit,title,summary,body,symbols,wish,files,project_dir,created,confirmed) VALUES (1,'p','u','memory title','','','','','[]',?1,1,1)", [repo.to_string_lossy().as_ref()]).unwrap();
    let entry = store.node_by_entry(1).unwrap().unwrap();
    let scope = entry.scope;
    let reference = |key: String| NodeRef { scope, key };
    let mut batch = Batch {
        scope,
        source: SourceSpec {
            kind: "test".into(),
            locator: "test/source".into(),
            sha256: "hash".into(),
            version: "1".into(),
            state: "active".into(),
        },
        nodes: vec![],
        edges: vec![],
        replace_source: false,
    };
    for i in 0..30 {
        let key = format!("file:src/{i:02}.rs");
        batch.nodes.push(NodeSpec {
            key: key.clone(),
            kind: "file".into(),
            label: format!("file {i}"),
            summary: String::new(),
            attrs: json!({}),
            entry_id: None,
            state: "active".into(),
        });
        batch.edges.push(EdgeSpec {
            src: reference(keys::entry(1)),
            dst: reference(key),
            kind: "about".into(),
            locator: String::new(),
            weight: 100,
            state: "active".into(),
            attrs: json!({}),
        });
    }
    store.write(&batch).unwrap();
    let refs = memory_refs(&home, &[1, 999]);
    let result = &refs[&1];
    assert_eq!(
        (&*result.key, &*result.kind, &*result.status),
        ("entry:1", "entry", "active")
    );
    assert_eq!(result.source, None);
    assert_eq!(result.revision, store.revision().unwrap());
    assert_eq!(result.neighbours.len(), MAX_NEIGHBOURS);
    assert_eq!(result.omitted, 6);
    assert_eq!(
        (
            &*result.neighbours[0].key,
            &*result.neighbours[0].kind,
            &*result.neighbours[0].label,
            &*result.neighbours[0].edge,
            result.neighbours[0].outgoing
        ),
        ("file:src/00.rs", "file", "file 0", "about", true)
    );
    assert_eq!(
        (&*refs[&999].key, &*refs[&999].status, refs[&999].revision),
        ("entry:999", "missing", store.revision().unwrap())
    );

    conn.execute("DELETE FROM entries WHERE id=1", []).unwrap();
    assert_eq!(memory_refs(&home, &[1])[&1].status, "tombstone");

    fs::create_dir_all(&wish).unwrap();
    fs::copy(
        "tests/fixtures/graph/records/modules.md",
        wish.join("modules.md"),
    )
    .unwrap();
    fs::copy(
        "tests/fixtures/graph/records/units.tsv",
        wish.join("units.tsv"),
    )
    .unwrap();
    sync_orch(&store, &wish, &repo).unwrap();
    let modules = module_refs(&home, &wish);
    let app_key = keys::module(wish.file_name().unwrap().to_str().unwrap(), "app");
    let base_key = keys::module(wish.file_name().unwrap().to_str().unwrap(), "base");
    assert_eq!(modules.len(), 2);
    assert_eq!(
        modules[&app_key].source.as_deref(),
        Some("orch:w-1234567890123-abcdef|modules.md")
    );
    assert_eq!(modules[&app_key].revision, store.revision().unwrap());
    assert!(modules[&app_key]
        .neighbours
        .iter()
        .any(|n| n.key == base_key && n.edge == "depends_on" && n.outgoing));
    assert!(modules[&base_key]
        .neighbours
        .iter()
        .any(|n| n.key == app_key && n.edge == "depends_on" && !n.outgoing));
    assert!(module_refs(&home, &temp.path().join("other")).is_empty());

    let mut virtual_seconds = 0_u64;
    let before = reads();
    virtual_seconds += 60;
    assert_eq!(virtual_seconds, 60);
    assert_eq!(reads(), before);
    assert!(!has_polling(include_str!("../src/graph/desktop.rs")));
    assert!(has_polling("sleep"));

    let corrupt_home = temp.path().join("corrupt");
    fs::create_dir_all(corrupt_home.join("memory")).unwrap();
    fs::write(corrupt_home.join("memory/memory.db"), b"not a database").unwrap();
    assert_eq!(memory_refs(&corrupt_home, &[5])[&5].status, "error");
}
