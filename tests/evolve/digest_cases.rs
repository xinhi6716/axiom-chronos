use axiom::{
    evolve::{
        digest::{self, MAX_DIGEST_BYTES, MAX_PROBLEMS, MAX_SOURCES_PER_PROBLEM},
        history::{MetricValue, RunRecord},
        Collected, Event, Source, SourceStatus,
    },
    usage::{AgentRow, Ledger, Period, PeriodRun, Prices, RunCost, Tally, Thresholds},
};

fn period() -> Period {
    Period {
        start_ms: 0,
        end_ms: 86_400_000,
        runs: vec![],
        history: vec![],
        undated_calls: 0,
        skipped_lines: 0,
    }
}
fn status(family: &str, samples: u64) -> SourceStatus {
    SourceStatus {
        family: family.into(),
        state: "observed".into(),
        samples,
        note: String::new(),
    }
}
fn event(id: &str, count: u64, cost: Option<f64>, key: &str) -> Event {
    Event {
        problem: id.into(),
        title: id.into(),
        count,
        cost_usd: cost,
        unpriced: if cost.is_none() { 2 } else { 0 },
        detail: String::new(),
        snapshot: false,
        source: Source {
            family: id.split('.').next().unwrap().into(),
            location: format!("file/{key}"),
            session: None,
            project: None,
            unit: None,
            ts_ms: None,
            key: key.into(),
        },
    }
}
fn collected(events: Vec<Event>) -> Collected {
    Collected {
        events,
        gaps: vec![],
        sources: vec![
            status("usage", 12),
            status("goal", 8),
            status("lint", 3),
            SourceStatus {
                family: "hook".into(),
                state: "not logged".into(),
                samples: 0,
                note: String::new(),
            },
        ],
    }
}
fn run(id: &str, started_ms: i64, metrics: Vec<MetricValue>) -> RunRecord {
    RunRecord {
        id: id.into(),
        started_ms,
        repo: String::new(),
        base_commit: String::new(),
        branch: String::new(),
        since_days: 7,
        start_ms: 0,
        end_ms: 1,
        problems: vec![],
        metrics,
        state: String::new(),
        session: None,
        goal_status: None,
        exit_code: None,
        error: None,
    }
}
fn metric(problem: &str, value: f64, samples: u64) -> MetricValue {
    MetricValue {
        problem: problem.into(),
        value,
        samples,
    }
}
fn history(cost: Option<f64>, skill: &str) -> RunCost {
    RunCost {
        session: String::new(),
        skill: Some(skill.into()),
        last_ms: None,
        calls: 1,
        usd: cost,
        priced_usd: cost.unwrap_or(0.0),
        unpriced_calls: u64::from(cost.is_none()),
    }
}

#[test]
fn ranks_aggregates_and_displays_sources_and_unknowns() {
    assert_eq!(
        (MAX_PROBLEMS, MAX_SOURCES_PER_PROBLEM, MAX_DIGEST_BYTES),
        (10, 3, 16 * 1024)
    );
    let events = vec![
        event("usage.z", 2, None, "z"),
        event("usage.b", 2, Some(0.0), "b"),
        event("usage.a", 2, Some(0.0), "a"),
        event("usage.top", 2, Some(1.0), "one"),
        event("usage.top", 1, None, "one"),
        event("usage.top", 1, Some(2.0), "two"),
    ];
    let d = digest::build(collected(events), &period(), &[], 86_400_000, 1);
    assert_eq!(
        d.problems.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["usage.top", "usage.a", "usage.b", "usage.z"]
    );
    assert_eq!(
        (
            d.problems[0].count,
            d.problems[0].cost_usd,
            d.problems[0].unpriced,
            d.problems[0].metric.samples,
            d.problems[0].sources.len()
        ),
        (4, Some(3.0), 2, 12, 2)
    );
    let text = digest::render(&d);
    assert!(
        text.contains("file/one")
            && text.contains("file/two")
            && text.contains("unknown (2 unpriced)"),
        "{text}"
    );
    assert!(text.contains("hook refusals: not logged"));
    assert!(text.contains("1970-01-01") && text.contains("1970-01-02"));
    assert!(!text.to_lowercase().contains("saving"));
}

#[test]
fn estimate_median_and_empty_week() {
    let mut p = period();
    p.history = vec![
        history(Some(7.0), "wish"),
        history(Some(1.0), "wish"),
        history(Some(3.0), "wish"),
        history(None, "wish"),
        history(Some(100.0), "other"),
    ];
    let d = digest::build(collected(vec![]), &p, &[], 86_400_000, 1);
    assert_eq!(
        (
            d.estimate.median_usd,
            d.estimate.samples,
            d.estimate.unpriced_runs
        ),
        (Some(3.0), 3, 1)
    );
    assert!(digest::render(&d).contains("no recorded problems"));
    p.history.push(history(Some(5.0), "wish"));
    assert_eq!(
        digest::build(collected(vec![]), &p, &[], 1, 1)
            .estimate
            .median_usd,
        Some(4.0)
    );
    p.history = vec![history(None, "wish")];
    let d = digest::build(collected(vec![]), &p, &[], 1, 1);
    assert_eq!(
        (
            d.estimate.median_usd,
            d.estimate.samples,
            d.estimate.unpriced_runs
        ),
        (None, 0, 1)
    );
    assert!(digest::render(&d).contains("cannot be estimated"));
}

#[test]
fn compares_newest_record_per_problem_including_absent() {
    let prior = vec![
        run(
            "a",
            1,
            vec![metric("usage.x", 9.0, 10), metric("goal.gone", 4.0, 5)],
        ),
        run("b", 2, vec![metric("usage.x", 1.0, 3)]),
        run("c", 2, vec![metric("usage.x", 2.0, 4)]),
    ];
    let d = digest::build(
        collected(vec![event("usage.x", 5, None, "x")]),
        &period(),
        &prior,
        1,
        1,
    );
    assert_eq!(d.comparisons.len(), 2);
    let a = &d.comparisons[0];
    assert_eq!(
        (
            a.problem.as_str(),
            a.earlier_value,
            a.earlier_samples,
            a.current_value,
            a.current_samples,
            a.difference
        ),
        ("goal.gone", 4.0, 5, 0.0, 8, -4.0)
    );
    let b = &d.comparisons[1];
    assert_eq!(
        (
            b.earlier_run.as_str(),
            b.earlier_value,
            b.earlier_samples,
            b.current_value,
            b.current_samples,
            b.difference
        ),
        ("c", 2.0, 4, 5.0, 12, 3.0)
    );
    let text = digest::render(&d);
    assert!(
        text.contains("goal.gone")
            && text.contains("-4")
            && text.contains("samples: 5")
            && text.contains("samples: 8"),
        "{text}"
    );
}

fn ledger(rows: Vec<AgentRow>) -> Ledger {
    Ledger {
        session: String::new(),
        cwd: None,
        source: "fixture",
        subagent_files: 0,
        output_tokens_partial: false,
        totals: Tally::default(),
        by_agent: rows,
        instances: vec![],
        codex: vec![],
        codex_runs_without_usage: 0,
        warnings: vec![],
        copied_calls: 0,
        compactions: 0,
        skipped_lines: 0,
        claude_code_usd: None,
        prices: Prices {
            as_of: "",
            source: "",
        },
        thresholds: Thresholds {
            context_warn: 0,
            large_cache_write: 0,
            cache_gap_seconds: 0,
            large_read_bytes: 0,
        },
    }
}
fn row(role: &str, model: &str, calls: u64, priced: f64, unpriced: u64) -> AgentRow {
    AgentRow {
        agent_type: role.into(),
        model: model.into(),
        tally: Tally {
            calls,
            priced_usd: priced,
            usd: (unpriced == 0).then_some(priced),
            unpriced_calls: unpriced,
            ..Tally::default()
        },
    }
}
fn period_run(rows: Vec<AgentRow>) -> PeriodRun {
    PeriodRun {
        session: String::new(),
        cwd: None,
        skill: None,
        first_ms: None,
        last_ms: None,
        ledger: ledger(rows),
    }
}
#[test]
fn cost_table_aggregates_role_model_and_unpriced() {
    let mut p = period();
    p.runs = vec![
        period_run(vec![row("a", "m", 2, 2.0, 0), row("b", "m", 1, 1.0, 0)]),
        period_run(vec![row("a", "m", 3, 1.0, 1), row("a", "n", 1, 1.0, 0)]),
    ];
    let d = digest::build(collected(vec![]), &p, &[], 1, 1);
    assert_eq!(
        d.cost
            .iter()
            .map(|r| (r.role.as_str(), r.model.as_str()))
            .collect::<Vec<_>>(),
        [("a", "m"), ("a", "n"), ("b", "m")]
    );
    assert_eq!(
        (
            d.cost[0].calls,
            d.cost[0].priced_usd,
            d.cost[0].usd,
            d.cost[0].unpriced_calls
        ),
        (5, 3.0, None, 1)
    );
    let text = digest::render(&d);
    assert!(
        text.contains("unknown (1 unpriced)") && text.contains("a") && text.contains("m"),
        "{text}"
    );
}

#[test]
fn bounded_utf8_omissions_and_snapshots() {
    let mut events = vec![];
    for i in 0..15 {
        for j in 0..5 {
            let mut e = event(&format!("lint.{i:02}"), 1, None, &format!("{i}-{j}"));
            e.source.location = format!("{i}-{j}{}", "長".repeat(9000));
            e.title = "長".repeat(9000);
            e.snapshot = true;
            events.push(e);
        }
    }
    let d = digest::build(collected(events), &period(), &[], 1, 1);
    assert_eq!(d.problems.len(), 15);
    let text = digest::render(&d);
    assert!(text.len() <= MAX_DIGEST_BYTES, "{}", text.len());
    assert!(
        text.contains("5 problems omitted") && text.contains("20 sources omitted"),
        "{text}"
    );
    assert!(text.contains("current snapshot"));
    assert!(!text.to_lowercase().contains("saving"));
}

#[test]
fn extreme_values_and_untrusted_text_remain_bounded() {
    let mut events = vec![];
    for i in 0..10 {
        for j in 0..3 {
            let mut e = event(
                &format!("usage.{i}{}", "長".repeat(1000)),
                u64::MAX,
                Some(f64::MAX),
                &format!("{i}-{j}"),
            );
            e.title = "SAVING".repeat(500);
            e.source.location = "長".repeat(1000);
            e.snapshot = true;
            events.push(e);
        }
    }
    let mut input = collected(events);
    input.sources = (0..200)
        .map(|i| status(&format!("source{i}{}", "長".repeat(100)), u64::MAX))
        .collect();
    input.sources.push(SourceStatus {
        family: "hook".into(),
        state: "not logged".into(),
        samples: 0,
        note: String::new(),
    });
    let mut p = period();
    p.history.push(history(Some(f64::MAX), "wish"));
    let mut d = digest::build(input, &p, &[], i64::MAX, u32::MAX);
    d.estimate.method = "SAVINGS".repeat(100);
    let text = digest::render(&d);
    assert!(text.len() <= MAX_DIGEST_BYTES, "{}", text.len());
    assert!(text.contains("hook refusals: not logged") && text.contains("latest snapshot"));
    assert!(!text.to_lowercase().contains("saving"));
}
