use axiom::graph::{self, code::SyncOptions, measure, GraphStore};
use serde_json::{json, Value};
use std::{fs, path::Path};

fn fixtures() -> &'static Path {
    Path::new("tests/fixtures/graph/eval")
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dst.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
    }
}

fn evaluate() -> Vec<Value> {
    let manifest = measure::load_manifest(fixtures()).unwrap();
    assert_eq!(manifest.scenarios.len(), 12);
    manifest.scenarios.iter().map(|s| {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let home = temp.path().join("home");
        copy_dir(&fixtures().join(&s.repo), &repo);
        let store = GraphStore::open(&home).unwrap();
        graph::code::sync(&store, &repo, None, &SyncOptions::default()).unwrap();
        measure::seed_home(&home, &repo, fixtures(), Path::new(&s.seed)).unwrap();
        let brief = measure::graph_brief(&home, &repo, s).unwrap();
        let payload = &brief[brief.find("\n\n{").unwrap() + 2..];
        let value: Value = serde_json::from_str(payload).unwrap();
        let present = |items: &[String]| items.iter().filter(|x| brief.contains(x.as_str())).cloned().collect::<Vec<_>>();
        json!({"id":s.id,"seeds":value["seeds"].as_array().unwrap().len(),"explore_bytes":payload.len(),"budget":s.budget,"evidence":present(&s.required_evidence),"conditions":present(&s.conditions),"findings":present(&s.major_findings)})
    }).collect()
}

#[test]
#[ignore]
fn write_baseline() {
    let path = Path::new("tests/fixtures/graph_seed/baseline.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(&evaluate()).unwrap()),
    )
    .unwrap();
}

#[test]
fn scenarios_retain_baseline_and_seed_multiword_queries() {
    let baseline: Vec<Value> =
        serde_json::from_slice(&fs::read("tests/fixtures/graph_seed/baseline.json").unwrap())
            .unwrap();
    let manifest = measure::load_manifest(fixtures()).unwrap();
    let evidence_floors = [0, 0, 2, 0, 0, 2, 2, 2, 2, 2, 2, 2];
    assert_eq!(baseline.len(), manifest.scenarios.len());
    for (((after, before), scenario), floor) in evaluate()
        .iter()
        .zip(&baseline)
        .zip(&manifest.scenarios)
        .zip(evidence_floors)
    {
        assert_eq!(after["id"], before["id"]);
        assert!(
            after["explore_bytes"].as_u64().unwrap() <= scenario.budget as u64,
            "{}",
            scenario.id
        );
        if scenario.query.split_whitespace().count() > 1 {
            assert!(
                after["seeds"].as_u64().unwrap() > 0,
                "{}: {}",
                scenario.id,
                scenario.query
            );
        }
        for field in ["evidence", "conditions", "findings"] {
            assert!(
                after[field].as_array().unwrap().len() >= before[field].as_array().unwrap().len(),
                "{} lost {field} count",
                scenario.id
            );
            for item in before[field].as_array().unwrap() {
                assert!(
                    after[field].as_array().unwrap().contains(item),
                    "{} lost {field}: {item}",
                    scenario.id
                );
            }
        }
        assert!(
            after["evidence"].as_array().unwrap().len() >= floor,
            "{} evidence below documented floor {floor}",
            scenario.id
        );
    }
}

#[test]
#[ignore]
fn print_after() {
    println!("{}", serde_json::to_string_pretty(&evaluate()).unwrap());
}

#[test]
fn ranking_snippets_scope_and_budget() {
    use axiom::graph::{
        query::{explore, ExploreRequest, View},
        Direction,
    };
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    let other = temp.path().join("other");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&other).unwrap();
    let source = format!(
        "# {}\ndef stock_reservation():\n    return 1\n",
        "測".repeat(120)
    );
    fs::write(repo.join("stock reservation.py"), &source).unwrap();
    fs::write(other.join("stock_secret.py"), "def stock_secret(): pass\n").unwrap();
    let store = GraphStore::open(&home).unwrap();
    for path in [&repo, &other] {
        graph::code::sync(&store, path, None, &SyncOptions::default()).unwrap();
    }
    let mut req = ExploreRequest {
        project_dir: repo,
        worktree: None,
        project: None,
        query: Some("stock reservation.py".into()),
        node: None,
        view: View::Code,
        direction: Direction::Both,
        hops: 0,
        budget: 4096,
        refresh: false,
    };
    let out = explore(&home, &req).unwrap();
    assert_eq!(out["seeds"][0], "file:stock reservation.py");
    assert_eq!(out, explore(&home, &req).unwrap());
    assert!(!out.to_string().contains("stock_secret"));
    assert!(out["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["snippet"].is_string()));
    for bundle in out["bundles"].as_array().unwrap() {
        assert!(bundle["rank"].is_u64());
        if let Some(snippet) = bundle["snippet"].as_str() {
            assert!(snippet.len() <= 240 && source.contains(snippet));
            assert!(bundle["line"].as_u64().unwrap() >= 1);
        }
    }
    for budget in [1, 100, 256, 500, 1200, 2400, 4096] {
        req.budget = budget;
        let value = explore(&home, &req).unwrap();
        assert!(serde_json::to_string_pretty(&value).unwrap().len() <= budget);
    }
}

#[test]
fn word_scores_deduplication_and_cjk_fallback() {
    use axiom::graph::{
        query::{explore, ExploreRequest, View},
        types::{Batch, Direction, NodeSpec, SourceSpec},
    };
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(repo.path(), None).unwrap();
    let nodes = [
        ("feedback:a", "alpha only"),
        ("feedback:b", "alpha beta"),
        ("feedback:c", "東京"),
    ]
    .into_iter()
    .map(|(key, label)| NodeSpec {
        key: key.into(),
        kind: "feedback".into(),
        label: label.into(),
        summary: String::new(),
        attrs: json!({}),
        entry_id: None,
        state: "active".into(),
    })
    .collect();
    store
        .write(&Batch {
            scope: scope.id,
            source: SourceSpec {
                kind: "test".into(),
                locator: "test".into(),
                sha256: String::new(),
                version: "1".into(),
                state: "active".into(),
            },
            nodes,
            edges: vec![],
            replace_source: false,
        })
        .unwrap();
    let mut req = ExploreRequest {
        project_dir: repo.path().into(),
        worktree: None,
        project: None,
        query: Some("alpha beta".into()),
        node: None,
        view: View::Memory,
        direction: Direction::Both,
        hops: 0,
        budget: 4096,
        refresh: false,
    };
    // Neither label equals the complete query after adding a harmless stop word.
    req.query = Some("alpha beta the".into());
    let value = explore(home.path(), &req).unwrap();
    let mut scores = std::collections::HashMap::<i64, f64>::new();
    for word in ["alpha", "beta"] {
        for (id, rank) in store.fts(&[scope.id], &format!("\"{word}\""), 500).unwrap() {
            *scores.entry(id).or_default() -= rank;
        }
    }
    let mut expected: Vec<_> = scores
        .into_iter()
        .map(|(id, score)| (store.node_by_id(id).unwrap().unwrap().key, score))
        .collect();
    expected.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    assert_eq!(
        value["seeds"],
        json!(expected.iter().map(|x| &x.0).collect::<Vec<_>>())
    );
    assert!(value["bundles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|b| b["line"].is_null() && b["snippet"].is_null()));
    req.query = Some("alpha ALPHA beta the".into());
    assert_eq!(value["seeds"], explore(home.path(), &req).unwrap()["seeds"]);
    req.query = Some("alpha beta the x".into());
    assert_eq!(value["seeds"], explore(home.path(), &req).unwrap()["seeds"]);
    req.query = Some("東京".into());
    let value = explore(home.path(), &req).unwrap();
    assert_eq!(value["query"]["fallback"], "short_cjk");
    assert_eq!(value["seeds"][0], "feedback:c");
}

#[test]
fn exact_then_alias_then_higher_fts_score() {
    use axiom::graph::{
        query::{explore, ExploreRequest, View},
        types::{Batch, Direction, NodeSpec, SourceSpec},
    };
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let dir = home.path().join("orchestrate/demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("aliases.csv"),
        "canonical,alias\ncanonical,shorthand\n",
    )
    .unwrap();
    let store = GraphStore::open(home.path()).unwrap();
    let scope = store.scope(repo.path(), None).unwrap();
    let nodes = [
        ("feedback:exact", "shorthand needle"),
        ("feedback:alias", "canonical"),
        ("feedback:fts", "canonical needle needle needle"),
    ]
    .into_iter()
    .map(|(key, label)| NodeSpec {
        key: key.into(),
        kind: "feedback".into(),
        label: label.into(),
        summary: String::new(),
        attrs: json!({}),
        entry_id: None,
        state: "active".into(),
    })
    .collect();
    store
        .write(&Batch {
            scope: scope.id,
            source: SourceSpec {
                kind: "test".into(),
                locator: "test".into(),
                sha256: String::new(),
                version: "1".into(),
                state: "active".into(),
            },
            nodes,
            edges: vec![],
            replace_source: false,
        })
        .unwrap();
    let mut scores = std::collections::HashMap::<String, f64>::new();
    for word in ["canonical", "needle"] {
        for (id, score) in store.fts(&[scope.id], &format!("\"{word}\""), 500).unwrap() {
            *scores
                .entry(store.node_by_id(id).unwrap().unwrap().key)
                .or_default() -= score;
        }
    }
    assert!(scores["feedback:fts"] > scores["feedback:alias"]);
    let req = ExploreRequest {
        project_dir: repo.path().into(),
        worktree: None,
        project: Some("demo".into()),
        query: Some("shorthand needle".into()),
        node: None,
        view: View::Memory,
        direction: Direction::Both,
        hops: 0,
        budget: 4096,
        refresh: false,
    };
    let out = explore(home.path(), &req).unwrap();
    assert_eq!(
        out["seeds"],
        json!(["feedback:exact", "feedback:alias", "feedback:fts"])
    );
}

#[test]
fn fit_sheds_snippets_then_edges_then_lowest_ranked_bundles() {
    use axiom::graph::budget;
    let bundles: Vec<_> = (0..3).map(|rank| json!({"kind":"file", "rank":rank, "snippet":"s".repeat(120), "edges":[{"evidence":format!("edge-{rank}")}] })).collect();
    let payload =
        json!({"budget":{"used":0,"memory_used":0}, "bundles":bundles, "omitted":0, "truncated":0});
    let mut found_all = false;
    let mut found_dropped = false;
    let mut found_partial_edges = false;
    for limit in 200..1500 {
        let out = budget::fit(payload.clone(), limit, usize::MAX);
        let Some(kept) = out["bundles"].as_array() else {
            continue;
        };
        if kept.len() == 3 {
            if kept.iter().any(|b| b["snippet"].is_string()) {
                assert!(kept
                    .iter()
                    .all(|b| b["edges"].as_array().unwrap().len() == 1));
            }
            if kept
                .iter()
                .any(|b| b["edges"].as_array().unwrap().is_empty())
            {
                assert!(kept.iter().all(|b| b["snippet"].is_null()));
            }
            if kept[0]["edges"].as_array().unwrap().len() == 1
                && kept[1]["edges"].as_array().unwrap().len() == 1
                && kept[2]["edges"].as_array().unwrap().is_empty()
            {
                found_partial_edges = true;
            }
        }
        if kept.len() == 3 && kept.iter().all(|b| b["snippet"].is_null()) {
            found_all = true;
        }
        if kept.len() == 2 {
            found_dropped = true;
            assert_eq!(out["omitted"], 1);
            assert_eq!(out["truncated"], 7);
            assert!(kept.iter().all(|b| b["snippet"].is_null()));
            assert_eq!(kept[0]["rank"], 0);
            assert_eq!(kept[1]["rank"], 1);
            assert_eq!(kept[0]["edges"], json!([]));
            assert_eq!(kept[1]["edges"], json!([]));
        }
    }
    assert!(found_all && found_partial_edges && found_dropped);
}
