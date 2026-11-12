use crate::base::{atomic_write, now_iso, Failure, Res};
use crate::research_program::history::{ProviderSpend, RunSummary};
use crate::seams::{sha256_hex, ApprovalRequest, Decision, PersonApproval};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RaiseReceipt {
    pub run_id: String,
    pub from_usd: f64,
    pub to_usd: f64,
    pub digest: String,
    pub approved_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BudgetState {
    pub run_id: String,
    pub initial_cap_usd: f64,
    pub spent: BTreeMap<String, ProviderSpend>,
    pub raises: Vec<RaiseReceipt>,
}

#[derive(Debug)]
pub struct Budget {
    path: PathBuf,
    state: BudgetState,
}

fn digest(run_id: &str, to_usd: f64) -> String {
    sha256_hex(format!("{run_id}\0{to_usd}").as_bytes())
}

fn persist(path: &Path, state: &BudgetState) -> Res<()> {
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|e| Failure::new("json", e.to_string()))?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| Failure::new("io", format!("{}: {e}", parent.display())))?;
    }
    atomic_write(path, &bytes).map_err(|e| Failure::new("io", format!("{}: {e}", path.display())))
}

impl Budget {
    pub fn open(path: &Path, run_id: &str, initial_cap_usd: f64) -> Res<Self> {
        let state = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<BudgetState>(&bytes)
                .map_err(|e| Failure::new("json", e.to_string()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !initial_cap_usd.is_finite() || initial_cap_usd < 0.0 {
                    return Err(Failure::new(
                        "invalid_argument",
                        "initial cap must be finite and nonnegative",
                    ));
                }
                let state = BudgetState {
                    run_id: run_id.to_owned(),
                    initial_cap_usd,
                    spent: BTreeMap::new(),
                    raises: Vec::new(),
                };
                persist(path, &state)?;
                state
            }
            Err(e) => return Err(Failure::new("io", format!("{}: {e}", path.display()))),
        };
        if state.run_id != run_id {
            return Err(Failure::new(
                "budget_mismatch",
                "budget belongs to a different run",
            ));
        }
        Ok(Self {
            path: path.to_owned(),
            state,
        })
    }

    pub fn cap_usd(&self) -> f64 {
        self.state
            .raises
            .iter()
            .rev()
            .find(|receipt| {
                receipt.run_id == self.state.run_id
                    && receipt.digest == digest(&receipt.run_id, receipt.to_usd)
            })
            .map_or(self.state.initial_cap_usd, |receipt| receipt.to_usd)
    }
    pub fn priced_spent(&self) -> f64 {
        self.state.spent.values().map(|spend| spend.usd).sum()
    }
    pub fn remaining_usd(&self) -> f64 {
        (self.cap_usd() - self.priced_spent()).max(0.0)
    }
    pub fn allows(&self, estimate_usd: f64) -> bool {
        estimate_usd.is_finite()
            && estimate_usd >= 0.0
            && self.priced_spent() + estimate_usd <= self.cap_usd()
    }
    pub fn exceeded(&self) -> bool {
        self.priced_spent() > self.cap_usd()
    }

    pub fn charge(&mut self, provider: &str, usd: Option<f64>, tokens: u64) -> Res<()> {
        if usd.is_some_and(|usd| !usd.is_finite() || usd < 0.0) {
            return Err(Failure::new(
                "invalid_argument",
                "charge must be finite and nonnegative",
            ));
        }
        let mut next = self.state.clone();
        let spend = next.spent.entry(provider.to_owned()).or_default();
        match usd {
            Some(usd) => {
                spend.usd += usd;
                spend.priced_tokens = spend
                    .priced_tokens
                    .checked_add(tokens)
                    .ok_or_else(|| Failure::new("invalid_argument", "token count overflow"))?;
            }
            None => {
                spend.unpriced_tokens = spend
                    .unpriced_tokens
                    .checked_add(tokens)
                    .ok_or_else(|| Failure::new("invalid_argument", "token count overflow"))?
            }
        }
        if !spend.usd.is_finite() {
            return Err(Failure::new("invalid_argument", "charge overflow"));
        }
        persist(&self.path, &next)?;
        self.state = next;
        Ok(())
    }

    pub fn raise(
        &mut self,
        to_usd: f64,
        approval: &dyn PersonApproval,
    ) -> Res<Option<RaiseReceipt>> {
        let from_usd = self.cap_usd();
        if !to_usd.is_finite() || to_usd <= from_usd || to_usd > 1000.0 {
            return Err(Failure::new(
                "invalid_argument",
                "new cap must exceed the current cap and be at most $1000",
            ));
        }
        let digest = digest(&self.state.run_id, to_usd);
        let request = ApprovalRequest {
            subject: format!("research:{}", self.state.run_id),
            action: "raise_budget".into(),
            summary: format!("Raise research budget from ${from_usd:.2} to ${to_usd:.2}"),
            digest: digest.clone(),
        };
        if approval.decide(&request) == Decision::Denied {
            return Ok(None);
        }
        let receipt = RaiseReceipt {
            run_id: self.state.run_id.clone(),
            from_usd,
            to_usd,
            digest,
            approved_at: now_iso(),
        };
        let mut next = self.state.clone();
        next.raises.push(receipt.clone());
        persist(&self.path, &next)?;
        self.state = next;
        Ok(Some(receipt))
    }

    pub fn summary(&self, program_type: &str, models: &[String], completed: bool) -> RunSummary {
        let mut models = models.to_vec();
        models.sort();
        RunSummary {
            run_id: self.state.run_id.clone(),
            program_type: program_type.to_owned(),
            models,
            per_provider: self.state.spent.clone(),
            completed,
            at: now_iso(),
        }
    }
}
