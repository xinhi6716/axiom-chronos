use axiom::research_workspace::{manifest::*, read::*, rram};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::BufReader,
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name)
}
fn tree(root: &Path) -> String {
    fn walk(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, files)
            } else {
                files.push(p.strip_prefix(root).unwrap().to_path_buf())
            }
        }
    }
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.sort();
    let mut h = Sha256::new();
    for p in files {
        let name = p.to_string_lossy().replace('\\', "/");
        let b = fs::read(root.join(&p)).unwrap();
        h.update((name.len() as u64).to_le_bytes());
        h.update(name.as_bytes());
        h.update((b.len() as u64).to_le_bytes());
        h.update(b);
    }
    format!("{:x}", h.finalize())
}
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
fn rows(l: &Located, id: &str) -> Vec<Row> {
    let mut all = Vec::new();
    let mut cursor = None;
    loop {
        let page = read_rows(l, id, cursor.as_ref(), 17, &Limits::default()).unwrap();
        all.extend(page.rows);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    all
}
fn table(path: &Path) -> Vec<BTreeMap<String, String>> {
    let mut c = CsvReader::new(
        BufReader::new(fs::File::open(path).unwrap()),
        Limits::default(),
    );
    let header = c.next_record().unwrap().unwrap();
    let mut out = Vec::new();
    while let Some(r) = c.next_record().unwrap() {
        assert_eq!(r.len(), header.len());
        out.push(header.iter().cloned().zip(r).collect());
    }
    out
}

#[test]
fn fixtures_and_adapter_are_read_only() {
    let r = fixture("rram");
    let raw = fixture("rawonly");
    let before = (tree(&r), tree(&raw));
    let tmp = tempfile::tempdir().unwrap();
    copy_tree(&r, &tmp.path().join("copy"));
    let altered = tmp.path().join("copy/store/units.tsv");
    let mut bytes = fs::read(&altered).unwrap();
    bytes[0] ^= 1;
    fs::write(altered, bytes).unwrap();
    assert_ne!(before.0, tree(&tmp.path().join("copy")));
    let project = r.join("project");
    assert!(rram::detect(&project));
    let l = locate(
        &project,
        &LocateOptions {
            store: Some(r.join("store")),
            orch_home: None,
        },
    )
    .unwrap();
    assert_eq!(l.manifest.research_id, "rram_fixture");
    assert_eq!(l.manifest.research_version, "20261111T204743Z");
    assert!(matches!(l.source, ManifestSource::Adapter { ref name } if name == "rram"));
    assert_eq!(
        l.manifest.settings["codex"]["effort_ladder"],
        serde_json::json!(["medium", "high", "xhigh", "max"])
    );
    assert_eq!(l.manifest.stall_after_s, Some(1800));
    assert_eq!(l.manifest.versions.seeds["global"], 20261112);
    assert_eq!(
        l.manifest.versions.prompts["screen.md"],
        hash_file(&project.join("src/hfo2/prompts/screen.md")).unwrap()
    );
    for id in [
        "inventory",
        "screening",
        "aliases",
        "evidence",
        "measurements",
        "raw_measurements",
        "progress_screen",
        "driver",
        "completed",
        "pilot",
        "acceptance",
        "acceptance_sha",
        "splits",
        "splits_sha",
        "validation",
        "effort_lock",
        "raw_summary",
        "protocol",
    ] {
        assert!(l.artifact(id).is_some(), "{id}");
    }
    assert!(l.artifact("recall").is_none());
    for cap in [
        Capability::PdfPages,
        Capability::RawMeasurements,
        Capability::Literature,
        Capability::Screening,
        Capability::Evidence,
        Capability::Dataset,
        Capability::Pilot,
        Capability::Acceptance,
        Capability::Splits,
        Capability::Progress,
    ] {
        assert!(l.has(cap));
    }
    let inventory_artifact = l.artifact("inventory").unwrap();
    assert_eq!(inventory_artifact.columns["path"], "rel_path");
    assert_eq!(inventory_artifact.columns["bytes"], "size_bytes");
    assert_eq!(inventory_artifact.documents_root.as_deref(), Some("pdf"));
    assert_eq!(
        l.artifact("screening").unwrap().columns["reason"],
        "scope_reason"
    );
    assert_eq!(
        l.artifact("screening").unwrap().columns["sha256"],
        "pdf_sha256"
    );
    for id in ["evidence", "measurements"] {
        assert_eq!(l.artifact(id).unwrap().columns["sha256"], "pdf_sha256");
    }
    for (id, pin) in [
        ("acceptance", "acceptance_sha"),
        ("splits", "splits_sha"),
        ("validation", "validation#/measurements_sha256"),
    ] {
        assert_eq!(
            l.artifact(id).unwrap().produced.as_ref().unwrap().inputs[0]
                .sha256_from
                .as_deref(),
            Some(pin)
        );
    }
    assert_eq!(
        l.manifest
            .stages
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "inventory",
            "screen",
            "pilot",
            "acceptance",
            "extract",
            "splits"
        ]
    );
    assert_eq!(
        l.manifest.stages[2]
            .gate
            .as_ref()
            .unwrap()
            .setting
            .as_deref(),
        Some("/pilot/accuracy_threshold")
    );
    assert_eq!(l.manifest.links.len(), 3);
    assert_eq!(l.manifest.groups.as_ref().unwrap().by, GroupBy::StudyGroups);
    let inventory = rows(&l, "inventory");
    assert_eq!(inventory.len(), 40);
    assert_eq!(inventory[0].fields["path"], "a.pdf");
    assert_eq!(
        inventory[0].fields["title"],
        "A \"quoted, title\"\nsecond line"
    );
    for (file, pin) in [
        (
            "a.pdf",
            "c0b3ff2c5cffd5bdcdf034af5e32399039fdc1a98dbbf4ff2a0214d4164b025b",
        ),
        (
            "b.pdf",
            "8341ce5648050280ee1ff6109dec4efd78e1aa0e57c3761844aad7fa7d577bb4",
        ),
        (
            "broken.pdf",
            "f125c3f4685ec03aa79d05f0d51ad3c024a51b2b115cb02b97604248fd5dc4ff",
        ),
    ] {
        assert_eq!(hash_file(&l.document_path(file).unwrap()).unwrap(), pin);
        assert_eq!(
            inventory
                .iter()
                .find(|r| r.fields["path"] == file)
                .unwrap()
                .fields["sha256"],
            pin
        );
    }
    let evidence = rows(&l, "evidence");
    assert_eq!(evidence.len(), 120);
    let expected = table(&r.join("expected_resolution.csv"));
    assert_eq!(expected.len(), 100);
    let by_id: BTreeMap<_, _> = evidence
        .iter()
        .map(|x| (x.fields["evidence_id"].as_str(), &x.fields))
        .collect();
    for x in &expected {
        let e = by_id[x["evidence_id"].as_str()];
        for (a, b) in [
            ("kind", "locator_type"),
            ("sha256", "sha256"),
            ("physical_page", "physical_page"),
            ("printed_page", "printed_page"),
            ("raw_file", "raw_file"),
            ("raw_row", "raw_row"),
            ("raw_column", "raw_column"),
            ("value", "quote"),
        ] {
            assert_eq!(x[a], e[b], "{}: {a}", x["evidence_id"]);
        }
    }
    assert_eq!(
        expected
            .iter()
            .map(|x| &x["evidence_id"])
            .collect::<BTreeSet<_>>()
            .len(),
        100
    );
    let raw_values = table(&project.join("external/hf02/pulse_run1.csv"));
    let raw_rows = rows(&l, "raw_measurements");
    assert_eq!(raw_rows.len(), 20);
    for e in evidence
        .iter()
        .filter(|e| e.fields["locator_type"] == "raw_field")
    {
        let i: usize = e.fields["raw_row"].parse().unwrap();
        assert_eq!(raw_values[i - 1]["key"], e.fields["raw_key"]);
        assert_eq!(
            raw_values[i - 1]["value"],
            raw_rows[i - 1].fields["value_original"]
        );
        assert_eq!(
            raw_rows[i - 1].fields["evidence_id"],
            e.fields["evidence_id"]
        );
    }
    assert_eq!(rows(&l, "measurements").len(), 60);
    assert_ne!(
        read_json(&l, "validation", &Limits::default()).unwrap()["measurements_sha256"]
            .as_str()
            .unwrap(),
        verify(&l, "measurements").unwrap()
    );
    let pilot: BTreeSet<_> = rows(&l, "pilot")
        .into_iter()
        .map(|x| x.fields["study_group"].clone())
        .collect();
    let acceptance: BTreeSet<_> = rows(&l, "acceptance")
        .into_iter()
        .map(|x| x.fields["study_group"].clone())
        .collect();
    assert!(!pilot.is_empty() && !acceptance.is_empty() && pilot.is_disjoint(&acceptance));
    let m = locate(&raw, &Default::default()).unwrap();
    assert!(matches!(m.source, ManifestSource::File { .. }));
    assert!(m.has(Capability::RawMeasurements));
    assert!(m.has(Capability::Dataset));
    let err = m.require(Capability::PdfPages).unwrap_err();
    assert_eq!(err.kind, "not_capable");
    assert!(err.message.contains("PdfPages"));
    assert!(!m.has(Capability::Literature));
    assert!(!m.has(Capability::Screening));
    assert_eq!(rows(&m, "raw_measurements").len(), 2);
    assert_eq!(rows(&m, "evidence").len(), 2);
    assert_eq!(rows(&m, "run_a")[0].fields["key"], "A1");
    assert_eq!(rows(&m, "run_b")[0].fields["key"], "B1");
    assert_eq!(m.manifest.groups.unwrap().by, GroupBy::Column("lab".into()));
    assert_eq!(before, (tree(&r), tree(&raw)));
}

#[test]
fn errors_paging_and_path_guards() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(MANIFEST_FILE), r#"{"manifest_version":2}"#).unwrap();
    let e = locate(d.path(), &Default::default()).unwrap_err();
    assert_eq!(e.kind, "unknown_version");
    assert!(e.message.contains('2'));
    copy_tree(&fixture("rawonly"), d.path());
    let mut l = locate(d.path(), &Default::default()).unwrap();
    assert_eq!(verify(&l, "missing").unwrap_err().kind, "missing_artifact");
    l.manifest
        .artifacts
        .iter_mut()
        .find(|a| a.id == "run_a")
        .unwrap()
        .sha256 = Some("0".repeat(64));
    assert_eq!(verify(&l, "run_a").unwrap_err().kind, "corrupt_artifact");
    l.manifest
        .artifacts
        .iter_mut()
        .find(|a| a.id == "run_a")
        .unwrap()
        .sha256 = None;
    let p = d.path().join("runs/device_A.csv");
    fs::write(&p, "key,value\none,1\ntwo,2\n").unwrap();
    let first = read_rows(&l, "run_a", None, 1, &Limits::default()).unwrap();
    let c = first.next.unwrap();
    assert_eq!(Cursor::from_token(&c.to_token()).unwrap(), c);
    assert_eq!(
        read_rows(&l, "run_a", Some(&c), 1, &Limits::default())
            .unwrap()
            .rows[0]
            .fields["key"],
        "two"
    );
    fs::write(&p, "key,value\nchanged,12345\n").unwrap();
    assert_eq!(
        read_rows(&l, "run_a", Some(&c), 1, &Limits::default())
            .unwrap_err()
            .kind,
        "stale_cursor"
    );
    fs::write(d.path().join("runs/device_B.jsonl"), "{\"x\":1}\n{\"x\":").unwrap();
    let e = read_rows(&l, "run_b", None, 10, &Limits::default()).unwrap_err();
    assert_eq!(e.kind, "corrupt_artifact");
    assert!(e.message.contains('2'));
    let mut csv = CsvReader::new(
        &b"\xef\xbb\xbfa,b\n\"x,\"\"y\"\"\nz\",q\n"[..],
        Limits::default(),
    );
    assert_eq!(csv.next_record().unwrap().unwrap(), vec!["a", "b"]);
    assert_eq!(csv.next_record().unwrap().unwrap(), vec!["x,\"y\"\nz", "q"]);
    let limits = Limits {
        max_record_bytes: 3,
        ..Default::default()
    };
    assert_eq!(
        CsvReader::new(&b"1234\n"[..], limits)
            .next_record()
            .unwrap_err()
            .kind,
        "oversized_record"
    );
    for path in [
        "../escape",
        "/absolute",
        "C:/absolute",
        "folder/../../outside",
    ] {
        assert_eq!(guard(d.path(), path).unwrap_err().kind, "escape", "{path}");
    }
    let outside = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    {
        let junction = d.path().join("junction");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(guard(d.path(), "junction/file").unwrap_err().kind, "escape");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), d.path().join("junction")).unwrap();
        assert_eq!(guard(d.path(), "junction/file").unwrap_err().kind, "escape");
    }
}
