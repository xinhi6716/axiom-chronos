use super::adapter::dataset::{cell_text, load_page, row_text, validation_text};
use super::dataset::filter_value;
use crate::desktop::ux::seams::research::{Filter as DisplayFilter, FilterOp, TabView};
use crate::{
    desktop::i18n::Lang,
    research_workspace::{
        dataset::{Cell, Conversion, DataLayer, Validation},
        index::{Filter, Index, Op},
        manifest::{locate, LocateOptions},
        read::Limits,
    },
};
use std::prelude::v1::test;
use std::{path::Path, sync::Arc};

fn index(root: &Path, store: Option<&Path>) -> Index {
    let located = locate(
        root,
        &LocateOptions {
            store: store.map(Path::to_path_buf),
            orch_home: None,
        },
    )
    .unwrap();
    Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap()
}
fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/research")
        .join(name)
}
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}
#[test]
fn cells_and_validation_are_localized() {
    for lang in [Lang::En, Lang::ZhTw] {
        assert_eq!(cell_text(&Cell::Value("0".into()), lang), "0");
        assert_eq!(cell_text(&Cell::Value("abc".into()), lang), "abc");
        let empty = cell_text(&Cell::Empty { reason: None }, lang);
        assert_ne!(empty, "0");
        assert_eq!(empty, if lang == Lang::En { "empty" } else { "空值" });
        let reason = cell_text(
            &Cell::Empty {
                reason: Some("missing".into()),
            },
            lang,
        );
        assert!(reason.contains(&empty) && reason.contains("missing"));
        let hash = "abcdef1234567890".to_string();
        assert!(validation_text(
            &Validation::Valid {
                data_sha256: hash.clone()
            },
            lang
        )
        .contains("abcdef123456"));
        assert!(validation_text(
            &Validation::Invalid {
                errors: 3,
                data_sha256: hash.clone()
            },
            lang
        )
        .contains('3'));
        let stale = validation_text(
            &Validation::Stale {
                recorded: hash,
                current: "123456789abcffff".into(),
            },
            lang,
        );
        assert!(stale.contains("abcdef123456") && stale.contains("123456789abc"));
        assert_eq!(
            validation_text(&Validation::NotRun, lang),
            if lang == Lang::En {
                "Not run"
            } else {
                "尚未執行"
            }
        );
    }
}
#[test]
fn layers_filter_and_conversion() {
    let root = fixture("rram");
    let ix = index(&root.join("project"), Some(&root.join("store")));
    let (page, _, missing) = load_page(&ix, DataLayer::Extracted, &[], 0).unwrap();
    assert!(!page.rows.is_empty());
    assert!(!missing.is_empty());
    let display = super::adapter::dataset_display(&ix, &TabView::default()).unwrap();
    assert_eq!(display.total, page.total);
    assert!(display
        .columns
        .iter()
        .any(|column| column.key == "variable"));
    assert_eq!(display.rows[0].id, page.rows[0].record_id);
    assert!(display.rows[0].text_en.contains("Original"));
    assert!(!display.validation_en.is_empty());
    assert!(!display.missing.is_empty());
    let f = Filter {
        column: "condition_id".into(),
        op: Op::Contains("C2".into()),
    };
    let filtered = load_page(&ix, DataLayer::Extracted, &[f], 0).unwrap().0;
    assert!(filtered.total > 0 && filtered.total < page.total);
    let row = page
        .rows
        .iter()
        .find(|r| matches!(r.conversion, Conversion::NotConvertible { .. }))
        .unwrap();
    assert!(row_text(row, Lang::En).contains("not convertible"));
    assert!(row_text(row, Lang::ZhTw).contains("無法轉換"));
    let raw_in_rram = load_page(&ix, DataLayer::Raw, &[], 0).unwrap().0;
    assert!(!raw_in_rram.rows.is_empty());
    assert!(raw_in_rram
        .rows
        .iter()
        .all(|r| !page.rows.iter().any(|x| x.record_id == r.record_id)));
    let temp = std::env::temp_dir().join(format!("dataset-no-raw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    copy(&root, &temp);
    std::fs::remove_file(temp.join("project/data/raw_measurements.csv")).unwrap();
    let without_raw = index(&temp.join("project"), Some(&temp.join("store")));
    assert_eq!(
        load_page(&without_raw, DataLayer::Raw, &[], 0)
            .unwrap_err()
            .kind,
        "not_capable"
    );
    drop(without_raw);
    std::fs::remove_dir_all(temp).unwrap();
    let raw = fixture("rawonly");
    let ix2 = index(&raw, None);
    let raw_page = load_page(&ix2, DataLayer::Raw, &[], 0).unwrap().0;
    assert!(!raw_page.rows.is_empty());
    assert!(raw_page
        .rows
        .iter()
        .all(|r| !page.rows.iter().any(|x| x.record_id == r.record_id)));
}
#[test]
fn paging_after_two_hundred() {
    let temp = std::env::temp_dir().join(format!("dataset-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    copy(&fixture("rram"), &temp);
    let path = temp.join("project/data/measurements.csv");
    let input = std::fs::read_to_string(&path).unwrap();
    let mut lines = input.lines();
    let header = lines.next().unwrap();
    let row = lines.next().unwrap();
    let mut data = format!("{header}\n");
    for i in 0..421 {
        data.push_str(&format!("M{i},{}\n", row.split_once(',').unwrap().1));
    }
    std::fs::write(path, data).unwrap();
    let ix = index(&temp.join("project"), Some(&temp.join("store")));
    let page = load_page(&ix, DataLayer::Extracted, &[], 400).unwrap().0;
    assert_eq!((page.offset, page.total, page.rows.len()), (400, 421, 21));
    assert!(page.rows[0].record_id.ends_with("M400"));
    drop(ix);
    std::fs::remove_dir_all(temp).unwrap();
}
#[test]
fn view_layer_mapping_preserves_filters_and_offset() {
    let mut v = TabView {
        node: Some("raw".into()),
        filters: vec![DisplayFilter {
            column: "variable".into(),
            op: FilterOp::Contains("voltage".into()),
        }],
        offset: 234,
        ..Default::default()
    };
    assert_eq!(v.node.as_deref(), Some("raw"));
    assert_eq!(filter_value(&v, "variable"), "voltage");
    let old = v.clone();
    v.node = None;
    assert_eq!(v.node, None);
    v.node = Some("raw".into());
    assert_eq!(v, old);
}
