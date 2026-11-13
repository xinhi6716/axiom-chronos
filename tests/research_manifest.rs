// Failing-first record (rwc-f-tests):
// stages and gates: expected 7 stages; the stage count assertion failed.
// links and Python requirement: expected 4 links; the link count assertion failed.
// revision and snapshots: expected snapshot version "wrong"; the newest-version assertion failed.
// roots, roles and store precedence: expected no store; the explicit-store assertion failed.
// declared artifacts: expected 19 artifacts; the artifact count assertion failed.
// row strictness: expected 3 CSV rows; the row count assertion failed.

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
    assert!(err.message.contains("pdf_pages"));
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
    assert!(e.message.contains("line 2"));
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

#[test]
fn declared_missing_paging_limits_and_cursor_validation() {
    let d = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rawonly"), d.path());
    let l = locate(d.path(), &Default::default()).unwrap();
    let p = d.path().join("runs/device_A.csv");
    fs::remove_file(&p).unwrap();
    assert_eq!(verify(&l, "run_a").unwrap_err().kind, "missing_artifact");
    assert_eq!(
        read_rows(&l, "run_a", None, 1, &Limits::default())
            .unwrap_err()
            .kind,
        "missing_artifact"
    );

    let content = format!(
        "key,value\n{}",
        (0..510).map(|i| format!("k{i},{i}\n")).collect::<String>()
    );
    fs::write(&p, &content).unwrap();
    let page = read_rows(&l, "run_a", None, 1000, &Limits::default()).unwrap();
    assert_eq!(page.rows.len(), 500);
    let cursor = page.next.unwrap();
    let rest = read_rows(&l, "run_a", Some(&cursor), 1000, &Limits::default()).unwrap();
    assert_eq!(rest.rows.len(), 10);
    assert!(rest.next.is_none());
    for offset in [0, cursor.bytes + 1] {
        let mut invalid = cursor.clone();
        invalid.offset = offset;
        let invalid = Cursor::from_token(&invalid.to_token()).unwrap();
        assert_eq!(
            read_rows(&l, "run_a", Some(&invalid), 1, &Limits::default())
                .unwrap_err()
                .kind,
            "stale_cursor"
        );
    }
    let modified = fs::metadata(&p).unwrap().modified().unwrap();
    fs::write(&p, &content).unwrap();
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified + std::time::Duration::from_secs(10)))
        .unwrap();
    let current = version(&p).unwrap();
    assert_eq!(current.bytes, cursor.bytes);
    assert_ne!(current.modified_ns, cursor.modified_ns);
    assert_eq!(
        read_rows(&l, "run_a", Some(&cursor), 1, &Limits::default())
            .unwrap_err()
            .kind,
        "stale_cursor"
    );

    fs::write(&p, "key,value\nlongvalue,123456\n").unwrap();
    let lim = Limits {
        max_record_bytes: 12,
        ..Default::default()
    };
    assert_eq!(
        read_rows(&l, "run_a", None, 1, &lim).unwrap_err().kind,
        "oversized_record"
    );
}

#[test]
fn location_and_public_path_guards() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(
        locate(d.path(), &Default::default()).unwrap_err().kind,
        "no_manifest"
    );
    fs::write(
        d.path().join(MANIFEST_FILE),
        r#"{"manifest_version":1,"settings":[]}"#,
    )
    .unwrap();
    assert_eq!(
        locate(d.path(), &Default::default()).unwrap_err().kind,
        "invalid_manifest"
    );

    let project = d.path().join("project");
    copy_tree(&fixture("rram").join("project"), &project);
    let home = d.path().join("orch");
    let store = home.join("rram_fixture");
    fs::create_dir_all(&store).unwrap();
    fs::create_dir_all(project.join(".git/refs/heads")).unwrap();
    fs::write(project.join(".git/HEAD"), "ref: refs/heads/test\n").unwrap();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    fs::write(
        project.join(".git/refs/heads/test"),
        format!("{revision}\n"),
    )
    .unwrap();
    let l = locate(
        &project,
        &LocateOptions {
            store: None,
            orch_home: Some(home),
        },
    )
    .unwrap();
    assert_eq!(l.store.as_deref(), Some(store.as_path()));
    assert_eq!(l.manifest.versions.code.as_deref(), Some(revision));

    let outside = tempfile::tempdir().unwrap();
    let link = project.join("pdfs").join("junction");
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .status()
        .unwrap()
        .success());
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    let mut a = l.artifact("inventory").unwrap().clone();
    for path in ["../escape", "/absolute", "C:/absolute", "junction/file"] {
        assert_eq!(l.document_path(path).unwrap_err().kind, "escape", "{path}");
        a.root = Some("pdf".into());
        a.path = path.into();
        assert_eq!(l.artifact_path(&a).unwrap_err().kind, "escape", "{path}");
    }
}

#[test]
fn rram_fixture_special_cases() {
    let r = fixture("rram");
    let l = locate(
        &r.join("project"),
        &LocateOptions {
            store: Some(r.join("store")),
            orch_home: None,
        },
    )
    .unwrap();
    let inventory = rows(&l, "inventory");
    let errors: BTreeSet<_> = inventory
        .iter()
        .filter(|x| !x.fields["read_error"].is_empty())
        .map(|x| x.fields["path"].as_str())
        .collect();
    assert_eq!(
        errors,
        BTreeSet::from(["broken.pdf", "paper3.pdf", "paper4.pdf"])
    );
    for row in inventory
        .iter()
        .filter(|x| !x.fields["read_error"].is_empty())
    {
        assert_eq!(row.fields["read_error"], "read failed");
    }
    let exact = inventory
        .iter()
        .find(|x| x.fields["path"] == "paper3.pdf")
        .unwrap();
    let a = inventory
        .iter()
        .find(|x| x.fields["path"] == "a.pdf")
        .unwrap();
    let b = inventory
        .iter()
        .find(|x| x.fields["path"] == "b.pdf")
        .unwrap();
    assert_eq!(exact.fields["duplicate_type"], "exact");
    assert_eq!(exact.fields["canonical"], "0");
    assert_eq!(exact.fields["sha256"], a.fields["sha256"]);
    assert_eq!(a.fields["doi"], "10.1234/same");
    assert_eq!(a.fields["doi"], b.fields["doi"]);
    assert_eq!(a.fields["duplicate_group"], "dup");
    assert_eq!(a.fields["duplicate_group"], b.fields["duplicate_group"]);

    let evidence = rows(&l, "evidence");
    let geometry: Vec<_> = evidence
        .iter()
        .filter(|x| !x.fields["bbox"].is_empty())
        .collect();
    assert_eq!(geometry.len(), 5);
    assert_eq!(
        geometry
            .iter()
            .map(|x| x.fields["evidence_id"].as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["E1", "E2", "E3", "E4", "E5"])
    );
    for e in geometry {
        for (key, value) in [
            ("bbox", "10,20,100,40"),
            ("page_width", "612"),
            ("page_height", "792"),
            ("coordinate_origin", "top_left"),
            ("coordinate_units", "pt"),
            ("rotation", "0"),
        ] {
            assert_eq!(e.fields[key], value, "{} {key}", e.fields["evidence_id"]);
        }
    }
    // These minimal fixture PDFs each have one /Pages node with a /Count.
    let counts: BTreeMap<_, u32> = [a, b]
        .into_iter()
        .map(|doc| {
            let pdf = fs::read_to_string(l.document_path(&doc.fields["path"]).unwrap()).unwrap();
            let count = pdf
                .split("/Count ")
                .nth(1)
                .unwrap()
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(doc.fields["n_pages"].parse::<u32>().unwrap(), count);
            (doc.fields["sha256"].clone(), count)
        })
        .collect();
    let pdf_rows: Vec<_> = evidence
        .iter()
        .filter(|x| x.fields["locator_type"] == "pdf_page")
        .collect();
    assert_eq!(pdf_rows.len(), 100);
    for e in pdf_rows {
        let page: u32 = e.fields["physical_page"].parse().unwrap();
        assert!((1..=counts[&e.fields["sha256"]]).contains(&page));
        if e.fields["sha256"] == a.fields["sha256"] {
            assert_eq!(e.fields["printed_page"].parse::<u32>().unwrap(), page + 10);
        }
    }
    let driver = read_json(&l, "driver", &Limits::default()).unwrap();
    assert_eq!(driver["state"], "paused");
    assert_eq!(driver["reason"], "rate limited");
    let completed = rows(&l, "completed");
    let limited: Vec<_> = completed
        .iter()
        .filter(|x| x.fields["status"] == "rate_limited")
        .collect();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].fields["retry_after"], ""); // JSON null maps to an empty field.
    let completed_text = read_text(&l, "completed", &Limits::default()).unwrap();
    let limited_json: Vec<serde_json::Value> = completed_text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|row: &serde_json::Value| row["status"] == "rate_limited")
        .collect();
    assert_eq!(limited_json.len(), 1);
    assert_eq!(
        limited_json[0].get("retry_after"),
        Some(&serde_json::Value::Null)
    );
    for (id, sidecar) in [("acceptance", "acceptance_sha"), ("splits", "splits_sha")] {
        assert_eq!(
            read_text(&l, sidecar, &Limits::default()).unwrap().trim(),
            verify(&l, id).unwrap()
        );
    }
    assert_ne!(
        read_json(&l, "validation", &Limits::default()).unwrap()["measurements_sha256"]
            .as_str()
            .unwrap(),
        verify(&l, "measurements").unwrap()
    );
}

fn copied_rram(d: &Path) -> PathBuf {
    let project = d.join("project");
    copy_tree(&fixture("rram").join("project"), &project);
    project
}

#[test]
fn restored_stages_and_gates() {
    let d = tempfile::tempdir().unwrap();
    let m = rram::manifest(&copied_rram(d.path()), &Default::default()).unwrap();
    assert_eq!(m.stages.len(), 6);
    assert_eq!(
        m.stages.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [
            "inventory",
            "screen",
            "pilot",
            "acceptance",
            "extract",
            "splits"
        ]
    );
    let stage = |id: &str| m.stages.iter().find(|s| s.id == id).unwrap();
    assert_eq!(stage("inventory").outputs, ["inventory"]);
    assert_eq!(stage("splits").outputs, ["splits"]);
    let screen = stage("screen");
    assert_eq!(screen.progress.as_deref(), Some("progress_screen"));
    let gate = screen.gate.as_ref().unwrap();
    assert_eq!(
        (
            &*gate.artifact,
            &*gate.pointer,
            &gate.op,
            &gate.value,
            &gate.setting
        ),
        (
            "recall",
            "/passed",
            &GateOp::Eq,
            &Some(serde_json::json!(true)),
            &None
        )
    );
    for id in ["pilot", "acceptance"] {
        let gate = stage(id).gate.as_ref().unwrap();
        assert_eq!(gate.artifact, "effort_lock");
        assert_eq!(gate.pointer, format!("/{id}/accuracy"));
        assert_eq!(gate.op, GateOp::Ge);
        assert_eq!(gate.value, None);
        assert_eq!(
            gate.setting.as_deref(),
            Some(format!("/{id}/accuracy_threshold").as_str())
        );
    }
    let extract = stage("extract");
    assert_eq!(extract.outputs, ["measurements", "evidence"]);
    let gate = extract.gate.as_ref().unwrap();
    assert_eq!(
        (
            &*gate.artifact,
            &*gate.pointer,
            &gate.op,
            &gate.value,
            &gate.setting
        ),
        (
            "validation",
            "/n_errors",
            &GateOp::Eq,
            &Some(serde_json::json!(0)),
            &None
        )
    );
}

#[test]
fn restored_links_and_python_requirement() {
    let d = tempfile::tempdir().unwrap();
    let project = copied_rram(d.path());
    let python_requirement = ">=3.11, <4";
    fs::write(
        project.join("pyproject.toml"),
        format!("[project]\nrequires-python = '{python_requirement}'\n"),
    )
    .unwrap();
    let m = rram::manifest(&project, &Default::default()).unwrap();
    assert_eq!(m.links.len(), 3);
    assert_eq!(
        m.links,
        vec![
            Link {
                from: "evidence".into(),
                to: "measurements".into(),
                key: "evidence_id".into(),
                separator: Some(";".into())
            },
            Link {
                from: "inventory".into(),
                to: "screening".into(),
                key: "path".into(),
                separator: None
            },
            Link {
                from: "measurements".into(),
                to: "splits".into(),
                key: "paper_id".into(),
                separator: None
            },
        ]
    );
    assert_eq!(
        m.versions.environment["requires-python"],
        python_requirement
    );
}

#[test]
fn restored_revision_and_snapshot_versions() {
    let d = tempfile::tempdir().unwrap();
    let project = copied_rram(d.path());
    fs::create_dir_all(project.join(".git/refs/heads")).unwrap();
    let revision = "0123456789abcdef0123456789abcdef01234567";
    fs::write(project.join(".git/HEAD"), "ref: refs/heads/test\n").unwrap();
    fs::write(
        project.join(".git/refs/heads/test"),
        format!("{revision}\n"),
    )
    .unwrap();
    for stamp in ["20240101T000000Z", "20280101T000000Z", "20270101T000000Z"] {
        fs::write(
            project.join(format!("state/source_snapshot_{stamp}.json")),
            "{}",
        )
        .unwrap();
    }
    let m = rram::manifest(&project, &Default::default()).unwrap();
    assert_eq!(m.versions.code.as_deref(), Some(revision));
    assert_eq!(m.research_version, "20280101T000000Z");
    let detached = "abcdef0123456789abcdef0123456789abcdef01";
    fs::write(project.join(".git/HEAD"), format!("{detached}\n")).unwrap();
    for entry in fs::read_dir(project.join("state")).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("source_snapshot_")
        {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let m = rram::manifest(&project, &Default::default()).unwrap();
    assert_eq!(m.versions.code.as_deref(), Some(detached));
    assert_eq!(m.research_version, "unversioned");
}

#[test]
fn restored_roots_roles_and_store_precedence() {
    let d = tempfile::tempdir().unwrap();
    let project = copied_rram(d.path());
    let home = d.path().join("orch");
    fs::create_dir_all(home.join("rram_fixture")).unwrap();
    let store = d.path().join("explicit_store");
    fs::create_dir_all(&store).unwrap();
    let l = locate(
        &project,
        &LocateOptions {
            store: Some(store.clone()),
            orch_home: Some(home),
        },
    )
    .unwrap();
    assert_eq!(l.store.as_deref(), Some(store.as_path()));
    let expected = BTreeSet::from([
        l.root.clone(),
        fs::canonicalize(project.join("pdfs")).unwrap(),
        store,
    ]);
    assert_eq!(l.roots().into_iter().collect::<BTreeSet<_>>(), expected);
    assert_eq!(
        l.by_role(Role::Inventory)
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["inventory"]
    );
    assert_eq!(
        l.by_role(Role::Gate)
            .iter()
            .map(|a| a.id.as_str())
            .collect::<Vec<_>>(),
        ["effort_lock"]
    );
    assert_eq!(
        l.root_path(Some("unknown")).unwrap_err().kind,
        "invalid_manifest"
    );
}

#[test]
fn restored_artifacts_follow_files() {
    let d = tempfile::tempdir().unwrap();
    let project = copied_rram(d.path());
    let ids = |m: &Manifest| {
        m.artifacts
            .iter()
            .map(|a| a.id.clone())
            .collect::<BTreeSet<_>>()
    };
    let m = rram::manifest(&project, &Default::default()).unwrap();
    assert_eq!(m.artifacts.len(), 18);
    let expected: BTreeSet<_> = [
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
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(ids(&m), expected);
    assert!(m.capabilities.contains(&Capability::PdfPages));
    fs::remove_file(project.join("data/evidence.csv")).unwrap();
    let m = rram::manifest(&project, &Default::default()).unwrap();
    let mut remaining = expected.clone();
    remaining.remove("evidence");
    assert_eq!(ids(&m), remaining);
    assert!(!m.capabilities.contains(&Capability::PdfPages));
    fs::copy(
        fixture("rram").join("project/data/evidence.csv"),
        project.join("data/evidence.csv"),
    )
    .unwrap();
    fs::remove_file(project.join("data/literature_inventory.csv")).unwrap();
    let m = rram::manifest(&project, &Default::default()).unwrap();
    remaining = expected;
    remaining.remove("inventory");
    assert_eq!(ids(&m), remaining);
    assert!(!m.capabilities.contains(&Capability::PdfPages));
}

#[test]
fn restored_row_strictness() {
    let d = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rawonly"), d.path());
    let l = locate(d.path(), &Default::default()).unwrap();
    fs::write(
        d.path().join("runs/device_A.csv"),
        "key,value\nshort\nlong,2,discard\n",
    )
    .unwrap();
    let csv = rows(&l, "run_a");
    assert_eq!(csv.len(), 2);
    assert_eq!(
        csv[0].fields,
        BTreeMap::from([("key".into(), "short".into()), ("value".into(), "".into())])
    );
    assert_eq!(
        csv[1].fields,
        BTreeMap::from([("key".into(), "long".into()), ("value".into(), "2".into())])
    );
    let jsonl = d.path().join("runs/device_B.jsonl");
    fs::write(&jsonl, "{\"x\":1}\n  \n{\"x\":2}").unwrap();
    let page = read_rows(&l, "run_b", None, 10, &Limits::default()).unwrap();
    assert_eq!(page.rows.len(), 2);
    assert_eq!(page.rows[0].fields["x"], "1");
    assert_eq!(page.rows[1].fields["x"], "2");
    assert_eq!(page.rows[1].number, 3);
    assert!(page.next.is_none());
    let limits = Limits {
        max_record_bytes: 8,
        ..Default::default()
    };
    fs::write(&jsonl, "{\"x\":12345}\n").unwrap();
    assert_eq!(
        read_rows(&l, "run_b", None, 10, &limits).unwrap_err().kind,
        "oversized_record"
    );
    fs::write(&jsonl, "{\"x\":1}\n{\"x\":").unwrap();
    let e = read_rows(&l, "run_b", None, 10, &Limits::default()).unwrap_err();
    assert_eq!(e.kind, "corrupt_artifact");
    assert!(e.message.contains("line 2"), "{}", e.message);
}
