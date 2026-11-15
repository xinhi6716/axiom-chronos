use super::sources::{funnel_nodes, row_text};
use crate::{
    desktop::i18n::Lang,
    desktop::ux::seams::research::{FunnelNode, SourceRow},
};

#[test]
fn nodes_report_all_counts_and_reason_nodes_in_both_languages() {
    let f = vec![
        FunnelNode {
            key: "total".into(),
            label: "Total".into(),
            count: 9,
        },
        FunnelNode {
            key: "deduplicated".into(),
            label: "Deduplicated".into(),
            count: 8,
        },
        FunnelNode {
            key: "screened_in".into(),
            label: "Screened in".into(),
            count: 2,
        },
        FunnelNode {
            key: "screened_out".into(),
            label: "Screened out".into(),
            count: 2,
        },
        FunnelNode {
            key: "pending".into(),
            label: "Pending".into(),
            count: 1,
        },
        FunnelNode {
            key: "read".into(),
            label: "Read".into(),
            count: 2,
        },
        FunnelNode {
            key: "read_failed".into(),
            label: "Read failed".into(),
            count: 2,
        },
        FunnelNode {
            key: "duplicates".into(),
            label: "Duplicates".into(),
            count: 1,
        },
        FunnelNode {
            key: "screened_out:scope".into(),
            label: "scope".into(),
            count: 2,
        },
        FunnelNode {
            key: "read_failed:broken".into(),
            label: "broken".into(),
            count: 2,
        },
        FunnelNode {
            key: "unprocessed".into(),
            label: "Unprocessed".into(),
            count: 1,
        },
    ];
    for lang in [Lang::En, Lang::ZhTw] {
        let nodes = funnel_nodes(&f, lang);
        for (node, count) in [
            ("total", 9),
            ("duplicates", 1),
            ("screened_out:scope", 2),
            ("read_failed:broken", 2),
            ("unprocessed", 1),
        ] {
            assert!(nodes
                .iter()
                .any(|(n, label)| n == node && label.contains(&count.to_string())));
        }
        assert_eq!(nodes.len(), 11);
    }
}

#[test]
fn source_row_reports_page_and_locate_state_from_dto() {
    let row = SourceRow {
        page: Some(3),
        locate_state: "rendered".into(),
        ..Default::default()
    };
    assert!(row_text(&row, Lang::En).contains("Page 3 · rendered"));
    assert!(row_text(&row, Lang::ZhTw).contains("頁 3 · 已呈現"));
}
