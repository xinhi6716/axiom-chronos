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
    if (!run.commit.is_empty() && run.commit != rest[v].commit)
        || (!rest[v].commit.is_empty() && last.commit != rest[v].commit)
        || current.iter().any(|r| r.command == run.command)
    {
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
/// Rebuild the compatibility projection from immutable, captured run evidence.
/// It is never an authority for checkpoint advancement. Stable run-qualified IDs
/// avoid overwriting another run or duplicating rows on recovery.
pub fn project_checkpoint(store: &Path, c: &super::checkpoint::Checkpoint) -> Res<()> {
    use super::checkpoint::{AttemptOutcome, CheckResult, IndependentReview, Purpose};
    use serde_json::Value;
    with_lock(store, false, || {
        let mut units = if store.join("units.tsv").exists() {
            read_units(store)?
        } else {
            vec![]
        };
        let rework = c
            .receipts
            .keys()
            .filter(|k| k.starts_with("rework-"))
            .count();
        let assembly = if rework == 0 {
            "assembly".into()
        } else {
            format!("assembly-rework-{rework}")
        };
        for s in c
            .steps
            .values()
            .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))
            .filter(|s| s.purpose != Purpose::Assembler || s.step_id == assembly)
        {
            let id = format!(
                "{}-{}",
                c.run_id,
                if s.purpose == Purpose::Assembler {
                    "assembly"
                } else {
                    &s.step_id
                }
            );
            let mut rows = vec![];
            for a in c.attempts.values().filter(|a| a.step_id == s.step_id) {
                let Some(raw) = &a.raw_result else { continue };
                let report: Value = serde_json::from_slice(&std::fs::read(&raw.path).map_err(io)?)
                    .map_err(|e| invalid(e.to_string()))?;
                let snapshot = report["scope_snapshot"].as_str().unwrap_or("").to_owned();
                let executor = format!(
                    "{}#{}",
                    report["model"].as_str().unwrap_or("unknown"),
                    a.thread_id.as_deref().unwrap_or("unknown")
                );
                rows.push(row(
                    "run",
                    &executor,
                    if a.outcome == Some(AttemptOutcome::Completed) {
                        0
                    } else {
                        1
                    },
                    snapshot.clone(),
                ));
                rows.push(row(
                    "scope",
                    "Rust captured scope audit",
                    if report["scope_checked"] == true {
                        0
                    } else {
                        1
                    },
                    snapshot,
                ));
            }
            let current = snapshot(&c.repo.canonical_root, &s.scope);
            for (key, r) in c.receipts.iter().filter(|(k, _)| k.starts_with("check-")) {
                let result: CheckResult =
                    serde_json::from_slice(&std::fs::read(&r.path).map_err(io)?)
                        .map_err(|e| invalid(e.to_string()))?;
                let matches = super::snapshot(&c.repo.canonical_root, &[".".into()])
                    == result.snapshot.source_digest;
                rows.push(row(
                    "verify",
                    key,
                    if matches && result.policy_applied {
                        result.exit_code.unwrap_or(1)
                    } else {
                        1
                    },
                    current.clone(),
                ));
            }
            for (_, r) in c.receipts.iter().filter(|(k, _)| k.starts_with("review-")) {
                let review: IndependentReview =
                    serde_json::from_slice(&std::fs::read(&r.path).map_err(io)?)
                        .map_err(|e| invalid(e.to_string()))?;
                rows.push(row(
                    "review",
                    &format!("{}#{}", review.executor.model, review.executor.thread_id),
                    if review.pass
                        && super::snapshot(&c.repo.canonical_root, &[".".into()])
                            == review.snapshot.source_digest
                    {
                        0
                    } else {
                        1
                    },
                    current.clone(),
                ));
            }
            let state = derive_state(&rows, false);
            let u = super::store::Unit {
                id: id.clone(),
                track: c.run_id.clone(),
                state,
                branch: String::new(),
                pr: String::new(),
                sha: current,
                brief: s.brief_ref.path.to_string_lossy().into_owned(),
            };
            if let Some(old) = units.iter_mut().find(|u| u.id == id) {
                *old = u;
            } else {
                units.push(u);
            }
            let mut text = format!("{HEADER}\n");
            for r in rows {
                let clean = |v: &str| v.replace(['\t', '\n', '\r'], " ");
                text.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    r.kind,
                    clean(&r.command),
                    r.exit_code,
                    r.commit,
                    r.ts
                ));
            }
            let reports = store.join("reports");
            std::fs::create_dir_all(&reports).map_err(io)?;
            base::atomic_write(&reports.join(format!("{id}.evidence.tsv")), text.as_bytes())
                .map_err(io)?;
        }
        super::store::save_units(store, &units)
    })
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
    use sha2::{Digest, Sha256};
    if git(repo, &["rev-parse", "--is-inside-work-tree"]) != "true" {
        return String::new();
    }
    // Git metadata is outside this input view. Reject a request for that path
    // instead of returning an incomplete identity.
    if scope.iter().any(|s| {
        s.replace('\\', "/").split('/').any(|part| {
            #[cfg(windows)]
            {
                part.eq_ignore_ascii_case(".git")
            }
            #[cfg(not(windows))]
            {
                part == ".git"
            }
        })
    }) {
        return String::new();
    }
    // Inspect links and hash the complete input view before invoking git add:
    // an unsafe link must not cause Git to visit data outside the worktree.
    let Ok(inputs) = super::lifecycle::input_contents(repo, Some(scope)) else {
        return String::new();
    };
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
        let candidates: Vec<_> = scope
            .iter()
            .filter(|s| repo.join(s).exists() || s.contains(['*', '?', '[']))
            .collect();
        let mut present = Vec::new();
        for candidate in candidates {
            // Explicitly named ignored files belong only in the SHA-256
            // manifest; never force their bytes into Git objects.
            if candidate != "."
                && !candidate.contains(['*', '?', '['])
                && !candidate.starts_with(':')
            {
                let ignored = Command::new("git")
                    .current_dir(repo)
                    .args(["check-ignore", "--no-index", "--quiet", "--", candidate])
                    .output()
                    .ok()?;
                match ignored.status.code() {
                    Some(0) => continue,
                    Some(1) => {}
                    _ => return None,
                }
            }
            present.push(candidate);
        }
        if !present.is_empty() {
            let mut args = vec!["add", "-A", "--"];
            args.extend(present.iter().map(|s| s.as_str()));
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
        return result;
    }
    let mut manifest = Sha256::new();
    manifest.update(b"axiom-worktree-inputs-v2\0");
    for (path, value) in inputs {
        manifest.update((path.len() as u64).to_be_bytes());
        manifest.update(path.as_bytes());
        manifest.update((value.len() as u64).to_be_bytes());
        manifest.update(value.as_bytes());
    }
    format!("tree-v2:{result}:{:x}", manifest.finalize())
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
        if rs.iter().any(|r| r.kind == "run" && r.command == reviewer) {
            return Err(Failure::new(
                "same_executor",
                format!(
                    "reviewer {reviewer} must be a distinct role or captured attempt for {unit}"
                ),
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
fn gate_checkpoint(store: &Path) -> Res<super::checkpoint::Checkpoint> {
    let runs = std::fs::read_dir(store.join("runs"))
        .map_err(|_| Failure::new("missing_evidence", "checkpoint is required"))?;
    let mut ids = Vec::new();
    for item in runs {
        let item = item.map_err(io)?;
        if item.path().join("checkpoint.json").is_file() {
            ids.push(item.file_name().to_string_lossy().into_owned());
        }
    }
    if ids.len() != 1 {
        return Err(Failure::new(
            "missing_evidence",
            "exactly one checkpoint is required; use a run-specific gate",
        ));
    }
    super::checkpoint::load_run(store, &ids[0])
}
pub fn astra_gate(store: &Path) -> Res<()> {
    let checkpoint = gate_checkpoint(store).map_err(|e| Failure::new("astra_gate", e.message))?;
    if super::checkpoint::approved_research(&checkpoint) {
        Ok(())
    } else {
        Err(Failure::new(
            "astra_gate",
            "missing approved research digest",
        ))
    }
}
pub fn done_gate(store: &Path, _change_mode: bool) -> Res<()> {
    let checkpoint = gate_checkpoint(store).map_err(|e| Failure::new("not_done", e.message))?;
    if super::checkpoint::ready_evidence(&checkpoint) {
        Ok(())
    } else {
        Err(Failure::new(
            "not_done",
            "missing current frozen checks, independent review, or assembly evidence",
        ))
    }
}
