use super::sources::funnel_nodes;
use crate::{
    desktop::i18n::Lang,
    research_workspace::funnel::{Funnel, Node},
};

#[test]
fn nodes_report_all_counts_and_reason_nodes_in_both_languages() {
    let f = Funnel {
        total: 9,
        deduplicated: 8,
        duplicates: 1,
        screened_in: 2,
        screened_out: 2,
        out_reasons: vec![("scope".into(), 2)],
        pending: 1,
        unprocessed: 1,
        read: 2,
        read_failed: 2,
        fail_reasons: vec![("broken".into(), 2)],
    };
    for lang in [Lang::En, Lang::ZhTw] {
        let nodes = funnel_nodes(&f, lang);
        for (node, count) in [
            (Node::Total, 9),
            (Node::Duplicates, 1),
            (
                Node::ScreenedOut {
                    reason: Some("scope".into()),
                },
                2,
            ),
            (
                Node::ReadFailed {
                    reason: Some("broken".into()),
                },
                2,
            ),
            (Node::Unprocessed, 1),
        ] {
            assert!(nodes
                .iter()
                .any(|(n, label)| *n == node && label.contains(&count.to_string())));
        }
        assert_eq!(nodes.len(), 11);
    }
}
