use axiom::usage::quota::{
    claude_log_time, freshness, level, parse_claude_event, parse_codex_read, parse_codex_update,
    Freshness, Level, Provider, ProviderQuota, Quotas, Window, WindowKind, STALE_AFTER_S,
};
use serde_json::{json, Value};

fn window(kind: WindowKind, used: f32, observed_at: i64) -> Window {
    Window {
        kind,
        used,
        observed_at,
        resets_at: None,
        status: None,
    }
}

#[test]
fn real_claude_fixtures() {
    let fixtures = [
        (
            include_str!("fixtures/quota/claude-rate-limit-1.jsonl"),
            0.17,
            0.11,
            1793652000,
            1794218400,
        ),
        (
            include_str!("fixtures/quota/claude-rate-limit-2.jsonl"),
            0.39,
            0.42,
            1794308400,
            1794823200,
        ),
        (
            include_str!("fixtures/quota/claude-rate-limit-3.jsonl"),
            0.03,
            0.5,
            1794344400,
            1794823200,
        ),
    ];
    for (text, five, weekly, five_reset, week_reset) in fixtures {
        let v: Value = serde_json::from_str(text.trim()).unwrap();
        let parsed = parse_claude_event(&v, 123).unwrap();
        let a = parsed.five_hour.unwrap();
        let b = parsed.weekly.unwrap();
        assert_eq!(
            (
                a.kind,
                a.used,
                a.resets_at,
                a.observed_at,
                a.status.as_deref()
            ),
            (
                WindowKind::FiveHour,
                five,
                Some(five_reset),
                123,
                Some("allowed")
            )
        );
        assert_eq!(
            (
                b.kind,
                b.used,
                b.resets_at,
                b.observed_at,
                b.status.as_deref()
            ),
            (
                WindowKind::Weekly,
                weekly,
                Some(week_reset),
                123,
                Some("allowed")
            )
        );
        if five == 0.03 {
            assert_eq!(a.left(), 0.97_f32);
        }
    }
}

#[test]
fn claude_shapes_and_validation() {
    // Synthetic: a one-window event was not observed in the real logs.
    let one = json!({"type":"rate_limit_event", "rate_limit_info":{"unifiedWindows":{
        "five_hour":{"utilization": 1.5,"resetsAt": 42,"status":"allowed"}
    }}});
    let parsed = parse_claude_event(&one, 7).unwrap();
    assert_eq!(parsed.five_hour.unwrap().used, 1.0);
    assert!(parsed.weekly.is_none());
    for invalid in [
        json!({"resetsAt":1,"status":"allowed"}),
        json!({"utilization":0.5,"status":"allowed"}),
        json!({"utilization":0.5,"resetsAt":1}),
        json!({"utilization":"0.5","resetsAt":1,"status":"allowed"}),
        json!({"utilization":0.5,"resetsAt":1.5,"status":"allowed"}),
        json!({"utilization":0.5,"resetsAt":1,"status":true}),
        json!({"utilization":-0.1,"resetsAt":1,"status":"allowed"}),
        Value::Null,
    ] {
        let event = json!({"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":invalid}}});
        assert_eq!(
            parse_claude_event(&event, 0),
            Some(ProviderQuota::default())
        );
    }
    assert_eq!(parse_claude_event(&json!({"type":"other"}), 0), None);
    assert_eq!(parse_claude_event(&json!({"type":42}), 0), None);
    assert!(serde_json::from_str::<Value>("{\"type\":\"rate_limit_event\"").is_err());
}

#[test]
fn codex_read_preferred_fallback_and_validation() {
    let result = json!({
        "rateLimits":{"primary":{"usedPercent":99,"windowDurationMins":300}},
        "rateLimitsByLimitId":{
            "other":{"primary":{"usedPercent":88,"windowDurationMins":300}},
            "codex":{"primary":{"usedPercent":3,"windowDurationMins":300,"resetsAt":22},
                     "secondary":{"usedPercent":150,"windowDurationMins":10080,"resetsAt":"bad"}}
        }
    });
    let parsed = parse_codex_read(&result, 10);
    let five = parsed.five_hour.unwrap();
    let weekly = parsed.weekly.unwrap();
    assert_eq!(
        (
            five.kind,
            five.used,
            five.resets_at,
            five.observed_at,
            five.status
        ),
        (WindowKind::FiveHour, 0.03, Some(22), 10, None)
    );
    assert_eq!(
        (weekly.kind, weekly.used, weekly.resets_at),
        (WindowKind::Weekly, 1.0, None)
    );
    for codex in [Value::Null, json!("not an object")] {
        let mut fallback = result.clone();
        fallback["rateLimitsByLimitId"]["codex"] = codex;
        assert_eq!(
            parse_codex_read(&fallback, 10).five_hour.unwrap().used,
            0.99
        );
    }
    assert_eq!(
        parse_codex_read(
            &json!({"rateLimits":{"primary":{"usedPercent":-3,"windowDurationMins":300}}}),
            0
        )
        .five_hour
        .unwrap()
        .used,
        0.0
    );
    for invalid in [
        json!(null),
        json!({"windowDurationMins":300}),
        json!({"usedPercent":null,"windowDurationMins":300}),
        json!({"usedPercent":"1","windowDurationMins":300}),
        json!({"usedPercent":1,"windowDurationMins":99}),
        json!({"usedPercent":1,"windowDurationMins":null}),
        json!({"usedPercent":1}),
    ] {
        assert_eq!(
            parse_codex_read(&json!({"rateLimits":{"primary":invalid}}), 0),
            ProviderQuota::default()
        );
    }
    // An existing but empty codex bucket is authoritative, not a fallback.
    assert_eq!(
        parse_codex_read(
            &json!({"rateLimitsByLimitId":{"codex":{}},"rateLimits":result["rateLimits"]}),
            0
        ),
        ProviderQuota::default()
    );
}

#[test]
fn codex_sparse_update_and_merge() {
    let mut quotas = Quotas::default();
    let initial = parse_codex_read(
        &json!({"rateLimits":{
            "primary":{"usedPercent":10,"windowDurationMins":300},
            "secondary":{"usedPercent":20,"windowDurationMins":10080}
        }}),
        10,
    );
    assert!(quotas.merge(Provider::Codex, initial));
    let update = parse_codex_update(
        &json!({"rateLimits":{"secondary":{"usedPercent":30,"windowDurationMins":10080}}}),
        11,
    );
    assert!(update.five_hour.is_none());
    assert!(quotas.merge(Provider::Codex, update));
    assert_eq!(quotas.codex.five_hour.unwrap().used, 0.1);
    assert_eq!(quotas.codex.weekly.unwrap().used, 0.3);
}

#[test]
fn merge_order_ties_and_no_clearing() {
    let mut q = Quotas::default();
    let item = |used, at| ProviderQuota {
        five_hour: Some(window(WindowKind::FiveHour, used, at)),
        weekly: None,
    };
    assert!(q.merge(Provider::Claude, item(0.2, 10)));
    assert!(!q.merge(Provider::Claude, item(0.2, 10)));
    assert!(!q.merge(Provider::Claude, item(0.8, 9)));
    assert!(!q.merge(Provider::Claude, ProviderQuota::default()));
    assert_eq!(q.claude.five_hour.as_ref().unwrap().used, 0.2);
    assert!(q.merge(Provider::Claude, item(0.3, 10)));
    assert!(q.merge(Provider::Claude, item(0.4, 11)));
    assert_eq!(q.claude.five_hour.unwrap().used, 0.4);
    assert_eq!(q.codex, ProviderQuota::default());
}

#[test]
fn levels_and_freshness_do_not_restore_quota() {
    assert_eq!(level(0.20), Level::Normal);
    assert_eq!(level(0.199), Level::Low);
    assert_eq!(level(0.0), Level::Exhausted);
    assert_eq!(level(-0.1), Level::Exhausted);
    let mut w = window(WindowKind::FiveHour, 0.8, 100);
    assert_eq!(freshness(&w, 100 + STALE_AFTER_S), Freshness::Fresh);
    assert_eq!(
        freshness(&w, 101 + STALE_AFTER_S),
        Freshness::Stale { age_s: 901 }
    );
    w.resets_at = Some(200);
    assert_eq!(freshness(&w, 200), Freshness::Fresh);
    let left = w.left();
    assert_eq!(freshness(&w, 201), Freshness::Stale { age_s: 101 });
    assert_eq!(w.left(), left);
    w.observed_at = 300;
    assert_eq!(freshness(&w, 201), Freshness::Stale { age_s: 0 });
    w.used = 2.0;
    assert_eq!(w.left(), 0.0);
    w.used = -1.0;
    assert_eq!(w.left(), 1.0);
}

#[test]
fn log_timestamps_and_fallback() {
    assert_eq!(
        claude_log_time(Some("2026-11-10T16:48:55.450Z"), Some(1)),
        Some(1794329335)
    );
    assert_eq!(
        claude_log_time(Some("2026-11-11T00:48:55+08:00"), None),
        Some(1794329335)
    );
    assert_eq!(claude_log_time(Some("1970-01-01T00:00:00Z"), None), Some(0));
    assert_eq!(
        claude_log_time(Some("1969-12-31T23:59:59Z"), None),
        Some(-1)
    );
    for bad in [
        "bad",
        "2026-02-29T00:00:00Z",
        "2026-11-10T16:48:55.Z",
        "2026-11-10T16:48:55+24:00",
        "2026-11-10T16:48:55Zextra",
        "2026-aa-10T16:48:55Z",
    ] {
        assert_eq!(claude_log_time(Some(bad), Some(123)), Some(123));
    }
    assert_eq!(claude_log_time(None, None), None);
}

#[test]
fn codex_schema_documents_consumed_fields() {
    let schema: Value =
        serde_json::from_str(include_str!("fixtures/quota/codex-0.159.3-schema.json")).unwrap();
    assert_eq!(schema["codex_cli"], "0.159.3");
    for root in [
        "GetAccountRateLimitsResponse",
        "AccountRateLimitsUpdatedNotification",
    ] {
        let snapshot = &schema[root]["definitions"]["RateLimitSnapshot"]["properties"];
        let window = &schema[root]["definitions"]["RateLimitWindow"]["properties"];
        for name in ["primary", "secondary"] {
            assert!(snapshot.get(name).is_some(), "{root}: {name}");
        }
        for name in ["usedPercent", "windowDurationMins", "resetsAt"] {
            assert!(window.get(name).is_some(), "{root}: {name}");
        }
        assert!(schema[root]["properties"].get("rateLimits").is_some());
    }
    assert!(schema["GetAccountRateLimitsResponse"]["properties"]
        .get("rateLimitsByLimitId")
        .is_some());
}
