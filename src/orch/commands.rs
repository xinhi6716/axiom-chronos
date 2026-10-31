use super::*;
use crate::base::{Failure, Res};
use std::path::Path;

pub fn wait_mutation(args: &[String]) -> bool {
    let mut args = args
        .iter()
        .map(String::as_str)
        .filter(|a| !["--json", "--force"].contains(a));
    args.next() == Some("wait") && args.next() != Some("show")
}

pub fn run_extra(store: &Path, args: &[String], json: bool) -> Option<Output> {
    let cmd = args.first()?.as_str();
    if !["ready", "accept", "fulltest", "review", "evidence", "wait"].contains(&cmd) {
        return None;
    }
    let result = (|| -> Res<(i32, String, serde_json::Value)> {
        let invalid = |s: &str| Failure::new("invalid_argument", s);
        let option = |name: &str| -> Res<&str> {
            let i = args
                .iter()
                .position(|x| x == name)
                .ok_or_else(|| invalid(&format!("missing {name}")))?;
            args.get(i + 1)
                .filter(|v| !v.starts_with("--"))
                .map(String::as_str)
                .ok_or_else(|| invalid(&format!("{name} requires a value")))
        };
        match cmd {
            "wait" => {
                let action = args.get(1).map(String::as_str).unwrap_or("");
                let state = match action {
                    "migrate" => migrate_wait(store)?,
                    "product-checkpoint" => product_checkpoint(store, option("--run")?)?,
                    "product-transition" => product_transition(
                        store, option("--run")?,
                        option("--round")?.parse().map_err(|_| invalid("invalid product round"))?,
                        option("--verifications")?.split(',').map(str::to_owned).collect(),
                        option("--assessment")?, option("--action")?,
                    )?,
                    "prepare" => {
                        if args.iter().any(|a| a == "--outputs" || a.starts_with("--outputs=")) { return Err(invalid("--outputs override is not permitted")); }
                        super::with_lock(store, false, || {
                        let task = option("--task")?;
                        let repo = crate::base::require_directory(option("--repo")?)?;
                        let state = super::wait::read(store)?;
                        let old = current_checkpoint(&state, task);
                        let min_rounds_count = args.iter().filter(|a| *a == "--min-rounds").count();
                        if min_rounds_count > 1 {
                            return Err(invalid("--min-rounds may only be specified once"));
                        }
                        let min_rounds = if min_rounds_count == 1 {
                            option("--min-rounds")?.parse::<u32>()
                                .ok().filter(|n| *n > 0)
                                .ok_or_else(|| invalid("--min-rounds requires a positive u32"))?
                        } else {
                            old.as_ref().map_or(5, |c| c.min_rounds)
                        };
                        let units = read_units(store)?;
                        let unit = units
                            .iter()
                            .find(|u| u.id == task)
                            .ok_or_else(|| invalid("unknown task"))?;
                        let brief = unit_brief(store, task)?;
                        let outputs = brief.outputs.clone().ok_or_else(|| invalid("new engineering contract requires OUTPUTS"))?;
                        let mut dependencies = vec![];
                        if store.join("modules.md").exists() {
                            let text = std::fs::read_to_string(store.join("modules.md"))
                                .map_err(|e| Failure::new("io", e.to_string()))?;
                            let map = parse_map(&text).map_err(|e| invalid(&e))?;
                            if let Some(module) = map.modules.iter().find(|m| m.id == unit.track) {
                                dependencies = units
                                    .iter()
                                    .filter(|u| module.depends.contains(&u.track))
                                    .map(|u| u.id.clone())
                                    .collect();
                            }
                        }
                        let dependency_versions = dependencies
                            .iter()
                            .map(|task| {
                                current_checkpoint(&state, task)
                                    .map(|c| (task.clone(), c.evidence_version))
                                    .ok_or_else(|| {
                                        invalid("dependency requires an accepted checkpoint")
                                    })
                            })
                            .collect::<Res<_>>()?;
                        super::wait::prepare_locked(store, state, Checkpoint {
                            task:task.into(),module:unit.track.clone(),round:1,min_rounds,
                            owner:"terra".into(),dependencies,dependency_versions,outputs,
                            evidence:vec!["host execution evidence pending; checkpoint is not a result".into()],
                            next_action:"Terra HIGH, Sol HIGH, Rust package, independent Astra HIGH, explicit core verdict".into(),
                            issue:option("--issue")?.into(),proposal:option("--proposal")?.into(),
                            evidence_version:1,contract:contract_identity(store,task)?,
                            repo:repo.to_string_lossy().into_owned(),severity:if args.iter().any(|a| a == "--severity") { option("--severity")?.into() } else { "normal".into() },findings:vec![],
                        })
                        })?
                    }
                    "enroll" => {
                        super::with_lock(store, false, || {
                            if !store.join("units.tsv").is_file() {
                                return Err(invalid("enrollment requires a registered unit store"));
                            }
                            super::read_units(store)?;
                            super::wait::read(store)?;
                            crate::base::atomic_write(&store.join("wait-required"), b"1\n")
                                .map_err(|e| Failure::new("io", e.to_string()))
                        })?;
                        wait_state(store)?
                    }
                    "verdict" => {
                        let decision = option("--decision")?;
                        if !["approve", "reject"].contains(&decision) {
                            return Err(invalid("decision requires approve or reject"));
                        }
                        core_verdict(
                            store,
                            option("--task")?,
                            option("--verification")?,
                            decision == "approve",
                            option("--assessment")?,
                        )?
                    }
                    "complete" => {
                        complete_round(store, option("--task")?, option("--verification")?)?
                    }
                    "show" => {
                        let state = wait_state(store)?;
                        reconcile_release(store, &state)?;
                        state
                    }
                    "checkpoint" | "event" | "recover" => {
                        let data = std::fs::read(option("--file")?)
                            .map_err(|e| Failure::new("io", e.to_string()))?;
                        if action == "checkpoint" {
                            checkpoint(
                                store,
                                serde_json::from_slice(&data)
                                    .map_err(|e| invalid(&format!("invalid checkpoint: {e}")))?,
                            )?
                        } else if action == "recover" {
                            recover_dispatch(
                                store,
                                serde_json::from_slice(&data)
                                    .map_err(|e| invalid(&format!("invalid recovery: {e}")))?,
                                option("--confirmed")?,
                            )?
                        } else {
                            wait_event(
                                store,
                                serde_json::from_slice(&data)
                                    .map_err(|e| invalid(&format!("invalid event: {e}")))?,
                            )?
                        }
                    }
                    _ => {
                        return Err(invalid(
                            "wait requires enroll, show, prepare, checkpoint, event, recover, verdict, complete, migrate, product-checkpoint or product-transition",
                        ))
                    }
                };
                let value = serde_json::to_value(state).map_err(|e| invalid(&e.to_string()))?;
                Ok((0, value.to_string(), value))
            }
            "ready" => {
                let ids = ready_units(store)?;
                let waiting = waiting_units(store)?;
                let mut text: String = if ids.is_empty() {
                    "(none ready)".into()
                } else {
                    ids.join("\n")
                };
                for w in &waiting {
                    text.push_str(&format!("\n{}\t{}\t{}", w.unit, w.state, w.next));
                }
                Ok((
                    0,
                    text,
                    serde_json::json!({"ready": ids, "waiting": waiting}),
                ))
            }
            "accept" | "fulltest" => {
                let repo = Path::new(option("--repo")?);
                let (name, result) = if cmd == "accept" {
                    let name = args
                        .get(1)
                        .filter(|s| !s.starts_with("--"))
                        .ok_or_else(|| invalid("missing module"))?;
                    (name.as_str(), record_accept(store, name, repo)?)
                } else {
                    let command = if args.iter().any(|s| s == "--command") {
                        Some(option("--command")?)
                    } else {
                        None
                    };
                    ("fulltest", record_full(store, repo, command)?)
                };
                let tail: String = result
                    .output
                    .chars()
                    .rev()
                    .take(2000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let text = format!("{name}\texit={}\n{}", result.exit_code, tail.trim_end());
                Ok((
                    if result.exit_code == 0 { 0 } else { 1 },
                    text,
                    serde_json::json!({"exit_code":result.exit_code,"output":tail}),
                ))
            }
            "review" => {
                let repo = Path::new(option("--repo")?);
                let reviewer = option("--reviewer")?;
                let path = option("--verdict-file")?;
                let units: Vec<String> = args
                    .iter()
                    .skip(1)
                    .take_while(|s| !s.starts_with("--"))
                    .cloned()
                    .collect();
                if units.is_empty() {
                    return Err(invalid("missing units"));
                }
                let data =
                    std::fs::read_to_string(path).map_err(|e| Failure::new("io", e.to_string()))?;
                let v: serde_json::Value =
                    serde_json::from_str(&data).map_err(|e| invalid(&e.to_string()))?;
                let verdict = v
                    .get("verdict")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing verdict"))?;
                let findings = v
                    .get("findings")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| invalid("missing findings"))?;
                let pass = verdict.eq_ignore_ascii_case("pass")
                    && !findings.iter().any(|f| {
                        f.get("severity").and_then(|s| s.as_str()).is_some_and(|s| {
                            s.eq_ignore_ascii_case("blocker") || s.eq_ignore_ascii_case("major")
                        })
                    });
                let rows = record_review(store, &units, reviewer, pass, repo)?;
                let text = rows
                    .iter()
                    .map(|(u, s)| format!("{u}\t{s}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((0, text, serde_json::json!(rows)))
            }
            _ => {
                let unit = args.get(1).ok_or_else(|| invalid("missing unit"))?;
                if !read_units(store)?.iter().any(|u| u.id == *unit) {
                    return Err(Failure::new("not_found", format!("unit {unit} not found")));
                }
                let rows = evidence(store, unit)?;
                let text = rows
                    .iter()
                    .map(|r| {
                        format!(
                            "{}\t{}\t{}\t{}\t{}",
                            r.kind, r.command, r.exit_code, r.commit, r.ts
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok((0, text, serde_json::json!(rows)))
            }
        }
    })();
    Some(match result {
        Ok((exit_code, text, value)) => Output {
            exit_code,
            stdout: format!("{}\n", if json { value.to_string() } else { text }),
            stderr: String::new(),
        },
        Err(e) => Output {
            exit_code: if e.kind == "not_found" { 2 } else { 1 },
            stdout: String::new(),
            stderr: format!("error: {}\n", e.message),
        },
    })
}
