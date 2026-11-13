use axiom::{
    paper::{self, http::CannedTransport, ArtifactKind, EvidenceBundle},
    research_program,
    research_workspace::{
        export::{self, Snapshot},
        index::Index,
        manifest::{locate, LocateOptions},
        paper_handoff::{store_for, HandoffRecord, PaperHandoff},
        read::{hash_file, Limits},
        seams::{SnapshotConsumer, SnapshotRef},
    },
    seams::{sha256_hex, FakeClock},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, result: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().into(),
                    hash_file(&path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
struct Case {
    tmp: tempfile::TempDir,
    source: PathBuf,
    source_store: PathBuf,
    snapshot: Snapshot,
    reference: SnapshotRef,
    consumer: PaperHandoff,
    transport: Arc<CannedTransport>,
}
impl Case {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram");
        let source = tmp.path().join("source");
        copy_tree(&fixture.join("project"), &source);
        let source_store = tmp.path().join("source-store");
        copy_tree(&fixture.join("store"), &source_store);
        let located = locate(
            &source,
            &LocateOptions {
                store: Some(source_store.clone()),
                orch_home: None,
            },
        )
        .unwrap();
        let index = Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap();
        let (snapshot, path) = export::snapshot(
            &index,
            &tmp.path().join("research-home"),
            &tmp.path().join("snapshots"),
            &FakeClock::new(1),
        )
        .unwrap();
        let reference = SnapshotRef {
            snapshot_id: snapshot.snapshot_id.clone(),
            research_id: snapshot.research_id.clone(),
            research_version: snapshot.research_version.clone(),
            path,
        };
        let canned = tmp.path().join("canned");
        fs::create_dir_all(&canned).unwrap();
        fs::write(canned.join("index.json"), "[]").unwrap();
        let transport = Arc::new(CannedTransport::from_dir(&canned).unwrap());
        let consumer = PaperHandoff::new(
            tmp.path(),
            &tmp.path().join("data"),
            transport.clone(),
            Arc::new(FakeClock::new(1)),
        );
        Self {
            tmp,
            source,
            source_store,
            snapshot,
            reference,
            consumer,
            transport,
        }
    }
    fn store(&self) -> PathBuf {
        store_for(self.tmp.path(), &self.reference)
    }
    fn rewrite_snapshot(&mut self) {
        let mut value = serde_json::to_value(&self.snapshot).unwrap();
        value.as_object_mut().unwrap().remove("snapshot_id");
        self.snapshot.snapshot_id = format!(
            "snapshot:{}",
            &sha256_hex(&serde_json::to_vec(&value).unwrap())[..16]
        );
        self.reference.snapshot_id = self.snapshot.snapshot_id.clone();
        fs::write(
            self.reference.path.join("snapshot.json"),
            serde_json::to_vec(&self.snapshot).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn snapshot_reaches_real_evidence_pipeline_without_source_writes() {
    let c = Case::new();
    let before = (
        tree(&c.source),
        tree(&c.source_store),
        tree(&c.reference.path),
    );
    assert_eq!(c.consumer.name(), "paper");
    c.consumer.consume(&c.reference).unwrap();
    let store = c.store();
    let record: HandoffRecord =
        serde_json::from_slice(&fs::read(store.join("handoff.json")).unwrap()).unwrap();
    let bundle: EvidenceBundle =
        serde_json::from_slice(&fs::read(paper::paper_dir(&store).join("evidence.json")).unwrap())
            .unwrap();
    assert_eq!(record.research_id, c.snapshot.research_id);
    assert_eq!(record.research_version, c.snapshot.research_version);
    assert_eq!(record.snapshot_id, c.snapshot.snapshot_id);
    assert_eq!(record.manifest_sha256, c.snapshot.manifest_sha256);
    assert_eq!(record.artifact_sha256, c.snapshot.data);
    assert_eq!(record.paper_store, store);
    assert_eq!(record.evidence_hash, bundle.hash);
    paper::evidence::verify(&bundle, &store).unwrap();
    let source_evidence: serde_json::Value =
        serde_json::from_slice(&fs::read(c.reference.path.join("evidence.json")).unwrap()).unwrap();
    for item in source_evidence["items"].as_array().unwrap() {
        assert!(bundle.claims.iter().any(|claim| {
            claim.id == item["evidence_id"].as_str().unwrap()
                && claim.text == item["quote"].as_str().unwrap()
        }));
    }
    assert!(!bundle.claims.is_empty());
    assert!(bundle
        .artifacts
        .iter()
        .any(|a| a.kind == ArtifactKind::Dataset && a.path.starts_with("inputs/dataset-")));
    assert!(research_program::program_dir(&store)
        .join("02-dossier.json")
        .is_file());
    assert!(!paper::paper_dir(&store).join("estimate.json").exists());
    assert!(c.transport.requests().is_empty());
    assert_eq!(
        before,
        (
            tree(&c.source),
            tree(&c.source_store),
            tree(&c.reference.path)
        )
    );
}

#[test]
fn listed_results_and_models_are_copied_and_included() {
    let mut c = Case::new();
    for (name, bytes) in [
        (
            "results.json",
            b"{\"producer\":\"analysis\",\"quantities\":[]}".as_slice(),
        ),
        ("model.bin", b"model bytes".as_slice()),
    ] {
        fs::write(c.reference.path.join(name), bytes).unwrap();
        c.snapshot.files.insert(name.into(), sha256_hex(bytes));
    }
    c.rewrite_snapshot();
    c.consumer.consume(&c.reference).unwrap();
    let store = c.store();
    let bundle: EvidenceBundle =
        serde_json::from_slice(&fs::read(paper::paper_dir(&store).join("evidence.json")).unwrap())
            .unwrap();
    for (name, kind) in [
        ("results.json", ArtifactKind::Result),
        ("model.bin", ArtifactKind::Model),
    ] {
        let path = format!("inputs/{name}");
        let artifact = bundle
            .artifacts
            .iter()
            .find(|a| a.path == path && a.kind == kind)
            .unwrap();
        assert_eq!(artifact.sha256, c.snapshot.files[name]);
    }
}

#[test]
fn missing_snapshot_manifest_is_named() {
    let c = Case::new();
    fs::remove_file(c.reference.path.join("snapshot.json")).unwrap();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "missing_input");
    assert!(error.message.contains("snapshot.json"));
    assert!(!c.store().exists());
}
#[test]
fn missing_evidence_is_rejected_before_store_creation() {
    let mut c = Case::new();
    c.snapshot.files.remove("evidence.json");
    c.rewrite_snapshot();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "missing_input");
    assert!(error.message.contains("evidence.json"));
    assert!(!c.store().exists());
}
#[test]
fn missing_listed_file_is_rejected_before_store_creation() {
    let c = Case::new();
    let name = c.snapshot.files.keys().next().unwrap();
    fs::remove_file(c.reference.path.join(name)).unwrap();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "missing_input");
    assert!(error.message.contains(name));
    assert!(!c.store().exists());
}

#[test]
fn missing_listed_evidence_is_named() {
    let c = Case::new();
    fs::remove_file(c.reference.path.join("evidence.json")).unwrap();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "missing_input");
    assert!(error.message.contains("evidence.json"));
    assert!(!c.store().exists());
}
#[test]
fn corrupt_listed_file_is_a_negative_control() {
    let c = Case::new();
    let name = c.snapshot.files.keys().next().unwrap();
    fs::write(c.reference.path.join(name), "corrupted").unwrap();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "corrupt_artifact");
    assert!(error.message.contains(name));
    assert!(!c.store().exists());
}
#[test]
fn corrupt_snapshot_id_is_rejected() {
    let mut c = Case::new();
    c.snapshot.manifest_sha256 = "tampered".into();
    fs::write(
        c.reference.path.join("snapshot.json"),
        serde_json::to_vec(&c.snapshot).unwrap(),
    )
    .unwrap();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "corrupt_artifact");
    assert!(error.message.contains("snapshot_id"));
    assert!(!c.store().exists());
}
#[test]
fn unsafe_research_id_is_rejected_without_writes() {
    let mut c = Case::new();
    let before = tree(c.tmp.path());
    c.reference.research_id = "..".into();
    let error = c.consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "escape");
    assert!(!c.tmp.path().join("paper").exists());
    assert_eq!(tree(c.tmp.path()), before);
}

#[test]
fn store_inside_snapshot_is_rejected_without_writes() {
    let c = Case::new();
    let before = tree(c.tmp.path());
    let home = c.reference.path.join("nested-home");
    let consumer = PaperHandoff::new(
        &home,
        &c.tmp.path().join("data"),
        c.transport.clone(),
        Arc::new(FakeClock::new(1)),
    );
    let error = consumer.consume(&c.reference).unwrap_err();
    assert_eq!(error.kind, "dest_in_source");
    assert!(!home.exists());
    assert_eq!(tree(c.tmp.path()), before);
}
