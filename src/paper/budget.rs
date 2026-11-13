use crate::base::{Failure, Res};
use crate::engine::budgets::{self, Role};
use crate::paper::{models::PassModels, paper_dir, SpendEstimate};
use crate::research_program::{budget::Budget, history};
use crate::seams::sha256_hex;
use std::path::Path;

pub const PAPER_PROGRAM_TYPE: &str = "paper_four_pass";

pub fn estimate(home: &Path, models: &[String], pass_models: &PassModels) -> Res<SpendEstimate> {
    let est = history::estimate(home, PAPER_PROGRAM_TYPE, models)?;
    let per_member = models
        .iter()
        .map(|name| {
            (
                name.clone(),
                est.per_provider.get(name).map(|avg| avg.priced_usd),
            )
        })
        .collect();
    let unpriced = models
        .iter()
        .filter(|name| !est.per_provider.contains_key(*name))
        .cloned()
        .collect();
    let line = format!(
        "{}; pass models: {}",
        est.line(),
        PassModels::display(pass_models)
    );
    let mut sorted_models = models.to_vec();
    sorted_models.sort();
    let digest = sha256_hex(format!("{line}\0{}", sorted_models.join(",")).as_bytes());
    Ok(SpendEstimate {
        per_member,
        priced_usd: est.priced_usd,
        samples: est.samples,
        unpriced,
        line,
        digest,
    })
}

fn role(member: &str) -> Role {
    if member == "reviewer" {
        Role::Reviewer
    } else {
        Role::CouncilMember
    }
}

#[derive(Debug)]
pub struct PaperBudget {
    budget: Budget,
    fable_cap_usd: f64,
    latched: bool,
}

impl PaperBudget {
    pub fn open(store: &Path, run_id: &str, fable_cap_usd: f64) -> Res<Self> {
        let budget = Budget::open(&paper_dir(store).join("budget.json"), run_id, fable_cap_usd)?;
        let fable_cap_usd = budget.cap_usd();
        Ok(Self {
            budget,
            fable_cap_usd,
            latched: false,
        })
    }

    pub fn before_call(&self, store: &Path, home: &Path, member: &str, model: &str) -> Res<()> {
        if self.latched {
            return Err(Failure::new("budget_exceeded", "budget recording failed"));
        }
        if member == "fable" {
            let summary = self.budget.summary(PAPER_PROGRAM_TYPE, &[], false);
            let unpriced = summary
                .per_provider
                .get("fable")
                .is_some_and(|spend| spend.unpriced_tokens > 0);
            if self.fable_cap_usd - self.fable_spent() <= 0.0 || unpriced {
                return Err(Failure::new(
                    "budget_exceeded",
                    "fable budget exceeded or unpriced",
                ));
            }
        }
        budgets::check_model(store, home, role(member), "paper", model)
    }

    pub fn charge(
        &mut self,
        store: &Path,
        home: &Path,
        member: &str,
        model: &str,
        usd: Option<f64>,
        tokens: u64,
    ) -> Res<()> {
        let result = self.budget.charge(member, usd, tokens).and_then(|()| {
            budgets::record_model(store, home, role(member), model, usd, 0, tokens, "paper")
                .map(|_| ())
        });
        if result.is_err() {
            self.latched = true;
        }
        result
    }

    pub fn fable_spent(&self) -> f64 {
        self.budget
            .summary(PAPER_PROGRAM_TYPE, &[], false)
            .per_provider
            .get("fable")
            .map_or(0.0, |spend| spend.usd)
    }

    pub fn finish(&self, home: &Path, models: &[String], completed: bool) -> Res<()> {
        history::append(
            home,
            &self.budget.summary(PAPER_PROGRAM_TYPE, models, completed),
        )
    }
}
