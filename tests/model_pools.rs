mod common;

use axiom::{
    engine::{
        budgets::{self, Role},
        policy,
    },
    orch::events,
    usage::pools::{self, Pool, Remaining, HYDROGEN_XHIGH},
};
use std::{fs, path::Path};

fn limits(home: &Path, contents: &str) {
    fs::create_dir_all(home.join("settings")).unwrap();
    fs::write(home.join("settings/pools.json"), contents).unwrap();
}

#[test]
fn model_mapping() {
    assert_eq!(
        pools::pool_for_model(HYDROGEN_XHIGH),
        Some(Pool::HydrogenFlash)
    );
    assert_eq!(
        pools::pool_for_model("gemini-4-hydrogen-low"),
        Some(Pool::HydrogenFlash)
    );
    assert_eq!(
        pools::pool_for_model("gemini-3.8-flash-high"),
        Some(Pool::HydrogenFlash)
    );
    assert_eq!(
        pools::pool_for_model("gemini-4-argon-xhigh"),
        Some(Pool::Argon)
    );
    assert_eq!(pools::pool_for_model("other"), None);
}

#[test]
fn charges_are_isolated_and_flash_shares_hydrogen() {
    let home = common::temp();
    let h = home.path();
    assert_eq!(
        pools::charge(h, HYDROGEN_XHIGH, Some(10), Some(0.25), "a").unwrap(),
        Some(Pool::HydrogenFlash)
    );
    pools::charge(h, "gemini-3.8-flash-high", Some(20), Some(0.5), "b").unwrap();
    let s = pools::status(h, 0).unwrap();
    assert_eq!((s[0].used_tokens, s[0].cost_usd), (Some(30), Some(0.75)));
    assert_eq!((s[1].used_tokens, s[1].cost_usd), (Some(0), Some(0.0)));
    pools::charge(h, "gemini-4-argon-high", Some(7), Some(1.0), "c").unwrap();
    let s = pools::status(h, 0).unwrap();
    assert_eq!((s[0].used_tokens, s[0].cost_usd), (Some(30), Some(0.75)));
    assert_eq!((s[1].used_tokens, s[1].cost_usd), (Some(7), Some(1.0)));
    assert_eq!(pools::charge(h, "other", None, None, "d").unwrap(), None);
    assert_eq!(
        fs::read_to_string(h.join("usage/pools.jsonl"))
            .unwrap()
            .lines()
            .count(),
        3
    );
}

#[test]
fn marks_and_limits_only_exhaust_their_pool() {
    let home = common::temp();
    let h = home.path();
    pools::mark_exhausted(h, Pool::Argon, i64::MAX, "provider").unwrap();
    let err = pools::check(h, "gemini-4-argon").unwrap_err();
    assert_eq!(err.kind, "pool_exhausted");
    assert!(err.message.contains("argon"));
    assert!(pools::check(h, HYDROGEN_XHIGH).is_ok());
    assert_eq!(
        pools::status(h, i64::MAX).unwrap()[1].exhausted_until_ms,
        None
    );
    let expired = common::temp();
    pools::mark_exhausted(expired.path(), Pool::Argon, 1, "expired").unwrap();
    assert!(pools::check(expired.path(), "gemini-4-argon").is_ok());
    limits(expired.path(), r#"{"hydrogen_flash":{"token_limit":10}}"#);
    pools::charge(expired.path(), HYDROGEN_XHIGH, Some(10), Some(0.0), "a").unwrap();
    assert_eq!(
        pools::check(expired.path(), HYDROGEN_XHIGH)
            .unwrap_err()
            .kind,
        "pool_exhausted"
    );
    assert!(pools::check(expired.path(), "gemini-4-argon").is_ok());
    assert!(pools::check(expired.path(), "other").is_ok());
}

#[test]
fn unknown_and_remaining() {
    let home = common::temp();
    let h = home.path();
    pools::charge(h, HYDROGEN_XHIGH, Some(4), Some(1.0), "known").unwrap();
    assert_eq!(
        pools::status(h, 0).unwrap()[0].remaining,
        Remaining::Unknown
    );
    limits(
        h,
        r#"{"hydrogen_flash":{"token_limit":10},"argon":{"token_limit":5}}"#,
    );
    let s = pools::status(h, 0).unwrap();
    assert_eq!(s[0].remaining, Remaining::Known { tokens: 6 });
    assert_eq!(s[1].remaining, Remaining::Known { tokens: 5 });
    pools::charge(h, HYDROGEN_XHIGH, None, None, "unknown").unwrap();
    let s = pools::status(h, 0).unwrap();
    assert_eq!(s[0].used_tokens, None);
    assert_eq!(s[0].cost_usd, None);
    assert_eq!(s[0].remaining, Remaining::Unknown);
    let separate = common::temp();
    pools::charge(
        separate.path(),
        HYDROGEN_XHIGH,
        None,
        Some(0.5),
        "unknown tokens",
    )
    .unwrap();
    pools::charge(
        separate.path(),
        "gemini-4-argon",
        Some(3),
        None,
        "unknown price",
    )
    .unwrap();
    let s = pools::status(separate.path(), 0).unwrap();
    assert_eq!((s[0].used_tokens, s[0].cost_usd), (None, Some(0.5)));
    assert_eq!((s[1].used_tokens, s[1].cost_usd), (Some(3), None));
}

#[test]
fn empty_status_has_both_pools_in_order() {
    let home = common::temp();
    let s = pools::status(home.path(), 0).unwrap();
    assert_eq!(s.iter().map(|s| s.pool).collect::<Vec<_>>(), Pool::ALL);
    assert!(s.iter().all(|s| s.used_tokens == Some(0)
        && s.cost_usd == Some(0.0)
        && s.remaining == Remaining::Unknown
        && s.exhausted_until_ms.is_none()));
}

#[test]
fn model_budgets_check_role_first_and_record_both() {
    let home = common::temp();
    let store = common::temp();
    let h = home.path();
    let p = store.path();
    policy::set(p, "budget.reviewer", "0.5").unwrap();
    budgets::record(p, Role::Reviewer, "other", Some(0.5), 0, 0, "old").unwrap();
    assert_eq!(
        budgets::check_model(p, h, Role::Reviewer, "call", HYDROGEN_XHIGH)
            .unwrap_err()
            .kind,
        "budget_exceeded"
    );
    pools::mark_exhausted(h, Pool::Argon, i64::MAX, "provider").unwrap();
    assert_eq!(
        budgets::check_model(p, h, Role::Reviewer, "call", "gemini-4-argon")
            .unwrap_err()
            .kind,
        "budget_exceeded"
    );
    assert_eq!(
        budgets::check_model(p, h, Role::Core, "call", "gemini-4-argon")
            .unwrap_err()
            .kind,
        "pool_exhausted"
    );
    let event =
        budgets::record_model(p, h, Role::Core, HYDROGEN_XHIGH, Some(0.25), 12, 8, "call").unwrap();
    assert_eq!(event.data["role"], "core");
    assert_eq!(event.data["call"], "call");
    assert!(events::read_events(p, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == event.kind && e.data == event.data));
    let s = pools::status(h, 0).unwrap();
    assert_eq!((s[0].used_tokens, s[0].cost_usd), (Some(20), Some(0.25)));
    assert_eq!((s[1].used_tokens, s[1].cost_usd), (Some(0), Some(0.0)));
}
