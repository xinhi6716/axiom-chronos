use crate::modes::{
    quick::{self, Brief},
    receipts,
};
use std::path::PathBuf;

pub(crate) const MODES_USAGE: &str = "\
chat: conversation (UI: phase 4)\n\
quick task: axiom do\n\
wish: axiom wish\n\
research: axiom research\n\
routine: opt-in automation\n\
plaza: group discussion (UI: phase 4)";

pub(crate) fn modes_command(args: &[String]) -> i32 {
    if !args.is_empty() {
        return super::usage("modes takes no arguments");
    }
    println!("{MODES_USAGE}");
    0
}

pub(crate) fn do_command(args: &[String]) -> i32 {
    let mut project = None;
    let mut scope = Vec::new();
    let mut verify = None;
    let mut model = None;
    let mut review = false;
    let mut task = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" | "--verify" | "--model" => {
                let flag = &args[i];
                i += 1;
                let Some(value) = args
                    .get(i)
                    .filter(|v| !v.starts_with('-') && !v.trim().is_empty())
                else {
                    return super::usage(format!("{flag} requires a value"));
                };
                match flag.as_str() {
                    "--project" => project = Some(PathBuf::from(value)),
                    "--verify" => verify = Some(value.clone()),
                    _ => model = Some(value.clone()),
                }
            }
            "--scope" => {
                let start = i + 1;
                let mut end = start;
                while end < args.len() && !args[end].starts_with('-') {
                    end += 1;
                }
                // The final positional argument is the task when the scope is last.
                let scope_end = if end == args.len() && task.is_none() {
                    end.saturating_sub(1)
                } else {
                    end
                };
                if scope_end <= start {
                    return super::usage("--scope requires a path and a task");
                }
                for value in &args[start..scope_end] {
                    scope.extend(value.split(',').map(str::to_owned));
                }
                i = scope_end - 1;
            }
            "--review" => review = true,
            flag if flag.starts_with('-') => {
                return super::usage(format!("unrecognized option: {flag}"))
            }
            value if task.is_none() => task = Some(value.to_owned()),
            _ => return super::usage("do accepts one task"),
        }
        i += 1;
    }
    let brief = Brief {
        goal: task.unwrap_or_default(),
        scope,
        verify: verify.unwrap_or_default(),
        review,
    };
    if let Err(e) = brief.validate() {
        return super::usage(e);
    }
    let project = match project {
        Some(project) => project,
        None => match std::env::current_dir() {
            Ok(project) => project,
            Err(e) => {
                eprintln!("axiom: {e}");
                return 1;
            }
        },
    };
    let mut cfg = match quick::QuickConfig::from_env(&project) {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("axiom: {e}");
            return 1;
        }
    };
    if let Some(model) = model {
        cfg.model = model;
    }
    if !review {
        cfg.reviewer = None;
    }
    match quick::run_quick(&cfg, None, None, &brief, None) {
        Ok(out) => {
            println!(
                "{}\nstore {}",
                receipts::render(&out.receipt),
                out.store.display()
            );
            if out.passed {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("axiom: {e}");
            1
        }
    }
}
