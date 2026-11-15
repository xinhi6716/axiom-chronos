use super::*;
use crate::desktop::i18n::Lang;
use std::prelude::v1::test;
use std::sync::mpsc;

#[test]
fn warnings_and_stamp() {
    let mut run = crate::desktop::demo::sample_run();
    run.notes = vec![crate::desktop::model::Note::ContextHigh {
        agent: "astra".into(),
        model: "model".into(),
        peak: 12345,
    }];
    run.warnings = vec![
        "ordinary warning".into(),
        run.notes[0].text(Lang::En).into(),
    ];
    for lang in [Lang::En, Lang::ZhTw] {
        assert_eq!(
            warning_texts(&run, lang),
            crate::desktop::usage::warning_texts(&run, lang)
        );
    }
    assert_eq!(CONTEXT_WARN, crate::usage::CONTEXT_WARN);
    for (secs, offset) in [
        (0, 0),
        (1_700_000_000, 28800),
        (-1, -18000),
        (2_000_000_000, 3600),
    ] {
        assert_eq!(
            log_stamp(secs, offset),
            crate::session::log_stamp(secs, offset)
        );
    }
}

#[test]
fn pools_are_honest_and_localized() {
    let home = tempfile::tempdir().unwrap();
    assert!(read_pools(home.path(), 0)
        .unwrap()
        .iter()
        .all(|r| r.remaining == PoolRemaining::Unknown));
    std::fs::create_dir_all(home.path().join("settings")).unwrap();
    std::fs::write(
        home.path().join("settings/pools.json"),
        r#"{"hydrogen_flash":{"token_limit":42000}}"#,
    )
    .unwrap();
    let rows = read_pools(home.path(), 0).unwrap();
    assert_eq!(rows[0].remaining, PoolRemaining::Known(42000));
    assert_eq!(rows[1].remaining, PoolRemaining::Unknown);
    assert_eq!(
        (rows[0].id, rows[0].label),
        ("hydrogen_flash", "Hydrogen/Flash")
    );
    crate::usage::pools::mark_exhausted(
        home.path(),
        crate::usage::pools::Pool::Argon,
        5000,
        "limit",
    )
    .unwrap();
    assert!(read_pools(home.path(), 0).unwrap()[1].exhausted);
    let mut row = unknown_pool_rows().remove(0);
    assert_eq!(pool_text(&row, Lang::En), "used — · remaining unknown");
    assert_eq!(pool_text(&row, Lang::ZhTw), "已用 — · 剩餘 未知");
    assert_eq!(pool_tone(&row), QuotaTone::Muted);
    row.used_tokens = Some(12300);
    row.cost_usd = Some(1.25);
    row.remaining = PoolRemaining::Known(42000);
    row.exhausted = true;
    assert_eq!(
        pool_text(&row, Lang::En),
        "used 12.3k tok · $1.25 · remaining 42.0k tok · exhausted"
    );
    assert_eq!(
        pool_text(&row, Lang::ZhTw),
        "已用 12.3k tok · $1.25 · 剩餘 42.0k tok · 已耗盡"
    );
    assert_eq!(pool_tone(&row), QuotaTone::Cinnabar);
    row.exhausted = false;
    assert_eq!(pool_tone(&row), QuotaTone::Gold);
}

#[test]
fn quotas_cover_fresh_stale_and_exhausted() {
    use crate::usage::quota::{ProviderQuota, Quotas, Window, WindowKind};
    let w = |kind, used, observed_at, resets_at| {
        Some(Window {
            kind,
            used,
            observed_at,
            resets_at,
            status: None,
        })
    };
    let q = Quotas {
        claude: ProviderQuota {
            observed_at: Some(3600),
            present: [true, true],
            five_hour: w(WindowKind::FiveHour, 0.5, 3600, Some(7200)),
            weekly: w(WindowKind::Weekly, 1.0, 3600, Some(7260)),
        },
        codex: ProviderQuota {
            observed_at: Some(0),
            present: [true, false],
            five_hour: w(WindowKind::FiveHour, 0.9, 0, None),
            weekly: None,
        },
    };
    let rows = quota_rows(&q, 3600, 0, Lang::En, false);
    assert_eq!(rows.len(), 4);
    assert_eq!(
        (
            rows[0].provider,
            rows[0].window_id,
            rows[0].name.as_str(),
            rows[0].left,
            rows[0].value.as_str(),
            rows[0].tone,
            rows[0].tooltip.as_str()
        ),
        (
            "Claude",
            "five-hour",
            "5 hours",
            Some(0.5),
            "50%",
            QuotaTone::Gold,
            "50.0% left (50.0% used) · observed 01:00 · resets 02:00"
        )
    );
    assert_eq!(
        (
            rows[1].value.as_str(),
            rows[1].tone,
            rows[1].tooltip.as_str()
        ),
        (
            "0% · resets in 1 h",
            QuotaTone::Cinnabar,
            "0.0% left (100.0% used) · observed 01:00 · resets 02:01"
        )
    );
    assert_eq!(
        (
            rows[2].value.as_str(),
            rows[2].tone,
            rows[2].tooltip.as_str()
        ),
        (
            "—",
            QuotaTone::Muted,
            "last known · 10% left · 1 h ago · 10.0% left (90.0% used) · observed 00:00"
        )
    );
    assert_eq!(
        (
            rows[3].window_id,
            rows[3].value.as_str(),
            rows[3].tooltip.as_str()
        ),
        ("weekly", "—", "—")
    );
    assert_eq!(quota_rows(&q, 3600, 0, Lang::En, true)[1].value, "0% ↻1h");
    let zh = quota_rows(&q, 3600, 480, Lang::ZhTw, false);
    assert_eq!(
        (zh[0].name.as_str(), zh[0].tooltip.as_str()),
        ("5 小時", "剩 50.0%（已用 50.0%） · 觀測 09:00 · 10:00 重置")
    );
    assert_eq!(zh[1].value, "0% · 1 小時後重置");
    let mut fresh_low = q.clone();
    fresh_low.codex.observed_at = Some(3600);
    fresh_low.codex.five_hour.as_mut().unwrap().observed_at = 3600;
    let fresh_rows = quota_rows(&fresh_low, 3600, 0, Lang::En, false);
    assert_eq!(fresh_rows.len(), 3);
    assert_eq!(
        (
            fresh_rows[2].provider,
            fresh_rows[2].value.as_str(),
            fresh_rows[2].tone
        ),
        ("Codex", "10%", QuotaTone::Amber)
    );
}

#[test]
fn council_and_background() {
    let home = tempfile::tempdir().unwrap();
    assert!(argon_every_round(home.path()));
    set_argon_every_round(home.path(), false).unwrap();
    assert!(!argon_every_round(home.path()));
    set_argon_every_round(home.path(), true).unwrap();
    assert!(argon_every_round(home.path()));
    assert!(validate_budgets("5", "200000", "400000", "3").is_ok());
    assert!(validate_budgets("0", "200000", "400000", "3").is_err());
    assert!(validate_budgets("5", "abc", "400000", "3").is_err());
    assert_eq!(budget_tokens(200000), "200k");
    assert_eq!(budget_tokens(1234), "1234");
    use crate::desktop::ux::celestial::Member;
    for (ours, theirs) in [
        (CouncilMember::Astra, Member::Astra),
        (CouncilMember::Fable, Member::Fable),
        (CouncilMember::Argon, Member::Argon),
    ] {
        assert_eq!(
            council_member(ours),
            crate::desktop::ux::celestial::member(theirs)
        );
    }
    let current = std::thread::current().id();
    let (tx, rx) = mpsc::channel();
    let tx1 = tx.clone();
    let a = spawn_read_pools(home.path().into(), 0, move |v| {
        tx1.send((std::thread::current().id(), v.is_ok())).unwrap();
    });
    let tx1 = tx.clone();
    let b = spawn_read_argon(home.path().into(), move |v| {
        tx1.send((std::thread::current().id(), v)).unwrap();
    });
    b.join().unwrap();
    let c = spawn_set_argon(home.path().into(), false, move |v| {
        tx.send((std::thread::current().id(), v.is_ok())).unwrap();
    });
    for _ in 0..3 {
        let (id, ok) = rx.recv().unwrap();
        assert_ne!(id, current);
        assert!(ok);
    }
    a.join().unwrap();
    c.join().unwrap();
}

#[test]
fn session_identity_and_concurrency() {
    let session = UsageSession {
        wish: "w".into(),
        session: "s".into(),
        label: "l".into(),
        usage: Default::default(),
        complete: true,
        started_at: None,
    };
    let _: crate::desktop::ux::usage::UsageSession = session;
    assert_eq!(parallel_text(Some(2), None, Lang::ZhTw), "Codex 並行 2 / —");
    let models = [crate::desktop::model::RunningModel {
        provider: crate::desktop::model::ModelProvider::Codex,
        model: "sol".into(),
        count: 2,
    }];
    assert_eq!(
        concurrency_groups(models.iter(), Lang::En),
        crate::desktop::concurrency::groups(models.iter(), Lang::En)
    );
}

fn forbidden(source: &str) -> bool {
    let names = [
        "settings",
        "palette",
        "wishes",
        "control",
        "detail",
        "usage",
        "quota",
        "plain_text",
        "evolve",
        "memory",
        "doctor",
        "run_view",
        "todo",
        "mesh",
        "shell",
        "composer",
        "mini",
        "superswitch",
        "research",
        "paper",
        "launch",
    ];
    source.contains("use super::")
        || source.contains("ux::celestial")
        || source.contains("ux::composer")
        || names.iter().any(|name| {
            source.contains(&format!("super::{name}"))
                || source.contains(&format!("desktop::{name}"))
        })
}

#[test]
fn no_view_references() {
    assert!(forbidden("use crate::desktop::usage::Foo;"));
    assert!(forbidden("super::quota::x"));
    assert!(!forbidden(
        include_str!("usage.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap()
    ));
}
