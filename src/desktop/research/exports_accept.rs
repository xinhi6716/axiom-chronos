use super::exports::Action as ExportAction;
use super::exports::*;
use super::*;
use crate::research_workspace::{
    dataset::{DataLayer, DatasetQuery},
    export::Delivery,
    index::{Filter, Index, Op},
    manifest::{locate, LocateOptions},
    read::Limits,
};
use std::prelude::v1::test;

fn fixture() -> Arc<Index> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram");
    let located = Arc::new(
        locate(
            &root.join("project"),
            &LocateOptions {
                store: Some(root.join("store")),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    Arc::new(Index::build(located, Limits::default(), &|_, _| {}).unwrap())
}
fn q() -> DatasetQuery {
    DatasetQuery {
        layer: DataLayer::Extracted,
        filters: vec![],
        sort: None,
        offset: 7,
        limit: 1,
    }
}
#[test]
fn deliveries_and_layers_are_localized() {
    for lang in [Lang::En, Lang::ZhTw] {
        let delivered = delivery_text(&Delivery::Delivered { to: "sink".into() }, "paper", lang);
        assert!(delivered.contains("sink"));
        let failed = delivery_text(
            &Delivery::Failed {
                kind: "429".into(),
                message: "later".into(),
            },
            "vault",
            lang,
        );
        assert!(failed.contains("429: later"));
        assert_ne!(
            delivery_text(&Delivery::Unwired, "paper", lang),
            delivery_text(&Delivery::Unwired, "vault", lang)
        );
    }
    assert_eq!(
        delivery_text(&Delivery::Unwired, "paper", Lang::En),
        "Paper writing is not connected yet, so the snapshot stays here for you to share"
    );
    assert_eq!(
        delivery_text(&Delivery::Unwired, "vault", Lang::En),
        "Vault is not connected yet, so the snapshot stays here for you to share"
    );
    let mut view = TabView::default();
    assert_eq!(layer_of(&view), DataLayer::Extracted);
    view.node = Some("raw".into());
    assert_eq!(layer_of(&view), DataLayer::Raw);
    view.node = Some("unknown".into());
    assert_eq!(layer_of(&view), DataLayer::Extracted);
}
#[test]
fn query_uses_dataset_view_filter_and_layer() {
    let view = TabView {
        node: Some("raw".into()),
        filters: vec![Filter {
            column: "row_id".into(),
            op: Op::Eq("R1".into()),
        }],
        offset: 99,
        ..Default::default()
    };
    let q = query(&view);
    assert_eq!(q.layer, DataLayer::Raw);
    assert_eq!(q.filters, view.filters);
    assert_eq!(q.sort, None);
    assert_eq!((q.offset, q.limit), (0, 500));
}
#[test]
fn evidence_ids_follow_filter_and_are_sorted() {
    let ix = fixture();
    let mut query = q();
    query.filters = vec![Filter {
        column: "row_id".into(),
        op: Op::Eq("M1".into()),
    }];
    assert_eq!(collect_evidence_ids(&ix, &query).unwrap(), vec!["E1"]);
    query.filters.clear();
    let ids = collect_evidence_ids(&ix, &query).unwrap();
    assert_eq!(ids.len(), 60);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    query.filters = vec![Filter {
        column: "row_id".into(),
        op: Op::Eq("missing".into()),
    }];
    assert!(collect_evidence_ids(&ix, &query).unwrap().is_empty());
}
#[test]
fn exports_create_outputs_and_reject_source_destination() {
    struct TempDir(PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let ix = fixture();
    let temp = TempDir(std::env::temp_dir().join(format!("exports-unit-{}", std::process::id())));
    std::fs::create_dir_all(&temp.0).unwrap();
    let dest = temp.0.join("exports");
    let seams = ResearchSeams {
        research_home: Some(temp.0.join("home")),
        ..Default::default()
    };
    let clock = crate::seams::FakeClock::new(0);
    let query = q();
    for action in [
        ExportAction::Csv,
        ExportAction::Bundle,
        ExportAction::Snapshot,
    ] {
        let outcome =
            perform_action(action, &ix, ix.located(), &query, &seams, &clock, &dest).unwrap();
        let path = match outcome {
            Outcome::Export(exported) => exported.path,
            Outcome::Snapshot(exported, _) => exported.path.join("snapshot.json"),
            _ => panic!("expected output"),
        };
        assert!(path.is_file());
        let forbidden = ix.located().roots()[0].join("should-not-exist-export-test");
        let failure = match perform_action(
            action,
            &ix,
            ix.located(),
            &query,
            &seams,
            &clock,
            &forbidden,
        ) {
            Err(failure) => failure,
            Ok(_) => panic!("source destination accepted"),
        };
        assert_eq!(failure.kind, "dest_in_source");
        assert!(
            failure.message.contains(" is inside "),
            "{}",
            failure.message
        );
        assert!(!forbidden.exists());
    }
}

#[test]
fn evidence_deduplicates_across_pages() {
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let temp = std::env::temp_dir().join(format!("exports-pages-{}", std::process::id()));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram");
    copy_tree(&source, &temp);
    let path = temp.join("project/data/measurements.csv");
    let input = std::fs::read_to_string(&path).unwrap();
    let mut lines = input.lines();
    let header = lines.next().unwrap();
    let row = lines.next().unwrap();
    let mut data = format!("{header}\n");
    for i in 0..501 {
        data.push_str(&format!("M{i},{}\n", row.split_once(',').unwrap().1));
    }
    std::fs::write(path, data).unwrap();
    let located = Arc::new(
        locate(
            &temp.join("project"),
            &LocateOptions {
                store: Some(temp.join("store")),
                orch_home: None,
            },
        )
        .unwrap(),
    );
    let index = Index::build(located, Limits::default(), &|_, _| {}).unwrap();
    assert_eq!(collect_evidence_ids(&index, &q()).unwrap(), vec!["E1"]);
    drop(index);
    std::fs::remove_dir_all(temp).unwrap();
}
