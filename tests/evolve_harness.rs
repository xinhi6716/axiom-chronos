use axiom::evolve::{
    digest::{self, Digest, Estimate, Metric, Problem, MAX_DIGEST_BYTES, MAX_PROBLEMS},
    harness::{
        self, CheckKind, FailureClass as Class, ItemStatus, MechanicalCheck, RuleProposal,
        CHECKLIST,
    },
    launch, Source, SourceStatus,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn problem(id: &str, family: &str) -> Problem {
    Problem {
        id: id.into(),
        title: "problem".into(),
        family: family.into(),
        count: 1,
        cost_usd: None,
        unpriced: 0,
        snapshot: false,
        metric: Metric {
            value: 1.0,
            samples: 1,
        },
        sources: vec![],
    }
}
fn digest(problem: Problem) -> Digest {
    Digest {
        now_ms: 0,
        since_days: 7,
        start_ms: 0,
        problems: vec![problem],
        gaps: vec![],
        sources: vec![],
        cost: vec![],
        estimate: Estimate {
            median_usd: None,
            samples: 0,
            unpriced_runs: 0,
            method: String::new(),
        },
        comparisons: vec![],
    }
}
fn full_rule() -> RuleProposal {
    RuleProposal {
        guide: Some("agent guide".into()),
        check: Some(MechanicalCheck {
            kind: CheckKind::Test,
            id: "test".into(),
            failed_before: Some("failed".into()),
            passed_after: Some("passed".into()),
        }),
    }
}

#[test]
fn classifies_collector_patterns_and_unknowns() {
    for (id, family, expected) in [
        ("lint.issue", "lint", Class::MissingContext),
        ("hook.refusal", "hook", Class::UnsafeAction),
        ("orch.verify-failed", "orch", Class::BadOutput),
        ("orch.run-failed", "orch", Class::BadOutput),
        ("review.finding/major", "review", Class::BadOutput),
        ("other", "review.finding/major", Class::BadOutput),
        ("orch.re-review", "orch", Class::RepeatedLoop),
        ("handoff.missing", "handoff", Class::LostDecision),
        ("goal.blocked", "goal", Class::Unknown),
        ("usage.cost", "usage", Class::Unknown),
        ("orch.failed-extra", "orch", Class::Unknown),
        ("other", "other", Class::Unknown),
    ] {
        assert_eq!(harness::classify(&problem(id, family)), expected, "{id}");
    }
    assert_eq!(
        Class::ALL
            .iter()
            .map(|c| c.label())
            .collect::<BTreeSet<_>>()
            .len(),
        7
    );
}

#[test]
fn audit_requires_evidence_for_pass_and_preserves_order() {
    let mut entries = BTreeMap::new();
    let empty = harness::audit(&entries);
    assert_eq!(empty.len(), 12);
    assert!(empty
        .iter()
        .all(|i| i.status == ItemStatus::Unknown && i.evidence.is_none()));
    entries.insert(CHECKLIST[1].into(), (true, "map".into()));
    entries.insert(CHECKLIST[3].into(), (false, "gap".into()));
    let items = harness::audit(&entries);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.name, CHECKLIST[i]);
        match i {
            1 => {
                assert_eq!(item.status, ItemStatus::Passed);
                assert_eq!(item.evidence.as_deref(), Some("map"));
            }
            3 => {
                assert_eq!(item.status, ItemStatus::Gap);
                assert_eq!(item.evidence.as_deref(), Some("gap"));
            }
            _ => {
                assert_eq!(item.status, ItemStatus::Unknown);
                assert_eq!(item.evidence, None);
            }
        }
    }
}

#[test]
fn rule_requires_guide_check_and_before_after_evidence() {
    let full = full_rule();
    assert!(harness::judge_rule(&full).is_ok());
    let mut failures = Vec::new();
    let mut r = full.clone();
    r.check = None;
    failures.push(harness::judge_rule(&r).unwrap_err());
    let mut r = full.clone();
    r.guide = None;
    failures.push(harness::judge_rule(&r).unwrap_err());
    let mut r = full.clone();
    r.check.as_mut().unwrap().failed_before = None;
    failures.push(harness::judge_rule(&r).unwrap_err());
    let mut r = full.clone();
    r.check.as_mut().unwrap().passed_after = None;
    failures.push(harness::judge_rule(&r).unwrap_err());
    assert_eq!(failures.iter().collect::<BTreeSet<_>>().len(), 4);
    let mut r = full.clone();
    r.guide = Some("  \t".into());
    assert!(harness::judge_rule(&r).is_err());
    let mut r = full.clone();
    r.check.as_mut().unwrap().failed_before = Some(" \n".into());
    assert!(harness::judge_rule(&r).is_err());
    let mut r = full;
    r.check.as_mut().unwrap().passed_after = Some("  ".into());
    assert!(harness::judge_rule(&r).is_err());
}

#[test]
fn documentation_covers_contract() {
    let doc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/evolve-harness.md"),
    )
    .unwrap();
    for class in Class::ALL {
        assert!(doc.contains(class.label()));
        assert!(doc.contains(class.change_type()));
    }
    for item in CHECKLIST {
        assert!(doc.contains(item), "{item}");
    }
}

#[test]
fn digest_and_wish_include_harness_guidance() {
    let mut p = problem("hook.refusal", "hook");
    p.sources.push(Source {
        family: "hook".into(),
        location: "hook.log".into(),
        session: None,
        project: None,
        unit: None,
        ts_ms: None,
        key: "hook".into(),
    });
    let d = digest(p);
    let text = digest::render(&d);
    assert!(
        text.contains("class: UNSAFE ACTION; evidence: hook.log; suggestion: a permission gate")
    );
    let wish = launch::wish_text(&d, Path::new("repo"));
    assert!(wish.contains(launch::HARNESS_RULE));
    assert!(wish.contains(&format!(
        "{}\n{}",
        launch::EVIDENCE_RULE,
        launch::HARNESS_RULE
    )));
}

#[test]
fn digest_size_cap_holds_with_full_problems_and_sections() {
    let mut p = problem(&format!("hook.refusal{}", "x".repeat(200)), "hook");
    p.title = "x".repeat(200);
    p.count = u64::MAX;
    p.unpriced = u64::MAX;
    p.cost_usd = Some(f64::MAX);
    p.metric.samples = u64::MAX;
    p.sources = (0..3)
        .map(|i| Source {
            family: "hook".into(),
            location: "長".repeat(100),
            session: None,
            project: None,
            unit: None,
            ts_ms: None,
            key: i.to_string(),
        })
        .collect();
    let mut d = digest(p.clone());
    d.problems = vec![p; MAX_PROBLEMS];
    d.sources = (0..200)
        .map(|_| SourceStatus {
            family: "x".repeat(100),
            state: "x".repeat(100),
            samples: u64::MAX,
            note: String::new(),
        })
        .collect();
    d.cost = (0..100)
        .map(|_| digest::CostRow {
            role: "x".repeat(100),
            model: "x".repeat(100),
            calls: u64::MAX,
            priced_usd: f64::MAX,
            usd: None,
            unpriced_calls: u64::MAX,
        })
        .collect();
    d.comparisons = (0..100)
        .map(|_| digest::Comparison {
            problem: "x".repeat(100),
            earlier_run: "x".repeat(100),
            earlier_value: f64::MAX,
            earlier_samples: u64::MAX,
            current_value: f64::MAX,
            current_samples: u64::MAX,
            difference: f64::MAX,
        })
        .collect();
    let text = digest::render(&d);
    assert!(text.len() <= MAX_DIGEST_BYTES, "{}", text.len());
    assert_eq!(text.matches("  class: ").count(), MAX_PROBLEMS);
}
