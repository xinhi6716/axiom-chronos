use axiom::graph::{
    budget,
    code::{sync, SyncOptions},
    query::{explore, walk_counts, ExploreRequest, View},
    types::{Batch, Direction, EdgeSpec, NodeRef, NodeSpec, SourceSpec},
    GraphStore,
};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn request(path: &Path, query: &str) -> ExploreRequest {
    ExploreRequest {
        project_dir: path.into(),
        worktree: None,
        project: None,
        query: Some(query.into()),
        node: None,
        view: View::Mixed,
        direction: Direction::Both,
        hops: 2,
        budget: 4096,
        refresh: false,
    }
}
fn size(v: &Value) -> usize {
    serde_json::to_string_pretty(v).unwrap().len()
}
fn fits(v: &Value, limit: usize) {
    assert!(size(v) <= limit, "{} > {}", size(v), limit);
}
fn node(key: &str, kind: &str, label: &str, state: &str) -> NodeSpec {
    NodeSpec {
        key: key.into(),
        kind: kind.into(),
        label: label.into(),
        summary: String::new(),
        attrs: json!({}),
        entry_id: None,
        state: state.into(),
    }
}
fn batch(scope: i64, nodes: Vec<NodeSpec>, edges: Vec<EdgeSpec>) -> Batch {
    Batch {
        scope,
        source: SourceSpec {
            kind: "test".into(),
            locator: "test".into(),
            sha256: String::new(),
            version: "1".into(),
            state: "active".into(),
        },
        nodes,
        edges,
        replace_source: false,
    }
}
fn edge(scope: i64, a: &str, b: &str, kind: &str, state: &str) -> EdgeSpec {
    EdgeSpec {
        src: NodeRef {
            scope,
            key: a.into(),
        },
        dst: NodeRef {
            scope,
            key: b.into(),
        },
        kind: kind.into(),
        locator: String::new(),
        weight: 1000,
        state: state.into(),
        attrs: json!({}),
    }
}
#[test]
fn budget_negative_control_and_indivisible_bundles() {
    let b = json!({"kind":"feedback","conditions":["mandatory"],"contradictions":["important"],"summary":"x".repeat(1000)});
    let p = json!({"budget":{"used":0,"memory_used":0},"bundles":[b.clone(),b],"omitted":0,"truncated":0});
    assert!(std::panic::catch_unwind(|| fits(&p, 600)).is_err());
    let out = budget::fit(p, 600, 1200);
    fits(&out, 600);
    assert_eq!(out["omitted"], 2);
    assert_eq!(out["truncated"], 2);
    let p = json!({"budget":{"used":0,"memory_used":0},"bundles":[{"kind":"feedback","conditions":["must"],"contradictions":["conflict"]}],"omitted":0,"truncated":0});
    let out = budget::fit(p, 1000, 1200);
    assert_eq!(out["bundles"][0]["conditions"][0], "must");
    assert_eq!(out["bundles"][0]["contradictions"][0], "conflict");
}
#[test]
fn aliases_exact_short_views_and_hops() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(project.path(), None).unwrap();
    let mut nodes = vec![
        node("file:target.rs", "file", "target.rs", "active"),
        node(
            "sym:target.rs#function:target",
            "symbol",
            "target",
            "active",
        ),
        node("sym:target.rs#function:next", "symbol", "next", "stale"),
        node(
            "feedback:memory",
            "feedback",
            "target memory",
            "unconfirmed",
        ),
        node("feedback:dead", "feedback", "target", "retired"),
        node("feedback:cjk", "feedback", "東京", "active"),
        node("sym:target.rs#function:x", "symbol", "x", "active"),
    ];
    nodes[3].attrs = json!({"conditions":["condition"],"contradictions":["contradiction"]});
    let edges = vec![
        edge(
            scope.id,
            "file:target.rs",
            "sym:target.rs#function:target",
            "defines",
            "active",
        ),
        edge(
            scope.id,
            "sym:target.rs#function:target",
            "sym:target.rs#function:next",
            "calls",
            "stale",
        ),
        edge(
            scope.id,
            "feedback:memory",
            "sym:target.rs#function:target",
            "about",
            "active",
        ),
    ];
    store.write(&batch(scope.id, nodes, edges)).unwrap();
    let dir = home.path().join("orchestrate/demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("aliases.csv"), "canonical,alias\ntarget,alias\n").unwrap();
    let mut req = request(project.path(), "alias");
    req.project = Some("demo".into());
    req.hops = 0;
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(out["query"]["alias"]["canonical"], "target");
    assert_eq!(out["seeds"][0], "sym:target.rs#function:target");
    assert!(!out["seeds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "feedback:dead"));
    let again = explore(home.path(), &req).unwrap();
    assert_eq!(
        serde_json::to_string_pretty(&out).unwrap(),
        serde_json::to_string_pretty(&again).unwrap()
    );
    req.query = Some("東京".into());
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(out["query"]["fallback"], "short_cjk");
    assert_eq!(out["seeds"][0], "feedback:cjk");
    req.query = Some("x".into());
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(out["query"]["fallback"], "short_symbol");
    assert!(!out["seeds"].as_array().unwrap().is_empty());
    req.node = Some("sym:target.rs#function:target".into());
    req.query = None;
    req.view = View::Code;
    req.hops = 0;
    let zero = explore(home.path(), &req).unwrap();
    assert_eq!(zero["bundles"].as_array().unwrap().len(), 1);
    req.hops = 1;
    let one = explore(home.path(), &req).unwrap();
    assert!(one["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["state"] == "stale"));
    assert!(one["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["kind"] != "feedback"));
    req.view = View::Memory;
    req.node = None;
    req.query = Some("target".into());
    let m = explore(home.path(), &req).unwrap();
    assert!(!m["bundles"].as_array().unwrap().is_empty());
    assert_eq!(m["bundles"][0]["state"], "unconfirmed");
    assert_eq!(m["bundles"][0]["conditions"], json!(["condition"]));
    assert_eq!(m["bundles"][0]["contradictions"], json!(["contradiction"]));
    assert!(m["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["kind"] == "feedback"));
}
#[test]
fn missing_and_clamped_sizes() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let mut req = request(p.path(), "nothing");
    let v = explore(home.path(), &req).unwrap();
    assert_eq!(v["freshness"]["state"], "missing");
    fits(&v, 4096);
    assert!(!home.path().join("memory/memory.db").exists());
    for b in [0, 256, 300, 500, 2400, 99999] {
        req.budget = b;
        fits(&explore(home.path(), &req).unwrap(), b.clamp(256, 4096));
    }
}
#[test]
fn indexed_code_refresh_and_dirty_seed() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    fs::write(p.path().join("main.py"), "def target():\n    pass\n").unwrap();
    fs::write(
        p.path().join("unrelated.py"),
        "def unrelated():\n    pass\n",
    )
    .unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    sync(&store, p.path(), None, &SyncOptions::default()).unwrap();
    let scope = store.find_scope(p.path(), None).unwrap().unwrap();
    let unrelated = NodeRef {
        scope: scope.id,
        key: "file:unrelated.py".into(),
    };
    let before = store.node(&unrelated).unwrap().unwrap();
    let mut req = request(p.path(), "target");
    req.view = View::Code;
    let a = explore(home.path(), &req).unwrap();
    assert!(!a["seeds"].as_array().unwrap().is_empty());
    fs::write(
        p.path().join("main.py"),
        "def target(value):\n    return value\n",
    )
    .unwrap();
    let b = explore(home.path(), &req).unwrap();
    assert!(b["freshness"]["dirty_files"].as_array().unwrap().is_empty());
    assert!(!b["seeds"].as_array().unwrap().is_empty());
    let fresh = store
        .node(&NodeRef {
            scope: scope.id,
            key: "file:main.py".into(),
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        fresh.source.unwrap().sha256,
        axiom::graph::code::index::sha256_hex(&fs::read(p.path().join("main.py")).unwrap())
    );
    assert!(b["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|bundle| bundle["label"] == "target"
            && bundle["summary"]
                .as_str()
                .unwrap()
                .contains("target(value)")));
    let after = store.node(&unrelated).unwrap().unwrap();
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.source.unwrap().sha256, after.source.unwrap().sha256);
    fs::write(p.path().join("other.py"), "def other(): pass\n").unwrap();
    req.refresh = true;
    req.query = Some("other".into());
    let c = explore(home.path(), &req).unwrap();
    assert!(!c["seeds"].as_array().unwrap().is_empty());
}
#[test]
fn fanout_is_bounded_and_omitted_counted() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let mut nodes = vec![node("file:many.rs", "file", "many.rs", "active")];
    let mut edges = Vec::new();
    for i in 0..150 {
        let key = format!("sym:many.rs#function:item{i:03}");
        nodes.push(node(&key, "symbol", &format!("item{i:03}"), "active"));
        edges.push(edge(scope.id, "file:many.rs", &key, "defines", "active"));
    }
    store.write(&batch(scope.id, nodes, edges)).unwrap();
    let mut req = request(p.path(), "");
    req.node = Some("file:many.rs".into());
    req.hops = 1;
    assert_eq!(walk_counts(home.path(), &req).unwrap(), (64, 128));
    let v = explore(home.path(), &req).unwrap();
    fits(&v, 4096);
    assert!(v["omitted"].as_u64().unwrap() > 0);
    assert!(v["truncated"].as_u64().unwrap() > 0);
    assert!(v["bundles"].as_array().unwrap().len() <= 64);
}
#[test]
fn hop_limits_states_and_seed_cap() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let mut ns = vec![
        node("sym:a.rs#function:root", "symbol", "root", "active"),
        node("sym:a.rs#function:one", "symbol", "one", "unconfirmed"),
        node("sym:a.rs#function:two", "symbol", "two", "stale"),
        node("sym:a.rs#function:blocked", "symbol", "blocked", "active"),
        node("sym:a.rs#function:retired", "symbol", "retired", "retired"),
    ];
    let es = vec![
        edge(
            scope.id,
            "sym:a.rs#function:root",
            "sym:a.rs#function:one",
            "calls",
            "active",
        ),
        edge(
            scope.id,
            "sym:a.rs#function:one",
            "sym:a.rs#function:two",
            "calls",
            "active",
        ),
        edge(
            scope.id,
            "sym:a.rs#function:root",
            "sym:a.rs#function:blocked",
            "calls",
            "ambiguous",
        ),
        edge(
            scope.id,
            "sym:a.rs#function:root",
            "sym:a.rs#function:retired",
            "calls",
            "active",
        ),
    ];
    for i in 0..10 {
        ns.push(node(
            &format!("sym:b.rs#function:match{i}"),
            "symbol",
            "same",
            "active",
        ));
    }
    store.write(&batch(scope.id, ns, es)).unwrap();
    let mut req = request(p.path(), "same");
    req.hops = 0;
    let v = explore(home.path(), &req).unwrap();
    assert_eq!(v["seeds"].as_array().unwrap().len(), 6);
    req.query = None;
    req.node = Some("sym:a.rs#function:root".into());
    req.direction = Direction::Out;
    for (h, expected) in [(0, 1), (1, 2), (2, 3)] {
        req.hops = h;
        let v = explore(home.path(), &req).unwrap();
        let bs = v["bundles"].as_array().unwrap();
        assert_eq!(bs.len(), expected);
        assert!(!bs
            .iter()
            .any(|n| n["label"] == "blocked" || n["label"] == "retired"));
    }
}
#[test]
fn worktree_and_project_scope_are_separate() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let mem = store.scope(p.path(), None).unwrap();
    let code = store.scope(p.path(), Some(p.path())).unwrap();
    store
        .write(&batch(
            mem.id,
            vec![
                node("feedback:one", "feedback", "needle", "active"),
                node("file:wrong.rs", "file", "needle", "active"),
            ],
            vec![],
        ))
        .unwrap();
    store
        .write(&batch(
            code.id,
            vec![
                node("file:right.rs", "file", "needle", "active"),
                node("feedback:wrong", "feedback", "needle", "active"),
            ],
            vec![],
        ))
        .unwrap();
    let mut req = request(p.path(), "needle");
    req.worktree = Some(p.path().into());
    req.hops = 0;
    for (view, expected) in [
        (View::Code, "file:right.rs"),
        (View::Memory, "feedback:one"),
    ] {
        req.view = view;
        let v = explore(home.path(), &req).unwrap();
        assert_eq!(v["seeds"].as_array().unwrap(), &vec![json!(expected)]);
    }
    req.view = View::Mixed;
    let v = explore(home.path(), &req).unwrap();
    assert_eq!(v["seeds"].as_array().unwrap().len(), 2);
}

#[test]
fn exact_precedes_fts_and_membership_is_downweighted() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let mut hit = node("sym:a.rs#function:other", "symbol", "other", "active");
    hit.summary = "needle appears here".into();
    let nodes = vec![
        node("file:a.rs", "file", "a.rs", "active"),
        node("sym:a.rs#function:needle", "symbol", "needle", "active"),
        hit,
    ];
    let edges = vec![
        edge(
            scope.id,
            "file:a.rs",
            "sym:a.rs#function:needle",
            "defines",
            "active",
        ),
        edge(
            scope.id,
            "file:a.rs",
            "sym:a.rs#function:other",
            "references",
            "active",
        ),
    ];
    store.write(&batch(scope.id, nodes, edges)).unwrap();
    let mut req = request(p.path(), "needle");
    req.hops = 0;
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(
        out["seeds"],
        json!(["sym:a.rs#function:needle", "sym:a.rs#function:other"])
    );
    req.query = None;
    req.node = Some("file:a.rs".into());
    req.hops = 1;
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(out["bundles"][0]["edges"][0]["kind"], "references");
    assert_eq!(out["bundles"][1]["key"], "sym:a.rs#function:other");
}

#[test]
fn fallback_words_are_newest_and_bounded() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let mut nodes = vec![node("feedback:old", "feedback", "zz", "active")];
    for i in 0..5000 {
        nodes.push(node(
            &format!("feedback:{i:04}"),
            "feedback",
            "filler",
            "active",
        ));
    }
    nodes.push(node("feedback:new", "feedback", "fresh xx", "active"));
    store.write(&batch(scope.id, nodes, vec![])).unwrap();
    let mut req = request(p.path(), "zz xx");
    req.view = View::Memory;
    req.hops = 0;
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(out["query"]["fallback"], "short_symbol");
    assert_eq!(out["seeds"], json!(["feedback:new"]));
}

#[test]
fn dirty_neighbours_and_states_are_filtered_and_reported() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    fs::write(p.path().join("dirty.py"), "changed").unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let root = "feedback:root";
    let nodes = vec![
        node(root, "feedback", "root", "active"),
        node("feedback:contradicted", "feedback", "conflict", "active"),
        node("feedback:tomb", "feedback", "dead", "tombstone"),
        node("feedback:dirty", "feedback", "dirty", "active"),
    ];
    let mut edges = vec![];
    for key in [
        "feedback:contradicted",
        "feedback:tomb",
        "feedback:dirty",
        "file:dirty.py",
    ] {
        edges.push(edge(
            scope.id,
            root,
            key,
            if key.starts_with("file:") {
                "about"
            } else {
                "supersedes"
            },
            "active",
        ));
    }
    let mut code_batch = batch(
        scope.id,
        vec![node("file:dirty.py", "file", "dirty.py", "active")],
        vec![],
    );
    code_batch.source.kind = "code".into();
    code_batch.source.locator = "dirty.py".into();
    code_batch.source.sha256 = "old".into();
    store.write(&code_batch).unwrap();
    store.write(&batch(scope.id, nodes, edges)).unwrap();
    store.read(|c| {
        c.execute("UPDATE g_nodes SET state = CASE key WHEN 'feedback:contradicted' THEN 'contradicted' ELSE 'dirty' END WHERE key IN ('feedback:contradicted','feedback:dirty')", []).unwrap();
        Ok(())
    }).unwrap();
    let mut req = request(p.path(), "");
    req.node = Some(root.into());
    req.hops = 1;
    let out = explore(home.path(), &req).unwrap();
    let keys: Vec<_> = out["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"feedback:contradicted"));
    assert!(
        !keys.contains(&"feedback:tomb")
            && !keys.contains(&"feedback:dirty")
            && !keys.contains(&"file:dirty.py")
    );
    assert_eq!(out["freshness"]["dirty_files"], json!(["dirty.py"]));
    assert!(matches!(
        out["freshness"]["state"].as_str(),
        Some("stale" | "partial")
    ));
}

#[test]
fn explore_enforces_memory_budget() {
    let home = tempfile::tempdir().unwrap();
    let p = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(p.path(), None).unwrap();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for i in 0..4 {
        let key = format!("feedback:{i}");
        let mut n = node(
            &key,
            "feedback",
            if i == 0 { "root" } else { "other" },
            "unconfirmed",
        );
        n.attrs = json!({"conditions": ["c".repeat(180)], "contradictions": ["x".repeat(180)]});
        nodes.push(n);
        if i > 0 {
            edges.push(edge(scope.id, "feedback:0", &key, "supersedes", "active"));
        }
    }
    store.write(&batch(scope.id, nodes, edges)).unwrap();
    let mut req = request(p.path(), "");
    req.node = Some("feedback:0".into());
    req.hops = 1;
    let out = explore(home.path(), &req).unwrap();
    assert!(!out["bundles"].as_array().unwrap().is_empty());
    assert!(out["budget"]["memory_used"].as_u64().unwrap() <= 1200);
    assert!(out["omitted"].as_u64().unwrap() > 0);
    fits(&out, 4096);
}
