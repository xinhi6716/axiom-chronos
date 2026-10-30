use crate::base;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub kind: String,
    pub command: String,
    pub exit_code: i32,
    pub commit: String,
    pub ts: String,
}

fn codex_role(model: &str) -> Option<&'static str> {
    ["terra", "sol", "astra", "luna"]
        .into_iter()
        .find(|role| crate::codex::role_matches(role, model))
}
fn same_reviewer(a: &str, b: &str) -> bool {
    if a.starts_with("chronos:") || b.starts_with("chronos:") {
        return true;
    }
    if codex_role(a) == Some("luna") || codex_role(b) == Some("luna") {
        return true;
    }
    match (codex_role(a), codex_role(b)) {
        (Some(x), Some(y)) => x == y,
        (Some(_), None) => matches!(
            base::family_of(b),
            base::Family::OpenAi | base::Family::Unknown
        ),
        (None, Some(_)) => matches!(
            base::family_of(a),
            base::Family::OpenAi | base::Family::Unknown
        ),
        _ => base::family_of(a) == base::family_of(b),
    }
}

pub fn derive_state(rows: &[Evidence], needs_accept: bool) -> String {
    let Some(i) = rows.iter().rposition(|r| r.kind == "run") else {
        return "pending".into();
    };
    let run = &rows[i];
    let rest = &rows[i + 1..];
    if run.exit_code != 0 || rest.iter().any(|r| r.kind == "scope" && r.exit_code != 0) {
        return "failed".into();
    }
    let Some(v) = rest.iter().rposition(|r| r.kind == "verify") else {
        return "built".into();
    };
    if rest[v].exit_code != 0 {
        return "failed".into();
    }
    let mut start = v + 1;
    if needs_accept {
        match rest[start..].iter().rposition(|r| r.kind == "accept") {
            None => return "built".into(),
            Some(a) => {
                start += a + 1;
                if rest[start - 1].exit_code != 0 {
                    return "accept-failed".into();
                }
            }
        }
    }
    let reviews: Vec<_> = rest[start..]
        .iter()
        .filter(|r| r.kind == "review")
        .collect();
    let Some(last) = reviews.last() else {
        return "built".into();
    };
    let current: Vec<_> = reviews.iter().filter(|r| r.commit == last.commit).collect();
    if current.iter().any(|r| r.exit_code != 0) {
        return "review-failed".into();
    }
    if current.iter().any(|r| {
        rows.iter()
            .any(|writer| writer.kind == "run" && same_reviewer(&r.command, &writer.command))
    }) {
        return "built".into();
    }
    "verified".into()
}
use super::{parse_map, read_units, set_unit_state, unit_brief, with_lock};
use crate::base::{proc::ShellRun, Failure, Res};
use std::{path::Path, process::Command, time::Duration};

const HEADER: &str = "kind\tcommand\texit_code\tcommit\tts";
fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(s: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", s)
}
fn rows(store: &Path, name: &str) -> Res<Vec<Evidence>> {
    let path = store.join("reports").join(format!("{name}.evidence.tsv"));
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(io(e)),
    };
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(invalid("invalid evidence header"));
    }
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let c: Vec<_> = l.split('\t').collect();
            if c.len() != 5 {
                return Err(invalid("malformed evidence row"));
            }
            Ok(Evidence {
                kind: c[0].into(),
                command: c[1].into(),
                exit_code: c[2]
                    .parse()
                    .map_err(|_| invalid("invalid evidence exit code"))?,
                commit: c[3].into(),
                ts: c[4].into(),
            })
        })
        .collect()
}
pub(crate) fn full_evidence(store: &Path) -> Res<Vec<Evidence>> {
    rows(store, "_full")
}
pub fn evidence(store: &Path, unit: &str) -> Res<Vec<Evidence>> {
    base::require_slug("unit", unit)?;
    rows(store, unit)
}
/// Append one evidence row under the store lock.
pub fn append_evidence(store: &Path, unit: &str, row: &Evidence) -> Res<()> {
    if !["_full", "_stages"].contains(&unit) {
        base::require_slug("unit", unit)?;
    }
    with_lock(store, false, || {
        let mut all = rows(store, unit)?;
        all.push(row.clone());
        let mut text = format!("{HEADER}\n");
        for r in all {
            let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                clean(&r.kind),
                clean(&r.command),
                r.exit_code,
                clean(&r.commit),
                clean(&r.ts)
            ));
        }
        let dir = store.join("reports");
        std::fs::create_dir_all(&dir).map_err(io)?;
        base::atomic_write(&dir.join(format!("{unit}.evidence.tsv")), text.as_bytes()).map_err(io)
    })
}
fn git(repo: &Path, args: &[&str]) -> String {
    Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
pub fn snapshot(repo: &Path, scope: &[String]) -> String {
    if git(repo, &["rev-parse", "--is-inside-work-tree"]) != "true" {
        return String::new();
    }
    let index = std::env::temp_dir().join(format!(
        "orch-index-{}-{}",
        std::process::id(),
        base::unique()
    ));
    let run = |args: &[&str]| {
        Command::new("git")
            .current_dir(repo)
            .env("GIT_INDEX_FILE", &index)
            .args(args)
            .output()
    };
    let result = (|| {
        if !run(&["read-tree", "--empty"]).ok()?.status.success() {
            return None;
        }
        if !scope.is_empty() {
            let mut args = vec!["add", "-A", "--"];
            args.extend(scope.iter().map(String::as_str));
            if !run(&args).ok()?.status.success() {
                return None;
            }
        }
        let out = run(&["write-tree"]).ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    })()
    .unwrap_or_default();
    let _ = std::fs::remove_file(index);
    if result.is_empty() {
        result
    } else {
        format!("tree:{result}")
    }
}
fn map(store: &Path) -> Res<super::Map> {
    let text = std::fs::read_to_string(store.join("modules.md")).map_err(io)?;
    parse_map(&text).map_err(invalid)
}
pub fn needs_accept(store: &Path, unit: &str) -> Res<bool> {
    let u = read_units(store)?
        .into_iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    if !store.join("modules.md").exists() {
        return Ok(false);
    }
    Ok(map(store)?.modules.iter().any(|m| m.id == u.track))
}
fn row(kind: &str, command: &str, exit_code: i32, commit: String) -> Evidence {
    Evidence {
        kind: kind.into(),
        command: command.into(),
        exit_code,
        commit,
        ts: base::now_iso(),
    }
}
fn update(store: &Path, unit: &str) -> Res<String> {
    let state = derive_state(&evidence(store, unit)?, needs_accept(store, unit)?);
    set_unit_state(store, unit, &state)?;
    Ok(state)
}
pub fn record_accept(store: &Path, module: &str, repo: &Path) -> Res<ShellRun> {
    let m = map(store)?
        .modules
        .into_iter()
        .find(|m| m.id == module)
        .ok_or_else(|| Failure::new("not_found", format!("module {module} not found")))?;
    let units: Vec<_> = read_units(store)?
        .into_iter()
        .filter(|u| u.track == module)
        .collect();
    let scopes = units
        .iter()
        .map(|u| unit_brief(store, &u.id).map(|b| b.scope))
        .collect::<Res<Vec<_>>>()?;
    let result = base::proc::run_shell(&m.accept_command, repo, Duration::from_secs(1800))?;
    for (u, scope) in units.into_iter().zip(scopes) {
        append_evidence(
            store,
            &u.id,
            &row(
                "accept",
                &m.accept_command,
                result.exit_code,
                snapshot(repo, &scope),
            ),
        )?;
        update(store, &u.id)?;
    }
    Ok(result)
}
pub fn record_full(store: &Path, repo: &Path, command: Option<&str>) -> Res<ShellRun> {
    let command = match command {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        Some(_) => return Err(invalid("full test command is empty")),
        None => {
            if !store.join("modules.md").exists() {
                return Err(invalid("missing FULL TEST command"));
            }
            map(store)?
                .full_test
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("missing FULL TEST command"))?
        }
    };
    let result = base::proc::run_shell(&command, repo, Duration::from_secs(1800))?;
    append_evidence(
        store,
        "_full",
        &row(
            "full",
            &command,
            result.exit_code,
            snapshot(repo, &[".".into()]),
        ),
    )?;
    Ok(result)
}
pub fn record_review(
    store: &Path,
    units: &[String],
    reviewer: &str,
    pass: bool,
    repo: &Path,
) -> Res<Vec<(String, String)>> {
    let existing = read_units(store)?;
    let mut commits = Vec::new();
    for unit in units {
        if !existing.iter().any(|u| u.id == *unit) {
            return Err(Failure::new("not_found", format!("unit {unit} not found")));
        }
        let rs = evidence(store, unit)?;
        if codex_role(reviewer) == Some("luna")
            || rs
                .iter()
                .any(|r| r.kind == "run" && same_reviewer(&r.command, reviewer))
        {
            return Err(Failure::new(
                "same_family",
                format!("reviewer {reviewer} has the same family as worker for {unit}"),
            ));
        }
        commits.push(
            unit_brief(store, unit)
                .map(|brief| snapshot(repo, &brief.scope))
                .unwrap_or_default(),
        );
    }
    let mut result = Vec::new();
    for (unit, commit) in units.iter().zip(commits) {
        append_evidence(
            store,
            unit,
            &row("review", reviewer, if pass { 0 } else { 1 }, commit),
        )?;
        result.push((unit.clone(), update(store, unit)?));
    }
    Ok(result)
}
pub fn record_stage(store: &Path, kind: &str, model: &str, ok: bool) -> Res<()> {
    if !["astra", "second_review", "rewrite"].contains(&kind) {
        return Err(invalid("invalid stage"));
    }
    append_evidence(
        store,
        "_stages",
        &row(kind, model, if ok { 0 } else { 1 }, String::new()),
    )
}
pub fn astra_gate(store: &Path) -> Res<()> {
    let stages = rows(store, "_stages")?;
    let astra = stages.iter().rposition(|r| {
        r.kind == "astra"
            && r.exit_code == 0
            && base::family_of(&r.command) == base::Family::OpenAi
            && r.command.ends_with("-astra")
    });
    let i = astra
        .ok_or_else(|| Failure::new("astra_gate", "missing successful OpenAI -astra stage"))?;
    if !stages[i + 1..].iter().any(|r| r.kind == "second_review") {
        return Err(Failure::new(
            "astra_gate",
            "missing later second_review stage",
        ));
    }
    for file in [
        "astra/plan.json",
        "astra/review.json",
        "astra/ruling.md",
        "astra/plan-final.md",
    ] {
        if !store.join(file).is_file() {
            return Err(Failure::new("astra_gate", format!("missing {file}")));
        }
    }
    for file in ["astra/review.json", "astra/ruling.md"] {
        let contents = std::fs::read_to_string(store.join(file)).map_err(io)?;
        if contents.trim().is_empty() {
            return Err(Failure::new("astra_gate", format!("empty {file}")));
        }
    }
    Ok(())
}
pub fn done_gate(store: &Path, change_mode: bool) -> Res<()> {
    let units = if !store.join("units.tsv").exists() {
        vec![]
    } else {
        read_units(store)?
    };
    if units.is_empty() {
        return if change_mode {
            Err(Failure::new("not_done", "no verified units"))
        } else {
            Ok(())
        };
    }
    let unfinished: Vec<_> = units
        .iter()
        .filter(|u| u.state != "verified")
        .map(|u| format!("{}={}", u.id, u.state))
        .collect();
    if !unfinished.is_empty() {
        return Err(Failure::new("not_done", unfinished.join(", ")));
    }
    let mut latest = String::new();
    for u in units {
        for r in evidence(store, &u.id)? {
            if r.ts > latest {
                latest = r.ts;
            }
        }
    }
    let full = rows(store, "_full")?
        .into_iter()
        .filter(|r| r.kind == "full")
        .last();
    if !full.is_some_and(|r| r.exit_code == 0 && r.ts > latest) {
        return Err(Failure::new(
            "not_done",
            "missing successful, current full test",
        ));
    }
    if !rows(store, "_stages")?
        .iter()
        .any(|r| r.kind == "rewrite" && r.ts > latest)
    {
        return Err(Failure::new("not_done", "missing current rewrite stage"));
    }
    Ok(())
}
