use axiom::usage::{
    extensions::{self, ExtRecord, Tier},
    price::CodexUsage,
};
use std::{
    fs,
    io::Write,
    sync::{Arc, Barrier},
    thread,
};

fn record(provider: &str) -> ExtRecord {
    ExtRecord {
        provider: provider.into(),
        ..ExtRecord::default()
    }
}

#[test]
fn groups_are_isolated_and_totals_include_resources() {
    let records = vec![
        ExtRecord {
            module: Some("a".into()),
            project: Some("x".into()),
            input_tokens: 10,
            cached_input_tokens: 3,
            output_tokens: 4,
            gpu_seconds: 3600.0,
            compute_units: 1.5,
            usd: Some(2.0),
            ..record("gpu_local")
        },
        ExtRecord {
            module: Some("b".into()),
            project: Some("y".into()),
            input_tokens: 20,
            gpu_seconds: 3600.0,
            compute_units: 2.5,
            usd: None,
            ..record("colab")
        },
        ExtRecord {
            project: Some("x".into()),
            usd: Some(1.0),
            ..record("claude")
        },
    ];
    let modules = extensions::by_module(&records);
    assert_eq!(modules.len(), 2);
    assert_eq!(modules["a"].input_tokens, 10);
    assert_eq!(modules["a"].priced_usd, 2.0);
    assert_eq!(modules["b"].input_tokens, 20);
    assert_eq!(modules["b"].priced_usd, 0.0);
    assert_eq!(modules["b"].unpriced_records, 1);
    let projects = extensions::by_project(&records);
    assert_eq!(projects["x"].records, 2);
    assert_eq!(projects["y"].records, 1);
    let total = extensions::total(&records);
    assert_eq!(total.records, 3);
    assert_eq!(
        (
            total.input_tokens,
            total.cached_input_tokens,
            total.output_tokens
        ),
        (30, 3, 4)
    );
    assert_eq!(total.gpu_hours(), 2.0);
    assert_eq!(total.compute_units, 4.0);
    assert_eq!(total.priced_usd, 3.0);
    assert_eq!(total.unpriced_records, 1);
    assert!(extensions::by_project(&[record("agy")]).is_empty());
}

#[test]
fn extreme_pricing_negative_control_and_codex_tier_split() {
    let usage = CodexUsage {
        input: 1_000_000,
        cached_input: 0,
        output: 0,
    };
    let normal = extensions::codex_usd("gpt-5.2", Tier::Normal, &usage);
    let extreme = extensions::codex_usd("gpt-5.2", Tier::Extreme, &usage);
    assert!(normal.is_some());
    assert_eq!(extreme, None);
    let records = vec![
        ExtRecord {
            tier: Tier::Normal,
            usd: normal,
            ..record("codex")
        },
        ExtRecord {
            tier: Tier::Extreme,
            usd: extreme,
            ..record("codex")
        },
        ExtRecord {
            tier: Tier::Extreme,
            usd: Some(100.0),
            ..record("claude")
        },
    ];
    let tiers = extensions::by_tier(&records);
    assert_eq!(tiers.len(), 2);
    assert_eq!(tiers[&Tier::Normal].priced_usd, normal.unwrap());
    assert_eq!(tiers[&Tier::Extreme].records, 1);
    assert_eq!(tiers[&Tier::Extreme].unpriced_records, 1);
    assert_eq!(tiers[&Tier::Extreme].priced_usd, 0.0);
}

#[test]
fn missing_and_malformed_lines_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(extensions::read(dir.path()).unwrap(), (vec![], 0));
    assert_eq!(
        extensions::path(dir.path()),
        dir.path().join("usage").join("extensions.jsonl")
    );
    let first = record("codex");
    extensions::append(dir.path(), &first).unwrap();
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(extensions::path(dir.path()))
        .unwrap();
    file.write_all(b"not json\n{\"model\":\"missing provider\"}\n\xff\n{\"provider\":\"agy\"}\n")
        .unwrap();
    let (records, skipped) = extensions::read(dir.path()).unwrap();
    assert_eq!(skipped, 3);
    assert_eq!(records, vec![first, record("agy")]);
}

#[test]
fn append_preserves_existing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    extensions::append(dir.path(), &record("claude")).unwrap();
    let before = fs::read(extensions::path(dir.path())).unwrap();
    extensions::append(dir.path(), &record("codex")).unwrap();
    let after = fs::read(extensions::path(dir.path())).unwrap();
    assert!(after.starts_with(&before));
    assert!(after.len() > before.len());
    assert_eq!(
        extensions::read(dir.path()).unwrap().0,
        vec![record("claude"), record("codex")]
    );
}

#[test]
fn concurrent_appends_remain_intact() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 50;
    let dir = tempfile::tempdir().unwrap();
    let barrier = Arc::new(Barrier::new(THREADS));
    thread::scope(|scope| {
        for worker in 0..THREADS {
            let barrier = Arc::clone(&barrier);
            let home = dir.path();
            scope.spawn(move || {
                barrier.wait();
                for n in 0..PER_THREAD {
                    let item = ExtRecord {
                        job: Some(format!("{worker}-{n}")),
                        ..record("codex")
                    };
                    extensions::append(home, &item).unwrap();
                }
            });
        }
    });
    let (records, skipped) = extensions::read(dir.path()).unwrap();
    assert_eq!(skipped, 0);
    assert_eq!(records.len(), THREADS * PER_THREAD);
    let jobs: std::collections::BTreeSet<_> =
        records.into_iter().map(|item| item.job.unwrap()).collect();
    assert_eq!(jobs.len(), THREADS * PER_THREAD);
    for worker in 0..THREADS {
        for n in 0..PER_THREAD {
            assert!(jobs.contains(&format!("{worker}-{n}")));
        }
    }
}
