use super::*;
use crate::base::{self, Failure, Res};
use std::path::Path;

pub fn run_extra(store: &Path, args: &[String], json: bool) -> Option<Output> {
    let cmd = args.first()?.as_str();
    if ![
        "ready",
        "accept",
        "fulltest",
        "review",
        "evidence",
        "release",
        "report",
        "review-pack",
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
                if reviewer != "chronos:reviewer" {
                    return Err(invalid("reviewer must be chronos:reviewer"));
                }
                let path = option("--verdict-file")?;
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
                let reviewed_unit = v
                    .get("unit")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| invalid("missing unit"))?;
                if reviewed_unit != units[0] {
                    return Err(invalid(&format!(
                        "review is for unit {reviewed_unit}, not {}",
                        units[0]
                    )));
                }
                let pass = verdict.eq_ignore_ascii_case("pass")
                    && !findings.iter().any(|f| {
                        f.get("severity").and_then(|s| s.as_str()).is_some_and(|s| {
                            s.eq_ignore_ascii_case("blocker") || s.eq_ignore_ascii_case("major")
                        })
                    });
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
                let rows = record_review(store, &units, reviewer, pass, repo)?;
                base::atomic_write(
                    &store.join("reports").join(format!("{}.verdict.json", units[0])),
                    data.as_bytes(),
                )
                .map_err(|e| Failure::new("io", e.to_string()))?;
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
                let text = format!("{}\t{}\n{ready_text}", rows[0].0, rows[0].1);
                Ok((
                    0,
                    text,
                    serde_json::json!({"states": rows, "ready": ids, "waiting": waiting}),
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
