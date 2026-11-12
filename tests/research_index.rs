use axiom::research_workspace::{index::*, manifest::*, read::Limits, session::*};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

struct CountingAllocator;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static MEASURE: Mutex<()> = Mutex::new(());
fn add(n: usize) {
    let current = CURRENT.fetch_add(n, Ordering::SeqCst) + n;
    PEAK.fetch_max(current, Ordering::SeqCst);
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            add(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
        System.dealloc(p, layout);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(p, layout, size);
        if !next.is_null() {
            if size >= layout.size() {
                add(size - layout.size());
            } else {
                CURRENT.fetch_sub(layout.size() - size, Ordering::SeqCst);
            }
        }
        next
    }
}
#[global_allocator]
static ALLOC: CountingAllocator = CountingAllocator;

fn check_limit(name: &str, value: f64, limit: f64) -> Result<(), String> {
    if value <= limit {
        Ok(())
    } else {
        Err(format!("{name}: {value:.3} exceeds {limit:.3}"))
    }
}
fn p95(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[(values.len() * 95).div_ceil(100) - 1]
}
fn located(root: &Path) -> Arc<Located> {
    Arc::new(locate(root, &LocateOptions::default()).unwrap())
}
fn build(l: Arc<Located>) -> Index {
    Index::build(l, Limits::default(), &|_, _| {}).unwrap()
}
fn write_evidence(root: &Path, changed: bool) {
    let mut file = BufWriter::new(File::create(root.join("data/evidence.csv")).unwrap());
    writeln!(file, "evidence_id,paper_id,sha256,physical_page,quote").unwrap();
    for n in 0..40_000 {
        let quote = if changed && n == 0 {
            "changed quote".to_string()
        } else {
            format!("experimental evidence group {}", n % 20)
        };
        writeln!(
            file,
            "E{n},P{},hash{:064x},{},{}",
            n % 2000,
            n,
            n % 30 + 1,
            quote
        )
        .unwrap();
    }
}
fn synthetic(root: &Path) -> Arc<Located> {
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("research-manifest.json"), r#"{"manifest_version":1,"research_id":"perf","research_version":"1","title":"Performance","settings":{},"versions":{},"artifacts":[{"id":"evidence","role":"evidence","path":"data/evidence.csv","format":"csv"},{"id":"measurements","role":"measurements","path":"data/measurements.csv","format":"csv"}]}"#).unwrap();
    write_evidence(root, false);
    let mut file = BufWriter::new(File::create(root.join("data/measurements.csv")).unwrap());
    writeln!(
        file,
        "row_id,paper_id,variable,value_original,unit_original,value_canonical"
    )
    .unwrap();
    for n in 0..10_000 {
        let canonical = if n % 7 == 0 {
            String::new()
        } else {
            format!("{}", n % 347)
        };
        writeln!(
            file,
            "M{n},P{},variable{},{},mV,{}",
            n % 2000,
            n % 12,
            n % 347,
            canonical
        )
        .unwrap();
    }
    drop(file);
    located(root)
}
fn filter(column: &str, op: Op) -> Filter {
    Filter {
        column: column.into(),
        op,
    }
}

#[test]
fn restore_returns_same_page_and_filters() {
    let _guard = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let start = Instant::now();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram/project");
    let index = build(located(&root));
    let query = Query {
        artifact: "evidence".into(),
        filters: vec![filter("quote", Op::Contains("evidence".into()))],
        offset: 1,
        limit: 2,
        ..Default::default()
    };
    let original = index.query(&query).unwrap();
    assert!(original.total > query.offset);
    let home = tempfile::tempdir().unwrap();
    let id = index.located().manifest.research_id.clone();
    let mut prefs = SessionPrefs::default();
    let mut state = ViewState {
        tab: "evidence".into(),
        ..Default::default()
    };
    state.tabs.insert(
        "evidence".into(),
        TabView {
            offset: query.offset,
            filters: query.filters.clone(),
            selection: Some("E2".into()),
            node: None,
        },
    );
    prefs.views.insert(id.clone(), state);
    save(home.path(), &prefs).unwrap();
    let restored = load(home.path());
    let state = &restored.views[&id];
    assert_eq!(state.tab, "evidence");
    let tab = &state.tabs[&state.tab];
    assert_eq!(tab.selection.as_deref(), Some("E2"));
    let recovered = index
        .query(&Query {
            filters: tab.filters.clone(),
            offset: tab.offset,
            ..query
        })
        .unwrap();
    assert_eq!(original.rows, recovered.rows);
    assert_eq!(original.total, recovered.total);
    fs::write(home.path().join("session.json"), "{broken").unwrap();
    assert_eq!(load(home.path()), SessionPrefs::default());
    for n in 0..10 {
        remember(
            &mut prefs,
            ResearchRef {
                root: n.to_string().into(),
                store: None,
            },
        );
    }
    assert_eq!(prefs.recent.len(), 8);
    assert_eq!(
        prefs
            .recent
            .iter()
            .map(|r| r.root.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["9", "8", "7", "6", "5", "4", "3", "2"]
    );
    println!(
        "restore rows={} total={} elapsed_ms={:.3}",
        recovered.rows.len(),
        recovered.total,
        start.elapsed().as_secs_f64() * 1000.0
    );
}

#[test]
fn indexing_does_not_block_caller() {
    let _guard = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let l = synthetic(dir.path());
    let (tx, rx) = mpsc::channel();
    let start = Instant::now();
    let mut indexer = Indexer::start(
        l,
        Limits::default(),
        Box::new(move |event| {
            let _ = tx.send(event);
        }),
    );
    let start_ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(indexer.latest().is_none());
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut iterations = 0usize;
    let mut max_loop = Duration::ZERO;
    loop {
        let begin = Instant::now();
        let event = rx.recv_timeout(Duration::from_millis(5));
        max_loop = max_loop.max(begin.elapsed());
        iterations += 1;
        match event {
            Ok(IndexEvent::Ready { .. }) => break,
            Ok(IndexEvent::Failed(e)) => panic!("indexing failed: {e:?}"),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("indexer disconnected"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "indexing timed out");
    }
    let index = indexer.latest().unwrap();
    assert_eq!(index.len("evidence"), 40_000);
    assert_eq!(index.len("measurements"), 10_000);
    indexer.stop();
    println!(
        "nonblocking start_ms={start_ms:.3} loops={iterations} max_loop_ms={:.3} elapsed_ms={:.3}",
        max_loop.as_secs_f64() * 1000.0,
        start.elapsed().as_secs_f64() * 1000.0
    );
    assert!(start_ms < 200.0);
    assert!(iterations > 0);
    assert!(max_loop <= Duration::from_millis(250));
}

#[test]
fn perf_50k_records() {
    let _guard = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let l = synthetic(dir.path());
    let baseline = CURRENT.load(Ordering::SeqCst);
    PEAK.store(baseline, Ordering::SeqCst);
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..10 {
        let start = Instant::now();
        let index = build(l.clone());
        let page = index
            .query(&Query {
                artifact: "evidence".into(),
                ..Default::default()
            })
            .unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            (
                index.len("evidence"),
                index.len("measurements"),
                page.rows.len(),
                page.total
            ),
            (40_000, 10_000, 50, 40_000)
        );
        last = Some(index);
        if times.len() != 10 {
            last = None;
        }
    }
    let index = last.unwrap();
    let mut queries = Vec::new();
    for n in 0..20 {
        let (artifact, filters) = match n % 6 {
            0 => (
                "evidence",
                vec![filter("paper_id", Op::Eq(format!("P{}", n * 19)))],
            ),
            1 => (
                "evidence",
                vec![filter("quote", Op::Contains(format!("group {n}")))],
            ),
            2 => (
                "measurements",
                vec![
                    filter(
                        "variable",
                        Op::In(vec![
                            format!("variable{}", n % 12),
                            format!("variable{}", (n + 1) % 12),
                        ]),
                    ),
                    filter("paper_id", Op::Contains(format!("P{}", n % 10))),
                ],
            ),
            3 => (
                "measurements",
                vec![
                    filter("value_canonical", Op::NotEmpty),
                    filter("paper_id", Op::Contains(format!("P{}", n % 10))),
                ],
            ),
            4 => (
                "measurements",
                vec![
                    filter("value_canonical", Op::Empty),
                    filter("paper_id", Op::Contains(format!("P{}", n % 10))),
                ],
            ),
            _ => (
                "measurements",
                vec![
                    filter("value_canonical", Op::NotEmpty),
                    filter("variable", Op::Eq(format!("variable{}", n % 12))),
                    filter("paper_id", Op::Contains(format!("P{}", n % 10))),
                ],
            ),
        };
        queries.push(Query {
            artifact: artifact.into(),
            filters,
            sort: if n % 4 == 0 {
                Some(Sort {
                    column: "paper_id".into(),
                    descending: n % 8 == 0,
                })
            } else {
                None
            },
            ..Default::default()
        });
    }
    let mut filter_times = Vec::new();
    for query in &queries {
        let start = Instant::now();
        let page = index.query(query).unwrap();
        filter_times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(page.total > 0);
    }
    let peak_growth = PEAK.load(Ordering::SeqCst).saturating_sub(baseline);
    write_evidence(dir.path(), true);
    let refresh_baseline = CURRENT.load(Ordering::SeqCst);
    PEAK.store(refresh_baseline, Ordering::SeqCst);
    let refresh_start = Instant::now();
    let (refreshed, report) = index.refresh().unwrap();
    let refresh_ms = refresh_start.elapsed().as_secs_f64() * 1000.0;
    let refresh_growth = PEAK.load(Ordering::SeqCst).saturating_sub(refresh_baseline);
    assert_eq!(report.reparsed, vec!["evidence"]);
    assert_eq!(report.reused, vec!["measurements"]);
    assert_eq!(refreshed.len("evidence"), 40_000);
    let screen_p95 = p95(&times);
    let filter_p95 = p95(&filter_times);
    let filter_max = filter_times.iter().copied().fold(0.0_f64, f64::max);
    println!("PERF screen_p95_ms={screen_p95:.3} filter_max_ms={filter_max:.3} filter_p95_ms={filter_p95:.3} peak_growth_bytes={peak_growth} refresh_ms={refresh_ms:.3} refresh_peak_extra_bytes={refresh_growth}");
    check_limit("first screen p95 ms", screen_p95, 2000.0).unwrap();
    for (n, ms) in filter_times.iter().enumerate() {
        check_limit(&format!("filter {n} ms"), *ms, 300.0).unwrap();
    }
    check_limit(
        "build and filters peak bytes",
        peak_growth as f64,
        (512 * 1024 * 1024) as f64,
    )
    .unwrap();
    check_limit(
        "refresh extra peak bytes",
        refresh_growth as f64,
        (512 * 1024 * 1024) as f64,
    )
    .unwrap();
}

#[test]
fn limit_check_helper_fires() {
    let _guard = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    assert!(check_limit("x", 3.0, 2.0).is_err());
    assert!(check_limit("x", 1.0, 2.0).is_ok());
    println!("limit_helper rejected=3.0 accepted=1.0 limit=2.0");
}

#[test]
fn build_matches_read_rows_for_fixtures() {
    let _guard = MEASURE.lock().unwrap_or_else(|e| e.into_inner());
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research");
    for fixture in ["rram/project", "rawonly"] {
        let l = located(&fixtures.join(fixture));
        let limits = Limits::default();
        let index = build(l.clone());
        for artifact in &l.manifest.artifacts {
            if !matches!(artifact.format, Format::Csv | Format::Jsonl) {
                continue;
            }
            assert!(
                matches!(index.status(&artifact.id), ArtifactStatus::Ready(_)),
                "{fixture}: {}: {:?}",
                artifact.id,
                index.status(&artifact.id)
            );
            let mut cursor = None;
            let mut row_index = 0u32;
            loop {
                let page = axiom::research_workspace::read::read_rows(
                    &l,
                    &artifact.id,
                    cursor.as_ref(),
                    limits.max_page_rows,
                    &limits,
                )
                .unwrap();
                for row in page.rows {
                    let indexed = index.row(&artifact.id, row_index).unwrap();
                    assert_eq!(
                        indexed.number(),
                        row.number,
                        "{fixture}: {} row {row_index}",
                        artifact.id
                    );
                    for (column, value) in &row.fields {
                        assert_eq!(
                            indexed.get(column),
                            value,
                            "{fixture}: {} row {row_index} column {column}",
                            artifact.id
                        );
                    }
                    row_index += 1;
                }
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            assert_eq!(
                index.len(&artifact.id),
                row_index as usize,
                "{fixture}: {}",
                artifact.id
            );
        }
    }
}
