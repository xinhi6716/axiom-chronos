use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const PACK_BYTES: usize = 80_000;
const PACK_CONTEXT: &str = "-U20";
const MAX_PATHS: usize = 200;
const MAX_FINDINGS: usize = 30;
const FINDING_CHARS: usize = 240;
const MAX_DECISIONS: usize = 16_000;
const MAX_COVERAGE: usize = 6_000;

fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn cut(text: &str, max: usize) -> (String, bool) {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    let more = chars.next().is_some();
    (head, more)
}
fn git(repo: &Path, args: &[&str]) -> Res<Vec<u8>> {
    let out = base::proc::cmd("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| Failure::new("process", e.to_string()))?;
    if !out.status.success() {
        return Err(Failure::new(
            "git_failed",
            format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(out.stdout)
}
fn tree_id(commit: &str) -> &str {
    commit.strip_prefix("tree:").unwrap_or(commit)
}
fn changed_files(repo: &Path, base: &str, tree: &str) -> Res<Vec<(String, u64, u64)>> {
    let raw = git(
        repo,
        &[
            "-c",
            "core.quotepath=off",
            "diff-tree",
            "-r",
            "--no-renames",
            "--numstat",
            "-z",
            tree_id(base),
            tree_id(tree),
        ],
    )?;
    let mut files = Vec::new();
    for entry in raw.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let line = String::from_utf8_lossy(entry);
        let mut parts = line.splitn(3, '\t');
        let added = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let removed = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let Some(path) = parts.next() else { continue };
        files.push((path.to_string(), added, removed));
    }
    Ok(files)
}
fn file_diff(repo: &Path, base: &str, tree: &str, path: &str) -> Res<String> {
    let raw = git(
        repo,
        &[
            "--literal-pathspecs",
            "-c",
            "core.quotepath=off",
            "diff-tree",
            "-r",
            "--no-renames",
            "--no-ext-diff",
            "--no-color",
            "-p",
            PACK_CONTEXT,
            tree_id(base),
            tree_id(tree),
            "--",
            path,
        ],
    )?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

pub fn pack_path(store: &Path, unit: &str) -> PathBuf {
    store.join("reports").join(format!("{unit}.review.md"))
}
#[derive(Debug)]
pub struct PackInfo {
    pub path: PathBuf,
    pub tree: String,
    pub files: usize,
    pub omitted: Vec<String>,
    pub bytes: usize,
}
fn module_of(store: &Path, track: &str) -> Res<Option<Module>> {
    let path = store.join("modules.md");
    if !path.exists() {
        return Ok(None);
    }
    let map = parse_map(&std::fs::read_to_string(path).map_err(io)?)
        .map_err(|e| Failure::new("invalid_argument", e))?;
    Ok(map.modules.into_iter().find(|m| m.id == track))
}
/// Write the unit's review pack: its contract, then the diff of every file the worker changed.
pub fn review_pack(store: &Path, unit: &str, repo: &Path) -> Res<PackInfo> {
    let row = read_units(store)?
        .into_iter()
        .find(|u| u.id == unit)
        .ok_or_else(|| Failure::new("not_found", format!("unit {unit} not found")))?;
    let brief = unit_brief(store, unit)?;
    let rows = evidence(store, unit)?;
    let tree = rows
        .iter()
        .rev()
        .find(|r| r.kind == "verify")
        .filter(|r| r.exit_code == 0 && r.commit.starts_with("tree:"))
        .map(|r| r.commit.clone())
        .ok_or_else(|| {
            Failure::new(
                "not_built",
                format!("unit {unit} has no passing VERIFY to review"),
            )
        })?;
    let mut head = format!("# Review pack for unit {unit}\n\nTree reviewed: {tree}\n");
    if let Some(module) = module_of(store, &row.track)? {
        head.push_str(&format!("\n## Module contract\n\n{}\n", module.contract));
    }
    head.push_str(&format!("\n## Brief\n\n{}\n", brief.text.trim_end()));
    let mut listing = String::from("\n## Changed files\n\n");
    let mut diffs = String::new();
    let mut omitted = Vec::new();
    let mut file_count = 0;
    let base = rows
        .iter()
        .find(|r| r.kind == "run" && r.commit.starts_with("tree:"))
        .map(|r| r.commit.as_str());
    let changed: BTreeSet<String> = match base {
        Some(base) => {
            let mut files = changed_files(repo, base, &tree)?;
            files.sort_by_key(|(path, added, removed)| (added + removed, path.clone()));
            file_count = files.len();
            if files.is_empty() {
                listing.push_str("The worker changed no file in the unit's SCOPE.\n");
            }
            for (path, added, removed) in &files {
                let diff = file_diff(repo, base, &tree, path)?;
                let fits = head.len() + diffs.len() + diff.len() <= PACK_BYTES;
                listing.push_str(&format!(
                    "- {path}  +{added} -{removed}{}\n",
                    if fits {
                        ""
                    } else {
                        "  (read this file directly)"
                    }
                ));
                if fits {
                    diffs.push_str(&diff);
                } else {
                    omitted.push(path.clone());
                }
            }
            files.into_iter().map(|(path, _, _)| path).collect()
        }
        None => {
            listing.push_str(&format!(
                "No base snapshot was recorded for this unit: review every file in SCOPE directly: {}\n",
                brief.scope.join(", ")
            ));
            BTreeSet::new()
        }
    };
    let report = store.join("reports").join(format!("{unit}.json"));
    let reported: BTreeSet<String> = match std::fs::read_to_string(&report) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map(|v| report_paths(&v, repo).into_iter().collect())
            .unwrap_or_default(),
        Err(_) => BTreeSet::new(),
    };
    let outside: Vec<_> = reported
        .difference(&changed)
        .filter(|path| base.is_some() || !brief.scope.iter().any(|s| s == *path))
        .cloned()
        .collect();
    if !outside.is_empty() {
        listing.push_str(&format!(
            "\nThe worker also reports these, which git does not show as changed (ignored, or unchanged at the end). Read any that exist: {}\n",
            outside.join(", ")
        ));
    }
    let text = format!(
        "{head}{listing}\n## Diffs\n\nEvery changed file follows with 20 lines of surrounding code; a new file appears in full.\n\n{diffs}"
    );
    let path = pack_path(store, unit);
    base::atomic_write(&path, text.as_bytes()).map_err(io)?;
    Ok(PackInfo {
        path,
        tree,
        files: file_count,
        omitted,
        bytes: text.len(),
    })
}

fn label(state: &str) -> &str {
    if state == "built" {
        "unreviewed"
    } else {
        state
    }
}
fn unit_paths(store: &Path, unit: &str, repo: &Path) -> Res<Vec<String>> {
    match std::fs::read_to_string(store.join("reports").join(format!("{unit}.json"))) {
        Ok(text) => Ok(serde_json::from_str::<Value>(&text)
            .map(|v| report_paths(&v, repo))
            .unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(io(e)),
    }
}
/// A bounded digest for the core or a dispatcher, in place of reading `reports/`.
pub fn digest(store: &Path, repo: &Path, module: Option<&str>) -> Res<String> {
    let all = read_units(store)?;
    let units: Vec<&Unit> = all
        .iter()
        .filter(|u| module.is_none_or(|m| u.track == m))
        .collect();
    if module.is_some() && units.is_empty() {
        return Err(Failure::new(
            "not_found",
            format!("module {} has no units", module.unwrap_or("")),
        ));
    }
    let mut out = String::new();
    let mut counts = BTreeMap::<&str, usize>::new();
    for u in &units {
        *counts.entry(label(&u.state)).or_default() += 1;
    }
    let summary: Vec<_> = counts.iter().map(|(k, v)| format!("{k} {v}")).collect();
    out.push_str(&format!(
        "units: {} ({})\n",
        units.len(),
        summary.join(", ")
    ));
    let unverified: Vec<_> = units.iter().filter(|u| u.state != "verified").collect();
    if !unverified.is_empty() {
        out.push_str("not verified:\n");
        for u in &unverified {
            out.push_str(&format!("  {}  {}\n", u.id, label(&u.state)));
        }
    }
    if store.join("modules.md").exists() {
        let map = parse_map(&std::fs::read_to_string(store.join("modules.md")).map_err(io)?)
            .map_err(|e| Failure::new("invalid_argument", e))?;
        out.push_str("acceptance:\n");
        for m in map
            .modules
            .iter()
            .filter(|m| module.is_none_or(|x| x == m.id))
        {
            let mut latest: Option<Evidence> = None;
            for u in all.iter().filter(|u| u.track == m.id) {
                for r in evidence(store, &u.id)? {
                    if r.kind == "accept" && latest.as_ref().map_or(true, |l| r.ts >= l.ts) {
                        latest = Some(r);
                    }
                }
            }
            let result = match latest {
                Some(r) if r.exit_code == 0 => "pass".to_string(),
                Some(r) => format!("fail (exit {})", r.exit_code),
                None => "not run".to_string(),
            };
            out.push_str(&format!("  {}  {result}\n", m.id));
        }
    }
    if module.is_none() {
        let full = full_evidence(store)?
            .into_iter()
            .filter(|r| r.kind == "full")
            .last();
        out.push_str(&match full {
            Some(r) => format!(
                "full test: {} {}\n",
                if r.exit_code == 0 {
                    "pass".to_string()
                } else {
                    format!("fail (exit {})", r.exit_code)
                },
                r.command
            ),
            None => "full test: not run\n".to_string(),
        });
        let mut sol = Vec::new();
        for u in &all {
            if evidence(store, &u.id)?
                .iter()
                .any(|r| r.kind == "run" && crate::codex::role_matches("sol", &r.command))
            {
                sol.push(u.id.clone());
            }
        }
        out.push_str(&if sol.is_empty() {
            "sol: did not run\n".to_string()
        } else {
            format!("sol: ran ({})\n", sol.join(", "))
        });
    }
    let mut changed = BTreeSet::new();
    for u in &units {
        changed.extend(unit_paths(store, &u.id, repo)?);
    }
    out.push_str(&format!("changed files ({}):\n", changed.len()));
    for path in changed.iter().take(MAX_PATHS) {
        out.push_str(&format!("  {path}\n"));
    }
    if changed.len() > MAX_PATHS {
        out.push_str(&format!(
            "  ... {} more in reports/<unit>.json file_changes\n",
            changed.len() - MAX_PATHS
        ));
    }
    let mut minors = 0;
    let mut minor_units = 0;
    let mut open = Vec::new();
    for u in &units {
        let Ok(text) =
            std::fs::read_to_string(store.join("reports").join(format!("{}.verdict.json", u.id)))
        else {
            continue;
        };
        let verdict: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let mut minor_here = 0;
        for f in verdict["findings"].as_array().into_iter().flatten() {
            let severity = f["severity"].as_str().unwrap_or("");
            if severity.eq_ignore_ascii_case("blocker") || severity.eq_ignore_ascii_case("major") {
                if u.state != "verified" {
                    let (evidence, more) = cut(f["evidence"].as_str().unwrap_or(""), FINDING_CHARS);
                    open.push(format!(
                        "  {}  {}  {}  {}: {evidence}{}\n",
                        u.id,
                        severity,
                        f["check"].as_str().unwrap_or(""),
                        f["location"].as_str().unwrap_or(""),
                        if more { "…" } else { "" }
                    ));
                }
            } else {
                minor_here += 1;
            }
        }
        minors += minor_here;
        minor_units += usize::from(minor_here > 0);
    }
    out.push_str(&format!(
        "open findings ({}; {minors} minor findings across {minor_units} units are in reports/<unit>.verdict.json):\n",
        open.len()
    ));
    for line in open.iter().take(MAX_FINDINGS) {
        out.push_str(line);
    }
    if open.len() > MAX_FINDINGS {
        out.push_str(&format!(
            "  ... {} more in reports/<unit>.verdict.json\n",
            open.len() - MAX_FINDINGS
        ));
    }
    if module.is_none() {
        let path = store.join("decisions.md");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (head, more) = cut(text.trim(), MAX_DECISIONS);
                out.push_str(&format!("decisions:\n{head}\n"));
                if more {
                    out.push_str(&format!(
                        "... cut at {MAX_DECISIONS} characters; the rest is in {}\n",
                        path.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push_str("decisions: none recorded\n");
            }
            Err(e) => return Err(io(e)),
        }
        if let Ok(plan) = std::fs::read_to_string(store.join("astra").join("plan-final.md")) {
            let coverage = plan.split("\n## ").find(|s| s.starts_with("Coverage"));
            if let Some(section) = coverage {
                let (head, more) = cut(section.trim_end(), MAX_COVERAGE);
                out.push_str(&format!(
                    "astra coverage (astra/plan-final.md):\n## {head}\n"
                ));
                if more {
                    out.push_str("... cut; the rest is in astra/plan-final.md\n");
                }
            }
        }
        if let Some(note) = crate::council::stage_note_at(&store.join("astra")) {
            out.push_str(&format!("council: {note}\n"));
        }
    }
    Ok(out)
}
