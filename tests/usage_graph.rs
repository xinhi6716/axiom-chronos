use axiom::usage::{ledger, CodexTokens, Locations, Tally};
use serde_json::json;
use std::fs;

#[test]
fn complete_records_keep_their_original_json() {
    let tally = Tally {
        calls: 1,
        input: 2,
        cache_write: 3,
        cache_write_5m: 3,
        cache_read: 4,
        output: 5,
        usd: Some(0.5),
        priced_usd: 0.5,
        peak_context: 9,
        ..Tally::default()
    };
    assert_eq!(serde_json::to_string(&tally).unwrap(), "{\"calls\":1,\"input\":2,\"cache_write\":3,\"cache_write_5m\":3,\"cache_write_1h\":0,\"cache_read\":4,\"output\":5,\"web_searches\":0,\"usd\":0.5,\"priced_usd\":0.5,\"unpriced_calls\":0,\"peak_context\":9,\"gap_rewrites\":0,\"gap_rewrite_tokens\":0,\"gap_rewrite_usd\":0.0}");
    let tokens = CodexTokens {
        input: 2,
        cached_input: 3,
        output: 4,
        reasoning_output: 1,
        total: 6,
        ..CodexTokens::default()
    };
    assert_eq!(
        serde_json::to_string(&tokens).unwrap(),
        "{\"input\":2,\"cached_input\":3,\"output\":4,\"reasoning_output\":1,\"total\":6}"
    );
}

#[test]
fn unknown_names_follow_contract_order() {
    for (index, name) in ["input", "cache_read", "cache_write", "output"]
        .into_iter()
        .enumerate()
    {
        let mut tally = Tally::default();
        tally.missing[index] = true;
        assert_eq!(
            serde_json::to_value(&tally).unwrap()["unknown"],
            json!([name])
        );
    }
    let tally = Tally {
        missing: [true, true, true, true],
        ..Tally::default()
    };
    assert_eq!(
        serde_json::to_value(&tally).unwrap()["unknown"],
        json!(["input", "cache_write", "cache_read", "output"])
    );
    let tokens = CodexTokens {
        missing: [false, true, false],
        ..CodexTokens::default()
    };
    assert_eq!(
        serde_json::to_value(&tokens).unwrap()["unknown"],
        json!(["cached_input"])
    );
    let tokens = CodexTokens {
        missing: [true, true, true],
        ..CodexTokens::default()
    };
    assert_eq!(
        serde_json::to_value(&tokens).unwrap()["unknown"],
        json!(["input", "cached_input", "output"])
    );
}

#[test]
fn unknown_is_not_indistinguishable_from_a_reported_zero() {
    let tally = Tally::default();
    let unknown = Tally {
        missing: [true, false, false, false],
        ..tally.clone()
    };
    assert_ne!(
        serde_json::to_string(&tally).unwrap(),
        serde_json::to_string(&unknown).unwrap()
    );
    let tokens = CodexTokens::default();
    let unknown = CodexTokens {
        missing: [false, true, false],
        ..tokens
    };
    assert_ne!(
        serde_json::to_string(&tokens).unwrap(),
        serde_json::to_string(&unknown).unwrap()
    );
}

#[test]
fn complete_call_does_not_clear_an_earlier_unknown() {
    let root = tempfile::tempdir().unwrap();
    let locations = Locations {
        home: root.path().join("home"),
        projects: root.path().join("projects"),
    };
    let session = "11111111-2222-3333-4444-555555555555";
    let dir = locations.projects.join("project");
    fs::create_dir_all(&dir).unwrap();
    let call = |id, usage| {
        json!({"type":"assistant", "timestamp":"2026-01-01T00:00:00Z", "message":{"id":id, "model":"claude-opus-5-5", "usage":usage, "content":[]}}).to_string()
    };
    let incomplete = call(
        "first",
        json!({"input_tokens":1, "cache_read_input_tokens":0, "output_tokens":1}),
    );
    let complete = call(
        "second",
        json!({"input_tokens":1, "cache_creation_input_tokens":0, "cache_read_input_tokens":0, "output_tokens":1}),
    );
    fs::write(
        dir.join(format!("{session}.jsonl")),
        format!("{incomplete}\n{complete}\n"),
    )
    .unwrap();
    let ledger = ledger(&locations, Some(session)).unwrap();
    assert_eq!(ledger.totals.calls, 2);
    assert_eq!(
        serde_json::to_value(&ledger.totals).unwrap()["unknown"],
        json!(["cache_write"])
    );
    assert_eq!(
        serde_json::to_value(&ledger.by_agent[0]).unwrap()["unknown"],
        json!(["cache_write"])
    );
}
