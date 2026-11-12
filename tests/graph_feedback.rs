use axiom::graph::{
    feedback::{collect, ingest},
    keys, Batch, Direction, GraphStore, NodeRef, NodeSpec, SourceSpec,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

const W1: &str = "w-1700000000001-abcdef";
const W2: &str = "w-1700000000002-abcdef";
fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn key(source: &str, text: &str) -> String {
    keys::feedback(
        &format!(
            "{:x}",
            Sha256::digest(format!("{source}\n{text}").as_bytes())
        )[..16],
    )
}
fn fixtures(home: &Path) {
    put(&home.join("goals/a.json"), &json!({"wish":"original","wish_id":W1,"project":"demo","status":"blocked","amendments":["Please correct this"],"reason":"user_stop","note":"Stop now"}).to_string());
    put(&home.join("goals/b.json"), &json!({"wish":"other","wish_id":W2,"project":"other","status":"active","amendments":["ignored"]}).to_string());
    let store = home.join("orchestrate/demo").join(W1);
    put(&store.join("nested/a/council.json"), &json!({"key":"k","question":"q","members":[],"rounds":[],"claims":[{"id":"c1","text":"Disputed claim","evidence":"e","stances":{},"status":"disputed","ruling":{"decision":"rejected","reason":"Wrong premise"}},{"id":"c2","text":"Narrowed claim","evidence":"e","stances":{},"status":"disputed","ruling":{"decision":"narrowed","reason":"Too broad"}},{"id":"c3","text":"upheld","evidence":"e","stances":{},"status":"disputed","ruling":{"decision":"upheld","reason":"yes"}}],"cost":{"usd":0.0,"cap_usd":1.0,"cut":[]},"degraded":false}).to_string());
    put(&store.join("units.tsv"), "id\ttrack\tstate\tbranch\tpr\tsha\tbrief\nu1\tbuild\tfailed\t\t\t\t\nu2\tbuild\tfailed\t\t\t\t\n");
    put(&store.join("reports/u1.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nrun\tbuild\t1\ttree\tt\nverify\ttest\t1\ttree\tt\nreview\treviewer\t1\ttree\tt\nwaive\t1 Accepted risk\t0\ttree\tt\n");
    put(&store.join("reports/u1.verdict.json"), &json!({"unit":"u1","tree":"tree","verdict":"fail","findings":[{"check":"security","severity":"major","location":"a","evidence":"Finding text","fix":"fix"}]}).to_string());
    put(&store.join("reports/u2.evidence.tsv"), "kind\tcommand\texit_code\tcommit\tts\nrun\tbuild\t1\ttree\tt\nverify\ttest\t1\ttree\tt\naccept\ttest\t1\ttree\tt\n");
    fs::create_dir_all(home.join("orchestrate/demo").join(W2)).unwrap();
}
#[test]
fn collects_explicit_feedback_only() {
    let t = tempfile::tempdir().unwrap();
    fixtures(t.path());
    let events = collect(t.path(), "demo").unwrap();
    assert_eq!(events.len(), 5);
    let mut order: Vec<_> = events
        .iter()
        .map(|e| (&e.wish, &e.source, &e.kind))
        .collect();
    let original = order.clone();
    order.sort();
    assert_eq!(original, order);
    let get = |kind: &str| events.iter().find(|e| e.kind == kind).unwrap();
    assert_eq!(
        (
            &get("correction").target,
            &get("correction").decider,
            &get("correction").text
        ),
        (
            &keys::wish(W1),
            &"user".to_string(),
            &"Please correct this".to_string()
        )
    );
    assert_eq!(get("user_stop").text, "Stop now");
    assert_eq!(get("overruled_ruling").reason, "Wrong premise");
    assert_eq!(get("overruled_ruling").text, "Disputed claim");
    let narrowed = events.iter().find(|e| e.text == "Narrowed claim").unwrap();
    assert_eq!(
        (&*narrowed.kind, &*narrowed.decider, &*narrowed.reason),
        ("overruled_ruling", "core", "Too broad")
    );
    assert_eq!(get("waived_finding").target, keys::finding(W1, "u1", 1));
    assert_eq!(get("waived_finding").reason, "Accepted risk");
    assert_eq!(get("waived_finding").text, "Finding text");
    for e in events {
        let file = e.source.split('#').next().unwrap();
        assert_eq!(
            e.source_sha256,
            format!(
                "{:x}",
                Sha256::digest(fs::read(t.path().join(file)).unwrap())
            )
        );
    }
}
#[test]
fn ingestion_replays_and_preserves_entries() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let project = t.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fixtures(&home);
    let store = GraphStore::open(&home).unwrap();
    let db = Connection::open(home.join("memory/memory.db")).unwrap();
    db.execute("INSERT INTO entries (project,unit,title,summary,body,symbols,wish,files,project_dir,created) VALUES ('demo','u','existing','summary','success memory','','wish','[]',?,1)", [project.to_string_lossy().as_ref()]).unwrap();
    let before: (String, String, String) = db
        .query_row("SELECT title,summary,body FROM entries", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    let scope = store.scope(&project, None).unwrap();
    let other = store
        .scope(&project, Some(&t.path().join("second-wish")))
        .unwrap();
    let result = ingest(&store, &home, "demo", &project).unwrap();
    assert_eq!(result.nodes_written, 5);
    let events = collect(&home, "demo").unwrap();
    let narrowed = events.iter().find(|e| e.text == "Narrowed claim").unwrap();
    assert_eq!(
        store
            .node(&NodeRef {
                scope: scope.id,
                key: key(&narrowed.source, &narrowed.text),
            })
            .unwrap()
            .unwrap()
            .kind,
        "feedback"
    );
    for event in events {
        let node = store
            .node(&NodeRef {
                scope: scope.id,
                key: key(&event.source, &event.text),
            })
            .unwrap()
            .unwrap();
        assert_eq!(node.kind, "feedback");
        assert_eq!(node.attrs, serde_json::to_value(&event).unwrap());
        assert_eq!(node.label, event.text.chars().take(80).collect::<String>());
        assert!(store
            .edges(node.id, Direction::Out, 100)
            .unwrap()
            .is_empty());
    }
    let scopes = store.scopes_of(&other.project_key).unwrap();
    assert!(scopes.iter().any(|s| s.id == scope.id));
    assert!(!store
        .fts(
            &scopes.iter().map(|s| s.id).collect::<Vec<_>>(),
            "correct",
            10
        )
        .unwrap()
        .is_empty());
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM g_nodes WHERE kind='feedback'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let replay = ingest(&store, &home, "demo", &project).unwrap();
    assert_eq!((replay.nodes_written, replay.edges_written), (0, 0));
    assert_eq!(
        count,
        db.query_row(
            "SELECT count(*) FROM g_nodes WHERE kind='feedback'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
    );
    assert_eq!(
        before,
        db.query_row("SELECT title,summary,body FROM entries", [], |r| Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?
        )))
        .unwrap()
    );
}
#[test]
fn supersedes_edges_require_existing_targets_and_replay_idempotently() {
    for kind in ["unit", "wish", "finding", "feedback"] {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let project = t.path().join("project");
        fs::create_dir_all(&project).unwrap();
        fixtures(&home);
        let target = format!("{kind}:old");
        let goal = home.join("goals/a.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&goal).unwrap()).unwrap();
        value["supersedes"] = json!(target);
        put(&goal, &value.to_string());
        let store = GraphStore::open(&home).unwrap();
        let scope = store.scope(&project, None).unwrap();
        let events: Vec<_> = collect(&home, "demo")
            .unwrap()
            .into_iter()
            .filter(|e| e.supersedes.is_some())
            .collect();
        assert_eq!(events.len(), 2);
        let missing = ingest(&store, &home, "demo", &project).unwrap();
        assert!(missing.skipped.is_empty());
        assert_eq!(missing.edges_written, 0);
        for event in &events {
            let node = store
                .node(&NodeRef {
                    scope: scope.id,
                    key: key(&event.source, &event.text),
                })
                .unwrap()
                .unwrap();
            assert!(store
                .edges(node.id, Direction::Out, 100)
                .unwrap()
                .is_empty());
        }
        store
            .write(&Batch {
                scope: scope.id,
                source: SourceSpec {
                    kind: "fixture".into(),
                    locator: "fixture".into(),
                    sha256: "hash".into(),
                    version: "1".into(),
                    state: "active".into(),
                },
                nodes: vec![NodeSpec {
                    key: target.clone(),
                    kind: kind.into(),
                    label: "old".into(),
                    summary: String::new(),
                    attrs: json!({}),
                    entry_id: None,
                    state: "active".into(),
                }],
                edges: vec![],
                replace_source: false,
            })
            .unwrap();
        let written = ingest(&store, &home, "demo", &project).unwrap();
        assert_eq!(written.edges_written, 2);
        assert!(written.skipped.is_empty());
        for event in &events {
            let reference = NodeRef {
                scope: scope.id,
                key: key(&event.source, &event.text),
            };
            let node = store.node(&reference).unwrap().unwrap();
            let edges = store.edges(node.id, Direction::Out, 100).unwrap();
            assert_eq!(edges.len(), 1);
            assert_eq!(edges[0].kind, "supersedes");
            assert_eq!(edges[0].src, reference);
            assert_eq!(
                edges[0].dst,
                NodeRef {
                    scope: scope.id,
                    key: target.clone()
                }
            );
        }
        let replay = ingest(&store, &home, "demo", &project).unwrap();
        assert_eq!((replay.nodes_written, replay.edges_written), (0, 0));
        assert!(replay.skipped.is_empty());
        for event in &events {
            let node = store
                .node(&NodeRef {
                    scope: scope.id,
                    key: key(&event.source, &event.text),
                })
                .unwrap()
                .unwrap();
            assert_eq!(store.edges(node.id, Direction::Out, 100).unwrap().len(), 1);
        }
    }
}
