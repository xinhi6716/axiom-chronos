use super::interjection::*;
use crate::desktop::ux::seams::{Draft, FakeInterjections, Receipt, ReceiptStage};
#[test]
fn receipt_order_and_finality() {
    let r = |stage| Receipt {
        client_id: "id".into(),
        session: "s".into(),
        stage,
        at_ms: 1,
    };
    let sent = r(ReceiptStage::Sent);
    let received = r(ReceiptStage::Received);
    let applied = r(ReceiptStage::Applied {
        first_action: "x".into(),
    });
    let rejected = r(ReceiptStage::Rejected { reason: "x".into() });
    assert_eq!(
        (
            rank(&sent.stage),
            rank(&received.stage),
            rank(&applied.stage),
            rank(&rejected.stage)
        ),
        (0, 1, 2, 2)
    );
    assert_eq!(merge(Some(&received), &sent), received);
    assert_eq!(merge(Some(&applied), &rejected), applied);
    assert_eq!(merge(None, &sent), sent);
}
#[test]
fn pause_failure_prevents_send_and_retry_is_deduplicated() {
    let seam = FakeInterjections::new();
    seam.set_pause_failure(true);
    let d = Draft {
        session: "s".into(),
        client_id: new_client_id("s", 1, 2),
        text: "hello".into(),
        pause_first: true,
        replan: true,
    };
    assert!(matches!(send(&seam, &d), Err(SendError::PauseFailed(_))));
    assert!(seam.sent().is_empty());
    seam.set_pause_failure(false);
    let a = send(&seam, &d).unwrap();
    let b = send(&seam, &d).unwrap();
    assert_eq!(a, b);
    assert_eq!(seam.sent(), vec![d]);
}
