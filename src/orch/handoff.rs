//! A dispatcher's handoff: a short note that lets a fresh dispatcher carry on a module.
//!
//! Units, claims and evidence already live in the store; the note adds only what they cannot
//! show: what was in flight, why something is stuck, what was decided and the next step.

use super::*;
use crate::base::{self, Failure, Res};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Each field's limit in characters; a longer value is refused, never cut.
pub const HANDOFF_FIELDS: [(&str, usize); 4] = [
    ("--next", 400),
    ("--in-flight", 600),
    ("--stuck", 600),
    ("--decisions", 800),
];
/// Upper bound of the resume text: the handoff, then the digest cut to fit.
pub const MAX_RESUME: usize = 12_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Handoff {
    pub module: Option<String>,
    pub ts: String,
    #[serde(default)]
    pub repo: Option<String>,
    pub next: String,
    #[serde(default)]
    pub in_flight: String,
    #[serde(default)]
    pub stuck: String,
    #[serde(default)]
    pub decisions: String,
    /// Set once a fresh dispatcher was started with this note.
    #[serde(default)]
    pub taken: bool,
}

fn io(e: std::io::Error) -> Failure {
    Failure::new("io", e.to_string())
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::new("invalid_argument", message)
}
fn path(store: &Path, module: Option<&str>) -> PathBuf {
    let dir = store.join("handoffs");
    match module {
        Some(m) => dir.join("module").join(format!("{m}.json")),
        None => dir.join("wish.json"),
    }
}
fn map_modules(store: &Path) -> Res<Option<Vec<String>>> {
    let file = store.join("modules.md");
    if !file.exists() {
        return Ok(None);
    }
    let map = parse_map(&std::fs::read_to_string(file).map_err(io)?).map_err(invalid)?;
    Ok(Some(map.modules.into_iter().map(|m| m.id).collect()))
}
fn check_module(store: &Path, module: Option<&str>) -> Res<()> {
    let Some(m) = module else { return Ok(()) };
    base::require_slug("module", m)?;
    match map_modules(store)? {
        Some(ids) if !ids.iter().any(|id| id == m) => Err(invalid(format!(
            "module {m} is not in the module map; modules: {}",
            ids.join(", ")
        ))),
        _ => Ok(()),
    }
}

/// `orch handoff [--module <m>] --next <text> [--in-flight <text>] [--stuck <text>] [--decisions <text>] [--repo <dir>]`
pub fn record_handoff(store: &Path, args: &[String]) -> Res<Handoff> {
    let mut module = None;
    let mut repo = None;
    let mut fields: Vec<(&str, String)> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args
            .get(i + 1)
            .filter(|v| !v.starts_with("--"))
            .ok_or_else(|| invalid(format!("{flag} requires a value")))?
            .clone();
        if fields.iter().any(|(f, _)| *f == flag)
            || (flag == "--module" && module.is_some())
            || (flag == "--repo" && repo.is_some())
        {
            return Err(invalid(format!("{flag} is given twice")));
        }
        match flag {
            "--module" => module = Some(value),
            "--repo" => repo = Some(value),
            _ => {
                let Some((name, limit)) = HANDOFF_FIELDS.iter().find(|(f, _)| *f == flag) else {
                    return Err(invalid(format!(
                        "unknown handoff option {flag}; options: --module, --repo, --next, --in-flight, --stuck, --decisions"
                    )));
                };
                let value = value.trim().to_string();
                let length = value.chars().count();
                if length > *limit {
                    return Err(invalid(format!(
                        "{name} is {length} characters; the limit is {limit}. Shorten it: unit states and reports are already in the store."
                    )));
                }
                fields.push((name, value));
            }
        }
        i += 2;
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(f, _)| *f == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let next = field("--next");
    if next.is_empty() {
        return Err(invalid(
            "--next is required: the step the fresh dispatcher takes first",
        ));
    }
    check_module(store, module.as_deref())?;
    let note = Handoff {
        module,
        ts: base::now_iso(),
        repo,
        next,
        in_flight: field("--in-flight"),
        stuck: field("--stuck"),
        decisions: field("--decisions"),
        taken: false,
    };
    let file = path(store, note.module.as_deref());
    with_lock(store, false, || {
        std::fs::create_dir_all(file.parent().unwrap()).map_err(io)?;
        base::atomic_write(&file, &serde_json::to_vec_pretty(&note).unwrap()).map_err(io)
    })?;
    Ok(note)
}

/// The latest handoff of a module, or of the whole wish when `module` is None.
pub fn read_handoff(store: &Path, module: Option<&str>) -> Res<Option<Handoff>> {
    if let Some(m) = module {
        base::require_slug("module", m)?;
    }
    match std::fs::read(path(store, module)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            invalid(format!(
                "handoff of {} is unreadable: {e}",
                module.unwrap_or("the wish")
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}

/// Handoffs no fresh dispatcher has taken yet, the whole wish's first.
pub fn pending_handoffs(store: &Path) -> Res<Vec<Handoff>> {
    let mut out: Vec<Handoff> = read_handoff(store, None)?.into_iter().collect();
    let dir = store.join("handoffs").join("module");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(keep_pending(out)),
        Err(e) => return Err(io(e)),
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_owned)
        })
        .filter(|m| base::require_slug("module", m).is_ok())
        .collect();
    names.sort();
    for m in names {
        out.extend(read_handoff(store, Some(&m))?);
    }
    Ok(keep_pending(out))
}
fn keep_pending(all: Vec<Handoff>) -> Vec<Handoff> {
    all.into_iter().filter(|h| !h.taken).collect()
}

/// Marks a handoff as taken by a fresh dispatcher.
pub fn take_handoff(store: &Path, module: Option<&str>) -> Res<()> {
    with_lock(store, false, || {
        let Some(mut note) = read_handoff(store, module)? else {
            return Ok(());
        };
        note.taken = true;
        base::atomic_write(
            &path(store, module),
            &serde_json::to_vec_pretty(&note).unwrap(),
        )
        .map_err(io)
    })
}

pub fn handoff_text(h: &Handoff) -> String {
    let mut text = format!(
        "HANDOFF {} at {}{}\nnext: {}\n",
        h.module
            .as_deref()
            .map_or("for the whole wish".to_string(), |m| format!(
                "for module {m}"
            )),
        h.ts,
        if h.taken { "" } else { " (pending)" },
        h.next
    );
    for (label, value) in [
        ("in flight", &h.in_flight),
        ("stuck", &h.stuck),
        ("decisions", &h.decisions),
    ] {
        if !value.is_empty() {
            text.push_str(&format!("{label}: {value}\n"));
        }
    }
    text
}

/// The latest handoff (all pending ones when `module` is None) and the digest, within MAX_RESUME.
pub fn resume_text(store: &Path, repo: Option<&Path>, module: Option<&str>) -> Res<String> {
    let notes: Vec<Handoff> = match module {
        Some(m) => read_handoff(store, Some(m))?.into_iter().collect(),
        None => {
            let mut notes: Vec<Handoff> = read_handoff(store, None)?.into_iter().collect();
            notes.extend(
                pending_handoffs(store)?
                    .into_iter()
                    .filter(|h| h.module.is_some()),
            );
            notes
        }
    };
    let mut out = String::new();
    if notes.is_empty() {
        out.push_str("HANDOFF none recorded\n");
    }
    for note in &notes {
        out.push_str(&handoff_text(note));
    }
    let repo = repo.map(Path::to_path_buf).or_else(|| {
        notes
            .iter()
            .find_map(|h| h.repo.as_ref().map(PathBuf::from))
    });
    let digest = match repo {
        None => "unavailable: pass --repo <project directory>\n".to_string(),
        Some(repo) => match digest(store, &repo, module) {
            Ok(text) => text,
            Err(e) if e.kind == "not_found" => format!("{}\n", e.message),
            Err(e) => return Err(e),
        },
    };
    out.push_str("DIGEST\n");
    let room = MAX_RESUME.saturating_sub(out.chars().count());
    let mut chars = digest.chars();
    let head: String = chars.by_ref().take(room).collect();
    out.push_str(&head);
    if chars.next().is_some() {
        let marker = "\n... cut; run orch report for the rest\n";
        let keep = MAX_RESUME.saturating_sub(marker.len());
        out = out.chars().take(keep).collect();
        out.push_str(marker);
    }
    Ok(out)
}
