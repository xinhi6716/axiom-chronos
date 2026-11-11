use super::*;
use crate::base::{self, Failure, Res};
use std::collections::BTreeSet;
use std::path::Path;

fn cut(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

pub fn run_extra(store: &Path, args: &[String], json: bool) -> Option<Output> {
    let cmd = args.first()?.as_str();
    if ![
        "ready",
        "verify",
        "accept",
        "fulltest",
        "review",
        "evidence",
        "release",
        "report",
        "review-pack",
        "handoff",
        "resume",
        "waive",
    ]
    .contains(&cmd)
    {
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
        let has = |name: &str| args.iter().any(|s| s == name);
        match cmd {
            "report" => {
                let repo = Path::new(option("--repo")?);
                let module = if args.iter().any(|s| s == "--module") {
                    Some(option("--module")?)
                } else {
                    None
                };
                let text = digest(store, repo, module)?;
                Ok((0, text.clone(), serde_json::json!({"digest":text})))
            }
            "handoff" => {
                let note = record_handoff(store, args)?;
                Ok((
                    0,
                    format!(
                        "handoff recorded for {}; return your report now",
                        note.module.as_deref().unwrap_or("the whole wish")
                    ),
                    serde_json::to_value(&note).unwrap(),
                ))
            }
            "resume" => {
                let module = if has("--module") {
                    Some(option("--module")?)
                } else {
                    None
                };
                let repo = if has("--repo") {
                    Some(Path::new(option("--repo")?))
                } else {
                    None
                };
                let text = resume_text(store, repo, module)?;
                Ok((0, text.clone(), serde_json::json!({"resume":text})))
            }
            "review-pack" => {
                let repo = Path::new(option("--repo")?);
                let unit = args
                    .get(1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or_else(|| invalid("missing unit"))?;
                let pack = review_pack(store, unit, repo)?;
                let text = format!(
                    "{}\tfiles={}\tread-directly={}\tbytes={}",
                    pack.path.display(),
                    pack.files,
                    pack.omitted.len(),
                    pack.bytes
                );
                Ok((
                    0,
                    text,
                    serde_json::json!({"pack":pack.path,"tree":pack.tree,"files":pack.files,"read_directly":pack.omitted,"bytes":pack.bytes}),
                ))
            }
            "release" => {
                let unit = args
                    .get(1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or_else(|| invalid("missing unit"))?;
                release_claim(store, unit, option("--attempt")?)?;
                Ok((
                    0,
                    format!("released {unit}"),
                    serde_json::json!({"released":unit}),
                ))
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
            "verify" => {
                let repo = Path::new(option("--repo")?);
                let unit = args
                    .get(1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or_else(|| invalid("missing unit"))?;
                if !read_units(store)?.iter().any(|u| u.id == *unit) {
                    return Err(Failure::new("not_found", format!("unit {unit} not found")));
                }
                let result = record_verify(store, unit, repo)?;
                let (pack, pack_error) = if result.exit_code == 0 {
                    match review_pack(store, unit, repo) {
                        Ok(info) => (Some(info.path), None),
                        Err(err) => (None, Some(err.message)),
                    }
                } else {
                    (None, None)
                };
                let next = next_step_for(store, unit)?;
                let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
                let tail: String = result
                    .output
                    .chars()
                    .rev()
                    .take(2000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let pack_text = pack
                    .as_ref()
                    .map(|p| format!("\tpack={}", p.display()))
                    .unwrap_or_default();
                let mut value = serde_json::json!({"exit_code":result.exit_code,"state":state,"next":next,"output":tail,"pack":pack});
                if let Some(error) = pack_error {
                    value["pack_error"] = serde_json::json!(error);
                }
                Ok((
                    if result.exit_code == 0 { 0 } else { 1 },
                    format!(
                        "{unit}\texit={}\t{state}\t{next}{pack_text}\n{}",
                        result.exit_code,
                        tail.trim_end()
                    ),
                    value,
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
                } else if has("--external") {
                    if has("--command") {
                        return Err(invalid("--external cannot be combined with --command"));
                    }
                    let exit_code = option("--exit")?
                        .parse::<i32>()
                        .map_err(|_| invalid("--exit must be an integer"))?;
                    let log = Path::new(option("--log")?);
                    let label = option("--external")?;
                    (
                        "fulltest",
                        record_external_full(store, repo, label, exit_code, log)?,
                    )
                } else if has("--exit") || has("--log") {
                    return Err(invalid("--exit and --log require --external"));
                } else {
                    let command = if has("--command") {
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
                if reviewer != "chronos:reviewer" {
                    return Err(invalid("reviewer must be chronos:reviewer"));
                }
                let mut units = Vec::new();
                let mut i = 1;
                while i < args.len() {
                    if ["--repo", "--reviewer", "--verdict-file"].contains(&args[i].as_str()) {
                        i += 2;
                    } else {
                        if !args[i].starts_with("--") {
                            units.push(args[i].clone());
                        }
                        i += 1;
                    }
                }
                if units.is_empty() {
                    return Err(invalid("missing units"));
                }
                if units.len() > 1 {
                    return Err(invalid("review takes one unit per verdict file"));
                }
                if !read_units(store)?.iter().any(|u| u.id == units[0]) {
                    return Err(Failure::new(
                        "not_found",
                        format!("unit {} not found", units[0]),
                    ));
                }
                let default_path = verdict_path(store, &units[0]);
                let path = if has("--verdict-file") {
                    Path::new(option("--verdict-file")?)
                } else {
                    &default_path
                };
                let data = std::fs::read_to_string(path).map_err(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        invalid(&format!(
                            "verdict file {} does not exist; save the reviewer's verdict there, then review", path.display()
                        ))
                    } else {
                        Failure::new("io", e.to_string())
                    }
                })?;
                let v = parse_verdict(&data)?;
                let reviewed_unit = &v.unit;
                if *reviewed_unit != units[0] {
                    return Err(invalid(&format!(
                        "review is for unit {reviewed_unit}, not {}",
                        units[0]
                    )));
                }
                let reviewed_tree = &v.tree;
                let pass = passes(&v, &BTreeSet::new());
                let verified = evidence(store, &units[0])?
                    .into_iter()
                    .rev()
                    .find(|r| r.kind == "verify")
                    .map(|r| r.commit)
                    .unwrap_or_default();
                let pack = std::fs::read_to_string(pack_path(store, &units[0])).unwrap_or_default();
                if !verified.is_empty() && !pack.contains(&verified) {
                    return Err(Failure::new(
                        "no_review_pack",
                        format!(
                            "the review pack of {} is missing or older than its VERIFY; run orch review-pack first",
                            units[0]
                        ),
                    ));
                }
                if !verified.is_empty() && *reviewed_tree != verified {
                    return Err(Failure::new(
                        "stale_verdict",
                        format!(
                            "the verdict reviewed {reviewed_tree}, but the last VERIFY of {} is {verified}; have chronos:reviewer review its current pack",
                            units[0]
                        ),
                    ));
                }
                let rows = record_review(store, &units, reviewer, pass, repo)?;
                if path != default_path {
                    base::atomic_write(&default_path, data.as_bytes())
                        .map_err(|e| Failure::new("io", e.to_string()))?;
                }
                let ids = ready_units(store)?;
                let waiting = waiting_units(store)?;
                let mut ready_text = if ids.is_empty() {
                    "(none ready)".into()
                } else {
                    ids.join("\n")
                };
                for w in &waiting {
                    ready_text.push_str(&format!("\n{}\t{}\t{}", w.unit, w.state, w.next));
                }
                let mut text = format!("{}\t{}\n{ready_text}", rows[0].0, rows[0].1);
                let serious: Vec<_> = v.findings.iter().filter(|f| f.is_serious()).collect();
                if !serious.is_empty() {
                    text.push_str("\nblocker and major findings:");
                }
                let findings: Vec<_> = serious.into_iter().map(|f| {
                    let severity = match f.severity { Severity::Blocker => "blocker", Severity::Major => "major", Severity::Minor => unreachable!() };
                    text.push_str(&format!("\n  {}", cut(&format!("#{} {severity} {} {}: {}", f.number, f.check, f.location, f.evidence))));
                    serde_json::json!({"number":f.number,"severity":severity,"check":f.check,"location":f.location,"evidence":cut(&f.evidence)})
                }).collect();
                Ok((
                    0,
                    text,
                    serde_json::json!({"states": rows, "ready": ids, "waiting": waiting, "findings":findings}),
                ))
            }
            "waive" => {
                let unit = args
                    .get(1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or_else(|| invalid("missing unit"))?;
                if !read_units(store)?.iter().any(|u| u.id == *unit) {
                    return Err(Failure::new("not_found", format!("unit {unit} not found")));
                }
                let rows = evidence(store, unit)?;
                let review = rows
                    .iter()
                    .rfind(|r| r.kind == "review")
                    .ok_or_else(|| invalid("no review to waive"))?;
                let verdict = read_verdict(store, unit)?
                    .ok_or_else(|| invalid("no verdict file to waive"))?;
                let number = args
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&n| {
                        verdict
                            .findings
                            .iter()
                            .any(|f| f.number == n && f.is_serious())
                    })
                    .ok_or_else(|| invalid("number must identify a blocker or major finding"))?;
                let reason = args
                    .iter()
                    .position(|s| s == "--reason")
                    .and_then(|i| args.get(i + 1))
                    .filter(|s| !s.starts_with("--"))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| invalid("reason must not be empty"))?;
                let mut waived: BTreeSet<_> =
                    waivers(&rows).into_iter().map(|w| w.number).collect();
                if !waived.insert(number) {
                    return Err(invalid("finding already waived for this review"));
                }
                let remaining = verdict
                    .findings
                    .iter()
                    .filter(|f| f.fails() && !waived.contains(&f.number))
                    .count();
                append_evidence(
                    store,
                    unit,
                    &waive_row(number, reason, &review.commit, remaining == 0),
                )?;
                let state = update_unit_state(store, unit)?;
                Ok((
                    0,
                    format!("{unit}\twaived #{number}\t{state}\tremaining={remaining}"),
                    serde_json::json!({"unit":unit,"number":number,"state":state,"remaining":remaining}),
                ))
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
