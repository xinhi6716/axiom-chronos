use super::*;
use crate::base::{Failure, Res};
use std::path::Path;

pub fn run_extra(store: &Path, args: &[String], json: bool) -> Option<Output> {
    let cmd = args.first()?.as_str();
    if !["ready", "accept", "fulltest", "review", "evidence"].contains(&cmd) {
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
            "ready" => {
                let ids = ready_units(store)?;
                let text = if ids.is_empty() {
                    "(none ready)".into()
                } else {
                    ids.join("\n")
                };
                Ok((0, text, serde_json::json!(ids)))
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
                let rows = record_review(store, &units, reviewer, pass)?;
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
