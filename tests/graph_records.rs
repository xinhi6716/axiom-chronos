use axiom::graph::{Batch, EdgeSpec, GraphStore, NodeRef, NodeSpec, SourceSpec};
use serde_json::json;
use sha2::{Digest, Sha256};

fn supports_edge(src_kind: &str, dst_kind: &str, kind: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let store = GraphStore::open(&tmp.path().join("home")).unwrap();
    let scope = store.scope(&project, None).unwrap().id;
    let node = |key: &str, kind: &str| NodeSpec {
        key: key.into(),
        kind: kind.into(),
        label: key.into(),
        summary: String::new(),
        attrs: json!({}),
        entry_id: None,
        state: "active".into(),
    };
    let reference = |key: &str| NodeRef {
        scope,
        key: key.into(),
    };
    let batch = Batch {
        scope,
        source: SourceSpec {
            kind: "test".into(),
            locator: "test".into(),
            sha256: "test".into(),
            version: "1".into(),
            state: "active".into(),
        },
        nodes: vec![node("source-node", src_kind), node("target-node", dst_kind)],
        edges: vec![EdgeSpec {
            src: reference("source-node"),
            dst: reference("target-node"),
            kind: kind.into(),
            locator: String::new(),
            weight: 1000,
            state: "active".into(),
            attrs: json!({}),
        }],
        replace_source: false,
    };
    store
        .write(&batch)
        .expect("graph-records edge must be accepted by GraphStore");
}

#[test]
fn research_artifact_can_cite_code() {
    supports_edge("artifact", "symbol", "cites");
}

#[test]
fn decision_can_be_decided_in_unit() {
    supports_edge("decision", "unit", "decided_in");
}

use axiom::graph::{
    self,
    code::SyncOptions,
    ingest::{import_source, sync_orch, SourceLink},
    keys, Direction,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst.join(e.file_name()));
        } else {
            fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
    }
}
fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
struct Records {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    wish: PathBuf,
    store: GraphStore,
    scope: i64,
    head: String,
}
impl Records {
    fn new() -> Self {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Tester"]);
        git(&repo, &["config", "user.email", "tester@example.test"]);
        git(&repo, &["config", "core.autocrlf", "false"]);
        fs::write(
            repo.join("src/main.rs"),
            "fn changed() {\n    let x = 1;\n}\n\nfn untouched() {\n    let y = 2;\n}\n",
        )
        .unwrap();
        fs::write(repo.join("src/report.rs"), "fn report_only() {}\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "before"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        fs::write(
            repo.join("src/main.rs"),
            "fn changed() {\n    let x = 3;\n}\n\nfn untouched() {\n    let y = 2;\n}\n",
        )
        .unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "after"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let wish = t.path().join("w-1234567890123-abcdef");
        copy_dir(Path::new("tests/fixtures/graph/records"), &wish);
        let evidence_path = wish.join("reports/u.evidence.tsv");
        let evidence = fs::read_to_string(&evidence_path)
            .unwrap()
            .replace("__BASE__", &base)
            .replace("__TREE__", &head);
        fs::write(evidence_path, evidence).unwrap();
        let verdict_path = wish.join("reports/u.verdict.json");
        let verdict = fs::read_to_string(&verdict_path)
            .unwrap()
            .replace("__TREE__", &head);
        fs::write(verdict_path, verdict).unwrap();
        let store = GraphStore::open(&t.path().join("home")).unwrap();
        graph::code::sync(&store, &repo, None, &SyncOptions::default()).unwrap();
        let scope = store.scope(&repo, None).unwrap().id;
        Self {
            _tmp: t,
            repo,
            wish,
            store,
            scope,
            head,
        }
    }
    fn get(&self, key: &str) -> axiom::graph::Node {
        self.store
            .node(&NodeRef {
                scope: self.scope,
                key: key.into(),
            })
            .unwrap()
            .unwrap()
    }
    fn out(&self, key: &str) -> Vec<axiom::graph::Edge> {
        self.store
            .edges(self.get(key).id, Direction::Out, 100)
            .unwrap()
    }
    fn sync(&self) -> axiom::graph::IngestReport {
        sync_orch(&self.store, &self.wish, &self.repo).unwrap()
    }
}
#[test]
fn projects_records_replays_and_isolates_wishes() {
    let r = Records::new();
    let w = "w-1234567890123-abcdef";
    let unit = keys::unit(w, "u");
    let first = r.sync();
    assert!(first.nodes_written > 0 && first.edges_written > 0);
    for key in [
        keys::wish(w),
        keys::module(w, "app"),
        unit.clone(),
        keys::scope_claim(w, "u", 1),
        keys::scope_claim(w, "u", 2),
        keys::artifact(&format!("orch/{w}/reports/u.md")),
        keys::finding(w, "u", 1),
        keys::decision(w, "u", 1),
    ] {
        r.get(&key);
    }
    let module = r.get(&keys::module(w, "app"));
    assert!(module.attrs["paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x == "future/"));
    assert!(r
        .out(&keys::module(w, "app"))
        .iter()
        .any(|e| e.kind == "depends_on" && e.dst.key == keys::module(w, "base")));
    assert!(r
        .out(&keys::module(w, "app"))
        .iter()
        .any(|e| e.kind == "owns" && e.dst.key == keys::file("src/main.rs")));
    assert!(r.out(&unit).iter().any(|e| e.kind == "in_module"));
    assert!(r
        .out(&keys::scope_claim(w, "u", 1))
        .iter()
        .any(|e| e.kind == "claims_path"));
    let pattern = r
        .out(&keys::scope_claim(w, "u", 2))
        .into_iter()
        .find(|e| e.kind == "claims_path" && r.get(&e.dst.key).kind == "source")
        .unwrap();
    assert_eq!(r.get(&pattern.dst.key).attrs["path"], "src/generated/");
    assert!(r
        .out(&keys::decision(w, "u", 1))
        .iter()
        .any(|e| e.kind == "decided_in" && e.dst.key == unit));
    let changes = r.out(&unit);
    assert!(changes
        .iter()
        .any(|e| e.kind == "changes" && e.dst.key == keys::file("src/main.rs")));
    let changed = keys::symbol("src/main.rs", "function", "changed", 1);
    let untouched = keys::symbol("src/main.rs", "function", "untouched", 1);
    assert!(changes
        .iter()
        .any(|e| e.kind == "changes" && e.dst.key == changed));
    assert!(!changes
        .iter()
        .any(|e| e.kind == "changes" && e.dst.key == untouched));
    let report_edges = r.out(&keys::unit(w, "report-only"));
    assert!(report_edges.iter().any(|e| e.kind == "changes"
        && e.dst.key == keys::file("src/report.rs")
        && e.locator == "report"));
    assert!(!report_edges.iter().any(|e| e.dst.key.starts_with("sym:")));
    let finding = keys::finding(w, "u", 1);
    let f = r.get(&finding);
    assert_eq!(f.attrs["tree"], r.head);
    assert_eq!(f.attrs["waived"], true);
    assert!(!f.attrs["sha256"].as_str().unwrap().is_empty());
    let fe = r.out(&finding);
    assert!(fe.iter().any(|e| e.kind == "located_in"));
    assert!(fe.iter().any(|e| e.kind == "about" && e.dst.key == changed));
    let second = r.sync();
    assert_eq!(
        (second.nodes_written, second.edges_written, second.revision),
        (0, 0, first.revision)
    );
    let evidence_path = r.wish.join("reports/u.evidence.tsv");
    let evidence = fs::read_to_string(&evidence_path).unwrap();
    fs::write(
        &evidence_path,
        evidence
            .lines()
            .filter(|line| !line.starts_with("waive\t"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    r.sync();
    assert!(r.get(&finding).attrs.get("waived").is_none());
    fs::write(&evidence_path, evidence).unwrap();
    let restored = r.sync();
    assert_eq!(r.get(&finding).attrs["waived"], true);
    let replay = r.sync();
    assert_eq!(
        (replay.nodes_written, replay.edges_written, replay.revision),
        (0, 0, restored.revision)
    );
    let other = r._tmp.path().join("w-1234567890124-abcdef");
    copy_dir(&r.wish, &other);
    sync_orch(&r.store, &other, &r.repo).unwrap();
    assert_ne!(
        r.get(&unit).id,
        r.get(&keys::unit("w-1234567890124-abcdef", "u")).id
    );
    let new_content = "fn changed() { let x = 4; }\n";
    fs::write(r.repo.join("src/main.rs"), new_content).unwrap();
    git(&r.repo, &["add", "src/main.rs"]);
    git(&r.repo, &["commit", "-qm", "new review tree"]);
    let new_tree = git(&r.repo, &["rev-parse", "HEAD"]);
    let verdict_path = r.wish.join("reports/u.verdict.json");
    let verdict = fs::read_to_string(&verdict_path)
        .unwrap()
        .replace(&r.head, &new_tree);
    fs::write(&verdict_path, verdict).unwrap();
    let rewritten = r.sync();
    let later = r.get(&finding);
    assert_eq!(later.attrs["tree"], f.attrs["tree"]);
    assert_eq!(later.attrs["sha256"], f.attrs["sha256"]);
    assert_eq!(later.attrs["waived"], true);
    let file = r.get(&keys::file("src/main.rs"));
    let newer_edge = r
        .store
        .edges(file.id, Direction::In, 100)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "located_in" && e.src.key.starts_with(&format!("{finding}@")))
        .unwrap();
    let newer = r.get(&newer_edge.src.key);
    assert_eq!(newer.attrs["tree"], new_tree);
    assert_eq!(
        newer.attrs["sha256"],
        format!("{:x}", Sha256::digest(new_content.as_bytes()))
    );
    assert_ne!(newer.attrs["sha256"], f.attrs["sha256"]);
    assert!(newer.attrs.get("waived").is_none());
    let replay = r.sync();
    assert_eq!(
        (replay.nodes_written, replay.edges_written, replay.revision),
        (0, 0, rewritten.revision)
    );
    fs::remove_file(r.wish.join("reports/report-only.json")).unwrap();
    r.sync();
    assert!(r
        .out(&keys::unit(w, "report-only"))
        .iter()
        .any(|e| e.locator == "report" && e.state == "stale"));
    assert_eq!(
        r.get(&keys::artifact(&format!(
            "orch/{w}/reports/report-only.json"
        )))
        .state,
        "stale"
    );
}
#[test]
fn research_quotes_and_stale_source() {
    let r = Records::new();
    fs::copy(r.wish.join("research.md"), r.repo.join("research.md")).unwrap();
    let target = keys::file("src/main.rs");
    let links = vec![
        SourceLink {
            to: target.clone(),
            quote: "Quote from research.".into(),
            line: Some(1),
        },
        SourceLink {
            to: target.clone(),
            quote: "fabricated quote".into(),
            line: None,
        },
        SourceLink {
            to: "sym:missing.rs#function:nope".into(),
            quote: "Quote from research.".into(),
            line: None,
        },
    ];
    let a = import_source(&r.store, &r.repo, "research.md", &links).unwrap();
    assert_eq!(a.skipped.len(), 2);
    assert!(a
        .skipped
        .iter()
        .any(|(name, reason)| name == "sym:missing.rs#function:nope" && reason.contains("target")));
    assert!(a
        .skipped
        .iter()
        .any(|(_, reason)| reason.contains("verbatim")));
    let key = keys::artifact("research.md");
    let n = r.get(&key);
    assert_eq!(n.attrs.as_object().unwrap().len(), 2);
    assert_eq!(n.attrs["state"], "reported/unreviewed");
    let edges = r.out(&key);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].dst.key, target);
    assert_eq!(edges[0].attrs.as_object().unwrap().len(), 3);
    assert_eq!(edges[0].attrs["quote"], "Quote from research.");
    assert_eq!(edges[0].attrs["line"], 1);
    assert_eq!(edges[0].attrs["sha256"], n.attrs["sha256"]);
    let b = import_source(&r.store, &r.repo, "research.md", &links).unwrap();
    assert_eq!(
        (b.nodes_written, b.edges_written, b.revision),
        (0, 0, a.revision)
    );
    fs::remove_file(r.repo.join("research.md")).unwrap();
    import_source(&r.store, &r.repo, "research.md", &links).unwrap();
    assert_eq!(r.get(&key).state, "stale");
    assert_eq!(r.out(&key)[0].state, "stale");
}

#[test]
fn absolute_research_and_crlf_replay() {
    let r = Records::new();
    let research = r._tmp.path().join("home/orchestrate/repo/research");
    fs::create_dir_all(&research).unwrap();
    let file = research.join("notes.md");
    fs::write(&file, "first\r\nsecond\r\n").unwrap();
    let link = SourceLink {
        to: keys::file("src/main.rs"),
        quote: "first".into(),
        line: None,
    };
    let first = import_source(&r.store, &r.repo, file.to_str().unwrap(), &[link.clone()]).unwrap();
    assert!(first.skipped.is_empty());
    let not_verbatim = SourceLink {
        to: keys::file("src/main.rs"),
        quote: "first\nsecond".into(),
        line: None,
    };
    assert_eq!(
        import_source(
            &r.store,
            &r.repo,
            file.to_str().unwrap(),
            &[link.clone(), not_verbatim]
        )
        .unwrap()
        .skipped
        .len(),
        1
    );
    fs::write(&file, "first\nsecond\n").unwrap();
    let next = import_source(&r.store, &r.repo, file.to_str().unwrap(), &[link]).unwrap();
    assert_eq!(
        (next.nodes_written, next.edges_written, next.revision),
        (0, 0, first.revision)
    );
    fs::remove_file(&file).unwrap();
    import_source(&r.store, &r.repo, file.to_str().unwrap(), &[]).unwrap();
    assert_eq!(r.get(&keys::artifact("research/notes.md")).state, "stale");
    let outside = r._tmp.path().join("outside.md");
    fs::write(&outside, "outside").unwrap();
    assert!(import_source(&r.store, &r.repo, outside.to_str().unwrap(), &[]).is_err());
    let home = r._tmp.path().join("home");
    fs::create_dir_all(home.join("goals")).unwrap();
    fs::write(
        home.join("goals/goal.json"),
        json!({"wish":"test","status":"active","cwd":r.repo,"project":"alias"}).to_string(),
    )
    .unwrap();
    let alias = home.join("orchestrate/alias/research");
    fs::create_dir_all(&alias).unwrap();
    fs::write(alias.join("alias.md"), "reference").unwrap();
    assert!(import_source(
        &r.store,
        &r.repo,
        alias.join("alias.md").to_str().unwrap(),
        &[]
    )
    .is_ok());
}
