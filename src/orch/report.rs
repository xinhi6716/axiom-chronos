use super::*;
use crate::base::{self, Failure, Res};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const PACK_BYTES: usize = 80_000;
const PACK_CONTEXT: &str = "-U8";
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
pub(crate) fn git(repo: &Path, args: &[&str]) -> Res<Vec<u8>> {
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
pub(crate) fn project_prefix(repo: &Path) -> Res<String> {
    let raw = git(repo, &["rev-parse", "--show-prefix"])?;
    Ok(String::from_utf8_lossy(&raw)
        .trim_end_matches(['\r', '\n'])
        .replace('\\', "/"))
}
fn changed_files(repo: &Path, base: &str, tree: &str) -> Res<Vec<(String, u64, u64)>> {
    let prefix = project_prefix(repo)?;
    let relative = format!("--relative={prefix}");
    let mut args = vec![
        "-c",
        "core.quotepath=off",
        "diff-tree",
        "-r",
        "--no-renames",
        "--numstat",
        "-z",
    ];
    if !prefix.is_empty() {
        args.push(&relative);
    }
    args.extend([tree_id(base), tree_id(tree)]);
    let raw = git(repo, &args)?;
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
    let prefix = project_prefix(repo)?;
    let relative = format!("--relative={prefix}");
    let mut args = vec![
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
    ];
    if !prefix.is_empty() {
        args.push(&relative);
    }
    args.extend([tree_id(base), tree_id(tree), "--", path]);
    let raw = git(repo, &args)?;
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

/// List each changed file, smallest change first, and append its diff while the whole pack
/// stays within `budget` bytes; the rest are named for a direct read. Returns those omitted.
pub(crate) fn pack_diffs(
    budget: usize,
    head_len: usize,
    files: &mut [(String, u64, u64)],
    listing: &mut String,
    diffs: &mut String,
    diff_of: impl FnMut(&str) -> Res<String>,
) -> Res<Vec<String>> {
    pack_diffs_noting(budget, head_len, files, listing, diffs, diff_of, |_| {
        Ok("read this file directly".into())
    })
}
/// `pack_diffs`, with `omitted_note` saying where the reviewer finds each file left out.
fn pack_diffs_noting(
    budget: usize,
    head_len: usize,
    files: &mut [(String, u64, u64)],
    listing: &mut String,
    diffs: &mut String,
    mut diff_of: impl FnMut(&str) -> Res<String>,
    mut omitted_note: impl FnMut(&str) -> Res<String>,
) -> Res<Vec<String>> {
    files.sort_by_key(|(path, added, removed)| (added + removed, path.clone()));
    let mut omitted = Vec::new();
    for (path, added, removed) in files.iter() {
        let diff = diff_of(path)?;
        let fits = head_len + diffs.len() + diff.len() <= budget;
        listing.push_str(&format!("- {path}  +{added} -{removed}"));
        if fits {
            listing.push('\n');
            diffs.push_str(&diff);
        } else {
            listing.push_str(&format!("  ({})\n", omitted_note(path)?));
            omitted.push(path.clone());
        }
    }
    Ok(omitted)
}
/// Where a file deleted between `base` and `tree` keeps its last version, written on first use.
fn deleted_copy(
    repo: &Path,
    dir: &Path,
    base: &str,
    tree: &str,
    path: &str,
) -> Res<Option<PathBuf>> {
    let present = git(
        repo,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "--name-only",
            "-z",
            tree_id(tree),
            "--",
            path,
        ],
    )?;
    if !present.is_empty() {
        return Ok(None);
    }
    let prefix = project_prefix(repo)?;
    let text = git(
        repo,
        &[
            "cat-file",
            "blob",
            &format!("{}:{prefix}{path}", tree_id(base)),
        ],
    )?;
    let mut out = dir.to_path_buf();
    out.extend(path.split('/'));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    base::atomic_write(&out, &text).map_err(io)?;
    Ok(Some(out))
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
fn severity(f: &findings::Finding) -> &'static str {
    match f.severity {
        findings::Severity::Blocker => "blocker",
        findings::Severity::Major => "major",
        findings::Severity::Minor => "minor",
    }
}
fn pack_findings(store: &Path, unit: &str, rows: &[Evidence], head: &mut String) -> Res<()> {
    let waived = findings::waivers(rows);
    if let Some(verdict) = findings::read_verdict(store, unit)? {
        for f in verdict
            .findings
            .iter()
            .filter(|f| f.is_serious() && !waived.iter().any(|w| w.number == f.number))
        {
            head.push_str(&format!(
                "{}. {} {} {}: {}\n",
                f.number,
                severity(f),
                f.check,
                f.location,
                f.evidence
            ));
        }
    }
    Ok(())
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
    let earlier = rows
        .iter()
        .rev()
        .find(|r| r.kind == "review")
        .filter(|r| r.commit.starts_with("tree:") && r.commit != tree);
    if let Some(review) = earlier {
        head.push_str(&format!("Earlier review at: {}\n", review.commit));
    }
    let fixed = fixed_unit(store, unit)?;
    if let Some(old) = &fixed {
        head.push_str(&format!("Fixes unit: {old}\n"));
    }
    if let Some(module) = module_of(store, &row.track)? {
        head.push_str(&format!("\n## Module contract\n\n{}\n", module.contract));
    }
    head.push_str(&format!("\n## Brief\n\n{}\n", brief.text.trim_end()));
    if earlier.is_some() {
        head.push_str("\n## Earlier findings\n\n");
        pack_findings(store, unit, &rows, &mut head)?;
    }
    if let Some(old) = &fixed {
        head.push_str("\n## Findings to fix\n\n");
        pack_findings(store, old, &evidence(store, old)?, &mut head)?;
        head.push_str("\nThe review checks these findings and the changed lines only.\n");
    }
    let mut listing = String::from("\n## Changed files\n\n");
    let mut diffs = String::new();
    let mut omitted = Vec::new();
    let mut file_count = 0;
    let base = earlier.map(|r| r.commit.as_str()).or_else(|| {
        rows.iter()
            .find(|r| r.kind == "run" && r.commit.starts_with("tree:"))
            .map(|r| r.commit.as_str())
    });
    let deleted_dir = store.join("reports").join(format!("{unit}.deleted"));
    match std::fs::remove_dir_all(&deleted_dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io(e)),
        _ => {}
    }
    let changed: BTreeSet<String> = match base {
        Some(base) => {
            let mut files = changed_files(repo, base, &tree)?;
            file_count = files.len();
            if files.is_empty() {
                listing.push_str(
                    "The worker changed no file in the unit's SCOPE.
",
                );
            }
            omitted = pack_diffs_noting(
                PACK_BYTES,
                head.len(),
                &mut files,
                &mut listing,
                &mut diffs,
                |path| file_diff(repo, base, &tree, path),
                |path| {
                    Ok(match deleted_copy(repo, &deleted_dir, base, &tree, path)? {
                        Some(copy) => {
                            format!("deleted; its last version is at {}", copy.display())
                        }
                        None => "read this file directly".into(),
                    })
                },
            )?;
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
    if earlier.is_none() && !outside.is_empty() {
        listing.push_str(&format!(
            "\nThe worker also reports these, which git does not show as changed (ignored, or unchanged at the end). Read any that exist: {}\n",
            outside.join(", ")
        ));
    }
    let text = format!(
        "{head}{listing}\n## Diffs\n\nEvery changed file follows with 8 lines of surrounding code; a new file appears in full.\n\n{diffs}"
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
    if state.starts_with("superseded") {
        "superseded"
    } else if state == "built" {
        "unreviewed"
    } else {
        state
    }
}
/// The files a unit changed: git's difference between the SCOPE snapshot taken before its first
/// run and its latest snapshot, merged with the worker's `file_changes`, which also names files
/// git cannot show. When git cannot compare the snapshots, `notes` says so.
fn unit_paths(
    store: &Path,
    unit: &str,
    repo: &Path,
    notes: &mut Vec<String>,
) -> Res<BTreeSet<String>> {
    let mut paths: BTreeSet<String> =
        match std::fs::read_to_string(store.join("reports").join(format!("{unit}.json"))) {
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map(|v| report_paths(&v, repo).into_iter().collect())
                .unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(io(e)),
        };
    let rows = evidence(store, unit)?;
    let tree = |r: &&Evidence| r.commit.starts_with("tree:");
    let base = rows.iter().filter(|r| r.kind == "run").find(tree);
    if let (Some(base), Some(last)) = (base, rows.iter().rev().find(tree)) {
        match changed_files(repo, &base.commit, &last.commit) {
            Ok(files) => paths.extend(files.into_iter().map(|(path, _, _)| path)),
            Err(e) => notes.push(format!(
                "  {unit}: git cannot compare its snapshots ({}); its files come from the worker's report only\n",
                e.message.lines().next().unwrap_or("").trim()
            )),
        }
    }
    Ok(paths)
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
    let unverified: Vec<_> = units
        .iter()
        .filter(|u| u.state != "verified" && !u.state.starts_with("superseded"))
        .collect();
    if !unverified.is_empty() {
        out.push_str("not verified:\n");
        for u in &unverified {
            out.push_str(&format!("  {}  {}\n", u.id, label(&u.state)));
        }
    }
    let superseded = units
        .iter()
        .filter(|u| u.state.starts_with("superseded"))
        .count();
    if superseded > 0 {
        out.push_str(&format!("superseded units: {superseded}\n"));
    }
    for h in pending_handoffs(store)?
        .iter()
        .filter(|h| module.is_none() || h.module.as_deref() == module)
    {
        out.push_str(&format!(
            "handoff pending ({}): next: {}\n",
            h.module.as_deref().unwrap_or("whole wish"),
            h.next
        ));
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
    let mut notes = Vec::new();
    for u in &units {
        changed.extend(unit_paths(store, &u.id, repo, &mut notes)?);
    }
    out.push_str(&format!("changed files ({}):\n", changed.len()));
    for path in changed.iter().take(MAX_PATHS) {
        out.push_str(&format!("  {path}\n"));
    }
    for note in &notes {
        out.push_str(note);
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
    let mut answered = 0;
    let mut needs = Vec::new();
    let mut ruled = Vec::new();
    for u in &units {
        let Ok(Some(verdict)) = findings::read_verdict(store, &u.id) else {
            continue;
        };
        let waivers = findings::waivers(&evidence(store, &u.id)?);
        let mut minor_here = 0;
        for f in &verdict.findings {
            match findings::classify(&all, u, f, &waivers) {
                findings::Class::Open => {
                    let (evidence, more) = cut(&f.evidence, FINDING_CHARS);
                    open.push(format!(
                        "  {}  {}  {}  {}: {evidence}{}\n",
                        u.id,
                        severity(f),
                        f.check,
                        f.location,
                        if more { "…" } else { "" }
                    ));
                }
                findings::Class::Answered => answered += 1,
                findings::Class::NeedsCheck => {
                    let (evidence, more) = cut(&f.evidence, FINDING_CHARS);
                    let kind = match f.needs {
                        Some(findings::Needs::Smoke) => "smoke",
                        Some(findings::Needs::Manual) => "manual",
                        None => unreachable!(),
                    };
                    needs.push(format!(
                        "  {}  {}  {}  {}: {evidence}{}\n",
                        u.id,
                        severity(f),
                        kind,
                        f.location,
                        if more { "…" } else { "" }
                    ));
                }
                findings::Class::Ruled(reason) => ruled.push(format!(
                    "  {}  #{}  {}  {}: {}\n",
                    u.id,
                    f.number,
                    severity(f),
                    f.location,
                    reason
                )),
                findings::Class::Minor => minor_here += 1,
                findings::Class::Resolved => {}
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
    if answered > 0 {
        out.push_str(&format!("answered by later units: {answered}\n"));
    }
    if !needs.is_empty() {
        out.push_str(&format!(
            "needs a smoke or manual check ({}):\n",
            needs.len()
        ));
        for line in needs {
            out.push_str(&line);
        }
    }
    if !ruled.is_empty() {
        out.push_str(&format!("ruled ({}):\n", ruled.len()));
        for line in ruled {
            out.push_str(&line);
        }
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
        let plan = match std::fs::read_to_string(store.join("astra").join("plan-final.md")) {
            Ok(plan) => Some(plan),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(Failure::new(
                    "io",
                    format!("cannot read astra/plan-final.md: {e}"),
                ))
            }
        };
        if let Some(plan) = plan {
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
