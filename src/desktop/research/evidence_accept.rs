use super::evidence::{degraded_text, locator_text};
use crate::desktop::{
    i18n::Lang,
    ux::seams::research::{EvidenceLocator, TabView},
};
use crate::research_workspace::{
    index::Index,
    manifest::{locate, LocateOptions},
    read::Limits,
};
use std::{path::Path, prelude::v1::test, sync::Arc};

fn fixture() -> (tempfile::TempDir, Index) {
    fn copy(from: &Path, to: &Path) {
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
    let root = temp.path().join("rram");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/research/rram"),
        &root,
    );
    let located = locate(
        &root.join("project"),
        &LocateOptions {
            store: Some(root.join("store")),
            orch_home: None,
        },
    )
    .unwrap();
    let index = Index::build(Arc::new(located), Limits::default(), &|_, _| {}).unwrap();
    (temp, index)
}
#[test]
fn locators_and_degraded_states_are_localized() {
    let mut r = EvidenceLocator::PdfPage {
        physical_page: 1,
        printed_page: Some("11".into()),
        highlight: true,
    };
    assert!(locator_text(&r, Lang::En).contains("Page 1 (printed 11)"));
    assert!(locator_text(&r, Lang::En)
        .contains("Region recorded; precise highlighting comes in a later version"));
    assert!(locator_text(&r, Lang::ZhTw).contains("第 1 頁（印刷頁碼 11）"));
    assert!(locator_text(&r, Lang::ZhTw).contains("已記錄區域；精確標示將於未來版本提供"));
    r = EvidenceLocator::PdfPage {
        physical_page: 1,
        printed_page: None,
        highlight: false,
    };
    assert!(locator_text(&r, Lang::En).contains("Page 1"));
    assert!(locator_text(&r, Lang::En).contains("No region recorded; showing the whole page"));
    assert!(locator_text(&r, Lang::ZhTw).contains("第 1 頁 · 未記錄區域；顯示整頁"));
    r = EvidenceLocator::RawField {
        file: "raw.csv".into(),
        row: 3,
        column: "value".into(),
    };
    assert_eq!(locator_text(&r, Lang::En), "raw.csv, row 3, column value");
    assert_eq!(locator_text(&r, Lang::ZhTw), "raw.csv，第 3 列，欄位 value");
    for (kind, expected_en, expected_zh) in [
        (
            "renderer_missing",
            "PDF renderer is unavailable",
            "轉換工具無法使用",
        ),
        ("corrupt_pdf", "The PDF is corrupt", "PDF 已損毀"),
        ("hash_mismatch", "The PDF hash does not match", "雜湊值不符"),
        ("render_timeout", "Page rendering timed out", "逾時"),
        (
            "page_out_of_range",
            "The page is out of range",
            "頁碼超出範圍",
        ),
        (
            "429",
            "The page or evidence could not be loaded",
            "無法載入",
        ),
        (
            "missing_artifact",
            "An evidence artifact is missing or corrupt",
            "遺失或損毀",
        ),
        (
            "corrupt_artifact",
            "An evidence artifact is missing or corrupt",
            "遺失或損毀",
        ),
    ] {
        assert!(degraded_text(kind, Lang::En).contains(expected_en));
        assert!(degraded_text(kind, Lang::ZhTw).contains(expected_zh));
    }
}
#[test]
fn fixture_group_preserves_conditions_and_contradictions() {
    let (_copy, ix) = fixture();
    let group = super::adapter::evidence_display(&ix, &TabView::default(), Some("HfO2")).unwrap();
    assert!(group.conditions.iter().any(|c| c.id == "C1"));
    assert!(group.conditions.iter().any(|c| c.id == "C2"));
    assert!(group.conditions.iter().all(|c| !c.rows.is_empty()));
    assert!(group
        .contradictions
        .iter()
        .any(|c| c.a != c.b && c.text.contains("C1")));
    assert!(group
        .conditions
        .iter()
        .flat_map(|c| &c.rows)
        .flat_map(|r| &r.evidence)
        .any(|id| group.items.iter().any(|item| &item.id == id)));
}
