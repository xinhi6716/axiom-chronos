use axiom::research_workspace::{
    dataset::{self, Cell, Conversion, DataLayer, DatasetQuery, Validation},
    index::{Filter, Index, Op, Sort},
    manifest::{locate, LocateOptions, Located, Role},
    read::{hash_file, Limits},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name)
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}
fn located(path: &Path) -> Located {
    locate(path, &LocateOptions::default()).unwrap()
}
fn build(l: Located) -> Index {
    Index::build(Arc::new(l), Limits::default(), &|_, _| {}).unwrap()
}
fn query(layer: DataLayer) -> DatasetQuery {
    DatasetQuery {
        layer,
        filters: vec![],
        sort: None,
        offset: 0,
        limit: 100,
    }
}

#[test]
fn values_layers_evidence_and_paging() {
    let ix = build(located(&fixture("rram/project")));
    let all = dataset::page(&ix, &query(DataLayer::Extracted)).unwrap();
    assert_eq!(all.total, 60);
    assert_eq!(all.generation, 1);
    let first = &all.rows[0];
    assert_eq!(first.record_id, "measurements:M1");
    assert_eq!(first.original, Cell::Value("0".into()));
    assert_eq!(first.canonical, Cell::Value("0".into()));
    assert_eq!(first.conversion, Conversion::Same);
    assert_eq!(
        all.rows[1].original,
        Cell::Empty {
            reason: Some("not reported".into())
        }
    );
    assert_eq!(
        all.rows[1].canonical,
        Cell::Empty {
            reason: Some("not reported".into())
        }
    );
    assert_eq!(
        all.rows[2].conversion,
        Conversion::NotConvertible {
            reason: "no canonical value recorded".into()
        }
    );
    assert!(all.rows.iter().all(|r| r.layer == DataLayer::Extracted
        && r.record_id.starts_with("measurements:M")
        && r.locator.is_none()));
    let mut q = query(DataLayer::Extracted);
    q.filters.push(dataset::evidence_filter("E3"));
    let hits = dataset::page(&ix, &q).unwrap();
    assert_eq!(
        hits.rows
            .iter()
            .map(|r| r.record_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "measurements:M3",
            "measurements:M30",
            "measurements:M31",
            "measurements:M32",
            "measurements:M33",
            "measurements:M34",
            "measurements:M35",
            "measurements:M36",
            "measurements:M37",
            "measurements:M38",
            "measurements:M39"
        ]
    );
    q.filters = vec![dataset::evidence_filter("E1")];
    q.filters.push(Filter {
        column: "row_id".into(),
        op: Op::Eq("M1".into()),
    });
    assert_eq!(dataset::page(&ix, &q).unwrap().rows[0].evidence, vec!["E1"]);
    q.filters = vec![Filter {
        column: "condition_id".into(),
        op: Op::Eq("C1".into()),
    }];
    q.sort = Some(Sort {
        column: "row_id".into(),
        descending: false,
    });
    let complete = dataset::page(&ix, &q).unwrap();
    q.offset = 2;
    q.limit = 3;
    let paged = dataset::page(&ix, &q).unwrap();
    assert_eq!(paged.total, complete.total);
    assert_eq!(paged.offset, 2);
    assert_eq!(paged.rows, complete.rows[2..5]);
    let raw = dataset::page(&ix, &query(DataLayer::Raw)).unwrap();
    assert_eq!(raw.total, 20);
    assert!(raw
        .rows
        .iter()
        .all(|r| r.layer == DataLayer::Raw && r.record_id.starts_with("raw_measurements:R")));
    assert_eq!(
        raw.rows[0].locator.as_deref(),
        Some("external/hf02/pulse_run1.csv#1/value")
    );
    let mut raw_query = query(DataLayer::Raw);
    raw_query.filters.push(dataset::evidence_filter("E101"));
    let hits = dataset::page(&ix, &raw_query).unwrap();
    assert_eq!(hits.total, 1);
    assert_eq!(hits.rows[0].record_id, "raw_measurements:R1");
}

#[test]
fn validation_tracks_current_file() {
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rram/project"), dir.path());
    let mut l = located(dir.path());
    let ix = build(l.clone());
    let hash = hash_file(&dir.path().join("data/measurements.csv")).unwrap();
    assert_eq!(
        dataset::validation(&ix),
        Validation::Stale {
            recorded: "0".repeat(64),
            current: hash.clone()
        }
    );
    let report = dir.path().join("research/validation_report.json");
    fs::write(
        &report,
        serde_json::json!({"measurements_sha256":hash, "n_errors":0}).to_string(),
    )
    .unwrap();
    assert_eq!(
        dataset::validation(&ix),
        Validation::Valid {
            data_sha256: hash.clone()
        }
    );
    fs::write(
        &report,
        serde_json::json!({"measurements_sha256":hash, "n_errors":2}).to_string(),
    )
    .unwrap();
    assert_eq!(
        dataset::validation(&ix),
        Validation::Invalid {
            errors: 2,
            data_sha256: hash.clone()
        }
    );
    fs::write(&report, "broken").unwrap();
    assert_eq!(dataset::validation(&ix), Validation::NotRun);
    l.manifest.artifacts.retain(|a| a.role != Role::Validation);
    assert_eq!(dataset::validation(&build(l)), Validation::NotRun);
    fs::write(
        &report,
        serde_json::json!({"measurements_sha256":hash, "n_errors":0}).to_string(),
    )
    .unwrap();
    fs::remove_file(dir.path().join("data/measurements.csv")).unwrap();
    assert_eq!(dataset::validation(&ix), Validation::NotRun);
}

#[test]
fn rawonly_capability_and_schema_missing_rates() {
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rawonly"), dir.path());
    let mut l = located(dir.path());
    let a = l
        .manifest
        .artifacts
        .iter_mut()
        .find(|a| a.role == Role::RawMeasurements)
        .unwrap();
    a.columns
        .insert("value_original".into(), "value_original".into());
    a.columns
        .insert("value_canonical".into(), "value_canonical".into());
    let path = dir.path().join("data/raw_measurements.csv");
    let data = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        data.replace(
            "R2,runs/device_B.jsonl,1,B1,value,D1,C1,voltage,HfO₂,HfO2,1,V,1,V",
            "R2,runs/device_B.jsonl,1,B1,value,D1,C1,voltage,HfO₂,HfO2,,V,,V",
        ),
    )
    .unwrap();
    let ix = build(l);
    assert_eq!(
        dataset::page(&ix, &query(DataLayer::Extracted))
            .unwrap_err()
            .kind,
        "not_capable"
    );
    assert_eq!(
        dataset::missing_rates(&ix, DataLayer::Extracted)
            .unwrap_err()
            .kind,
        "not_capable"
    );
    let rows = dataset::page(&ix, &query(DataLayer::Raw)).unwrap().rows;
    assert_eq!(rows[0].original, Cell::Value("0".into()));
    assert!(matches!(rows[1].original, Cell::Empty { .. }));
    let rates = dataset::missing_rates(&ix, DataLayer::Raw).unwrap();
    assert_eq!(
        rates,
        vec![
            dataset::MissingRate {
                column: "value_canonical".into(),
                missing: 1,
                total: 2
            },
            dataset::MissingRate {
                column: "value_original".into(),
                missing: 1,
                total: 2
            }
        ]
    );
}

#[test]
fn mapping_conversion_and_artifact_errors() {
    let dir = tempfile::tempdir().unwrap();
    copy_tree(&fixture("rawonly"), dir.path());
    let mut l = located(dir.path());
    let a = l
        .manifest
        .artifacts
        .iter_mut()
        .find(|a| a.role == Role::RawMeasurements)
        .unwrap();
    a.columns.insert("value_original".into(), "measured".into());
    let path = dir.path().join("data/raw_measurements.csv");
    fs::write(&path, "row_id,source_file,source_row,source_column,entity_raw,entity_canonical,measured,unit_original,value_canonical,unit_canonical,conversion_rule,missing_reason,evidence_id,flags\nR1,file,1,v,raw,,2,V,,mV,scale,unknown, E1 ; E2 | , x | y ;\nR2,file,,v,raw,,text,V,text,V,,,E3,\nR3,file,3,v,raw,canon,2,V,2000,mV,scale,,,\n").unwrap();
    let ix = build(l.clone());
    let rows = dataset::page(&ix, &query(DataLayer::Raw)).unwrap().rows;
    assert_eq!(rows[0].entity, "raw");
    assert_eq!(
        rows[0].conversion,
        Conversion::NotConvertible {
            reason: "unknown".into()
        }
    );
    assert_eq!(rows[0].evidence, vec!["E1", "E2"]);
    assert_eq!(rows[0].flags, vec!["x", "y"]);
    assert_eq!(rows[1].conversion, Conversion::NotNumeric);
    assert_eq!(rows[1].locator, None);
    assert_eq!(
        rows[2].conversion,
        Conversion::Converted {
            rule: "scale".into()
        }
    );
    fs::remove_file(&path).unwrap();
    let missing = build(l.clone());
    assert_eq!(
        dataset::page(&missing, &query(DataLayer::Raw))
            .unwrap_err()
            .kind,
        "missing_artifact"
    );
    assert_eq!(
        dataset::missing_rates(&missing, DataLayer::Raw)
            .unwrap_err()
            .kind,
        "missing_artifact"
    );
    fs::write(&path, "row_id\nR1\nR1\n").unwrap();
    let corrupt = build(l);
    assert_eq!(
        dataset::page(&corrupt, &query(DataLayer::Raw))
            .unwrap_err()
            .kind,
        "corrupt_artifact"
    );
}
