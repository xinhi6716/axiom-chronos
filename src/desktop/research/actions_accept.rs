use super::*;
use std::prelude::v1::test;

#[test]
fn resolve_manifest_goal_and_missing_session() {
    use super::adapter::actions::resolve_session;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let home = std::env::temp_dir().join(format!(
        "research-actions-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(home.join("goals")).unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram/project");
    let mut located = locate(&root, &LocateOptions::default()).unwrap();
    located.store = Some(home.join("study"));
    located.manifest.session = Some("explicit".into());
    assert_eq!(
        resolve_session(&located, Some(&home)).as_deref(),
        Some("explicit")
    );
    located.manifest.session = Some(String::new());
    assert_eq!(resolve_session(&located, Some(&home)), None);
    std::fs::write(
        home.join("goals/active.json"),
        r#"{"wish":"test","status":"active","project":"study"}"#,
    )
    .unwrap();
    assert_eq!(
        resolve_session(&located, Some(&home)).as_deref(),
        Some("active")
    );
    assert_eq!(resolve_session(&located, None), None);
    std::fs::remove_dir_all(home).unwrap();
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "research-actions-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn index(name: &str) -> Arc<Index> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name);
    let (root, store) = if name == "rram" {
        (base.join("project"), Some(base.join("store")))
    } else {
        (base, None)
    };
    let l = locate(
        &root,
        &LocateOptions {
            store,
            orch_home: None,
        },
    )
    .unwrap();
    Arc::new(Index::build(Arc::new(l), Limits::default(), &|_, _| {}).unwrap())
}
fn draft(
    target: &str,
    field: &str,
    after: &str,
) -> crate::research_workspace::annotations::CorrectionDraft {
    crate::research_workspace::annotations::CorrectionDraft {
        target: target.into(),
        field: field.into(),
        after: after.into(),
        who: "Ada".into(),
        why: "source review".into(),
    }
}
fn state(
    home: &Path,
    id: &str,
    bridge: Option<Arc<dyn Interjections>>,
    session: Option<&str>,
) -> adapter::actions::State {
    adapter::actions::State {
        store: crate::research_workspace::annotations::AnnotationStore::open(home, id).unwrap(),
        bridge,
        session: session.map(str::to_owned),
    }
}

#[test]
fn correction_reverification_and_exact_downstream_set() {
    use crate::research_workspace::annotations::{register_snapshot, Reverify, SnapshotEntry};
    let tmp = Temp::new();
    let ix = index("rram");
    for (id, artifact) in [
        ("snap-evidence", "evidence"),
        ("snap-splits", "splits"),
        ("other", "inventory"),
    ] {
        register_snapshot(
            &tmp.0,
            "rram",
            &SnapshotEntry {
                snapshot_id: id.into(),
                artifacts: vec![artifact.into()],
                sha256: "hash".into(),
            },
        )
        .unwrap();
    }
    let mut s = state(&tmp.0, "rram", None, None);
    s.correct(
        &ix,
        draft("evidence:E1", "sha256", "new"),
        &crate::seams::FakeClock::new(42),
    )
    .unwrap();
    let c = &s.store.corrections(None)[0];
    assert_eq!(
        (&*c.who, &*c.why, &*c.research_version),
        (
            "Ada",
            "source review",
            ix.located().manifest.research_version.as_str()
        )
    );
    assert_eq!(
        c.invalidates,
        vec![
            "measurements:M1",
            "snap-evidence",
            "snap-splits",
            "splits:1"
        ]
    );
    assert_ne!(c.reverify, Reverify::Pending);
    let ix = index("rawonly");
    let mut s = state(&tmp.0, "rawonly", None, None);
    for after in ["runs/device_A.csv", "runs/missing.csv"] {
        s.correct(
            &ix,
            draft("evidence:RE1", "raw_file", after),
            &crate::seams::FakeClock::new(1),
        )
        .unwrap();
    }
    let cs = s.store.corrections(None);
    assert_eq!(cs[0].reverify, Reverify::Passed);
    assert!(matches!(cs[1].reverify, Reverify::Failed { .. }));
}

#[test]
fn bridge_receipts_and_failed_retry_keep_id() {
    use crate::desktop::ux::seams::{
        Amendment, Draft, FakeInterjections, Receipt, ReceiptStage, RunningWork,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    struct FailOnce {
        failed: AtomicBool,
        fake: FakeInterjections,
        ids: Mutex<Vec<String>>,
    }
    impl Interjections for FailOnce {
        fn send(&self, d: &Draft) -> crate::base::Res<Receipt> {
            self.ids.lock().unwrap().push(d.client_id.clone());
            if !self.failed.swap(true, Ordering::SeqCst) {
                Err(Failure::new("offline", "offline"))
            } else {
                self.fake.send(d)
            }
        }
        fn receipt(&self, id: &str) -> Option<Receipt> {
            self.fake.receipt(id)
        }
        fn history(&self, id: &str) -> Vec<Amendment> {
            self.fake.history(id)
        }
        fn running_work(&self, id: &str) -> RunningWork {
            self.fake.running_work(id)
        }
        fn pause_batch(&self, id: &str) -> crate::base::Res<()> {
            self.fake.pause_batch(id)
        }
    }
    let tmp = Temp::new();
    let ix = index("rawonly");
    let bridge = Arc::new(FailOnce {
        failed: AtomicBool::new(false),
        fake: FakeInterjections::new(),
        ids: Mutex::new(vec![]),
    });
    let mut s = state(&tmp.0, "study", Some(bridge.clone()), Some("session"));
    s.correct(
        &ix,
        draft("evidence:RE1", "raw_file", "runs/device_A.csv"),
        &crate::seams::FakeClock::new(1),
    )
    .unwrap();
    let c = s.store.corrections(None).remove(0);
    assert!(c.draft.is_some());
    assert!(c.receipt.is_none());
    s.retry(&c.id).unwrap();
    let ids = bridge.ids.lock().unwrap().clone();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(
        s.store.corrections(None)[0].receipt.as_ref().unwrap().stage,
        "sent"
    );
    bridge.fake.advance(&ids[0], ReceiptStage::Received);
    s.poll().unwrap();
    assert_eq!(
        s.store.corrections(None)[0].receipt.as_ref().unwrap().stage,
        "received"
    );
    bridge.fake.advance(
        &ids[0],
        ReceiptStage::Applied {
            first_action: "review".into(),
        },
    );
    s.poll().unwrap();
    let receipt = s.store.corrections(None)[0].receipt.clone().unwrap();
    assert_eq!(receipt.stage, "applied");
    assert_eq!(receipt.detail.as_deref(), Some("review"));
    let mut no_session = state(&tmp.0, "other", Some(bridge.clone()), None);
    no_session
        .correct(
            &ix,
            draft("evidence:RE1", "raw_file", "runs/device_A.csv"),
            &crate::seams::FakeClock::new(1),
        )
        .unwrap();
    assert_eq!(bridge.ids.lock().unwrap().len(), 2);
}

#[test]
fn spot_check_persists_without_touching_sources() {
    use crate::research_workspace::{annotations::Verdict, read::hash_file};
    fn hashes(root: &Path, path: &Path, out: &mut std::collections::BTreeMap<PathBuf, String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                hashes(root, &path, out);
            } else if entry.file_type().unwrap().is_file() {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    hash_file(&path).unwrap(),
                );
            }
        }
    }
    let tmp = Temp::new();
    let ix = index("rawonly");
    let root = &ix.located().root;
    let mut before = std::collections::BTreeMap::new();
    hashes(root, root, &mut before);
    let mut s = state(&tmp.0, "rawonly", None, None);
    s.spot(
        &ix,
        "evidence:RE1",
        Some("node"),
        Verdict::Confirm,
        &crate::seams::FakeClock::new(42),
    )
    .unwrap();
    let spot = s.store.spot_checks(None).remove(0);
    assert_eq!(spot.who, adapter::actions::who());
    assert_eq!(
        spot.research_version,
        ix.located().manifest.research_version
    );
    assert_eq!(
        spot.target_sha256,
        hash_file(&root.join("data/evidence.csv")).unwrap()
    );
    let mut after = std::collections::BTreeMap::new();
    hashes(root, root, &mut after);
    assert_eq!(before, after);
}

#[test]
fn row_ids_are_stable_and_strings_are_localized() {
    let mut rows = vec![];
    assert_eq!(actions::row_index(&mut rows, "one", None), 0);
    assert_eq!(actions::row_index(&mut rows, "one", Some("node")), 1);
    assert_eq!(actions::row_index(&mut rows, "two", None), 2);
    assert_eq!(actions::row_index(&mut rows, "one", None), 0);
    for key in [
        "Confirm",
        "Dispute",
        "Not sent; your request is kept",
        "There is no active session for this research.",
        "Pending",
        "Passed",
        "Failed",
    ] {
        assert_eq!(actions::tr(Lang::En, key), key);
        assert_ne!(actions::tr(Lang::ZhTw, key), key);
    }
}
