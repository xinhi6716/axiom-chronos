use axiom::{
    research_workspace::{
        annotations::{self, AnnotationStore, CorrectionDraft, Reverify, Verdict},
        dataset::{self, DataLayer, DatasetQuery},
        evidence::{self, Layer},
        export,
        funnel::{self, Node},
        index::{ArtifactStatus, Index, Query},
        manifest::{locate, LocateOptions},
        read::{hash_file, Limits},
        stages::{self, Retry, StageState},
    },
    seams::FakeClock,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

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
fn set_readonly(root: &Path, readonly: bool) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            set_readonly(&entry.path(), readonly);
        } else {
            let mut p = entry.metadata().unwrap().permissions();
            p.set_readonly(readonly);
            fs::set_permissions(entry.path(), p).unwrap();
        }
    }
}
// Include directories (including empty ones) as well as every file and its contents.
fn tree(root: &Path) -> BTreeMap<PathBuf, (bool, String)> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, (bool, String)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let is_dir = e.file_type().unwrap().is_dir();
            out.insert(
                p.strip_prefix(root).unwrap().into(),
                (
                    is_dir,
                    if is_dir {
                        String::new()
                    } else {
                        hash_file(&p).unwrap()
                    },
                ),
            );
            if is_dir {
                visit(root, &p, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}
struct ReadOnlyCopy(PathBuf);
impl Drop for ReadOnlyCopy {
    fn drop(&mut self) {
        set_readonly(&self.0, false);
    }
}
fn fixture(name: &str, dest: &Path) {
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/research")
            .join(name),
        dest,
    );
}
fn index(project: &Path, store: Option<PathBuf>) -> Index {
    let located = locate(
        project,
        &LocateOptions {
            store,
            orch_home: None,
        },
    )
    .unwrap();
    Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap()
}
fn query(layer: DataLayer) -> DatasetQuery {
    DatasetQuery {
        layer,
        filters: vec![],
        sort: None,
        offset: 0,
        limit: 3,
    }
}
fn walk_dataset(ix: &Index, layer: DataLayer, evidence_id: &str) {
    let mut q = query(layer);
    let first = dataset::page(ix, &q).unwrap();
    assert!(first.total > 0);
    let mut count = first.rows.len();
    while count < first.total {
        q.offset = count;
        let p = dataset::page(ix, &q).unwrap();
        assert_eq!(p.total, first.total);
        assert!(!p.rows.is_empty());
        count += p.rows.len();
    }
    assert_eq!(count, first.total);
    q.offset = 0;
    q.filters = vec![dataset::evidence_filter(evidence_id)];
    let filtered = dataset::page(ix, &q).unwrap();
    assert!(filtered.total > 0);
    assert!(filtered
        .rows
        .iter()
        .all(|r| r.evidence.iter().any(|id| id.contains(evidence_id))));
    dataset::missing_rates(ix, layer).unwrap();
}
fn walk_entities(ix: &Index, layer: Layer) {
    let (names, total) = evidence::entities(ix, layer.clone(), 0, usize::MAX).unwrap();
    assert_eq!(names.len(), total);
    assert!(total > 0);
    for name in names {
        assert_eq!(
            evidence::group(ix, &name, layer.clone()).unwrap().entity,
            name
        );
    }
}
fn walk_funnel(ix: &Index) {
    let f = funnel::funnel(ix).unwrap();
    for (node, count) in [
        (Node::Total, f.total),
        (Node::Deduplicated, f.deduplicated),
        (Node::Duplicates, f.duplicates),
        (Node::ScreenedIn, f.screened_in),
        (Node::ScreenedOut { reason: None }, f.screened_out),
        (Node::Pending, f.pending),
        (Node::Unprocessed, f.unprocessed),
        (Node::Read, f.read),
        (Node::ReadFailed { reason: None }, f.read_failed),
    ] {
        let (all, total) = funnel::rows(ix, &node, 0, usize::MAX).unwrap();
        assert_eq!((total, all.len()), (count, count));
        let mut paged = Vec::new();
        for offset in (0..count).step_by(3) {
            paged.extend(funnel::rows(ix, &node, offset, 3).unwrap().0);
        }
        assert_eq!(paged, all);
    }
    for (reason, count) in &f.out_reasons {
        let reason = if reason == "(none)" { "" } else { reason };
        assert_eq!(
            funnel::rows(
                ix,
                &Node::ScreenedOut {
                    reason: Some(reason.into())
                },
                0,
                usize::MAX
            )
            .unwrap()
            .1,
            *count
        );
    }
    for (reason, count) in &f.fail_reasons {
        assert_eq!(
            funnel::rows(
                ix,
                &Node::ReadFailed {
                    reason: Some(reason.clone())
                },
                0,
                usize::MAX
            )
            .unwrap()
            .1,
            *count
        );
    }
    funnel::sample(ix, &Node::Total, 3, 42).unwrap();
}
#[test]
fn read_only_acceptance_and_negative_control() {
    let tmp = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    for name in ["rram", "rawonly"] {
        let source = tmp.path().join(name);
        fixture(name, &source);
        set_readonly(&source, true);
        let _restore = ReadOnlyCopy(source.clone());
        let before = tree(&source);
        let (project, store) = if name == "rram" {
            (source.join("project"), Some(source.join("store")))
        } else {
            (source.clone(), None)
        };
        let ix = index(&project, store);
        let map = stages::research_map(&ix, 0);
        if name == "rram" {
            assert!(!map.stages.is_empty());
            walk_funnel(&ix);
        } else {
            assert_eq!(funnel::funnel(&ix).unwrap_err().kind, "not_capable");
        }
        let mut ids = Vec::new();
        for n in 0..ix.len("evidence") {
            let id = ix
                .row("evidence", n as u32)
                .unwrap()
                .get("evidence_id")
                .to_owned();
            evidence::resolve(&ix, &id, &Limits::default()).unwrap();
            ids.push(id);
        }
        assert!(!ids.is_empty());
        if name == "rram" {
            walk_dataset(&ix, DataLayer::Extracted, "E3");
            walk_dataset(&ix, DataLayer::Raw, "E101");
            walk_entities(&ix, Layer::Literature);
        } else {
            assert_eq!(
                dataset::page(&ix, &query(DataLayer::Extracted))
                    .unwrap_err()
                    .kind,
                "not_capable"
            );
            walk_dataset(&ix, DataLayer::Raw, "RE1");
        }
        walk_entities(&ix, Layer::Raw);
        let home = tmp.path().join(format!("{name}-home"));
        let out = tmp.path().join(format!("{name}-out"));
        let clock = FakeClock::new(42);
        let mut annotations =
            AnnotationStore::open(&home, &ix.located().manifest.research_id).unwrap();
        let target = format!("evidence:{}", ids[0]);
        annotations
            .spot_check(
                &ix,
                &target,
                Some("total"),
                Verdict::Confirm,
                "tester",
                &clock,
            )
            .unwrap();
        let field = if name == "rram" { "sha256" } else { "raw_file" };
        let after = if name == "rram" {
            "corrected"
        } else {
            "runs/device_A.csv"
        };
        let correction = annotations
            .correct(
                &ix,
                CorrectionDraft {
                    target,
                    field: field.into(),
                    after: after.into(),
                    who: "tester".into(),
                    why: "review".into(),
                },
                &clock,
            )
            .unwrap();
        let result = annotations::reverify(&ix, &correction);
        assert_eq!(result, Reverify::Passed);
        annotations.set_reverify(&correction.id, result).unwrap();
        assert_eq!(annotations.corrections(None)[0].reverify, Reverify::Passed);
        assert!(
            export::export_csv(&ix, &query(DataLayer::Raw), &out, "raw.csv")
                .unwrap()
                .rows
                > 0
        );
        assert_eq!(
            export::export_evidence_bundle(&ix, &ids, &out)
                .unwrap()
                .rows,
            ids.len()
        );
        let (_, snapshot_dir) = export::snapshot(&ix, &home, &out, &clock).unwrap();
        assert!(snapshot_dir.join("snapshot.json").is_file());
        assert_eq!(before, tree(&source), "source changed during {name} views");
        let path = project.join("data/evidence.csv");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        assert_ne!(
            before,
            tree(&source),
            "negative control must detect a source write"
        );
    }
}

#[test]
fn missing_corrupt_and_rate_limited() {
    let tmp = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
    let raw = tmp.path().join("raw");
    fixture("rawonly", &raw);
    fs::remove_file(raw.join("data/raw_measurements.csv")).unwrap();
    let ix = index(&raw, None);
    assert_eq!(
        dataset::page(&ix, &query(DataLayer::Raw)).unwrap_err().kind,
        "missing_artifact"
    );
    assert!(evidence::resolve(&ix, "RE1", &Limits::default()).is_ok());
    assert!(stages::research_map(&ix, 0).stages.is_empty());
    fs::write(raw.join("runs/device_B.jsonl"), b"{broken\n").unwrap();
    let ix = index(&raw, None);
    assert!(matches!(ix.status("run_b"), ArtifactStatus::Corrupt(_)));
    assert_eq!(
        ix.query(&Query {
            artifact: "run_b".into(),
            ..Default::default()
        })
        .unwrap_err()
        .kind,
        "unavailable"
    );
    assert_eq!(
        evidence::resolve(&ix, "RE2", &Limits::default())
            .unwrap_err()
            .kind,
        "corrupt_artifact"
    );
    assert!(evidence::resolve(&ix, "RE1", &Limits::default()).is_ok());
    let rram = tmp.path().join("rram");
    fixture("rram", &rram);
    let ix = index(&rram.join("project"), Some(rram.join("store")));
    let paused = stages::research_map(&ix, 0);
    assert_eq!(
        paused
            .stages
            .iter()
            .find(|s| s.id == "screen")
            .unwrap()
            .state,
        StageState::Paused {
            reason: "rate limited".into()
        }
    );
    // The fixture's explicit paused driver takes precedence over its 429 log.
    // Remove only that override in the copy to expose the existing unknown reset.
    fs::write(rram.join("project/state/progress/driver.json"), "{}").unwrap();
    let ix = index(&rram.join("project"), Some(rram.join("store")));
    let map = stages::research_map(&ix, 0);
    assert_eq!(
        map.status,
        stages::ResearchStatus::RateLimited {
            retry: Retry::Unknown
        }
    );
    assert_eq!(
        map.stages.iter().find(|s| s.id == "screen").unwrap().state,
        StageState::RateLimited {
            retry: Retry::Unknown
        }
    );
}

#[test]
fn prompt_size_report() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let total: usize = [
        "engine/agents/dispatcher.md",
        "engine/DISPATCH.md",
        "engine/skills/wish/SKILL.md",
    ]
    .iter()
    .map(|p| {
        fs::read_to_string(root.join(p))
            .unwrap()
            .replace("\r\n", "\n")
            .len()
    })
    .sum();
    println!("PROMPT_BYTES total={total} cap=20813");
    assert!(total <= 20813);
}
fn check_manifest_section(doc: &str) -> Result<(), String> {
    let start = doc
        .find("<!-- research workspace (phase 9) -->")
        .ok_or("missing start")?;
    let rest = &doc[start..];
    let end = rest
        .find("<!-- end research workspace (phase 9) -->")
        .ok_or("missing end")?;
    let section = &rest[..end];
    if section.contains("Filled by rw-gates") {
        return Err("placeholder".into());
    }
    for required in [
        "## Measured performance",
        "v2 follow-ups",
        "2026-11-13",
        "screen_p95_ms",
        "filter_max_ms",
        "filter_p95_ms",
        "peak_growth_bytes",
        "refresh_ms",
        "precise highlighting",
        "snapshot diff",
        "model and proposal detail",
        "paper (phase 8)",
        "memory (phase 6)",
        "interactive DAG",
        "collaboration sync",
    ] {
        if !section.contains(required) {
            return Err(format!("missing {required}"));
        }
    }
    Ok(())
}
#[test]
fn manifest_performance_and_followups() {
    let doc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/research-manifest.md"))
            .unwrap();
    check_manifest_section(&doc).unwrap();
    assert!(check_manifest_section("<!-- research workspace (phase 9) -->\n## Measured performance\nFilled by rw-gates.\n<!-- end research workspace (phase 9) -->").is_err());
}

// Snapshot all files in both roots: this is a superset of files opened by locate,
// index and map, so metadata and hashes are captured before even locate runs.
fn real_tree(root: &Path) -> BTreeMap<PathBuf, (u64, SystemTime, bool, String)> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, (u64, SystemTime, bool, String)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let kind = e.file_type().unwrap();
            assert!(
                !kind.is_symlink(),
                "cannot verify symlink {}",
                e.path().display()
            );
            if kind.is_dir() {
                walk(&e.path(), out);
            } else if kind.is_file() {
                let m = e.metadata().unwrap();
                out.insert(
                    e.path(),
                    (
                        m.len(),
                        m.modified().unwrap(),
                        m.permissions().readonly(),
                        hash_file(&e.path()).unwrap(),
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, &mut result);
    result
}
#[test]
#[ignore]
fn real_store_read_only() {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let Some(home) = home else {
        println!("SKIP real_store_read_only: home directory unavailable");
        return;
    };
    let store = home.join(".axiom/orchestrate/ml_hfo2_v2");
    let project = PathBuf::from(r"E:\codex\ML_HfO2_v2");
    if !store.is_dir() || !project.is_dir() {
        println!(
            "SKIP real_store_read_only: {} or {} absent",
            store.display(),
            project.display()
        );
        return;
    }
    let before = (real_tree(&project), real_tree(&store));
    let ix = index(&project, Some(store.clone()));
    // Refuse to silently omit an external named root from the before snapshot.
    for root in ix.located().roots() {
        let root = fs::canonicalize(root).unwrap();
        assert!(
            root.starts_with(fs::canonicalize(&project).unwrap())
                || root.starts_with(fs::canonicalize(&store).unwrap()),
            "external root outside captured trees: {}",
            root.display()
        );
    }
    stages::research_map(&ix, 0);
    assert_eq!(before, (real_tree(&project), real_tree(&store)));
}
