use axiom::modes::{
    receipts::{self, codex_part, Cost, Receipt, Usage},
    Mode,
};
use serde_json::{json, Value};
use std::fs;

fn fixture() -> Receipt {
    let codex = codex_part(
        "codex",
        "gpt-6.1-terra",
        &json!({"inputTokens": 1_000_000, "cachedInputTokens": 0, "outputTokens": 100_000}),
        8_000,
    );
    let review = codex_part(
        "review",
        "gpt-6.1-sol",
        &json!({"inputTokens": 100_000, "cachedInputTokens": 50_000, "outputTokens": 10_000}),
        2_000,
    );
    Receipt::new(
        "run-1",
        "session-1",
        Mode::Quick,
        "done",
        "start",
        "finish",
        12_345,
        vec![codex, review],
    )
}

#[test]
fn priced_receipt_and_render() {
    let receipt = fixture();
    assert_eq!(receipt.parts[0].cost, Cost::Usd(1.5));
    assert_eq!(receipt.parts[1].cost, Cost::Usd(0.205));
    assert_eq!(receipt.cost, Cost::Usd(1.5 + 0.205));
    assert_eq!(
        receipt.parts[0].usage,
        Some(Usage {
            input: 1_000_000,
            cached_input: 0,
            output: 100_000
        })
    );
    assert_eq!(receipt.parts[1].label, "review");
    assert_eq!(receipt.parts[1].wall_ms, 2_000);
    let text = receipts::render(&receipt);
    assert!(text.contains("mode quick"));
    assert!(text.contains("done"));
    assert!(text.contains("12.3 s"));
    assert!(text.contains("$1.7050"));
    assert!(text.lines().count() > 1);
}

#[test]
fn unknown_costs_and_serialization() {
    let priced = fixture().parts[0].clone();
    let unknown_model = codex_part(
        "review",
        "no-such-model",
        &json!({"inputTokens": 1, "outputTokens": 1}),
        1,
    );
    assert_eq!(unknown_model.cost, Cost::Unknown);
    assert_eq!(
        unknown_model.usage,
        Some(Usage {
            input: 1,
            cached_input: 0,
            output: 1
        })
    );
    let null = codex_part("codex", "gpt-6.1-terra", &Value::Null, 1);
    assert_eq!(null.cost, Cost::Unknown);
    assert_eq!(null.usage, None);
    for usage in [
        json!({}),
        json!([]),
        json!({"inputTokens": 1}),
        json!({"inputTokens": 1, "cachedInputTokens": null, "outputTokens": 1}),
    ] {
        assert_eq!(
            codex_part("codex", "gpt-6.1-terra", &usage, 1).cost,
            Cost::Unknown
        );
    }
    assert_eq!(receipts::sum(&[]), Cost::Usd(0.0));
    assert_eq!(
        receipts::sum(&[priced.cost, unknown_model.cost]),
        Cost::Unknown
    );
    let r = Receipt::new(
        "run",
        "session",
        Mode::Quick,
        "failed",
        "start",
        "finish",
        1,
        vec![priced, null],
    );
    assert_eq!(r.cost, Cost::Unknown);
    let text = receipts::render(&r);
    assert!(text.contains("cost unknown"));
    assert!(!text.contains("$0.0000"));
    for (cost, expected) in [
        (Cost::Unknown, json!("unknown")),
        (Cost::Usd(1.5), json!({"usd": 1.5})),
    ] {
        assert_eq!(serde_json::to_value(cost).unwrap(), expected);
        assert_eq!(serde_json::from_value::<Cost>(expected).unwrap(), cost);
    }
}

#[test]
fn persistence_and_invalid_run_ids() {
    let home = tempfile::tempdir().unwrap();
    let r = fixture();
    let expected = home.path().join("modes/receipts/run-1.json");
    assert_eq!(receipts::write(home.path(), &r).unwrap(), expected);
    assert!(expected.is_file());
    assert_eq!(receipts::load(home.path(), "run-1").unwrap(), r);
    assert!(receipts::load(home.path(), "missing").is_err());
    for bad in ["../x", "A/b"] {
        let mut invalid = r.clone();
        invalid.run = bad.into();
        assert_eq!(
            receipts::write(home.path(), &invalid).unwrap_err().kind,
            "invalid_argument"
        );
        assert_eq!(
            receipts::load(home.path(), bad).unwrap_err().kind,
            "invalid_argument"
        );
    }
}

#[test]
fn every_mode_is_written() {
    let home = tempfile::tempdir().unwrap();
    for mode in Mode::ALL {
        let mut r = fixture();
        r.mode = mode;
        r.run = mode.as_str().into();
        let path = receipts::write(home.path(), &r).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["mode"], mode.as_str());
        assert_eq!(receipts::load(home.path(), mode.as_str()).unwrap(), r);
    }
}
