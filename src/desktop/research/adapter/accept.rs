use super::*;
use crate::desktop::data::{Cursor, Delta, Source};

#[test]
fn feed_source_patches_appends_and_reports_unchanged() {
    let shared = SharedRows::<String>::default();
    let mut source = shared.source();
    shared.replace(vec!["one".into()]);
    let (delta, cursor) = source.poll(Cursor(0)).unwrap();
    let mut rows = match delta {
        Delta::Replace(rows) => rows,
        _ => panic!("expected initial replace"),
    };
    shared.append(vec!["two".into()]);
    let (delta, cursor) = source.poll(cursor).unwrap();
    match delta {
        Delta::Patch(patch) => patch(&mut rows),
        _ => panic!("expected append patch"),
    }
    assert_eq!(rows, ["one", "two"]);
    assert!(matches!(source.poll(cursor).unwrap().0, Delta::Unchanged));
}

#[test]
fn worker_executes_engine_on_another_thread() {
    let caller = std::thread::current().id();
    let (tx, rx) = std::sync::mpsc::channel();
    let adapter = ResearchAdapter::new(ResearchSeams::default(), None);
    adapter.send(Command::Probe(Box::new(move || {
        tx.send(std::thread::current().id()).unwrap();
    })));
    assert_ne!(
        caller,
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap()
    );
}

#[test]
fn failed_feed_does_not_affect_another_feed() {
    let a = SharedRows::<String>::default();
    let b = SharedRows::<String>::default();
    a.fail("missing", "store missing");
    b.replace(vec!["available".into()]);
    assert!(a.source().poll(Cursor(0)).is_err());
    assert!(matches!(
        b.source().poll(Cursor(0)).unwrap().0,
        Delta::Replace(_)
    ));
}

#[test]
fn fixture_actions_leave_source_tree_unchanged() {
    use crate::desktop::ux::seams::research::{
        Correction, Filter, FilterOp, ResearchPort, Tab, TabView,
    };
    use std::{collections::BTreeMap, fs, path::Path};
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    fn tree(root: &Path) -> BTreeMap<String, String> {
        fn visit(root: &Path, dir: &Path, result: &mut BTreeMap<String, String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let key = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if entry.file_type().unwrap().is_dir() {
                    result.insert(key, "directory".into());
                    visit(root, &path, result);
                } else {
                    result.insert(key, engine::read::hash_file(&path).unwrap());
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rawonly"),
        &root,
    );
    let before = tree(&root);
    let seams = ResearchSeams {
        research_home: Some(temp.path().join("home")),
        ..Default::default()
    };
    let adapter = ResearchAdapter::new(seams, None);
    let check = || {
        adapter.flush();
        assert_eq!(tree(&root), before);
    };
    adapter.open(root.clone(), None);
    check();
    assert!(matches!(adapter.load_state(), dto::LoadState::Ready));
    adapter.set_view(
        Tab::Dataset,
        TabView {
            node: Some("raw".into()),
            ..Default::default()
        },
    );
    check();
    assert!(!adapter.dataset().unwrap().rows.is_empty());
    adapter.set_filters(
        Tab::Dataset,
        vec![Filter {
            column: "variable".into(),
            op: FilterOp::Eq("voltage".into()),
        }],
    );
    check();
    assert!(!adapter.dataset().unwrap().rows.is_empty());
    adapter.sample("total".into(), 2, 42);
    check();
    assert!(adapter.sources().is_err());
    assert!(!adapter.dataset().unwrap().rows.is_empty());
    adapter.set_view(
        Tab::Evidence,
        TabView {
            node: Some("raw".into()),
            ..Default::default()
        },
    );
    check();
    adapter.locate_page("RE1".into());
    check();
    assert!(adapter
        .evidence()
        .unwrap()
        .items
        .iter()
        .any(|item| item.id == "RE1"));
    adapter.render_page("RE1".into());
    check();
    assert_eq!(
        adapter
            .evidence()
            .unwrap()
            .items
            .iter()
            .find(|i| i.id == "RE1")
            .unwrap()
            .notice
            .as_ref()
            .unwrap()
            .kind,
        "renderer_missing"
    );
    adapter.locate_page("missing-id".into());
    check();
    assert!(adapter
        .evidence()
        .unwrap()
        .items
        .iter()
        .any(|i| i.id == "missing-id" && i.notice.is_some()));
    adapter.correct(Correction {
        target: "evidence:RE1".into(),
        field: "raw_file".into(),
        after: "runs/device_A.csv".into(),
        who: "tester".into(),
        why: "review".into(),
    });
    check();
    assert!(
        !engine::annotations::AnnotationStore::open(&temp.path().join("home"), "rawonly")
            .unwrap()
            .corrections(None)
            .is_empty()
    );
    adapter.export_csv(temp.path().join("out"));
    check();
    assert!(temp.path().join("out/dataset-raw.csv").exists());
    adapter.export_bundle(temp.path().join("out"));
    check();
    assert_eq!(adapter.exports().unwrap().results.len(), 2);
    adapter.export_snapshot(temp.path().join("out"));
    check();
    assert_eq!(adapter.exports().unwrap().results.len(), 3);
    adapter.refresh();
    check();
}

#[test]
fn missing_dataset_does_not_disable_map_or_exports() {
    use crate::desktop::ux::seams::research::{ResearchPort, Tab, TabView};
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &dest);
            } else {
                std::fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rawonly"),
        &root,
    );
    std::fs::remove_file(root.join("data/raw_measurements.csv")).unwrap();
    let adapter = ResearchAdapter::new(
        ResearchSeams {
            research_home: Some(temp.path().join("home")),
            ..Default::default()
        },
        None,
    );
    adapter.open(root, None);
    adapter.set_view(
        Tab::Dataset,
        TabView {
            node: Some("raw".into()),
            ..Default::default()
        },
    );
    adapter.flush();
    assert!(adapter.dataset().is_err());
    assert_eq!(adapter.map().unwrap().title, "Raw only");
    assert!(adapter.exports().is_ok());
}

#[test]
fn sampling_and_fake_renderer_use_worker_and_preserve_fixture() {
    use crate::desktop::ux::seams::research::ResearchPort;
    use engine::seams::{FakeRenderer, PageImage, PageRenderer};
    use std::{
        collections::BTreeMap,
        fs,
        path::Path,
        sync::{Arc, Mutex},
    };
    struct Renderer {
        thread: Arc<Mutex<Option<std::thread::ThreadId>>>,
        fake: FakeRenderer,
    }
    impl PageRenderer for Renderer {
        fn render(
            &self,
            pdf: &Path,
            sha: &str,
            page: u32,
            scale: f32,
            out: &Path,
        ) -> crate::base::Res<PageImage> {
            *self.thread.lock().unwrap() = Some(std::thread::current().id());
            self.fake.render(pdf, sha, page, scale, out)
        }
    }
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    fn hashes(root: &Path) -> BTreeMap<std::path::PathBuf, String> {
        fn visit(root: &Path, dir: &Path, output: &mut BTreeMap<std::path::PathBuf, String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(root, &path, output);
                } else {
                    output.insert(
                        path.strip_prefix(root).unwrap().into(),
                        engine::read::hash_file(&path).unwrap(),
                    );
                }
            }
        }
        let mut output = BTreeMap::new();
        visit(root, root, &mut output);
        output
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram"),
        &root,
    );
    let before = hashes(&root);
    let recorded = Arc::new(Mutex::new(None));
    let seams = ResearchSeams {
        research_home: Some(temp.path().join("home")),
        renderer: Some(Arc::new(Renderer {
            thread: recorded.clone(),
            fake: FakeRenderer::new(),
        })),
        page_cache_bytes: 1024 * 1024,
        ..Default::default()
    };
    let adapter = ResearchAdapter::new(seams, None);
    adapter.open(root.join("project"), Some(root.join("store")));
    adapter.flush();
    assert!(adapter
        .exports()
        .unwrap()
        .notices
        .iter()
        .any(|n| n.message.contains("not in this version")));
    assert_eq!(hashes(&root), before);
    adapter.sample("total".into(), 2, 42);
    adapter.flush();
    assert_eq!(adapter.sources().unwrap().sample.len(), 2);
    let sample = adapter.sources().unwrap().sample;
    adapter.set_view(dto::Tab::Sources, dto::TabView::default());
    adapter.flush();
    assert_eq!(adapter.sources().unwrap().sample, sample);
    assert_eq!(hashes(&root), before);
    adapter.render_page("E1".into());
    adapter.flush();
    assert_ne!(*recorded.lock().unwrap(), Some(std::thread::current().id()));
    assert!(recorded.lock().unwrap().is_some());
    assert!(adapter
        .evidence()
        .unwrap()
        .items
        .iter()
        .any(|item| item.id == "E1" && item.image.is_some()));
    adapter.set_view(
        dto::Tab::Evidence,
        dto::TabView {
            offset: 10000,
            ..Default::default()
        },
    );
    adapter.flush();
    adapter.set_view(dto::Tab::Evidence, dto::TabView::default());
    adapter.flush();
    assert!(adapter
        .evidence()
        .unwrap()
        .items
        .iter()
        .any(|item| item.id == "E1" && item.image.is_some()));
    assert!(adapter
        .sources()
        .unwrap()
        .rows
        .iter()
        .any(|row| row.page == Some(1) && row.locate_state == "rendered"));
    assert_eq!(hashes(&root), before);
}

#[test]
fn adapter_feed_patches_export_results_and_snapshot_consumer_runs_on_worker() {
    use crate::desktop::ux::seams::research::ResearchPort;
    use engine::seams::{SnapshotConsumer, SnapshotRef};
    struct Consumer(mpsc::Sender<std::thread::ThreadId>);
    impl SnapshotConsumer for Consumer {
        fn name(&self) -> String {
            "thread probe".into()
        }
        fn consume(&self, _: &SnapshotRef) -> Res<()> {
            self.0.send(std::thread::current().id()).unwrap();
            Ok(())
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rawonly"),
        &root,
    );
    let (tx, rx) = mpsc::channel();
    let adapter = ResearchAdapter::new(
        ResearchSeams {
            research_home: Some(temp.path().join("home")),
            snapshot_consumer: Some(Arc::new(Consumer(tx))),
            ..Default::default()
        },
        None,
    );
    adapter.open(root, None);
    adapter.flush();
    let mut source = adapter.outputs.exports.source();
    let (delta, cursor) = source.poll(Cursor(0)).unwrap();
    let mut value = match delta {
        Delta::Replace(v) => v,
        _ => panic!("expected replace"),
    };
    adapter.export_snapshot(temp.path().join("out"));
    adapter.flush();
    assert_ne!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id()
    );
    let (delta, cursor) = source.poll(cursor).unwrap();
    match delta {
        Delta::Patch(patch) => patch(&mut value),
        _ => panic!("expected patch"),
    }
    assert_eq!(value.results.len(), 1);
    assert!(matches!(source.poll(cursor).unwrap().0, Delta::Unchanged));
}

fn copy_fixture(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_fixture(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn invalid_manifest_does_not_remove_exports_unavailability_notice() {
    use crate::desktop::ux::seams::research::ResearchPort;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rawonly"),
        &root,
    );
    std::fs::write(root.join("research-manifest.json"), "not json").unwrap();
    let adapter = ResearchAdapter::new(ResearchSeams::default(), None);
    adapter.open(root, None);
    adapter.flush();
    assert!(adapter.map().is_err());
    assert!(adapter.sources().is_err());
    assert!(adapter.evidence().is_err());
    assert!(adapter.dataset().is_err());
    assert!(adapter
        .exports()
        .unwrap()
        .notices
        .iter()
        .any(|n| n.message.contains("not in this version")));
}

#[test]
fn missing_and_corrupt_stores_only_affect_dependent_tabs() {
    use crate::desktop::ux::seams::research::ResearchPort;
    for fault in [
        "missing_store",
        "corrupt_store",
        "corrupt_inventory",
        "missing_measurements",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        copy_fixture(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram"),
            &root,
        );
        match fault {
            "missing_store" => std::fs::remove_dir_all(root.join("store")).unwrap(),
            "corrupt_store" => std::fs::write(root.join("store/units.tsv"), b"\xff").unwrap(),
            "corrupt_inventory" => {
                std::fs::write(root.join("project/data/literature_inventory.csv"), b"\xff").unwrap()
            }
            "missing_measurements" => {
                std::fs::remove_file(root.join("project/data/measurements.csv")).unwrap()
            }
            _ => unreachable!(),
        }
        let adapter = ResearchAdapter::new(ResearchSeams::default(), None);
        adapter.open(root.join("project"), Some(root.join("store")));
        adapter.flush();
        let map = adapter.map().expect("map stages remain available");
        if fault == "missing_store" || fault == "corrupt_store" {
            assert!(map
                .notices
                .iter()
                .any(|notice| notice.kind == "store_unavailable"));
        }
        if fault == "corrupt_inventory" {
            assert!(
                map.notices
                    .iter()
                    .any(|notice| notice.kind == "unavailable"),
                "{fault}: missing funnel notice"
            );
        }
        assert_eq!(
            adapter.sources().is_err(),
            fault == "corrupt_inventory",
            "{fault}: sources"
        );
        assert_eq!(
            adapter.evidence().is_err(),
            fault == "missing_measurements",
            "{fault}: evidence"
        );
        assert_eq!(
            adapter.dataset().is_err(),
            fault == "missing_measurements",
            "{fault}: dataset"
        );
        assert!(adapter.exports().is_ok(), "{fault}: exports");
    }
}

#[test]
fn feeds_report_independent_tab_results() {
    use crate::desktop::{components::panel::PanelState, ux::seams::research::ResearchPort};
    use std::{cell::RefCell, rc::Rc};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("source");
    copy_fixture(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram"),
        &root,
    );
    std::fs::write(root.join("project/data/literature_inventory.csv"), b"\xff").unwrap();
    let adapter = ResearchAdapter::new(ResearchSeams::default(), None);
    adapter.open(root.join("project"), Some(root.join("store")));
    adapter.flush();
    let result = Rc::new(RefCell::new(None));
    let saved = result.clone();
    gpui_kit::platform::headless().run(move |cx| {
        let feeds = adapter.feeds(cx);
        cx.spawn(async move |cx| {
            for _ in 0..100 {
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
                let states = [
                    feeds.map.snapshot().state,
                    feeds.sources.snapshot().state,
                    feeds.evidence.snapshot().state,
                    feeds.dataset.snapshot().state,
                    feeds.exports.snapshot().state,
                ];
                if states.iter().all(|s| *s != PanelState::Loading) {
                    *saved.borrow_mut() = Some(states);
                    break;
                }
            }
            let _ = cx.update(|cx| cx.quit());
        })
        .detach();
    });
    let states = result
        .borrow()
        .clone()
        .expect("feeds did not finish polling");
    assert_eq!(states[0], PanelState::Ready);
    assert!(matches!(
        states[1],
        PanelState::Failed { .. } | PanelState::Corrupt { .. }
    ));
    assert_eq!(states[2], PanelState::Ready);
    assert_eq!(states[3], PanelState::Ready);
    assert_eq!(states[4], PanelState::Ready);
}
