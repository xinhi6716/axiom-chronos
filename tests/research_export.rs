use axiom::{
    research_workspace::{
        annotations::{self, AnnotationStore, Verdict},
        dataset::{self, DataLayer, DatasetQuery},
        evidence,
        export::{self, Delivery},
        index::{ArtifactStatus, Index},
        manifest::{locate, LocateOptions},
        read::{hash_file, Limits},
        repro,
        seams::{FakeSnapshotConsumer, FakeVault},
    },
    seams::{sha256_hex, FakeClock},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let e = entry.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}
fn tree(root: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                visit(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), hash_file(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
fn fixture(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf, Index) {
    let tmp = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name);
    let project = tmp.path().join("project");
    let store = tmp.path().join("store");
    if name == "rram" {
        copy_tree(&src.join("project"), &project);
        copy_tree(&src.join("store"), &store);
    } else {
        copy_tree(&src, &project);
        fs::create_dir_all(&store).unwrap();
    }
    let l = locate(
        &project,
        &LocateOptions {
            store: Some(store.clone()),
            orch_home: None,
        },
    )
    .unwrap();
    let ix = Index::build(Arc::new(l), Limits::default(), &|_, _| {}).unwrap();
    (tmp, project, store, ix)
}
fn build(l: axiom::research_workspace::manifest::Located) -> Index {
    Index::build(Arc::new(l), Limits::default(), &|_, _| {}).unwrap()
}
fn query(layer: DataLayer) -> DatasetQuery {
    DatasetQuery {
        layer,
        filters: vec![],
        sort: None,
        offset: 99,
        limit: 1,
    }
}
fn parse_csv(s: &str) -> Vec<Vec<String>> {
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut chars = s.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => {
                    row.push(std::mem::take(&mut field));
                }
                '\r' => {
                    assert_eq!(chars.next(), Some('\n'));
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    assert!(!quoted && field.is_empty() && row.is_empty());
    rows
}
fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn canonical_json_sorts_nested_object_keys() {
    let value = serde_json::json!({"z": {"z": 1, "a": 2}, "a": 3});
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"a":3,"z":{"a":2,"z":1}}"#
    );
}

#[test]
fn csv_pages_beyond_index_limit() {
    let (tmp, project, store, _) = fixture("rawonly");
    let mut rows = String::from("row_id,variable,value_original,value_canonical\n");
    for i in 0..1101 {
        rows.push_str(&format!("R{i},voltage,{i},{i}\n"));
    }
    fs::write(project.join("data/raw_measurements.csv"), rows).unwrap();
    let ix = build(
        locate(
            &project,
            &LocateOptions {
                store: Some(store),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    let result = export::export_csv(
        &ix,
        &query(DataLayer::Raw),
        &tmp.path().join("out"),
        "all.csv",
    )
    .unwrap();
    let rows = parse_csv(&fs::read_to_string(result.path).unwrap());
    assert_eq!(result.rows, 1101);
    assert_eq!(rows.len(), 1102);
    assert_eq!(rows[1101][0], "raw_measurements:R1100");
}

#[test]
fn csv_roundtrip_filters_and_no_source_writes() {
    let (tmp, project, store, _) = fixture("rram");
    let path = project.join("data/measurements.csv");
    let csv = fs::read_to_string(&path).unwrap().replacen(
        ",voltage,HfO₂,HfO2,",
        ",\"voltage,\"\"quoted\"\"\nline\",HfO₂,HfO2,",
        1,
    );
    fs::write(path, csv).unwrap();
    let ix = build(
        locate(
            &project,
            &LocateOptions {
                store: Some(store.clone()),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    let before = (tree(&project), tree(&store));
    let out = tmp.path().join("out");
    let result = export::export_csv(&ix, &query(DataLayer::Extracted), &out, "all.csv").unwrap();
    let rows = parse_csv(&fs::read_to_string(&result.path).unwrap());
    assert_eq!(result.rows, 60);
    assert_eq!(rows.len(), 61);
    assert_eq!(rows[0][0], "record_id");
    assert_eq!(rows[0][14], "locator");
    assert_eq!(rows[1][4], "voltage,\"quoted\"\nline");
    assert_eq!(rows[3][8], "");
    assert_eq!(rows[3][10], "no canonical value recorded");
    assert_eq!(hash_file(&result.path).unwrap(), result.sha256);
    let mut q = query(DataLayer::Extracted);
    q.filters.push(dataset::evidence_filter("E3"));
    let filtered = export::export_csv(&ix, &q, &out, "filtered.csv").unwrap();
    assert_eq!(filtered.rows, 11);
    assert_eq!(
        parse_csv(&fs::read_to_string(filtered.path).unwrap()).len(),
        12
    );
    export::export_csv(&ix, &q, &out, "filtered.csv").unwrap();
    for bad in ["../x", "x/y", "x\\y", "a..b"] {
        assert_eq!(
            export::export_csv(&ix, &q, &out, bad).unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert_eq!(before, (tree(&project), tree(&store)));
}

#[test]
fn bundles_resolve_sources_and_fail_without_writes() {
    let (tmp, project, store, ix) = fixture("rram");
    let before = (tree(&project), tree(&store));
    let ids = vec!["E2".into(), "E1".into(), "E3".into()];
    let out = tmp.path().join("out");
    let result = export::export_evidence_bundle(&ix, &ids, &out).unwrap();
    assert!(result.path.ends_with(format!(
        "evidence-bundle-{}.json",
        &sha256_hex(b"E1\nE2\nE3")[..12]
    )));
    assert_eq!(result.rows, 3);
    assert_eq!(result.sha256, hash_file(&result.path).unwrap());
    let doc = json(&result.path);
    assert_eq!(doc["bundle_version"], 1);
    assert_eq!(doc["items"][0]["evidence_id"], "E2");
    assert_eq!(doc["sources"].as_array().unwrap().len(), 2);
    assert_eq!(doc["sources"][0]["paper_id"], "P0");
    assert_eq!(doc["sources"][0]["path"], "a.pdf");
    for (i, id) in ids.iter().enumerate() {
        let item = evidence::resolve(&ix, id, &Limits::default()).unwrap().item;
        assert_eq!(
            doc["items"][i]["locator"],
            serde_json::to_value(item.locator).unwrap()
        );
        assert_eq!(doc["items"][i]["quote"], item.quote);
        assert_eq!(doc["items"][i]["quote_verified"], item.quote_verified);
        let file = if id == "E2" { "b.pdf" } else { "a.pdf" };
        assert_eq!(
            doc["items"][i]["source_sha256"],
            hash_file(&project.join("pdfs").join(file)).unwrap()
        );
    }
    let missing = tmp.path().join("missing-output");
    let err = export::export_evidence_bundle(&ix, &["missing-id".into()], &missing).unwrap_err();
    assert_eq!(err.kind, "not_found");
    assert!(err.message.contains("missing-id"));
    assert!(!missing.exists());
    assert_eq!(before, (tree(&project), tree(&store)));
    let (tmp, project, store, ix) = fixture("rawonly");
    let before = (tree(&project), tree(&store));
    let b =
        export::export_evidence_bundle(&ix, &["RE1".into(), "RE2".into()], &tmp.path().join("out"))
            .unwrap();
    let doc = json(&b.path);
    assert_eq!(doc["sources"].as_array().unwrap().len(), 0);
    for (n, file) in ["device_A.csv", "device_B.jsonl"].iter().enumerate() {
        assert_eq!(
            doc["items"][n]["source_sha256"],
            hash_file(&project.join("runs").join(file)).unwrap()
        );
    }
    assert_eq!(before, (tree(&project), tree(&store)));
}

#[test]
fn source_destinations_are_rejected_without_writes() {
    let (tmp, project, store, ix) = fixture("rram");
    let named = tmp.path().join("external-root");
    fs::create_dir_all(&named).unwrap();
    fs::write(named.join("source.txt"), "source").unwrap();
    let mut l = ix.located().as_ref().clone();
    l.manifest
        .roots
        .insert("external".into(), named.to_string_lossy().into_owned());
    let ix = build(l);
    let before = (tree(&project), tree(&store), tree(&named));
    let paths = [
        project.join("new"),
        named.join("new"),
        store.join("new"),
        tmp.path()
            .join("elsewhere")
            .join("..")
            .join("project")
            .join("new"),
    ];
    for p in paths {
        assert_eq!(
            export::check_dest(ix.located(), &p).unwrap_err().kind,
            "dest_in_source"
        );
        assert_eq!(
            export::export_csv(&ix, &query(DataLayer::Raw), &p, "a.csv")
                .unwrap_err()
                .kind,
            "dest_in_source"
        );
        assert_eq!(
            export::export_evidence_bundle(&ix, &[], &p)
                .unwrap_err()
                .kind,
            "dest_in_source"
        );
        assert_eq!(
            export::snapshot(&ix, &tmp.path().join("home"), &p, &FakeClock::new(1))
                .unwrap_err()
                .kind,
            "dest_in_source"
        );
        assert!(!p.exists());
    }
    assert!(export::check_dest(ix.located(), &tmp.path().join("sibling")).is_ok());
    assert!(!tmp.path().join("sibling").exists());
    assert_eq!(
        export::snapshot(
            &ix,
            &project.join("bad-home"),
            &tmp.path().join("out"),
            &FakeClock::new(1)
        )
        .unwrap_err()
        .kind,
        "dest_in_source"
    );
    assert_eq!(before, (tree(&project), tree(&store), tree(&named)));
}

#[test]
fn snapshots_determinism_changes_and_delivery() {
    let (tmp, project, store, ix) = fixture("rram");
    let before = (tree(&project), tree(&store));
    let home = tmp.path().join("home");
    let (s, dir) =
        export::snapshot(&ix, &home, &tmp.path().join("one"), &FakeClock::new(1)).unwrap();
    let (again, dir2) =
        export::snapshot(&ix, &home, &tmp.path().join("two"), &FakeClock::new(2)).unwrap();
    assert_eq!(s, again);
    assert_eq!(
        dir.file_name().unwrap().to_string_lossy(),
        format!("snapshot-{}", &s.snapshot_id[9..])
    );
    let mut a = json(&dir.join("snapshot.json"));
    let mut b = json(&dir2.join("snapshot.json"));
    assert_eq!(a.as_object_mut().unwrap().remove("written_at").unwrap(), 1);
    assert_eq!(b.as_object_mut().unwrap().remove("written_at").unwrap(), 2);
    assert_eq!(a, b);
    a.as_object_mut().unwrap().remove("snapshot_id");
    let full_hash = sha256_hex(&serde_json::to_vec(&a).unwrap());
    assert_eq!(s.snapshot_id, format!("snapshot:{}", &full_hash[..16]));
    let entries = annotations::snapshots(&home, &s.research_id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].snapshot_id, s.snapshot_id);
    assert_eq!(entries[0].sha256, full_hash);
    assert_eq!(
        entries[0].artifacts,
        s.data.keys().cloned().collect::<Vec<_>>()
    );
    let ready: BTreeSet<_> = ix
        .located()
        .manifest
        .artifacts
        .iter()
        .filter(|a| matches!(ix.status(&a.id), ArtifactStatus::Ready(_)))
        .map(|a| a.id.clone())
        .collect();
    assert_eq!(s.data.keys().cloned().collect::<BTreeSet<_>>(), ready);
    assert_eq!(s.repro, repro::guard(&ix));
    assert_eq!(
        s.files.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["dataset-extracted.csv", "dataset-raw.csv", "evidence.json"])
    );
    for (name, hash) in &s.files {
        assert_eq!(*hash, hash_file(&dir.join(name)).unwrap());
    }
    let consumer = FakeSnapshotConsumer::new();
    let vault = FakeVault::new();
    let h = export::hand_off(&s, &dir, Some(&consumer), Some(&vault));
    assert!(matches!(h.consumer, Delivery::Delivered { .. }));
    assert!(matches!(h.vault, Delivery::Delivered { .. }));
    let call = &consumer.calls()[0];
    assert_eq!(
        (
            &call.snapshot_id,
            &call.research_id,
            &call.research_version,
            &call.path
        ),
        (&s.snapshot_id, &s.research_id, &s.research_version, &dir)
    );
    let v = &vault.calls()[0];
    assert_eq!(
        (&v.research_id, &v.research_version, &v.snapshot_id),
        (
            &s.research_id,
            &s.research_version,
            &Some(s.snapshot_id.clone())
        )
    );
    let h = export::hand_off(&s, &dir, None, None);
    assert_eq!(
        (h.consumer, h.vault),
        (Delivery::Unwired, Delivery::Unwired)
    );
    consumer.fail_next("scripted");
    let h = export::hand_off(&s, &dir, Some(&consumer), Some(&vault));
    assert!(matches!(h.consumer, Delivery::Failed { kind, .. } if kind == "scripted"));
    assert!(matches!(h.vault, Delivery::Delivered { .. }));
    for kind in ["seed", "prompt"] {
        let mut l = ix.located().as_ref().clone();
        if kind == "seed" {
            l.manifest.versions.seeds.insert("new".into(), 123);
        } else {
            l.manifest
                .versions
                .prompts
                .insert("new".into(), "v2".into());
        }
        let changed =
            export::snapshot(&build(l), &home, &tmp.path().join(kind), &FakeClock::new(1))
                .unwrap()
                .0;
        assert_ne!(s.snapshot_id, changed.snapshot_id);
    }
    let mut store_ann = AnnotationStore::open(&home, &s.research_id).unwrap();
    store_ann
        .spot_check(
            &ix,
            "measurements:M1",
            None,
            Verdict::Confirm,
            "tester",
            &FakeClock::new(5),
        )
        .unwrap();
    let changed = export::snapshot(
        &ix,
        &home,
        &tmp.path().join("annotation"),
        &FakeClock::new(1),
    )
    .unwrap()
    .0;
    assert_ne!(s.snapshot_id, changed.snapshot_id);
    assert_eq!(before, (tree(&project), tree(&store)));
    let measurements = project.join("data/measurements.csv");
    let mut contents = fs::read_to_string(&measurements).unwrap();
    contents.push_str("\r\n");
    fs::write(&measurements, contents).unwrap();
    let new_ix = build(
        locate(
            &project,
            &LocateOptions {
                store: Some(store),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    let changed = export::snapshot(
        &new_ix,
        &tmp.path().join("new-home"),
        &tmp.path().join("artifact"),
        &FakeClock::new(1),
    )
    .unwrap()
    .0;
    assert_ne!(s.snapshot_id, changed.snapshot_id);
}

#[test]
fn raw_snapshot_and_missing_artifact() {
    let (tmp, project, store, ix) = fixture("rawonly");
    let before = (tree(&project), tree(&store));
    let mut l = ix.located().as_ref().clone();
    let mut missing = l.manifest.artifacts[0].clone();
    missing.id = "missing".into();
    missing.path = "absent.csv".into();
    l.manifest.artifacts.push(missing);
    let ix = build(l);
    let (s, dir) = export::snapshot(
        &ix,
        &tmp.path().join("home"),
        &tmp.path().join("out"),
        &FakeClock::new(5),
    )
    .unwrap();
    assert!(!s.data.contains_key("missing"));
    assert!(s.files.contains_key("dataset-raw.csv"));
    assert!(!dir.join("dataset-extracted.csv").exists());
    assert!(dir.join("evidence.json").exists());
    assert_eq!(s.repro, repro::guard(&ix));
    assert_eq!(before, (tree(&project), tree(&store)));
    let pilot = project.join("data/pilot.csv");
    let mut contents = fs::read_to_string(&pilot).unwrap();
    contents.push_str("\r\n");
    fs::write(pilot, contents).unwrap();
    let changed_ix = build(
        locate(
            &project,
            &LocateOptions {
                store: Some(store),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    let changed = export::snapshot(
        &changed_ix,
        &tmp.path().join("other-home"),
        &tmp.path().join("changed"),
        &FakeClock::new(5),
    )
    .unwrap()
    .0;
    assert_ne!(s.splits["pilot"], changed.splits["pilot"]);
    assert_ne!(s.snapshot_id, changed.snapshot_id);
}
