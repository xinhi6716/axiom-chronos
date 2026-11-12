use axiom::usage::laya::{self, UsageEvent};
use std::{fs::OpenOptions, io::Write, sync::Arc, thread};

fn event(id: &str, model: &str) -> UsageEvent {
    UsageEvent {
        id: id.into(),
        ts_ms: 123,
        model_version: model.into(),
        kind: "inference".into(),
        backend: "cpu".into(),
        tokens: None,
        gpu_hours: None,
        compute_units: None,
    }
}

#[test]
fn missing_file_is_empty() {
    let home = tempfile::tempdir().unwrap();
    let usage = laya::summarize(home.path()).unwrap();
    assert!(usage.models.is_empty());
    assert_eq!(usage.skipped_lines, 0);
    assert_eq!(laya::cli_text(home.path()).unwrap(), "");
    assert_eq!(laya::cli_json(home.path()).unwrap(), None);
}

#[test]
fn sums_deduplicates_sorts_and_skips_corrupt_lines() {
    let home = tempfile::tempdir().unwrap();
    let mut first = event("one", "z-model");
    first.tokens = Some(120);
    first.gpu_hours = Some(1.5);
    first.compute_units = Some(2.0);
    laya::record(home.path(), &first).unwrap();
    let mut duplicate = first.clone();
    duplicate.model_version = "a-model".into();
    duplicate.tokens = Some(999);
    laya::record(home.path(), &duplicate).unwrap();
    laya::record(home.path(), &event("two", "z-model")).unwrap();
    laya::record(home.path(), &event("three", "z-model")).unwrap();
    let mut other = event("four", "a-model");
    other.tokens = Some(5);
    other.gpu_hours = Some(0.5);
    other.compute_units = Some(3.0);
    laya::record(home.path(), &other).unwrap();
    let mut extra_z = event("five", "z-model");
    extra_z.tokens = Some(30);
    extra_z.gpu_hours = Some(0.25);
    extra_z.compute_units = Some(1.0);
    laya::record(home.path(), &extra_z).unwrap();
    let mut extra_a = event("six", "a-model");
    extra_a.tokens = Some(7);
    extra_a.gpu_hours = Some(0.25);
    extra_a.compute_units = Some(4.0);
    laya::record(home.path(), &extra_a).unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(home.path().join("laya/usage.jsonl"))
        .unwrap();
    file.write_all(b"\xffnot-json\n").unwrap();

    let usage = laya::summarize(home.path()).unwrap();
    assert_eq!(usage.skipped_lines, 1);
    assert_eq!(usage.models.len(), 2);
    let a = &usage.models[0];
    assert_eq!(a.model_version, "a-model");
    assert_eq!((a.events, a.tokens, a.tokens_unknown), (2, 12, 0));
    assert_eq!((a.gpu_hours, a.gpu_hours_unknown), (0.75, 0));
    assert_eq!((a.compute_units, a.compute_units_unknown), (7.0, 0));
    let z = &usage.models[1];
    assert_eq!(z.model_version, "z-model");
    assert_eq!((z.events, z.tokens, z.tokens_unknown), (4, 150, 2));
    assert_eq!((z.gpu_hours, z.gpu_hours_unknown), (1.75, 2));
    assert_eq!((z.compute_units, z.compute_units_unknown), (3.0, 2));
    let rendered = laya::render(&usage);
    assert!(rendered.contains("Laya:"));
    assert!(rendered.contains("GPU hours"));
    assert!(rendered.contains("tokens 150 (+2 unknown)"));
    assert_eq!(laya::cli_text(home.path()).unwrap(), rendered);
    assert_eq!(
        laya::cli_json(home.path()).unwrap(),
        Some(serde_json::to_value(&usage).unwrap())
    );
}

#[test]
fn unknown_is_not_known_zero_negative_control() {
    let unknown_home = tempfile::tempdir().unwrap();
    let zero_home = tempfile::tempdir().unwrap();
    let unknown = event("same", "model");
    let mut zero = unknown.clone();
    zero.tokens = Some(0);
    zero.gpu_hours = Some(0.0);
    zero.compute_units = Some(0.0);
    laya::record(unknown_home.path(), &unknown).unwrap();
    laya::record(zero_home.path(), &zero).unwrap();
    let unknown_usage = laya::summarize(unknown_home.path()).unwrap();
    let zero_usage = laya::summarize(zero_home.path()).unwrap();
    assert_ne!(unknown_usage, zero_usage);
    let u = &unknown_usage.models[0];
    let z = &zero_usage.models[0];
    assert_eq!(
        (
            u.tokens_unknown,
            u.gpu_hours_unknown,
            u.compute_units_unknown
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (
            z.tokens_unknown,
            z.gpu_hours_unknown,
            z.compute_units_unknown
        ),
        (0, 0, 0)
    );
    let unknown_text = laya::render(&unknown_usage);
    let zero_text = laya::render(&zero_usage);
    assert!(unknown_text.contains("tokens unknown, GPU hours unknown, compute units unknown"));
    assert!(zero_text.contains("tokens 0, GPU hours 0, compute units 0"));
    assert_ne!(unknown_text, zero_text);
}

#[test]
fn concurrent_records_preserve_all_lines() {
    let home = tempfile::tempdir().unwrap();
    let path = Arc::new(home.path().to_path_buf());
    let workers: Vec<_> = (0..8)
        .map(|worker| {
            let path = Arc::clone(&path);
            thread::spawn(move || {
                for index in 0..25 {
                    laya::record(&path, &event(&format!("{worker}-{index}"), "model")).unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let contents = std::fs::read_to_string(path.join("laya/usage.jsonl")).unwrap();
    assert_eq!(contents.lines().count(), 200);
    let usage = laya::summarize(&path).unwrap();
    assert_eq!(usage.skipped_lines, 0);
    assert_eq!(usage.models[0].events, 200);
}
