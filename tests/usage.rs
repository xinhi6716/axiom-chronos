use axiom::usage::price::{cache_write_usd, usd, CallUsage};

fn near(actual: Option<f64>, expected: f64) {
    let actual = actual.expect("priced");
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} is not {expected}"
    );
}
fn tokens(input: u64, write_5m: u64, write_1h: u64, read: u64, output: u64) -> CallUsage {
    CallUsage {
        input,
        cache_write: write_5m + write_1h,
        cache_write_1h: write_1h,
        cache_read: read,
        output,
        ..CallUsage::default()
    }
}
const M: u64 = 1_000_000;

#[test]
fn each_category_is_priced_at_its_own_rate() {
    for (model, input, write_5m, write_1h, read, output) in [
        ("claude-opus-5-5", 4.0, 5.0, 8.0, 0.20, 20.0),
        ("claude-sonnet-5-5", 2.0, 2.5, 4.0, 0.10, 10.0),
        ("claude-sonnet-5", 2.0, 2.5, 4.0, 0.20, 10.0),
        ("claude-opus-5", 5.0, 6.25, 10.0, 0.50, 25.0),
        ("claude-opus-4-1", 15.0, 18.75, 30.0, 1.50, 75.0),
        ("claude-haiku-4-5", 1.0, 1.25, 2.0, 0.10, 5.0),
        ("claude-fable-5-1", 10.0, 12.5, 20.0, 0.25, 50.0),
    ] {
        near(usd(model, &tokens(M, 0, 0, 0, 0)), input);
        near(usd(model, &tokens(0, M, 0, 0, 0)), write_5m);
        near(usd(model, &tokens(0, 0, M, 0, 0)), write_1h);
        near(usd(model, &tokens(0, 0, 0, M, 0)), read);
        near(usd(model, &tokens(0, 0, 0, 0, M)), output);
    }
}

#[test]
fn cache_writes_without_a_breakdown_are_five_minute_writes() {
    let u = CallUsage {
        cache_write: M,
        ..CallUsage::default()
    };
    near(usd("claude-opus-5-5", &u), 5.0);
    near(cache_write_usd("claude-opus-5-5", &u), 5.0);
}

#[test]
fn cache_write_cost_excludes_every_other_category() {
    let u = tokens(M, M, M, M, M);
    near(cache_write_usd("claude-sonnet-5-5", &u), 2.5 + 4.0);
}

#[test]
fn haiku_5_5_is_priced_per_request_by_prompt_length() {
    let at_limit = tokens(40_000, 30_000, 0, 30_000, 1_000);
    near(
        usd("claude-haiku-5-5", &at_limit),
        (40_000.0 * 0.10 + 30_000.0 * 0.125 + 30_000.0 * 0.01 + 1_000.0 * 0.50) / 1e6,
    );
    let over = tokens(40_001, 30_000, 0, 30_000, 1_000);
    near(
        usd("claude-haiku-5-5", &over),
        (40_001.0 * 0.50 + 30_000.0 * 0.625 + 30_000.0 * 0.05 + 1_000.0 * 2.50) / 1e6,
    );
}

#[test]
fn date_suffixes_resolve_to_their_family_and_nothing_else_does() {
    near(
        usd("claude-opus-4-1-20250805", &tokens(M, 0, 0, 0, 0)),
        15.0,
    );
    near(usd("claude-opus-4-20250514", &tokens(M, 0, 0, 0, 0)), 15.0);
    near(usd("claude-sonnet-4-5-20250929", &tokens(M, 0, 0, 0, 0)), 3.0);
    for unknown in [
        "claude-opus-5-6",
        "claude-opus-4-2",
        "claude-sonnet-6",
        "claude-opus-5-5-preview",
        "anthropic.claude-opus-5-5",
        "gpt-6.1-terra",
        "<synthetic>",
        "",
    ] {
        assert_eq!(usd(unknown, &tokens(M, M, M, M, M)), None, "{unknown}");
        assert_eq!(cache_write_usd(unknown, &tokens(0, M, 0, 0, 0)), None);
    }
}

#[test]
fn fast_mode_doubles_every_rate_where_it_exists_and_is_unpriced_elsewhere() {
    let u = CallUsage {
        fast: true,
        ..tokens(M, M, M, M, M)
    };
    near(usd("claude-opus-5-5", &u), 2.0 * (4.0 + 5.0 + 8.0 + 0.20 + 20.0));
    near(usd("claude-opus-4-8", &u), 2.0 * (5.0 + 6.25 + 10.0 + 0.5 + 25.0));
    assert_eq!(usd("claude-sonnet-5-5", &u), None);
}

#[test]
fn us_only_inference_costs_more_on_tokens_but_not_on_searches() {
    let u = CallUsage {
        us_only: true,
        web_searches: 100,
        ..tokens(M, 0, 0, 0, M)
    };
    near(usd("claude-opus-5-5", &u), 1.1 * (4.0 + 20.0) + 1.0);
}

#[test]
fn non_standard_service_tiers_are_not_guessed() {
    let u = CallUsage {
        non_standard_tier: true,
        ..tokens(M, 0, 0, 0, 0)
    };
    assert_eq!(usd("claude-opus-5-5", &u), None);
}
