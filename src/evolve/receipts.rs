use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub retries: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cost {
    pub claude_usd: Option<f64>,
    pub codex_usd: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Receipt {
    pub schema: u32,
    pub run_id: String,
    pub context_sources: Vec<String>,
    pub engine_build: String,
    pub role_routing: BTreeMap<String, String>,
    pub tools: Vec<String>,
    pub tests: Vec<TestResult>,
    pub retries: u64,
    pub cost: Cost,
    pub human_corrections: Vec<String>,
    pub adopted_artifacts: Vec<String>,
    pub rollback_commit: Option<String>,
}

fn io(error: impl std::fmt::Display) -> Failure {
    Failure::new("io", error.to_string())
}

fn validate_id(run_id: &str) -> Res<()> {
    let mut parts = Path::new(run_id).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        return Err(Failure::new("invalid_argument", "invalid run id"));
    }
    Ok(())
}

pub fn path(home: &Path, run_id: &str) -> PathBuf {
    home.join("evolve/receipts").join(format!("{run_id}.json"))
}

pub fn write(home: &Path, receipt: &Receipt) -> Res<PathBuf> {
    validate_id(&receipt.run_id)?;
    let file = path(home, &receipt.run_id);
    fs::create_dir_all(file.parent().expect("receipt has a parent")).map_err(io)?;
    let bytes = serde_json::to_vec_pretty(receipt).map_err(io)?;
    base::atomic_write(&file, &bytes).map_err(io)?;
    Ok(file)
}

pub fn read(home: &Path, run_id: &str) -> Res<Option<Receipt>> {
    validate_id(run_id)?;
    match fs::read(path(home, run_id)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(error)),
    }
}

pub fn list(home: &Path) -> Res<Vec<Receipt>> {
    let entries = match fs::read_dir(home.join("evolve/receipts")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(error)),
    };
    let mut receipts: Vec<Receipt> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    receipts.sort_by(|a, b| a.run_id.cmp(&b.run_id));
    Ok(receipts)
}

pub fn from_run(
    home: &Path,
    run: &super::history::RunRecord,
    sources: &[super::SourceStatus],
) -> Receipt {
    let mut receipt = Receipt {
        schema: 1,
        run_id: run.id.clone(),
        context_sources: sources.iter().map(|source| source.family.clone()).collect(),
        engine_build: crate::engine::build_id(),
        role_routing: BTreeMap::new(),
        tools: Vec::new(),
        tests: Vec::new(),
        retries: 0,
        cost: Cost {
            claude_usd: None,
            codex_usd: None,
        },
        human_corrections: Vec::new(),
        adopted_artifacts: Vec::new(),
        rollback_commit: Some(run.base_commit.clone()),
    };
    if let Some(goal) = run
        .session
        .as_deref()
        .and_then(|id| crate::goal::load(home, id).ok().flatten())
    {
        receipt.human_corrections = goal.amendments.clone();
        if let Ok(store) = crate::orch::store_for_goal(home, &goal) {
            if let Ok(records) = crate::orch::receipts::read_runs(&store) {
                let mut tests: BTreeMap<String, TestResult> = BTreeMap::new();
                for record in records {
                    use std::collections::btree_map::Entry;
                    match tests.entry(record.unit.clone()) {
                        Entry::Vacant(entry) => {
                            entry.insert(TestResult {
                                name: record.unit,
                                passed: record.status == "completed",
                                retries: 0,
                            });
                        }
                        Entry::Occupied(mut entry) => {
                            let test = entry.get_mut();
                            test.passed = record.status == "completed";
                            test.retries += 1;
                            receipt.retries += 1;
                        }
                    }
                }
                receipt.tests = tests.into_values().collect();
            }
        }
    }
    receipt
}
