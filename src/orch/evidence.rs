use crate::base;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    let identity = &rest[v].commit;
    if identity.is_empty() {
        return "built".into();
    }
    let mut start = v + 1;
    if needs_accept {
        match rest[start..].iter().rposition(|r| r.kind == "accept") {
            None => return "built".into(),
            Some(a) => {
                let latest = start + a;
                let accept = &rest[latest];
                if accept.exit_code != 0 {
                    return "accept-failed".into();
                }
                if accept.commit != *identity {
                    return "built".into();
                }
                // Repeating the same successful acceptance on unchanged source
                // does not invalidate an independent review. A failed acceptance,
                // changed command or changed source still starts a new chain.
                let mut first = latest;
                for (index, prior) in rest[start..latest].iter().enumerate().rev() {
                    if prior.kind != "accept" {
                        continue;
                    }
                    if prior.exit_code != 0
                        || prior.commit != accept.commit
                        || prior.command != accept.command
                    {
                        break;
                    }
                    first = start + index;
                }
                start = first + 1;
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
    if last.commit != *identity {
        return "built".into();
    }
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
        for path in scope {
            // Only skip a missing literal path. Let Git interpret pathspecs and
            // reject invalid, ignored, inaccessible, or out-of-repository paths.
            let literal = !path.starts_with(':') && !path.contains(['*', '?', '[']);
            if literal {
                match std::fs::symlink_metadata(repo.join(path)) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Windows can report NotFound even when an ancestor is a
                        // file. A dangling symlink ancestor is not an absent directory either.
                        let full = repo.join(path);
                        let mut parent = full.parent();
                        while let Some(dir) = parent {
                            match std::fs::metadata(dir) {
                                Ok(m) if m.is_dir() => break,
                                Ok(_) => return None,
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    match std::fs::symlink_metadata(dir) {
                                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                        _ => return None,
                                    }
                                }
                                Err(_) => return None,
                            }
                            parent = dir.parent();
                        }
                        // Git still validates the pathspec (e.g. it must be inside
                        // the worktree); --ignore-missing is only valid in dry-run.
                        if !run(&["add", "--dry-run", "--ignore-missing", "--", path])
                            .ok()?.status.success()
                        {
                            return None;
                        }
                        continue;
                    }
                    Err(_) => return None,
                }
            }
            if !run(&["add", "-A", "--", path]).ok()?.status.success() {
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
pub fn contract_identity(store: &Path, unit: &str) -> Res<String> {
    let brief = unit_brief(store, unit)?;
    let u = read_units(store)?
        .into_iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let mut modules = Vec::new();
    let mut dep_evidence = Vec::new();
    if store.join("modules.md").exists() {
        let map = map(store)?;
        let mut wanted = vec![u.track.clone()];
        while let Some(id) = wanted.pop() {
            if modules.iter().any(|m: &super::Module| m.id == id) {
                continue;
            }
            if let Some(m) = map.modules.iter().find(|m| m.id == id) {
                wanted.extend(m.depends.iter().cloned());
                modules.push(m.clone());
            }
        }
        modules.sort_by(|a, b| a.id.cmp(&b.id));
        let units = read_units(store)?;
        for dep in units
            .iter()
            .filter(|d| d.track != u.track && modules.iter().any(|m| m.id == d.track))
        {
            let rows = evidence(store, &dep.id)?;
            let verified = rows
                .iter()
                .rev()
                .find(|r| r.kind == "verify")
                .map(|r| r.commit.clone());
            let contract = rows
                .iter()
                .rev()
                .find(|r| r.kind == "contract")
                .map(|r| r.command.clone());
            // Review timestamps/history do not change the dependency's content.
            // Current validity is checked separately by current_verified_unit.
            dep_evidence.push((dep.id.clone(), verified, contract));
        }
        dep_evidence.sort_by(|a, b| a.0.cmp(&b.0));
    }
    let bytes = serde_json::to_vec(&(brief.text, modules, dep_evidence)).unwrap();
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}
pub(crate) fn current_contract(store: &Path, unit: &str, rows: &[Evidence]) -> Res<bool> {
    let Some(run) = rows.iter().rposition(|r| r.kind == "run") else {
        return Ok(false);
    };
    let expected = contract_identity(store, unit)?;
    Ok(rows[run + 1..]
        .iter()
        .find(|r| r.kind == "contract")
        .is_some_and(|r| r.command == expected))
}
/// Check both the saved Host receipt and its binding to the contract's command/source.
pub(crate) fn verification_receipt_current(
    envelope: &serde_json::Value,
    command: &str,
    source: &str,
) -> bool {
    let receipt = &envelope["receipt"];
    !source.is_empty()
        && receipt["command"] == command
        && receipt["source_before"] == source
        && receipt["source_after"] == source
        && receipt["exit_code"] == 0
        && receipt["timed_out"] == false
        && receipt["error"].is_null()
        && receipt["stdout"].is_string()
        && receipt["stderr"].is_string()
        && envelope["path"]
            .as_str()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|saved| saved == *receipt)
}

/// Whether the unit and all its dependencies have current independent verification,
/// matching contracts, and worktree snapshots. Does not mutate stored evidence.
pub fn current_verified_unit(store: &Path, unit: &str, repo: &Path) -> Res<bool> {
    let units = read_units(store)?;
    let u = units
        .iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let rows = evidence(store, unit)?;
    if derive_state(&rows, needs_accept(store, unit)?) != "verified"
        || !current_contract(store, unit, &rows)?
    {
        return Ok(false);
    }
    // Enrolled units must retain the Host's receipt, not just a successful TSV
    // row. Reuse and final handoff reject removed, changed or stale receipts.
    let run = rows.iter().rposition(|row| row.kind == "run").unwrap();
    if rows[run + 1..].iter().any(|row| row.kind == "dispatch") {
        let command = unit_brief(store, unit)?.verify;
        let source = snapshot(repo, &[".".into()]);
        let current = rows[run + 1..].iter().rev()
            .find(|row| row.kind == "verification_receipt")
            .is_some_and(|row| {
                let Ok(bytes) = std::fs::read(&row.command) else { return false; };
                let Ok(receipt) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return false; };
                row.exit_code == 0
                    && row.commit == format!("sha256:{:x}", Sha256::digest(&bytes))
                    && verification_receipt_current(
                        &serde_json::json!({"path":row.command,"receipt":receipt}),
                        &command,
                        &source,
                    )
            });
        if !current {
            return Ok(false);
        }
    }
    let identity = snapshot(repo, &unit_brief(store, unit)?.scope);
    if identity.is_empty()
        || !rows
            .iter()
            .rev()
            .find(|r| r.kind == "verify")
            .is_some_and(|r| r.commit == identity)
    {
        return Ok(false);
    }
    if store.join("modules.md").exists() {
        let map = map(store)?;
        if let Some(module) = map.modules.iter().find(|m| m.id == u.track) {
            for dep in &module.depends {
                let dependent: Vec<_> = units.iter().filter(|x| &x.track == dep).collect();
                if dependent.is_empty() {
                    return Ok(false);
                }
                for d in dependent {
                    if !current_verified_unit(store, &d.id, repo)? {
                        return Ok(false);
                    }
                }
            }
        }
    }
    Ok(true)
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
fn verification_shell(command: &str, repo: &Path) -> Res<ShellRun> {
    // Acceptance/full tests must not create untracked target files after the
    // package source was bound. Match the already-isolated Host/package runners.
    let target=base::cargo_target(repo,&format!("acceptance:{}",base::unique()))?;
    base::proc::run_shell_with_env(command,repo,Duration::from_secs(1800),&[("CARGO_TARGET_DIR",target.as_os_str())])
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
    let result = verification_shell(&m.accept_command, repo)?;
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
    let result = verification_shell(&command, repo)?;
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
    done_gate_inner(store, change_mode, false, true)
}
fn done_gate_inner(store: &Path, change_mode: bool, repo_check: bool, require_rewrite: bool) -> Res<()> {
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
    let mut unfinished = Vec::new();
    for u in &units {
        let evidence = evidence(store, &u.id)?;
        if u.state != "verified"
            || derive_state(&evidence, needs_accept(store, &u.id)?) != "verified"
            || !current_contract(store, &u.id, &evidence).unwrap_or(false)
        {
            unfinished.push(format!("{}={}", u.id, u.state));
        }
    }
    if !unfinished.is_empty() {
        return Err(Failure::new("not_done", unfinished.join(", ")));
    }
    let mut latest = String::new();
    for u in units {
        for r in evidence(store, &u.id)? {
            if r.kind != "review" && r.ts > latest {
                latest = r.ts;
            }
        }
    }
    let full = rows(store, "_full")?
        .into_iter()
        .filter(|r| r.kind == "full")
        .last();
    if !full.is_some_and(|r| r.exit_code == 0 && (repo_check || r.ts > latest)) {
        return Err(Failure::new(
            "not_done",
            "missing successful, current full test",
        ));
    }
    if require_rewrite && !rows(store, "_stages")?
        .iter()
        .rev()
        .find(|r| r.kind == "rewrite")
        .is_some_and(|r| r.exit_code == 0 && r.ts > latest)
    {
        return Err(Failure::new("not_done", "missing current rewrite stage"));
    }
    Ok(())
}

/// Additionally require unit verification and the full test to cover current sources.
pub fn done_gate_in_repo(store: &Path, change_mode: bool, repo: Option<&Path>) -> Res<()> {
    tested_gate_in_repo(store, change_mode, repo, true)
}
pub(crate) fn tested_gate_in_repo(store: &Path, change_mode: bool, repo: Option<&Path>, require_rewrite: bool) -> Res<()> {
    done_gate_inner(store, change_mode, true, require_rewrite)?;
    if !store.join("units.tsv").exists() {
        return Ok(());
    }
    let units = read_units(store)?;
    if units.is_empty() {
        return Ok(());
    }
    let repo = repo.ok_or_else(|| Failure::new("not_done", "missing goal cwd"))?;
    for unit in units {
        if !current_verified_unit(store, &unit.id, repo)? {
            return Err(Failure::new(
                "not_done",
                format!("stale verification: {}", unit.id),
            ));
        }
    }
    let identity = snapshot(repo, &[".".into()]);
    if identity.is_empty()
        || !full_evidence(store)?
            .iter()
            .rev()
            .find(|r| r.kind == "full")
            .is_some_and(|r| r.commit == identity)
    {
        return Err(Failure::new("not_done", "stale full test"));
    }
    Ok(())
}

// S1 inventories deliberately do not use `snapshot`: Git filters and its temporary
// index would hide ignored or byte-level changes from the operation boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S1Classification {
    pub version: u32,
    pub source_inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Paths tracked when the classification was frozen remain protected even if
    /// the index is subsequently changed.
    pub tracked_at_freeze: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S1Entry {
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S1Inventory {
    pub version: u32,
    pub classification: S1Classification,
    pub entries: Vec<S1Entry>,
    pub digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct S1Change {
    pub path: String,
    pub change: String,
    pub in_scope: bool,
}

fn s1_error(message: impl Into<String>) -> Failure { Failure::new("inventory", message) }
fn s1_time(deadline: std::time::Instant) -> Res<()> {
    if std::time::Instant::now() >= deadline { Err(s1_error("inventory deadline exceeded")) } else { Ok(()) }
}
fn s1_path(path: &str) -> Res<String> {
    use std::path::Component;
    if path.is_empty() || path.contains('\\') || path.contains('\0') { return Err(s1_error("invalid inventory path")); }
    let p = Path::new(path);
    let mut parts = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {},
            Component::Normal(s) => parts.push(s.to_str().ok_or_else(|| s1_error("unsupported path"))?),
            _ => return Err(s1_error("inventory path escapes repository")),
        }
    }
    let result = if parts.is_empty() { ".".to_string() } else { parts.join("/") };
    if result == ".git" || result.starts_with(".git/") { return Err(s1_error("Git control path")); }
    Ok(result)
}
fn s1_under(path: &str, base: &str) -> bool {
    base == "." || path == base || path.strip_prefix(base).is_some_and(|x| x.starts_with('/'))
}
fn s1_artifact(path: &str) -> bool {
    path == "target" || path.starts_with("target/") ||
        ((path.ends_with(".pyc") || path.ends_with(".pyo")) && path.split('/').any(|p| p == "__pycache__"))
}
fn s1_reparse(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; m.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))] { m.file_type().is_symlink() }
}
fn s1_mode(m: &std::fs::Metadata) -> u32 {
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; m.permissions().mode() & 0o7777 }
    #[cfg(not(unix))] { if m.permissions().readonly() { 0o444 } else { 0o666 } }
}
fn s1_git(repo: &Path, args: &[&str], deadline: std::time::Instant) -> Res<Vec<u8>> {
    use std::{fs::OpenOptions, process::Stdio};
    s1_time(deadline)?;
    let p = std::env::temp_dir().join(format!("s1-git-{}-{}", std::process::id(), base::unique()));
    let f = OpenOptions::new().write(true).create_new(true).open(&p).map_err(io)?;
    let result = (|| {
        let mut child = Command::new("git").current_dir(repo).args(args)
            .env_remove("GIT_INDEX_FILE").env_remove("GIT_DIR").env_remove("GIT_WORK_TREE")
            .env("GIT_OPTIONAL_LOCKS", "0").stdin(Stdio::null()).stdout(Stdio::from(f)).stderr(Stdio::null())
            .spawn().map_err(io)?;
        loop {
            if let Some(status) = child.try_wait().map_err(io)? {
                if !status.success() { return Err(s1_error("Git index listing failed")); }
                s1_time(deadline)?;
                return std::fs::read(&p).map_err(io);
            }
            if s1_time(deadline).is_err() { let _ = child.kill(); let _ = child.wait(); return Err(s1_error("Git deadline exceeded")); }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    })();
    let _ = std::fs::remove_file(p);
    result
}
fn s1_check_repo(repo: &Path) -> Res<()> {
    if !repo.is_absolute() || std::fs::canonicalize(repo).map_err(io)? != repo { return Err(s1_error("repository must be canonical")); }
    Ok(())
}
fn s1_validate_declared(repo: &Path, path: &str) -> Res<()> {
    let mut current = repo.to_path_buf();
    let components: Vec<_> = path.split('/').filter(|x| *x != ".").collect();
    for (i, part) in components.iter().enumerate() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(m) if i + 1 < components.len() && (s1_reparse(&m) || !m.is_dir()) => return Err(s1_error("declared path traverses non-directory or reparse point")),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}
/// Freeze source and output rules before the writer runs. Paths are repository-relative literals.
pub fn s1_freeze_inventory(repo: &Path, source_inputs: &[String], outputs: &[String], deadline: std::time::Instant) -> Res<S1Inventory> {
    s1_time(deadline)?; s1_check_repo(repo)?;
    let normalize = |paths: &[String]| -> Res<Vec<String>> {
        let mut result = Vec::new();
        for path in paths {
            let path = s1_path(path)?;
            if path == "target" || path.starts_with("target/") { return Err(s1_error("declared source or output is in root target")); }
            s1_validate_declared(repo, &path)?;
            result.push(path);
        }
        result.sort(); result.dedup(); Ok(result)
    };
    let rules = S1Classification { version: 1, source_inputs: normalize(source_inputs)?, outputs: normalize(outputs)?, tracked_at_freeze: s1_tracked(repo, deadline)?.into_iter().collect() };
    s1_inventory(repo, &rules, deadline)
}
/// Repeat a scan with exactly the frozen rules, never new worker exclusions.
fn s1_tracked(repo: &Path, deadline: std::time::Instant) -> Res<std::collections::BTreeSet<String>> {
    use std::collections::BTreeSet;
    let list = s1_git(repo, &["ls-files", "--cached", "-z", "--full-name"], deadline)?;
    if !list.is_empty() && list.last() != Some(&0) { return Err(s1_error("incomplete Git index listing")); }
    let mut tracked = BTreeSet::new();
    for raw in list.split(|b| *b == 0).filter(|x| !x.is_empty()) {
        let path = std::str::from_utf8(raw).map_err(|_| s1_error("unsupported index path"))?;
        let normalized = s1_path(path)?;
        if normalized != path { return Err(s1_error("noncanonical index path")); }
        tracked.insert(normalized);
    }
    Ok(tracked)
}
pub fn s1_inventory(repo: &Path, rules: &S1Classification, deadline: std::time::Instant) -> Res<S1Inventory> {
    use std::collections::BTreeSet;
    s1_time(deadline)?; s1_check_repo(repo)?;
    if rules.version != 1 { return Err(s1_error("unsupported classification version")); }
    for p in rules.source_inputs.iter().chain(&rules.outputs) {
        if s1_path(p)? != *p || p == "target" || p.starts_with("target/") { return Err(s1_error("invalid frozen classification")); }
        s1_validate_declared(repo, p)?;
    }
    let top = s1_git(repo, &["rev-parse", "--show-toplevel"], deadline)?;
    let top = std::str::from_utf8(&top).map_err(|_| s1_error("unsupported repository path"))?.trim_end_matches(['\r','\n']);
    if std::fs::canonicalize(top).map_err(io)? != repo { return Err(s1_error("not repository root")); }
    let mut tracked = s1_tracked(repo, deadline)?;
    for path in &rules.tracked_at_freeze {
        if s1_path(path)? != *path { return Err(s1_error("invalid frozen tracked path")); }
        tracked.insert(path.clone());
    }
    let mut entries = Vec::new();
    fn walk(repo: &Path, rel: &str, rules: &S1Classification, tracked: &BTreeSet<String>, out: &mut Vec<S1Entry>, deadline: std::time::Instant) -> Res<()> {
        s1_time(deadline)?;
        let dir = if rel.is_empty() { repo.to_path_buf() } else { repo.join(rel) };
        for item in std::fs::read_dir(dir).map_err(io)? {
            s1_time(deadline)?;
            let item = item.map_err(io)?;
            let name = item.file_name().into_string().map_err(|_| s1_error("unsupported filesystem path"))?;
            let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if path == ".git" { continue; }
            if s1_path(&path)? != path { return Err(s1_error("unsupported filesystem path")); }
            let m = std::fs::symlink_metadata(item.path()).map_err(io)?;
            let protected = tracked.contains(&path) || rules.source_inputs.iter().chain(&rules.outputs).any(|p| s1_under(&path, p));
            let has_tracked_children = tracked.iter().any(|p| s1_under(p, &path));
            if path == "target" && !has_tracked_children {
                if protected { return Err(s1_error("source/artifact mixing")); }
                if !m.is_dir() || s1_reparse(&m) { return Err(s1_error("root target is not an artifact directory")); }
                continue;
            }
            if s1_artifact(&path) && !protected && !has_tracked_children { continue; }
            if s1_artifact(&path) && !tracked.contains(&path) && protected && !rules.source_inputs.iter().chain(&rules.outputs).any(|p| p == &path) { return Err(s1_error("source/artifact mixing")); }
            if has_tracked_children && !m.is_dir() && !tracked.contains(&path) { return Err(s1_error("tracked path traverses non-directory or reparse point")); }
            if s1_reparse(&m) {
                if !m.file_type().is_symlink() { return Err(s1_error("unsupported reparse point")); }
                let target = std::fs::read_link(item.path()).map_err(io)?;
                #[cfg(unix)] let bytes = { use std::os::unix::ffi::OsStrExt; target.as_os_str().as_bytes().to_vec() };
                #[cfg(not(unix))] let bytes = target.to_str().ok_or_else(|| s1_error("unsupported symlink target"))?.as_bytes().to_vec();
                out.push(S1Entry { path, kind: "symlink".into(), mode: s1_mode(&m), sha256: format!("{:x}", Sha256::digest(bytes)) });
            } else if m.is_dir() {
                if tracked.contains(&path) { return Err(s1_error("tracked path is a directory")); }
                walk(repo, &path, rules, tracked, out, deadline)?;
            } else if m.is_file() {
                use std::io::Read;
                let mut file = std::fs::File::open(item.path()).map_err(io)?;
                let mut hash = Sha256::new();
                let mut buf = [0u8; 65536];
                loop {
                    s1_time(deadline)?;
                    let n = file.read(&mut buf).map_err(io)?;
                    if n == 0 { break; }
                    hash.update(&buf[..n]);
                }
                s1_time(deadline)?;
                out.push(S1Entry { path, kind: "file".into(), mode: s1_mode(&m), sha256: format!("{:x}", hash.finalize()) });
            } else { return Err(s1_error("unsupported source type")); }
        }
        Ok(())
    }
    walk(repo, "", rules, &tracked, &mut entries, deadline)?;
    entries.sort_by(|a,b| a.path.cmp(&b.path));
    let digest = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&(1u32, rules, &entries)).map_err(|e| s1_error(e.to_string()))?));
    s1_time(deadline)?;
    Ok(S1Inventory { version: 1, classification: rules.clone(), entries, digest })
}
/// Compare inventories only when they use identical frozen rules.
pub fn s1_compare(before: &S1Inventory, after: &S1Inventory) -> Res<Vec<S1Change>> {
    use std::collections::BTreeMap;
    if before.version != 1 || after.version != 1 || before.classification != after.classification { return Err(s1_error("inventory classifications differ")); }
    let a: BTreeMap<_,_> = before.entries.iter().map(|e| (&e.path, e)).collect();
    let b: BTreeMap<_,_> = after.entries.iter().map(|e| (&e.path, e)).collect();
    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).copied().collect();
    Ok(keys.into_iter().filter_map(|path| {
        let change = match (a.get(path), b.get(path)) { (None, Some(_)) => "added", (Some(_), None) => "deleted", (Some(x), Some(y)) if x != y => "changed", _ => return None };
        Some(S1Change { path: path.clone(), change: change.into(), in_scope: before.classification.outputs.iter().any(|p| s1_under(path, p)) })
    }).collect())
}
