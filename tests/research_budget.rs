use axiom::research_program::{
    budget::{Budget, BudgetState},
    history::{self, ProviderSpend, RunSummary},
};
use axiom::seams::{sha256_hex, FakeApproval};
use std::{collections::BTreeMap, fs, io::Write};
use tempfile::tempdir;

fn models(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn run(id: &str, kind: &str, names: &[&str], usd: f64, unpriced: u64) -> RunSummary {
    RunSummary {
        run_id: id.into(),
        program_type: kind.into(),
        models: models(names),
        per_provider: BTreeMap::from([(
            "provider".into(),
            ProviderSpend {
                usd,
                priced_tokens: 100,
                unpriced_tokens: unpriced,
            },
        )]),
        completed: true,
        at: "2026-01-01".into(),
    }
}

#[test]
fn history_averages_matching_type_and_model_set_and_skips_bad_lines() {
    let dir = tempdir().unwrap();
    let home = dir.path();
    assert!(history::read(home).unwrap().is_empty());
    history::append(home, &run("1", "research", &["b", "a"], 2.0, 100)).unwrap();
    history::append(home, &run("2", "research", &["a", "b"], 6.0, 300)).unwrap();
    history::append(home, &run("3", "other", &["a", "b"], 500.0, 0)).unwrap();
    history::append(home, &run("4", "research", &["a"], 500.0, 0)).unwrap();
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(history::path(home))
            .unwrap(),
        "not json"
    )
    .unwrap();
    assert_eq!(history::read(home).unwrap().len(), 4);
    let estimate = history::estimate(home, "research", &models(&["b", "a"])).unwrap();
    assert_eq!(estimate.models, models(&["a", "b"]));
    assert_eq!(estimate.samples, 2);
    assert_eq!(estimate.priced_usd, Some(4.0));
    assert_eq!(estimate.per_provider["provider"].priced_usd, 4.0);
    assert_eq!(estimate.per_provider["provider"].unpriced_tokens, 200);
    assert!(estimate.per_provider["provider"].has_unpriced);
    assert_eq!(estimate.line(), "estimate from 2 past runs: provider $4.00 + unknown (200 tokens unpriced); priced total $4.00");
    let empty = history::estimate(home, "missing", &models(&["a"])).unwrap();
    assert_eq!(empty.samples, 0);
    assert!(empty.per_provider.is_empty());
    assert_eq!(empty.priced_usd, None);
    assert_eq!(empty.line(), "no history for missing with a: no estimate");
}

#[test]
fn unknown_charges_remain_separate_from_priced_spend() {
    let dir = tempdir().unwrap();
    let mut budget = Budget::open(&dir.path().join("budget.json"), "r", 10.0).unwrap();
    budget.charge("provider", Some(3.0), 20).unwrap();
    budget.charge("provider", None, 120).unwrap();
    let summary = budget.summary("research", &models(&["b", "a"]), false);
    assert_eq!(summary.models, models(&["a", "b"]));
    assert!(!summary.completed);
    assert_eq!(
        summary.per_provider["provider"],
        ProviderSpend {
            usd: 3.0,
            priced_tokens: 20,
            unpriced_tokens: 120
        }
    );
    history::append(dir.path(), &summary).unwrap();
    let estimate = history::estimate(dir.path(), "research", &models(&["a", "b"])).unwrap();
    assert_eq!(estimate.priced_usd, Some(3.0));
    assert!(estimate.line().contains("+ unknown (120 tokens unpriced)"));
}

#[test]
fn raises_require_approval_and_validate_values() {
    let dir = tempdir().unwrap();
    let mut budget = Budget::open(&dir.path().join("budget.json"), "run-1", 10.0).unwrap();
    let denied = FakeApproval::denying();
    assert!(budget.allows(10.0));
    assert!(!budget.allows(10.01));
    assert!(budget.raise(20.0, &denied).unwrap().is_none());
    assert_eq!(budget.cap_usd(), 10.0);
    assert!(budget.allows(10.0));
    assert!(!budget.allows(10.01));
    let approval = FakeApproval::approving();
    for value in [f64::NAN, f64::INFINITY, 10.0, 9.0, 1000.01] {
        assert_eq!(
            budget.raise(value, &approval).unwrap_err().kind,
            "invalid_argument"
        );
    }
    assert!(approval.asked().is_empty());
    let receipt = budget.raise(20.0, &approval).unwrap().unwrap();
    let digest = sha256_hex(b"run-1\x0020");
    assert_eq!(receipt.digest, digest);
    assert_eq!(receipt.from_usd, 10.0);
    assert_eq!(receipt.to_usd, 20.0);
    assert!(!receipt.approved_at.is_empty());
    let asked = approval.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].subject, "research:run-1");
    assert_eq!(asked[0].action, "raise_budget");
    assert_eq!(asked[0].digest, digest);
    assert!(!asked[0].summary.is_empty());
    assert_eq!(budget.cap_usd(), 20.0);
    assert!(budget.allows(20.0));
    assert!(!budget.allows(20.01));
}

#[test]
fn reopen_preserves_state_and_rejects_wrong_run() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("nested").join("budget.json");
    let mut budget = Budget::open(&path, "run", 10.0).unwrap();
    budget.charge("p", Some(3.0), 20).unwrap();
    budget.charge("p", None, 30).unwrap();
    budget
        .raise(25.0, &FakeApproval::approving())
        .unwrap()
        .unwrap();
    drop(budget);
    let reopened = Budget::open(&path, "run", 1.0).unwrap();
    assert_eq!(reopened.cap_usd(), 25.0);
    assert_eq!(reopened.priced_spent(), 3.0);
    assert_eq!(reopened.remaining_usd(), 22.0);
    assert!(reopened.allows(22.0));
    assert!(!reopened.allows(22.01));
    assert!(!reopened.exceeded());
    assert_eq!(
        reopened.summary("x", &[], true).per_provider["p"].unpriced_tokens,
        30
    );
    let state: BudgetState = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(state.initial_cap_usd, 10.0);
    assert_eq!(state.raises.len(), 1);
    assert_eq!(
        Budget::open(&path, "other", 100.0).unwrap_err().kind,
        "budget_mismatch"
    );
}

#[test]
fn tampered_receipt_is_ignored_and_previous_valid_raise_used() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("budget.json");
    let mut budget = Budget::open(&path, "run", 10.0).unwrap();
    let approval = FakeApproval::approving();
    budget.raise(20.0, &approval).unwrap();
    budget.raise(30.0, &approval).unwrap();
    let mut state: BudgetState = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state.raises[1].to_usd = 1000.0;
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    let reopened = Budget::open(&path, "run", 10.0).unwrap();
    assert_eq!(reopened.cap_usd(), 20.0);
    state.raises[0].to_usd = 900.0;
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    assert_eq!(Budget::open(&path, "run", 10.0).unwrap().cap_usd(), 10.0);
}

#[test]
fn exceeding_cap_and_failed_persistence_are_visible() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("budget.json");
    let mut budget = Budget::open(&path, "r", 10.0).unwrap();
    budget.charge("p", Some(11.0), 1).unwrap();
    assert!(budget.exceeded());
    assert!(!budget.allows(0.0));
    assert_eq!(budget.remaining_usd(), 0.0);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(budget.charge("p", Some(1.0), 1).is_err());
    assert_eq!(budget.priced_spent(), 11.0);
    assert!(budget.raise(20.0, &FakeApproval::approving()).is_err());
    assert_eq!(budget.cap_usd(), 10.0);
}

#[test]
fn default_models_follow_council_setting() {
    let home = tempdir().unwrap();
    let on = axiom::cli::research_budget::default_models_for(home.path()).unwrap();
    assert_eq!(on.iter().filter(|m| *m == "argon").count(), 11);
    axiom::council::settings::save(home.path(), &axiom::council::settings::CouncilSettings { argon_every_round: false }).unwrap();
    let off = axiom::cli::research_budget::default_models_for(home.path()).unwrap();
    assert_eq!(off, models(&["argon", "argon", "astra", "astra", "fable", "fable"]));
}

#[test]
fn estimate_mentions_argon_pool_only_when_present() {
    let home = tempdir().unwrap();
    let argon = models(&["argon"]);
    let estimate = axiom::research_program::history::estimate(home.path(), "research", &argon).unwrap();
    assert!(axiom::research_program::budget::estimate_line(&estimate, &argon).contains("Argon spend lands in the Argon pool"));
    assert!(!axiom::research_program::budget::estimate_line(&estimate, &models(&["astra"])).contains("Argon spend lands in the Argon pool"));
}
