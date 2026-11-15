use crate::{
    base::{self, Failure, Res},
    storage::{
        retention::{self, Mode},
        usage,
        worktree::{self, Removal, State},
        Ledger, Policy,
    },
};
use serde_json::json;
use std::{
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn failure(e: Failure) -> i32 {
    if e.kind == "invalid_argument" {
        return super::usage(e.message);
    }
    eprintln!("axiom: {}", e.message);
    1
}
pub fn command(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("clean") => clean(&args[1..]),
        Some("worktrees") => worktrees(&args[1..]),
        _ => listing(args),
    }
}
fn listing(args: &[String]) -> i32 {
    let (mut json, mut measure) = (false, false);
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--measure" => measure = true,
            _ => return super::usage(format!("storage: unrecognized argument: {arg}")),
        }
    }
    let home = base::axiom_home();
    let now = now();
    let result = (|| -> Res<_> {
        let ledger = Ledger::open(&home)?;
        ledger.reconcile(&home)?;
        if measure {
            ledger.measure_stale(0, Duration::from_secs(30), now)?;
        }
        let policy = Policy::load(&home)?;
        usage::summarize(&ledger, &home, &|e| {
            retention::is_reclaimable(e, &policy, now)
        })
    })();
    match result {
        Ok(u) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&u).unwrap());
            } else {
                print!("{}", usage::render_text(&u));
            }
            0
        }
        Err(e) => failure(e),
    }
}
fn clean(args: &[String]) -> i32 {
    let (mut json, mut yes, mut pressure) = (false, false, false);
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--yes" => yes = true,
            "--pressure" => pressure = true,
            _ => return super::usage(format!("storage clean: unrecognized argument: {arg}")),
        }
    }
    let home = base::axiom_home();
    let now = now();
    let result = (|| -> Res<_> {
        let policy = Policy::load(&home)?;
        retention::plan(
            &home,
            &policy,
            if pressure {
                Mode::Pressure
            } else {
                Mode::Routine
            },
            now,
        )
    })();
    match result {
        Ok(plan) => {
            if !json {
                for item in &plan.items {
                    println!(
                        "{} {} {} {} {}",
                        match item.action {
                            retention::Action::Delete => "delete",
                            retention::Action::Compress => "compress",
                        },
                        item.class.as_str(),
                        item.path.display(),
                        item.size_bytes
                            .map_or_else(|| "unmeasured".into(), |n| n.to_string()),
                        item.reason
                    );
                }
                println!(
                    "total: {} bytes ({} unmeasured)",
                    plan.reclaimable_bytes, plan.unmeasured
                );
            }
            let outcome = if yes {
                match retention::execute(&home, &plan, now, &AtomicBool::new(false)) {
                    Ok(outcome) => Some(outcome),
                    Err(e) => return failure(e),
                }
            } else {
                None
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&json!({"plan": plan, "outcome": outcome, "message": if yes { None } else { Some("nothing changed; run again with --yes") }})).unwrap());
            } else {
                if let Some(outcome) = outcome {
                    for record in outcome.records {
                        println!("{}: {}", record.path, record.outcome);
                    }
                } else {
                    println!("nothing changed; run again with --yes");
                }
            }
            0
        }
        Err(e) => failure(e),
    }
}
fn worktrees(args: &[String]) -> i32 {
    let home = base::axiom_home();
    let now = now();
    if args.first().is_some_and(|a| a == "remove") {
        let mut path = None;
        let mut yes = false;
        for arg in &args[1..] {
            match arg.as_str() {
                "--yes" => yes = true,
                s if s.starts_with('-') => {
                    return super::usage(format!("worktrees remove: unrecognized argument: {s}"))
                }
                _ if path.is_none() => path = Some(arg),
                _ => return super::usage("worktrees remove takes one path"),
            }
        }
        let Some(path) = path else {
            return super::usage("worktrees remove requires a path");
        };
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(e) => {
                    eprintln!("axiom: {e}");
                    return 1;
                }
            }
        };
        return match worktree::remove(&home, &path, yes, now) {
            Ok(Removal::Planned { worktree, caches }) => {
                println!(
                    "remove {} ({})",
                    worktree.path.display(),
                    worktree.state.as_str()
                );
                for cache in caches {
                    println!("cache: {}", cache.display());
                }
                println!("nothing changed; run again with --yes");
                0
            }
            Ok(Removal::Removed { records }) => {
                println!("removed {}", path.display());
                for r in records {
                    println!("{}: {}", r.path, r.outcome);
                }
                0
            }
            Err(e) => failure(e),
        };
    }
    let (mut json, mut repo, mut i) = (false, None, 0);
    while i < args.len() {
        match args[i].as_str() {
            "--json" => json = true,
            "--repo" => match super::value(args, &mut i, "--repo") {
                Ok(v) => repo = Some(v),
                Err(e) => return super::usage(e),
            },
            s => return super::usage(format!("worktrees: unrecognized argument: {s}")),
        }
        i += 1;
    }
    let result = (|| -> Res<_> {
        if let Some(repo) = repo {
            worktree::adopt(&home, Path::new(&repo))?;
        }
        worktree::scan(&home, now)
    })();
    match result {
        Ok(trees) => {
            if json {
                let trees: Vec<_> = trees
                    .iter()
                    .map(|w| {
                        let mut v = serde_json::to_value(w).unwrap();
                        v["cleanup_offer"] = json!(w.state == State::Merged);
                        v
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&trees).unwrap());
            } else {
                for w in trees {
                    println!(
                        "{} {}{}",
                        w.state.as_str(),
                        w.path.display(),
                        if w.state == State::Merged {
                            " (cleanup offer)"
                        } else {
                            ""
                        }
                    );
                    for cache in w.build_caches {
                        println!("  cache: {}", cache.display());
                    }
                }
            }
            0
        }
        Err(e) => failure(e),
    }
}
