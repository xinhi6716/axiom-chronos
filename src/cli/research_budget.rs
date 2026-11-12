use crate::{
    base::{self, Failure, Res},
    research_program::{
        self,
        budget::{Budget, RaiseReceipt},
        history, PROGRAM_TYPE,
    },
    seams::{PersonApproval, TtyApproval},
};
use std::path::{Path, PathBuf};

pub fn default_models() -> Vec<String> {
    ["argon", "argon", "astra", "astra", "fable", "fable"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

pub fn raise(store: &Path, cap: f64, approval: &dyn PersonApproval) -> Res<Option<RaiseReceipt>> {
    let run_id = store
        .file_name()
        .ok_or_else(|| Failure::new("invalid_argument", "store has no directory name"))?
        .to_string_lossy();
    let path = research_program::program_dir(store).join("budget.json");
    std::fs::metadata(&path).map_err(|e| Failure::new("io", format!("{}: {e}", path.display())))?;
    Budget::open(&path, &run_id, 0.0)?.raise(cap, approval)
}

pub(super) fn command(args: &[String]) -> i32 {
    let Some(action) = args.first().map(String::as_str) else {
        return super::usage("research-budget: missing command");
    };
    match action {
        "estimate" => {
            let models = match &args[1..] {
                [] => default_models(),
                [flag, value]
                    if flag == "--models"
                        && !value.is_empty()
                        && value.split(',').all(|s| !s.is_empty()) =>
                {
                    value.split(',').map(str::to_owned).collect()
                }
                _ => return super::usage("research-budget estimate: expected [--models a,b,c]"),
            };
            match base::data_home().and_then(|home| history::estimate(&home, PROGRAM_TYPE, &models))
            {
                Ok(estimate) => {
                    println!("{}", estimate.line());
                    0
                }
                Err(e) => {
                    eprintln!("axiom: {}", e.message);
                    1
                }
            }
        }
        "raise" => {
            let (mut store, mut cap, mut i) = (None, None, 1);
            while i < args.len() {
                match args[i].as_str() {
                    "--store" if store.is_none() => match super::value(args, &mut i, "--store") {
                        Ok(v) if !v.starts_with('-') => store = Some(PathBuf::from(v)),
                        _ => return super::usage("--store requires a directory"),
                    },
                    "--cap" if cap.is_none() => match super::value(args, &mut i, "--cap")
                        .ok()
                        .and_then(|v| v.parse::<f64>().ok())
                    {
                        Some(v) => cap = Some(v),
                        None => return super::usage("--cap requires a number"),
                    },
                    _ => return super::usage("research-budget raise: invalid arguments"),
                }
                i += 1;
            }
            let (Some(store), Some(cap)) = (store, cap) else {
                return super::usage("research-budget raise: --store and --cap are required");
            };
            match raise(&store, cap, &TtyApproval) {
                Ok(Some(receipt)) => {
                    println!("{}", serde_json::to_string(&receipt).unwrap());
                    0
                }
                Ok(None) => {
                    eprintln!("raise denied");
                    1
                }
                Err(e) => {
                    eprintln!("axiom: {}", e.message);
                    1
                }
            }
        }
        _ => super::usage("research-budget: unknown command"),
    }
}
