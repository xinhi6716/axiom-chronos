use axiom::seams::{
    sha256_hex, ApprovalRequest, Clock, Decision, DenyAll, FakeApproval, FakeClock, PersonApproval,
    SystemClock, TtyApproval,
};
use std::process::{Command, Stdio};

fn request(subject: &str) -> ApprovalRequest {
    ApprovalRequest {
        subject: subject.into(),
        action: "run".into(),
        summary: "summary".into(),
        digest: "digest".into(),
    }
}

#[test]
fn fake_approvals_record_in_order_and_exhaust_to_denied() {
    let approval = FakeApproval::scripted(vec![Decision::Approved, Decision::Denied]);
    let requests = [
        request("workshop:first"),
        request("routine:second"),
        request("compute:third"),
    ];
    assert_eq!(approval.decide(&requests[0]), Decision::Approved);
    assert_eq!(approval.decide(&requests[1]), Decision::Denied);
    assert_eq!(approval.decide(&requests[2]), Decision::Denied);
    assert_eq!(approval.asked(), requests);

    let approving = FakeApproval::approving();
    let denying = FakeApproval::denying();
    for item in &requests {
        assert_eq!(approving.decide(item), Decision::Approved);
        assert_eq!(denying.decide(item), Decision::Denied);
    }
    assert_eq!(approving.asked(), requests);
    assert_eq!(denying.asked(), requests);
    assert_eq!(DenyAll.decide(&requests[0]), Decision::Denied);
}

#[test]
fn tty_approval_denies_non_terminal_input() {
    if std::env::var_os("P5B_TTY_TEST_CHILD").is_some() {
        assert_eq!(
            TtyApproval.decide(&request("research:run")),
            Decision::Denied
        );
        return;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tty_approval_denies_non_terminal_input"])
        .env("P5B_TTY_TEST_CHILD", "1")
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn decision_and_request_serialize() {
    assert_eq!(
        serde_json::to_string(&Decision::Approved).unwrap(),
        "\"approved\""
    );
    assert_eq!(
        serde_json::to_string(&Decision::Denied).unwrap(),
        "\"denied\""
    );
    assert_eq!(
        serde_json::from_str::<Decision>("\"approved\"").unwrap(),
        Decision::Approved
    );
    assert_eq!(
        serde_json::from_str::<Decision>("\"denied\"").unwrap(),
        Decision::Denied
    );
    let input = request("research:run");
    assert_eq!(
        serde_json::from_str::<ApprovalRequest>(&serde_json::to_string(&input).unwrap()).unwrap(),
        input
    );
}

#[test]
fn clocks_and_hash() {
    let clock = FakeClock::new(42);
    assert_eq!(clock.now_ms(), 42);
    clock.set(-5);
    clock.advance(8);
    assert_eq!(clock.now_ms(), 3);
    assert!(SystemClock.now_ms() > 0);
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
