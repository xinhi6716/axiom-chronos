use axiom::{
    engine::{
        budgets::{self, Role},
        kinds, policy,
    },
    orch::events,
    paper::{budget::*, models::PassModels, paper_dir},
    research_program::history::{self, ProviderSpend, RunSummary},
    seams::sha256_hex,
};
use std::{collections::BTreeMap, fs, path::Path};
use tempfile::tempdir;

fn models(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn assert_exceeded(budget: &PaperBudget, store: &Path, member: &str) {
    assert_eq!(
        budget
            .before_call(store, store, member, member)
            .unwrap_err()
            .kind,
        "budget_exceeded"
    );
}

#[test]
fn estimate_without_history_is_unpriced() {
    let dir = tempdir().unwrap();
    let names = models(&["zeta", "alpha"]);
    let expected = history::estimate(dir.path(), PAPER_PROGRAM_TYPE, &names).unwrap();
    let estimate = estimate(dir.path(), &names, &PassModels::default()).unwrap();
    assert_eq!(PAPER_PROGRAM_TYPE, "paper_four_pass");
    assert_eq!(estimate.samples, 0);
    assert_eq!(estimate.priced_usd, None);
    assert_eq!(
        estimate.per_member,
        BTreeMap::from([("alpha".into(), None), ("zeta".into(), None)])
    );
    let mut unpriced = estimate.unpriced.clone();
    unpriced.sort();
    assert_eq!(unpriced, models(&["alpha", "zeta"]));
    assert!(estimate.line.starts_with(&expected.line()));
    assert_eq!(
        estimate.digest,
        sha256_hex(format!("{}\0alpha,zeta", estimate.line).as_bytes())
    );
}

#[test]
fn estimate_maps_matching_history_and_uses_sorted_models_for_digest() {
    let dir = tempdir().unwrap();
    let names = models(&["zeta", "alpha", "missing"]);
    for (id, alpha, zeta) in [("one", 2.0, 4.0), ("two", 6.0, 8.0)] {
        history::append(
            dir.path(),
            &RunSummary {
                run_id: id.into(),
                program_type: PAPER_PROGRAM_TYPE.into(),
                models: names.clone(),
                per_provider: BTreeMap::from([
                    (
                        "alpha".into(),
                        ProviderSpend {
                            usd: alpha,
                            priced_tokens: 20,
                            unpriced_tokens: 0,
                        },
                    ),
                    (
                        "zeta".into(),
                        ProviderSpend {
                            usd: zeta,
                            priced_tokens: 30,
                            unpriced_tokens: 5,
                        },
                    ),
                    (
                        "extra".into(),
                        ProviderSpend {
                            usd: 1.0,
                            priced_tokens: 10,
                            unpriced_tokens: 0,
                        },
                    ),
                ]),
                completed: true,
                at: "2026-01-01".into(),
            },
        )
        .unwrap();
    }
    let expected = history::estimate(dir.path(), PAPER_PROGRAM_TYPE, &names).unwrap();
    let actual = estimate(dir.path(), &names, &PassModels::default()).unwrap();
    assert_eq!(
        actual.per_member,
        BTreeMap::from([
            ("alpha".into(), Some(4.0)),
            ("zeta".into(), Some(6.0)),
            ("missing".into(), None),
        ])
    );
    assert_eq!(actual.unpriced, models(&["missing"]));
    assert_eq!(actual.samples, 2);
    assert_eq!(actual.priced_usd, expected.priced_usd);
    assert!(actual.line.starts_with(&expected.line()));
    assert_eq!(
        actual.digest,
        sha256_hex(format!("{}\0alpha,missing,zeta", actual.line).as_bytes())
    );
    assert_eq!(
        estimate(
            dir.path(),
            &models(&["missing", "alpha", "zeta"]),
            &PassModels::default()
        )
        .unwrap()
        .digest,
        actual.digest
    );
}

#[test]
fn fresh_budget_allows_call_without_policy() {
    let dir = tempdir().unwrap();
    let budget = PaperBudget::open(dir.path(), "r", 1.0).unwrap();
    assert!(budget
        .before_call(dir.path(), dir.path(), "fable", "fable")
        .is_ok());
    assert!(budget
        .before_call(dir.path(), dir.path(), "astra", "astra")
        .is_ok());
    assert!(budget
        .before_call(dir.path(), dir.path(), "reviewer", "reviewer")
        .is_ok());
}

#[test]
fn zero_fable_cap_refuses_before_call() {
    let dir = tempdir().unwrap();
    let budget = PaperBudget::open(dir.path(), "r", 0.0).unwrap();
    assert_exceeded(&budget, dir.path(), "fable");
}

#[test]
fn zero_role_budgets_refuse_before_call() {
    let dir = tempdir().unwrap();
    policy::set(dir.path(), "budget.council_member", "0").unwrap();
    policy::set(dir.path(), "budget.reviewer", "0").unwrap();
    let budget = PaperBudget::open(dir.path(), "r", 1.0).unwrap();
    assert_exceeded(&budget, dir.path(), "astra");
    assert_exceeded(&budget, dir.path(), "reviewer");
}

#[test]
fn fable_spend_accumulates_across_passes_without_blocking_other_members() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    let mut budget = PaperBudget::open(store, "r", 1.0).unwrap();
    budget
        .charge(store, store, "fable", "fable", Some(0.4), 10)
        .unwrap();
    budget
        .charge(store, store, "fable", "fable", Some(0.4), 20)
        .unwrap();
    assert_eq!(budget.fable_spent(), 0.8);
    assert!(budget.before_call(store, store, "fable", "fable").is_ok());
    budget
        .charge(store, store, "fable", "fable", Some(0.3), 30)
        .unwrap();
    assert_exceeded(&budget, store, "fable");
    assert!(budget.before_call(store, store, "argon", "argon").is_ok());
}

#[test]
fn unknown_council_spend_blocks_and_records_reason() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    policy::set(store, "budget.council_member", "10").unwrap();
    let mut budget = PaperBudget::open(store, "r", 10.0).unwrap();
    budget
        .charge(store, store, "argon", "argon", None, 10)
        .unwrap();
    assert_exceeded(&budget, store, "argon");
    assert!(events::read_events(store, 0).unwrap().iter().any(|e| e.kind
        == kinds::BUDGET_EXCEEDED
        && e.data["role"] == "council_member"
        && e.data["reason"] == "unknown_spend"
        && e.data["call"] == "paper"));
}

#[test]
fn unknown_fable_spend_blocks_fable() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    let mut budget = PaperBudget::open(store, "r", 10.0).unwrap();
    budget
        .charge(store, store, "fable", "fable", None, 10)
        .unwrap();
    assert_exceeded(&budget, store, "fable");
}

#[test]
fn charges_record_review_and_council_roles() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    let mut budget = PaperBudget::open(store, "r", 10.0).unwrap();
    budget
        .charge(store, store, "reviewer", "review-model", Some(0.2), 20)
        .unwrap();
    budget
        .charge(store, store, "argon", "argon-model", Some(0.3), 30)
        .unwrap();
    let rows: Vec<_> = events::read_events(store, 0)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == kinds::ROLE_SPEND)
        .collect();
    assert_eq!(rows.len(), 2);
    for (row, role, model, usd) in [
        (&rows[0], Role::Reviewer, "review-model", 0.2),
        (&rows[1], Role::CouncilMember, "argon-model", 0.3),
    ] {
        assert_eq!(row.data["role"], role.name());
        assert_eq!(row.data["model"], model);
        assert_eq!(row.data["usd"], usd);
        assert_eq!(row.data["call"], "paper");
    }
    assert_eq!(budgets::spent(store, Role::Reviewer).unwrap().usd, 0.2);
    assert_eq!(budgets::spent(store, Role::CouncilMember).unwrap().usd, 0.3);
}

#[test]
fn reopening_preserves_fable_spend_and_original_cap_and_rejects_other_run() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    let mut budget = PaperBudget::open(store, "r", 1.0).unwrap();
    budget
        .charge(store, store, "fable", "fable", Some(0.4), 10)
        .unwrap();
    drop(budget);
    let budget = PaperBudget::open(store, "r", 0.1).unwrap();
    assert_eq!(budget.fable_spent(), 0.4);
    assert!(budget.before_call(store, store, "fable", "fable").is_ok());
    assert_eq!(
        PaperBudget::open(store, "other", 10.0).unwrap_err().kind,
        "budget_mismatch"
    );
}

#[test]
fn failed_save_latches_all_later_calls() {
    let dir = tempdir().unwrap();
    let store = dir.path();
    let mut budget = PaperBudget::open(store, "r", 10.0).unwrap();
    let path = paper_dir(store).join("budget.json");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(budget
        .charge(store, store, "argon", "argon", Some(0.1), 10)
        .is_err());
    for member in ["fable", "argon", "reviewer"] {
        assert_exceeded(&budget, store, member);
    }
}

#[test]
fn finish_appends_summary_and_enables_future_estimates() {
    for completed in [false, true] {
        let dir = tempdir().unwrap();
        let store = dir.path();
        let names = models(&["fable", "argon"]);
        let mut budget = PaperBudget::open(store, "run-1", 10.0).unwrap();
        budget
            .charge(store, store, "fable", "fable", Some(0.4), 12)
            .unwrap();
        budget
            .charge(store, store, "argon", "argon", Some(0.2), 23)
            .unwrap();
        budget
            .charge(store, store, "argon", "argon", None, 7)
            .unwrap();
        budget.finish(store, &names, completed).unwrap();
        let records = history::read(store).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.run_id, "run-1");
        assert_eq!(record.program_type, "paper_four_pass");
        assert_eq!(record.completed, completed);
        assert_eq!(
            record.per_provider,
            BTreeMap::from([
                (
                    "fable".into(),
                    ProviderSpend {
                        usd: 0.4,
                        priced_tokens: 12,
                        unpriced_tokens: 0
                    }
                ),
                (
                    "argon".into(),
                    ProviderSpend {
                        usd: 0.2,
                        priced_tokens: 23,
                        unpriced_tokens: 7
                    }
                ),
            ])
        );
        assert_eq!(
            estimate(store, &names, &PassModels::default())
                .unwrap()
                .samples,
            1
        );
    }
}
