use axiom::{
    research_workspace::{
        annotations::*,
        index::Index,
        manifest::{locate, Format, Link, LocateOptions},
        read::{hash_file, Limits},
    },
    seams::FakeClock,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tempfile::TempDir;

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest)
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

#[test]
fn downstream_splits_both_sides_and_terminates_on_cycles() {
    let (_tmp, project, _, ix) = fixture("rawonly");
    fs::write(
        project.join("data/evidence.csv"),
        "evidence_id\nRE1; shared\nRE2\n",
    )
    .unwrap();
    fs::write(
        project.join("data/raw_measurements.csv"),
        "row_id,evidence_id\nR1,other; shared\nR2,unrelated\n",
    )
    .unwrap();
    let mut located = ix.located().as_ref().clone();
    located.manifest.links = vec![
        Link {
            from: "evidence".into(),
            to: "raw_measurements".into(),
            key: "evidence_id".into(),
            separator: Some(";".into()),
        },
        Link {
            from: "raw_measurements".into(),
            to: "evidence".into(),
            key: "evidence_id".into(),
            separator: Some(";".into()),
        },
        Link {
            from: "evidence".into(),
            to: "run_a".into(),
            key: "evidence_id".into(),
            separator: None,
        },
    ];
    located
        .manifest
        .artifacts
        .iter_mut()
        .find(|a| a.id == "run_a")
        .unwrap()
        .format = Format::Text;
    let ix = Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap();
    let pins = [
        snap("reached", &["raw_measurements"]),
        snap("unreached", &["run_a"]),
    ];
    assert_eq!(
        downstream(&ix, "evidence:RE1; shared", &pins),
        vec!["raw_measurements:R1", "reached"]
    );
    assert!(downstream(&ix, "evidence:absent", &pins).is_empty());
}
fn tree(p: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_owned(),
                    hash_file(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(p, p, &mut out);
    out
}
fn fixture(name: &str) -> (TempDir, PathBuf, PathBuf, Index) {
    let tmp = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name);
    let project = tmp.path().join("project");
    let store = tmp.path().join("store");
    if name == "rram" {
        copy_tree(&source.join("project"), &project);
        copy_tree(&source.join("store"), &store);
    } else {
        copy_tree(&source, &project);
        fs::create_dir_all(&store).unwrap();
    }
    let located = locate(
        &project,
        &LocateOptions {
            store: if name == "rram" {
                Some(store.clone())
            } else {
                None
            },
            orch_home: None,
        },
    )
    .unwrap();
    let ix = Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap();
    (tmp, project, store, ix)
}
fn draft(target: &str, field: &str, after: &str) -> CorrectionDraft {
    CorrectionDraft {
        target: target.into(),
        field: field.into(),
        after: after.into(),
        who: "Ada".into(),
        why: "typo".into(),
    }
}
fn snap(id: &str, artifacts: &[&str]) -> SnapshotEntry {
    SnapshotEntry {
        snapshot_id: id.into(),
        artifacts: artifacts.iter().map(|s| s.to_string()).collect(),
        sha256: "hash".into(),
    }
}
fn receipt(stage: &str) -> ReceiptRef {
    ReceiptRef {
        client_id: "client".into(),
        session: "session".into(),
        stage: stage.into(),
        detail: None,
    }
}

#[test]
fn downstream_state_persistence_and_source_immutability() {
    let (tmp, project, orch, ix) = fixture("rram");
    let home = tmp.path().join("home");
    let before = (tree(&project), tree(&orch));
    for s in [
        snap("snap-evidence", &["evidence"]),
        snap("snap-splits", &["splits"]),
        snap("snap-other", &["inventory"]),
    ] {
        register_snapshot(&home, "rram", &s).unwrap();
    }
    let clock = FakeClock::new(42);
    let mut store = AnnotationStore::open(&home, "rram").unwrap();
    assert_eq!(store.file_sha256().unwrap(), None);
    let spot = store
        .spot_check(
            &ix,
            "evidence:E1",
            Some("node"),
            Verdict::Dispute {
                reason: "ambiguous".into(),
            },
            "Grace",
            &clock,
        )
        .unwrap();
    let other = store
        .spot_check(&ix, "evidence:E2", None, Verdict::Confirm, "Ada", &clock)
        .unwrap();
    assert_ne!(spot.id, other.id);
    assert_eq!(spot.who, "Grace");
    assert_eq!(spot.node.as_deref(), Some("node"));
    assert_eq!(
        spot.verdict,
        Verdict::Dispute {
            reason: "ambiguous".into()
        }
    );
    assert_eq!(
        spot.research_version,
        ix.located().manifest.research_version
    );
    assert_eq!(
        spot.target_sha256,
        hash_file(&project.join("data/evidence.csv")).unwrap()
    );
    assert_eq!(store.spot_checks(Some("evidence:E1")), vec![spot]);
    let c = store
        .correct(&ix, draft("evidence:E1", "sha256", "new"), &clock)
        .unwrap();
    assert_eq!(
        c.before,
        ix.row("evidence", ix.find("evidence", "E1").unwrap())
            .unwrap()
            .get("sha256")
    );
    let expected = vec![
        "measurements:M1",
        "snap-evidence",
        "snap-splits",
        "splits:1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(c.invalidates, expected);
    assert_eq!(
        store.invalidated(),
        expected.into_iter().collect::<BTreeSet<_>>()
    );
    assert_eq!(c.reverify, Reverify::Pending);
    for stage in ["sent", "received", "applied"] {
        store.set_receipt(&c.id, receipt(stage)).unwrap();
        assert_eq!(
            AnnotationStore::open(&home, "rram")
                .unwrap()
                .corrections(None)[0]
                .receipt
                .as_ref()
                .unwrap()
                .stage,
            stage
        );
    }
    store.set_receipt(&c.id, receipt("failed")).unwrap();
    store.keep_draft(&c.id, "request body").unwrap();
    store.set_reverify(&c.id, Reverify::Passed).unwrap();
    let restored = AnnotationStore::open(&home, "rram")
        .unwrap()
        .corrections(Some("evidence:E1"));
    assert_eq!(restored[0].receipt.as_ref().unwrap().stage, "failed");
    assert_eq!(restored[0].draft.as_deref(), Some("request body"));
    assert_eq!(restored[0].reverify, Reverify::Passed);
    assert!(store.file_sha256().unwrap().is_some());
    assert_eq!(
        store
            .set_reverify("missing", Reverify::Pending)
            .unwrap_err()
            .kind,
        "not_found"
    );
    assert_eq!(
        store.keep_draft("missing", "x").unwrap_err().kind,
        "not_found"
    );
    assert_eq!(
        store
            .set_receipt("missing", receipt("sent"))
            .unwrap_err()
            .kind,
        "not_found"
    );
    assert_eq!(before, (tree(&project), tree(&orch)));
    let names = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        ["project", "store", "home"]
            .into_iter()
            .map(Into::into)
            .collect()
    );
}
#[test]
fn validation_and_reverify_raw_source() {
    let (tmp, project, _, ix) = fixture("rawonly");
    let home = tmp.path().join("home");
    let clock = FakeClock::new(7);
    let mut store = AnnotationStore::open(&home, "rawonly").unwrap();
    let mut d = draft("evidence:RE1", "raw_file", "runs/device_A.csv");
    d.who = " ".into();
    assert_eq!(
        store.correct(&ix, d, &clock).unwrap_err().kind,
        "invalid_argument"
    );
    let mut d = draft("evidence:RE1", "raw_file", "x");
    d.why.clear();
    assert_eq!(
        store.correct(&ix, d, &clock).unwrap_err().kind,
        "invalid_argument"
    );
    assert_eq!(
        store
            .correct(&ix, draft("evidence:missing", "raw_file", "x"), &clock)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    assert_eq!(
        store
            .correct(&ix, draft("evidence:RE1", "unknown", "x"), &clock)
            .unwrap_err()
            .kind,
        "invalid_argument"
    );
    let c = store
        .correct(
            &ix,
            draft("evidence:RE1", "raw_file", "runs/device_A.csv"),
            &clock,
        )
        .unwrap();
    assert_eq!(c.before, "runs/device_A.csv");
    assert_eq!(reverify(&ix, &c), Reverify::Passed);
    let missing = store
        .correct(
            &ix,
            draft("evidence:RE1", "raw_file", "runs/missing.csv"),
            &clock,
        )
        .unwrap();
    assert!(matches!(reverify(&ix, &missing), Reverify::Failed { .. }));
    let invalid_page = store
        .correct(&ix, draft("evidence:RE1", "physical_page", "0"), &clock)
        .unwrap();
    assert!(matches!(
        reverify(&ix, &invalid_page),
        Reverify::Failed { .. }
    ));
    let file = project.join("data/evidence.csv");
    let original = fs::read(&file).unwrap();
    fs::write(&file, b"changed").unwrap();
    assert!(matches!(reverify(&ix, &c), Reverify::Failed { .. }));
    let text = String::from_utf8(original.clone()).unwrap();
    let no_row = text
        .lines()
        .filter(|s| !s.starts_with("RE1,"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&file, no_row).unwrap();
    let gone = Index::build(ix.located().clone(), Limits::default(), &|_, _| {}).unwrap();
    fs::write(&file, original).unwrap();
    assert!(matches!(reverify(&gone, &c), Reverify::Failed { .. }));
    let (client, request) = reverify_request(&c, "review-session");
    assert_eq!(client, format!("research-reverify-{}", c.id));
    for s in [
        &c.target,
        &c.field,
        &c.before,
        &c.after,
        &c.who,
        &c.why,
        "review-session",
    ] {
        assert!(request.contains(s));
    }
}
#[test]
fn damaged_lines_and_snapshot_last_wins() {
    let (tmp, _, _, ix) = fixture("rawonly");
    let home = tmp.path().join("home");
    let clock = FakeClock::new(1);
    let mut store = AnnotationStore::open(&home, "rawonly").unwrap();
    assert!(store.corrections(None).is_empty());
    let c = store
        .correct(
            &ix,
            draft("evidence:RE1", "raw_file", "runs/device_A.csv"),
            &clock,
        )
        .unwrap();
    let file = home.join("annotations/rawonly/annotations.jsonl");
    OpenOptionsExt::append(&file, b"{\"kind\":\"correction\"");
    assert_eq!(
        AnnotationStore::open(&home, "rawonly")
            .unwrap()
            .corrections(None),
        vec![c.clone()]
    );
    store.set_reverify(&c.id, Reverify::Passed).unwrap();
    assert_eq!(
        AnnotationStore::open(&home, "rawonly")
            .unwrap()
            .corrections(None)[0]
            .reverify,
        Reverify::Passed
    );
    assert!(snapshots(&home, "empty").is_empty());
    let a = snap("a", &["evidence"]);
    let b = snap("b", &["other"]);
    let replacement = snap("a", &["new"]);
    for s in [&a, &b, &replacement] {
        register_snapshot(&home, "rawonly", s).unwrap();
    }
    OpenOptionsExt::append(&home.join("snapshots/rawonly/registry.jsonl"), b"{broken\n");
    assert_eq!(snapshots(&home, "rawonly"), vec![b, replacement]);
}
struct OpenOptionsExt;
impl OpenOptionsExt {
    fn append(p: &Path, bytes: &[u8]) {
        fs::OpenOptions::new()
            .append(true)
            .open(p)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}
