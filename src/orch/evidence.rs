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
#[cfg(test)]
thread_local! {
    static BEFORE_PROJECTION_RECHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
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
        let steps: Vec<_> = c
            .steps
            .values()
            .filter(|s| matches!(s.purpose, Purpose::Engineer | Purpose::Assembler))
            .filter(|s| s.purpose != Purpose::Assembler || s.step_id == assembly)
            .collect();
        if steps.is_empty() {
            return super::store::save_units(store, &units);
        }
        // This view is local to one compatibility projection. Authority gates
        // continue to validate their own fresh source and receipt evidence.
        let source = super::snapshot(&c.repo.canonical_root, &[".".into()]);
        let checks = c
            .receipts
            .iter()
            .filter(|(key, _)| key.starts_with("check-"))
            .map(|(key, reference)| {
                let result: CheckResult =
                    serde_json::from_slice(&std::fs::read(&reference.path).map_err(io)?)
                        .map_err(|e| invalid(e.to_string()))?;
                Ok((key, result))
            })
            .collect::<Res<Vec<_>>>()?;
        let reviews = c
            .receipts
            .iter()
            .filter(|(key, _)| key.starts_with("review-"))
            .map(|(_, reference)| {
                serde_json::from_slice::<IndependentReview>(
                    &std::fs::read(&reference.path).map_err(io)?,
                )
                .map_err(|e| invalid(e.to_string()))
            })
            .collect::<Res<Vec<_>>>()?;
        let mut pending = Vec::new();
        let mut snapshots_valid = !source.is_empty();
        for s in steps {
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
            snapshots_valid &= !current.is_empty();
            for (key, result) in &checks {
                let matches = source == result.snapshot.source_digest;
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
            for review in &reviews {
                rows.push(row(
                    "review",
                    &format!("{}#{}", review.executor.model, review.executor.thread_id),
                    if review.pass && source == review.snapshot.source_digest {
                        0
                    } else {
                        1
                    },
                    current.clone(),
                ));
            }
            let u = super::store::Unit {
                id: id.clone(),
                track: c.run_id.clone(),
                state: String::new(),
                branch: String::new(),
                pr: String::new(),
                sha: current,
                brief: s.brief_ref.path.to_string_lossy().into_owned(),
            };
            pending.push((u, rows));
        }
        #[cfg(test)]
        BEFORE_PROJECTION_RECHECK.with(|hook| {
            if let Some(hook) = hook.borrow_mut().take() {
                hook();
            }
        });
        // Prepare every unit before writing any projection. Empty snapshots and
        // changes during preparation invalidate the entire batch consistently.
        let final_source = super::snapshot(&c.repo.canonical_root, &[".".into()]);
        snapshots_valid &= !final_source.is_empty() && final_source == source;
        for (mut u, mut rows) in pending {
            if !snapshots_valid {
                for row in &mut rows {
                    if matches!(row.kind.as_str(), "verify" | "review") {
                        row.exit_code = 1;
                    }
                }
            }
            u.state = derive_state(&rows, false);
            let id = u.id.clone();
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
/// Hash the exact ordered input manifest used by `tree-v2` snapshots.
pub(crate) fn input_manifest_digest(inputs: &std::collections::BTreeMap<String, String>) -> String {
    use sha2::{Digest, Sha256};
    let mut manifest = Sha256::new();
    manifest.update(b"axiom-worktree-inputs-v2\0");
    for (path, value) in inputs {
        manifest.update((path.len() as u64).to_be_bytes());
        manifest.update(path.as_bytes());
        manifest.update((value.len() as u64).to_be_bytes());
        manifest.update(value.as_bytes());
    }
    format!("{:x}", manifest.finalize())
}

pub fn snapshot(repo: &Path, scope: &[String]) -> String {
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
    format!("tree-v2:{result}:{}", input_manifest_digest(&inputs))
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

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::orch::checkpoint::{ArtifactRef, Checkpoint};
    use serde_json::json;
    use std::{fs, path::PathBuf};

    fn fixture() -> (tempfile::TempDir, PathBuf, Checkpoint) {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let store = temp.path().join("store");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&store).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        fs::write(repo.join(".gitignore"), "ignored\n").unwrap();
        fs::write(repo.join("ignored"), "original").unwrap();
        for name in ["one", "two"] {
            fs::write(repo.join(name), name).unwrap();
        }
        let source = snapshot(&repo, &[".".into()]);
        assert!(!source.is_empty());
        let reference = |name: &str, value: serde_json::Value| {
            let path = store.join(name);
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            ArtifactRef {
                path,
                digest: "projection-is-not-authority".into(),
            }
        };
        let brief = reference("brief", json!({}));
        let executor = json!({"attempt_id":"verify", "thread_id":"reviewer-thread", "model":"reviewer",
            "purpose":"verifier", "policy":"scratch_check"});
        let frozen =
            json!({"source_digest":source, "plan_digest":"plan", "checks_digest":"checks"});
        let check = reference(
            "check",
            json!({"check_id":"check", "snapshot":frozen,
            "spec_digest":"spec", "exit_code":0, "policy_applied":true,
            "stdout":brief, "stderr":brief, "executor":executor}),
        );
        let review = reference(
            "review",
            json!({"review_id":"review", "snapshot":frozen,
            "executor":executor, "check_ids":["check"], "pass":true, "report":brief}),
        );
        let mut c: Checkpoint = serde_json::from_value(json!({"schema_version":1, "revision":0,
            "run_id":"run", "repo":{"canonical_root":repo,"identity_digest":"repo"},
            "intent":brief,"research":null,"plan":null,"phase":"engineering","status":"running",
            "core_decision":null,"steps":{},"checks":{},"attempts":{},"reservations":[],
            "pending_evidence":[],"receipts":{"check-check":check,"review-review":review}}))
        .unwrap();
        for name in ["one", "two"] {
            let scope_snapshot = snapshot(&repo, &[name.into()]);
            let report = reference(
                &format!("report-{name}"),
                json!({"model":"worker",
                "scope_snapshot":scope_snapshot,"scope_checked":true}),
            );
            c.steps.insert(name.into(), serde_json::from_value(json!({"run_id":"run", "step_id":name,
                "purpose":"engineer", "brief_ref":brief, "scope":[name],"dependency_ids":[],"check_ids":["check"]})).unwrap());
            c.attempts.insert(name.into(), serde_json::from_value(json!({"attempt_id":name,"step_id":name,
                "input_digest":"input", "state":"terminal", "generation":1,"thread_id":"worker-thread",
                "turn_id":"turn","raw_result":report,"outcome":"completed","last_event_sequence":1})).unwrap());
        }
        (temp, store, c)
    }

    fn assert_states(store: &Path, expected: &str) {
        let units = read_units(store).unwrap();
        assert_eq!(units.len(), 2);
        assert_ne!(
            units[0].sha, units[1].sha,
            "each unit retains its own scope snapshot"
        );
        for unit in units {
            assert_eq!(unit.state, expected);
            let rows = evidence(store, &unit.id).unwrap();
            assert_eq!(
                rows.iter().map(|row| row.kind.as_str()).collect::<Vec<_>>(),
                ["run", "scope", "verify", "review"]
            );
            for row in rows
                .iter()
                .filter(|row| matches!(row.kind.as_str(), "verify" | "review"))
            {
                assert_eq!(row.exit_code, i32::from(expected != "verified"));
            }
        }
    }

    #[test]
    fn projection_rechecks_ignored_sources_before_any_write_and_recomputes_after_restore() {
        let (_temp, store, c) = fixture();
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "verified");
        for unit in ["run-one", "run-two"] {
            fs::write(
                store.join("reports").join(format!("{unit}.evidence.tsv")),
                "not-written-yet",
            )
            .unwrap();
        }
        let report_dir = store.join("reports");
        let repo = c.repo.canonical_root.clone();
        BEFORE_PROJECTION_RECHECK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                for unit in ["run-one", "run-two"] {
                    assert_eq!(
                        fs::read_to_string(report_dir.join(format!("{unit}.evidence.tsv")))
                            .unwrap(),
                        "not-written-yet"
                    );
                }
                fs::write(repo.join("ignored"), "changed during projection").unwrap();
            }))
        });
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "failed");
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "failed");
        fs::write(c.repo.canonical_root.join("ignored"), "original").unwrap();
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "verified");

        // Receipt contents are also read anew by the next projection.
        let receipt = &c.receipts["review-review"].path;
        let mut review: serde_json::Value =
            serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
        review["pass"] = json!(false);
        fs::write(receipt, serde_json::to_vec(&review).unwrap()).unwrap();
        project_checkpoint(&store, &c).unwrap();
        for unit in read_units(&store).unwrap() {
            assert_eq!(unit.state, "review-failed");
        }
    }

    #[test]
    fn projection_empty_final_or_unit_snapshot_invalidates_the_whole_batch() {
        let (_temp, store, mut c) = fixture();
        let repo = c.repo.canonical_root.clone();
        BEFORE_PROJECTION_RECHECK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(repo.join(".git"), repo.join("saved-git")).unwrap();
            }))
        });
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "failed");
        fs::rename(
            c.repo.canonical_root.join("saved-git"),
            c.repo.canonical_root.join(".git"),
        )
        .unwrap();
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "verified");
        c.steps.get_mut("two").unwrap().scope = vec![".git".into()];
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "failed");
        c.steps.get_mut("two").unwrap().scope = vec!["two".into()];
        project_checkpoint(&store, &c).unwrap();
        assert_states(&store, "verified");
    }
}
