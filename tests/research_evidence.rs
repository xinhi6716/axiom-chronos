use axiom::research_workspace::{
    evidence::{self, Layer, Locator, Target},
    index::Index,
    manifest::{locate, LocateOptions},
    read::{hash_file, Limits},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name)
}
fn index(path: &Path) -> Index {
    Index::build(
        Arc::new(locate(path, &LocateOptions::default()).unwrap()),
        Limits::default(),
        &|_, _| {},
    )
    .unwrap()
}
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}
fn hashes(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(p.strip_prefix(root).unwrap().into(), hash_file(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn replace(path: &Path, from: &str, to: &str) {
    let s = fs::read_to_string(path).unwrap();
    assert!(s.contains(from), "missing {from}");
    fs::write(path, s.replacen(from, to, 1)).unwrap();
}

#[test]
fn resolution_geometry_errors_and_untouched_sources() {
    let fixtures = fixture("");
    let before = hashes(&fixtures);
    let ix = index(&fixture("rram/project"));
    let lim = Limits::default();
    let expected = fs::read_to_string(fixture("rram/expected_resolution.csv")).unwrap();
    let mut count = 0;
    for line in expected.lines().skip(1) {
        let cols: Vec<_> = line.split(',').collect();
        let resolved = evidence::resolve(&ix, cols[0], &lim).unwrap();
        match (&resolved.item.locator, resolved.target) {
            (
                Locator::PdfPage { sha256, .. },
                Target::PdfPage {
                    document_path,
                    physical_page,
                    printed_page,
                    ..
                },
            ) => {
                assert_eq!(cols[1], "pdf_page");
                assert_eq!(sha256, cols[2]);
                assert_eq!(physical_page.to_string(), cols[3]);
                assert_eq!(printed_page.as_deref().unwrap_or(""), cols[4]);
                let inventory = ix
                    .located()
                    .by_role(axiom::research_workspace::manifest::Role::Inventory)[0];
                let found = (0..ix.len(&inventory.id))
                    .filter_map(|n| ix.row(&inventory.id, n as u32))
                    .find(|r| r.get("sha256") == cols[2])
                    .unwrap();
                assert_eq!(document_path, found.get("path"));
                assert_eq!(resolved.item.quote, cols[8]);
            }
            (
                Locator::RawField { .. },
                Target::RawField {
                    file,
                    row,
                    column,
                    value,
                },
            ) => {
                assert_eq!(cols[1], "raw_field");
                assert_eq!(file, cols[5]);
                assert_eq!(row.to_string(), cols[6]);
                assert_eq!(column, cols[7]);
                assert_eq!(value, cols[8]);
            }
            _ => panic!("locator mismatch"),
        }
        count += 1;
    }
    assert_eq!(count, 100);
    let raw = index(&fixture("rawonly"));
    for (id, file, value) in [
        ("RE1", "runs/device_A.csv", "1"),
        ("RE2", "runs/device_B.jsonl", "2"),
    ] {
        assert_eq!(
            evidence::resolve(&raw, id, &lim).unwrap().target,
            Target::RawField {
                file: file.into(),
                row: 1,
                column: "value".into(),
                value: value.into()
            }
        );
    }
    assert_eq!(
        evidence::item(&ix, "missing").unwrap_err().kind,
        "not_found"
    );
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("rram");
    copy_tree(&fixture("rram/project"), &project);
    let path = project.join("data/evidence.csv");
    replace(
        &path,
        "E1,P0,c0b3ff2c5cffd5bdcdf034af5e32399039fdc1a98dbbf4ff2a0214d4164b025b,1,11",
        "E1,P0,c0b3ff2c5cffd5bdcdf034af5e32399039fdc1a98dbbf4ff2a0214d4164b025b,99,11",
    );
    assert_eq!(
        evidence::resolve(&index(&project), "E1", &lim)
            .unwrap_err()
            .kind,
        "corrupt_artifact"
    );
    let rawdir = temp.path().join("raw");
    copy_tree(&fixture("rawonly"), &rawdir);
    fs::remove_file(rawdir.join("runs/device_A.csv")).unwrap();
    assert_eq!(
        evidence::resolve(&index(&rawdir), "RE1", &lim)
            .unwrap_err()
            .kind,
        "missing_artifact"
    );
    replace(
        &rawdir.join("data/evidence.csv"),
        "runs/device_A.csv",
        "../outside.csv",
    );
    assert_eq!(
        evidence::resolve(&index(&rawdir), "RE1", &lim)
            .unwrap_err()
            .kind,
        "escape"
    );
    let geom = temp.path().join("geometry");
    copy_tree(&fixture("rram/project"), &geom);
    let ev = geom.join("data/evidence.csv");
    replace(
        &ev,
        "\"10,20,100,40\",612,792,top_left,pt,0",
        ",612,792,top_left,pt,0",
    );
    replace(
        &ev,
        "\"10,20,100,40\",612,792,top_left,pt,0",
        "\"10,20,100,40\",612,,top_left,pt,0",
    );
    let gi = index(&geom);
    for (id, present) in [("E1", false), ("E2", false), ("E3", true)] {
        match evidence::resolve(&gi, id, &lim).unwrap().target {
            Target::PdfPage { highlight, .. } => {
                assert_eq!(highlight.is_some(), present);
                if let Some(h) = highlight {
                    assert_eq!(
                        (h.bbox.x0, h.bbox.y0, h.bbox.x1, h.bbox.y1),
                        (10., 20., 100., 40.)
                    );
                }
            }
            _ => panic!(),
        }
    }
    assert_eq!(hashes(&fixtures), before);
}

#[test]
fn aliases_conditions_contradictions_and_layers() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rram/project"), temp.path());
    let path = temp.path().join("data/measurements.csv");
    fs::write(&path, "row_id,condition_id,variable,entity_raw,entity_canonical,value_original,value_canonical,evidence_id\nM1,C1,v,HfO₂,,1,1,E1;E2\nM2,C1,v,hfo2,,2,2,E3\nM3,C2,v,HfO₂,,3,3,E4\n").unwrap();
    let ix = index(temp.path());
    let group = evidence::group(&ix, "hFo₂", Layer::Literature).unwrap();
    assert_eq!(group.entity, "HfO2");
    assert_eq!(group.aliases, ["HfO₂", "hfo2"]);
    assert_eq!(
        group
            .conditions
            .iter()
            .map(|c| (c.condition_id.as_str(), c.rows.len()))
            .collect::<Vec<_>>(),
        [("C1", 2), ("C2", 1)]
    );
    assert_eq!(group.contradictions.len(), 1);
    assert_eq!(group.contradictions[0].condition_id, "C1");
    assert_eq!(group.conditions[0].rows[0].evidence, ["E1", "E2"]);
    assert!(group
        .conditions
        .iter()
        .flat_map(|c| &c.rows)
        .all(|r| r.layer == Layer::Literature && r.record_id.starts_with("measurements:")));
    let raw = evidence::group(&ix, "HfO₂", Layer::Raw).unwrap();
    assert!(raw
        .conditions
        .iter()
        .flat_map(|c| &c.rows)
        .all(|r| r.layer == Layer::Raw && r.record_id.starts_with("raw_measurements:")));
    let (names, total) = evidence::entities(&ix, Layer::Literature, 0, 10).unwrap();
    assert_eq!(total, 1);
    assert_eq!(names, ["HfO2"]);
    fs::write(&path, "row_id,condition_id,variable,entity_raw,value_original,value_canonical\nM1,C1,v,HfO₂,1,1\nM2,C2,v,HfO₂,2,2\n").unwrap();
    assert!(
        evidence::group(&index(temp.path()), "HfO2", Layer::Literature)
            .unwrap()
            .contradictions
            .is_empty()
    );
}

#[test]
fn raw_key_lookup_pagination_and_unresolved_cells() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rawonly"), temp.path());
    let path = temp.path().join("data/evidence.csv");
    replace(
        &path,
        "runs/device_A.csv,project,1,A1,value",
        "runs/device_A.csv,project,,A2,value",
    );
    fs::write(
        temp.path().join("runs/device_A.csv"),
        "key,value\nA1,first\nA2,second\n",
    )
    .unwrap();
    let limits = Limits {
        max_page_rows: 1,
        ..Limits::default()
    };
    let ix = index(temp.path());
    assert_eq!(
        evidence::resolve(&ix, "RE1", &limits).unwrap().target,
        Target::RawField {
            file: "runs/device_A.csv".into(),
            row: 2,
            column: "value".into(),
            value: "second".into()
        }
    );
    replace(&path, "A2,value", "absent,value");
    assert_eq!(
        evidence::resolve(&index(temp.path()), "RE1", &limits)
            .unwrap_err()
            .kind,
        "unresolved_locator"
    );
    replace(&path, "absent,value", "A2,absent");
    assert_eq!(
        evidence::resolve(&index(temp.path()), "RE1", &limits)
            .unwrap_err()
            .kind,
        "unresolved_locator"
    );
}

#[test]
fn aliases_use_unicode_casefold() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rram/project"), temp.path());
    fs::write(
        temp.path().join("data/aliases.csv"),
        "category,alias,canonical\nmaterial,Straße,canonical\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("data/measurements.csv"),
        "row_id,condition_id,variable,entity_raw,value_original\nM1,C1,v,STRASSE,1\n",
    )
    .unwrap();
    let group = evidence::group(&index(temp.path()), "strasse", Layer::Literature).unwrap();
    assert_eq!(group.entity, "canonical");
    assert_eq!(group.conditions[0].rows[0].record_id, "measurements:M1");
}
